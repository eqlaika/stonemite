//! Pre-login support: signals DI readiness so the main app knows when
//! it is safe to start writing keys into shared memory.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, SetEvent};

use crate::log;

/// Set to true once DirectInput8Create is called.
static DI_INITIALIZED: AtomicBool = AtomicBool::new(false);

static mut EVENT_HANDLE: windows::Win32::Foundation::HANDLE =
    windows::Win32::Foundation::HANDLE(std::ptr::null_mut());

/// Create the named event at DllMain time. The main app waits on this.
pub fn create_event() {
    let pid = unsafe { GetCurrentProcessId() };
    let name = format!("Local\\Stonemite_DI_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    unsafe {
        match CreateEventW(None, true, false, windows::core::PCWSTR(wide.as_ptr())) {
            Ok(h) => {
                EVENT_HANDLE = h;
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
    DI_INITIALIZED.store(true, Ordering::Release);
    unsafe {
        if !EVENT_HANDLE.0.is_null() {
            let _ = SetEvent(EVENT_HANDLE);
            log::write("login_input: signaled DI ready");
        }
    }
}
