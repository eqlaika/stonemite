#![windows_subsystem = "windows"]

mod broadcast;
mod character_cache;
mod class_icons;
mod config;
mod eq_characters;
mod eq_windows;
mod log_watcher;
mod overlay;
mod settings_dialog;
mod telemetry;
mod tray;
mod trusik_deploy;
mod trusik_shm;
mod updater;

fn main() {
    // `--settings` flag: run the settings dialog as a standalone window and exit.
    // The tray app spawns us with this flag so eframe gets a clean main thread.
    if std::env::args().any(|a| a == "--settings") {
        settings_dialog::run_standalone();
        return;
    }

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

    // Check if this is a first launch (no config file yet).
    let first_launch = config::Config::path().map_or(false, |p| !p.exists());

    // Load config (creates default if missing).
    let config = config::Config::load();

    // Deploy or remove trusik DLL based on config.
    let eq_dir = config.eq_directory();
    if config.trusik {
        if let Err(e) = trusik_deploy::deploy(&eq_dir) {
            eprintln!("trusik deploy failed: {e}");
        }
    } else {
        let _ = trusik_deploy::remove(&eq_dir);
    }

    // Send anonymous telemetry ping (fire-and-forget background thread).
    telemetry::send_app_start(&config);

    // Initialize overlay (creates the overlay window, hidden until EQ windows are detected).
    overlay::init();

    // Initialize broadcast engine if trusik is enabled.
    if config.trusik {
        broadcast::init();
    }

    // On first launch, open settings so the user can configure the app.
    if first_launch {
        settings_dialog::show();
    }

    // Run tray icon and message loop (blocks until exit).
    tray::run();

    // Cleanup broadcast engine before exit.
    broadcast::cleanup();

    // Cleanup overlay before exit.
    overlay::cleanup();
}
