mod device_proxy;
mod di8_proxy;
mod iat_hook;
mod kbd_patch;
mod key_shm;
mod log;
mod login_input;
pub mod shm;

use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{GUID, HRESULT, PCSTR};
use windows::Win32::Foundation::{BOOL, HINSTANCE, TRUE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

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

/// Called by the OS when the DLL is loaded/unloaded.
#[unsafe(no_mangle)]
extern "system" fn DllMain(_hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    const DLL_PROCESS_ATTACH: u32 = 1;

    if reason == DLL_PROCESS_ATTACH {
        log::init();
        log::write("DllMain: PROCESS_ATTACH");

        // Load the real dinput8.dll from System32.
        let real_dll = unsafe {
            LoadLibraryW(windows::core::w!("C:\\Windows\\System32\\dinput8.dll"))
        };
        let real_dll = match real_dll {
            Ok(h) => h,
            Err(_) => {
                log::write("DllMain: FAILED to load real dinput8.dll");
                return BOOL(0);
            }
        };
        log::write("DllMain: loaded real dinput8.dll");

        // Resolve the real DirectInput8Create.
        let proc =
            unsafe { GetProcAddress(real_dll, PCSTR(b"DirectInput8Create\0".as_ptr())) };
        let proc = match proc {
            Some(p) => p,
            None => {
                log::write("DllMain: FAILED to resolve DirectInput8Create");
                return BOOL(0);
            }
        };

        let func: DirectInput8CreateFn = unsafe { std::mem::transmute(proc) };
        let _ = REAL_DI8_CREATE.set(func);
        log::write("DllMain: resolved DirectInput8Create");

        // Create shared memory for character detection.
        shm::create();

        // Install IAT hook for CreateFileW.
        unsafe { iat_hook::install() };

        // Install keyboard state hooks early so auto-login can inject
        // keystrokes via shared memory before DirectInput is initialized.
        unsafe { iat_hook::install_keyboard_hooks() };

        // Prepare the keyboard_process foreground-check patch.
        unsafe { kbd_patch::init() };

        // Create event that the main app waits on before auto-typing.
        login_input::create_event();

        log::write("DllMain: ready");
    }

    TRUE
}

/// The exported DirectInput8Create that EQ will call.
///
/// We call the real function, then wrap the returned IDirectInput8 interface
/// in our proxy so we can intercept CreateDevice calls.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DirectInput8Create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: *mut c_void,
) -> HRESULT {
    let real_create = match REAL_DI8_CREATE.get() {
        Some(f) => f,
        None => return HRESULT(-1),
    };

    log::write("DirectInput8Create called");
    login_input::signal_ready();

    let hr = unsafe { real_create(hinst, dwversion, riidltf, ppvout, punkouter) };
    if hr.is_err() {
        log::write(&format!("DirectInput8Create: real call failed (0x{:08X})", hr.0));
        return hr;
    }

    // Wrap the real IDirectInput8 in our proxy.
    let real_di8 = unsafe { *ppvout };
    let proxy = di8_proxy::DI8Proxy::new(real_di8);
    let proxy_ptr = Box::into_raw(Box::new(proxy));
    unsafe { *ppvout = proxy_ptr as *mut c_void };

    // Install keyboard IAT hooks (once).
    use std::sync::Once;
    static IAT_KB_ONCE: Once = Once::new();
    IAT_KB_ONCE.call_once(|| {
        unsafe { iat_hook::install_keyboard_hooks() };
    });

    log::write("DirectInput8Create: wrapped in proxy");
    hr
}
