mod com_ids;
mod device_proxy;
mod di8_proxy;
mod iat_hook;
mod key_shm;
mod log;
mod login_input;
pub mod shm;

use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HMODULE, TRUE};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GetProcAddress, LoadLibraryW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_PIN,
};

/// Signature of the real DirectInput8Create function.
type DirectInput8CreateFn = unsafe extern "system" fn(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: *mut c_void,
) -> HRESULT;

/// The real DirectInput8Create resolved from System32\dinput8.dll.
static REAL_DI8_CREATE: OnceLock<DirectInput8CreateFn> = OnceLock::new();
static RUNTIME_INITIALIZED: OnceLock<Result<(), HRESULT>> = OnceLock::new();

const E_FAIL: HRESULT = HRESULT(0x8000_4005u32 as i32);

/// Keeps the integration test linked to this crate so Cargo builds the current
/// cdylib before the child loads `dinput8.dll` from the target directory.
#[doc(hidden)]
pub const fn integration_test_marker() -> u32 {
    trusik_protocol::VERSION
}

/// Loader entry point. All substantive initialization is deferred until the
/// exported factory is called, after the loader lock has been released.
#[unsafe(no_mangle)]
extern "system" fn DllMain(_hinst: HINSTANCE, _reason: u32, _reserved: *mut c_void) -> BOOL {
    TRUE
}

fn initialize_runtime() -> Result<(), HRESULT> {
    log::init();
    log::write("runtime: initializing outside DllMain");

    // Hooks and polling workers live for the process lifetime. Pin this module
    // before publishing any callback address so FreeLibrary cannot invalidate it.
    let mut this_module = HMODULE(std::ptr::null_mut());
    let module_address = initialize_runtime as *const () as *const u16;
    if unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_PIN | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(module_address),
            &mut this_module,
        )
    }
    .is_err()
    {
        log::write("runtime: failed to pin proxy module");
        return Err(E_FAIL);
    }

    // A basename lookup can return this already-loaded proxy despite search
    // flags. Build an absolute path from the actual Windows system directory.
    let mut system_path = [0u16; 32_768];
    let system_length = unsafe {
        windows::Win32::System::SystemInformation::GetSystemDirectoryW(Some(&mut system_path))
    } as usize;
    let suffix: Vec<u16> = "\\dinput8.dll\0".encode_utf16().collect();
    if system_length == 0 || system_length + suffix.len() > system_path.len() {
        log::write("runtime: failed to resolve the Windows system directory");
        return Err(E_FAIL);
    }
    system_path[system_length..system_length + suffix.len()].copy_from_slice(&suffix);
    let real_dll = unsafe { LoadLibraryW(PCWSTR(system_path.as_ptr())) }.map_err(|error| {
        log::write(&format!(
            "runtime: failed to load real dinput8.dll: {error}"
        ));
        E_FAIL
    })?;
    let proc = unsafe { GetProcAddress(real_dll, windows::core::s!("DirectInput8Create")) }
        .ok_or_else(|| {
            log::write("runtime: failed to resolve DirectInput8Create");
            E_FAIL
        })?;
    let function: DirectInput8CreateFn = unsafe { std::mem::transmute(proc) };
    REAL_DI8_CREATE.set(function).map_err(|_| E_FAIL)?;

    shm::create();
    unsafe {
        iat_hook::install();
        iat_hook::install_keyboard_hooks();
        // Open and acknowledge a controller-created input mapping before the
        // ready event can wake auto-type.
        let _ = key_shm::is_compatible();
    }
    login_input::create_event();

    log::write("runtime: ready");
    Ok(())
}

fn ensure_runtime() -> Result<DirectInput8CreateFn, HRESULT> {
    match RUNTIME_INITIALIZED.get_or_init(initialize_runtime) {
        Ok(()) => REAL_DI8_CREATE.get().copied().ok_or(E_FAIL),
        Err(error) => Err(*error),
    }
}

/// The exported DirectInput8Create that EQ will call.
///
/// We call the real function, then wrap the returned IDirectInput8 interface
/// in our proxy so we can intercept CreateDevice calls.
///
/// # Safety
///
/// The caller must provide the valid pointers and interface identifier required
/// by the Windows `DirectInput8Create` ABI.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DirectInput8Create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: *mut c_void,
) -> HRESULT {
    let real_create = match ensure_runtime() {
        Ok(function) => function,
        Err(error) => return error,
    };

    log::write("DirectInput8Create called");
    let hr = unsafe { real_create(hinst, dwversion, riidltf, ppvout, punkouter) };
    if hr.is_err() {
        log::write(&format!(
            "DirectInput8Create: real call failed (0x{:08X})",
            hr.0
        ));
        return hr;
    }

    // Adopt and wrap only the exact IDirectInput8 A/W interfaces implemented
    // by the proxy. Aggregated or secondary interfaces must remain untouched.
    if !ppvout.is_null() {
        let requested_iid = (!riidltf.is_null()).then(|| unsafe { *riidltf });
        let real_di8 = unsafe { *ppvout };
        if punkouter.is_null()
            && !real_di8.is_null()
            && requested_iid.is_some_and(com_ids::is_direct_input_8)
        {
            let proxy = di8_proxy::DI8Proxy::from_owned(real_di8, requested_iid.unwrap());
            let proxy_ptr = Box::into_raw(Box::new(proxy));
            unsafe { *ppvout = proxy_ptr as *mut c_void };
            log::write("DirectInput8Create: wrapped in proxy");
        }
    }
    login_input::signal_ready();
    hr
}
