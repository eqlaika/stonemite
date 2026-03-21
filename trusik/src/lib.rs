mod iat_hook;
mod log;
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

        log::write("DllMain: ready");
    }

    TRUE
}

/// Pure passthrough — call the real DirectInput8Create and return its result.
/// No COM wrapping, no interface interception.
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

    log::write("DirectInput8Create: passthrough");
    unsafe { real_create(hinst, dwversion, riidltf, ppvout, punkouter) }
}
