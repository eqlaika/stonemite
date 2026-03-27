//! Pre-login support: signals DI readiness and keeps background windows
//! active by posting WM_ACTIVATEAPP when the shm becomes active.

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::log;

/// Set to true once DirectInput8Create is called; the wakeup thread exits.
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

/// Start the wakeup thread that posts WM_ACTIVATEAPP to keep the window
/// responsive while the shm is active. Call from DllMain.
pub fn start_wakeup_thread() {
    std::thread::spawn(|| unsafe { wakeup_thread() });
}

/// WM_ACTIVATEAPP = 0x001C
const WM_ACTIVATEAPP: u32 = 0x001C;

/// Shared memory layout — must match key_shm.rs.
#[repr(C)]
struct SharedKeyState {
    magic: u32,
    version: u32,
    active: u32,
    suppress: u32,
    seq: u32,
    keys: [u8; 256],
}

const MAGIC: u32 = 0x53544D54;
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();

unsafe fn wakeup_thread() {
    log::write("login_input: wakeup thread started");

    let pid = GetCurrentProcessId();
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let mut ptr: *const SharedKeyState = std::ptr::null();
    let mut was_active = false;
    let mut ever_active = false;
    let start = std::time::Instant::now();

    loop {
        // Timeout after 60s to avoid leaking the thread.
        if start.elapsed() > std::time::Duration::from_secs(60) {
            log::write("login_input: wakeup thread exiting (timeout)");
            return;
        }

        // If shm was active and is now inactive, typing is done — exit.
        if ever_active && !was_active {
            log::write("login_input: wakeup thread exiting (typing done)");
            return;
        }

        std::thread::sleep(std::time::Duration::from_millis(16));

        // Try to open shm.
        if ptr.is_null() {
            let handle = match windows::Win32::System::Memory::OpenFileMappingW(
                windows::Win32::System::Memory::FILE_MAP_READ.0,
                false,
                windows::core::PCWSTR(wide.as_ptr()),
            ) {
                Ok(h) => h,
                Err(_) => continue,
            };

            let view = windows::Win32::System::Memory::MapViewOfFile(
                handle,
                windows::Win32::System::Memory::FILE_MAP_READ,
                0,
                0,
                SHM_SIZE,
            );
            let p = view.Value as *const SharedKeyState;
            if p.is_null() {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
                continue;
            }
            ptr = p;
            log::write("login_input: wakeup thread opened shm");
        }

        let magic = std::ptr::read_volatile(&(*ptr).magic);
        let active = std::ptr::read_volatile(&(*ptr).active);
        let is_active = magic == MAGIC && active != 0;

        if is_active && !was_active {
            // shm just became active — post WM_ACTIVATEAPP(TRUE).
            let eq_hwnd = crate::device_proxy::eq_hwnd();
            if eq_hwnd != 0 {
                let hwnd = HWND(eq_hwnd as *mut _);
                let _ = PostMessageW(hwnd, WM_ACTIVATEAPP, WPARAM(1), LPARAM(0));
                log::write(&format!(
                    "login_input: posted WM_ACTIVATEAPP(1) to hwnd={eq_hwnd:#x}"
                ));
            } else {
                log::write("login_input: shm active but eq_hwnd not set yet");
            }
            ever_active = true;
        } else if !is_active && was_active {
            // shm deactivated — post WM_ACTIVATEAPP(FALSE).
            let eq_hwnd = crate::device_proxy::eq_hwnd();
            if eq_hwnd != 0 {
                let hwnd = HWND(eq_hwnd as *mut _);
                let _ = PostMessageW(hwnd, WM_ACTIVATEAPP, WPARAM(0), LPARAM(0));
                log::write(&format!(
                    "login_input: posted WM_ACTIVATEAPP(0) to hwnd={eq_hwnd:#x}"
                ));
            }
        }
        was_active = is_active;
    }
}
