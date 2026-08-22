//! Pre-login support: signals DI readiness so the main app knows when
//! it is safe to start writing keys into shared memory.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, SetEvent};

use crate::log;

static EVENT_HANDLE: AtomicIsize = AtomicIsize::new(0);

/// Create the named event during deferred runtime initialization.
pub fn create_event() {
    let pid = unsafe { GetCurrentProcessId() };
    let name = format!("Local\\Stonemite_DI_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    unsafe {
        match CreateEventW(None, true, false, windows::core::PCWSTR(wide.as_ptr())) {
            Ok(h) => {
                EVENT_HANDLE.store(h.0 as isize, Ordering::Release);
                log::write(&format!(
                    "login_input: created event Local\\Stonemite_DI_{pid}"
                ));
            }
            Err(e) => {
                log::write(&format!("login_input: CreateEventW failed: {e}"));
            }
        }
    }
}

/// Signal that DirectInput is ready. Called from DirectInput8Create.
pub fn signal_ready() {
    let handle = EVENT_HANDLE.load(Ordering::Acquire);
    if handle != 0 {
        unsafe {
            let _ = SetEvent(windows::Win32::Foundation::HANDLE(handle as *mut c_void));
        }
        log::write("login_input: signaled DI ready");
    }
}
