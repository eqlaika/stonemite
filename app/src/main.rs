#![windows_subsystem = "windows"]

mod config;
mod eq_characters;
mod eq_windows;
mod overlay;
mod settings_dialog;
mod telemetry;
mod tray;
mod updater;

fn main() {
    // Ensure only one instance is running via a named mutex.
    let _mutex = unsafe {
        extern "system" {
            fn CreateMutexW(
                attrs: *const std::ffi::c_void,
                initial_owner: i32,
                name: *const u16,
            ) -> *mut std::ffi::c_void;
        }
        const NAME: &[u16] = &[
            b'G' as u16, b'l' as u16, b'o' as u16, b'b' as u16, b'a' as u16, b'l' as u16,
            b'\\' as u16, b'S' as u16, b't' as u16, b'o' as u16, b'n' as u16, b'e' as u16,
            b'm' as u16, b'i' as u16, b't' as u16, b'e' as u16, 0,
        ];
        let h = CreateMutexW(std::ptr::null(), 1, NAME.as_ptr());
        if h.is_null() || windows::Win32::Foundation::GetLastError() == windows::Win32::Foundation::ERROR_ALREADY_EXISTS {
            return;
        }
        h
    };

    // Load config (creates default if missing).
    let config = config::Config::load();

    // Send anonymous telemetry ping (fire-and-forget background thread).
    telemetry::send_app_start(&config);

    // Initialize overlay (creates the overlay window, hidden until EQ windows are detected).
    overlay::init();

    // Run tray icon and message loop (blocks until exit).
    tray::run();

    // Cleanup overlay before exit.
    overlay::cleanup();
}
