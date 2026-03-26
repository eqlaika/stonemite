#![windows_subsystem = "windows"]

mod broadcast;
mod character_cache;
mod class_icons;
mod config;
mod crypt;
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

// Minimal VEH to log access violations before the process dies.
#[repr(C)]
struct ExceptionRecord {
    exception_code: u32,
    exception_flags: u32,
    exception_record: *mut ExceptionRecord,
    exception_address: *mut std::ffi::c_void,
    number_parameters: u32,
    _pad: u32,
    exception_information: [u64; 2],
}

#[repr(C)]
struct ExceptionPointers {
    exception_record: *mut ExceptionRecord,
    context_record: *mut Context,
}

// Minimal CONTEXT — we only need Rip (at offset 0xF8) and Rsp (at offset 0x98).
#[repr(C, align(16))]
struct Context {
    _head: [u8; 0x98],
    rsp: u64,
    _mid: [u8; 0xF8 - 0x98 - 8],
    rip: u64,
}

extern "system" {
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32,
    ) -> *mut std::ffi::c_void;
}

unsafe extern "system" fn crash_handler(info: *mut ExceptionPointers) -> i32 {
    const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC0000005;
    const EXCEPTION_STACK_OVERFLOW: u32 = 0xC00000FD;

    let record = &*(*info).exception_record;
    let code = record.exception_code;
    if code == EXCEPTION_ACCESS_VIOLATION || code == EXCEPTION_STACK_OVERFLOW {
        let ctx = &*(*info).context_record;

        // Try to identify which module the crashing RIP belongs to.
        let rip = ctx.rip;
        let mut mod_name = String::from("unknown");
        #[repr(C)]
        struct MemoryBasicInfo {
            base_address: *mut std::ffi::c_void,
            allocation_base: *mut std::ffi::c_void,
            _rest: [u8; 32],
        }
        extern "system" {
            fn VirtualQuery(
                addr: *const std::ffi::c_void,
                buf: *mut MemoryBasicInfo,
                len: usize,
            ) -> usize;
            fn GetModuleFileNameW(
                module: *mut std::ffi::c_void,
                buf: *mut u16,
                size: u32,
            ) -> u32;
        }
        let mut mbi = std::mem::zeroed::<MemoryBasicInfo>();
        if VirtualQuery(rip as *const _, &mut mbi, std::mem::size_of::<MemoryBasicInfo>()) != 0
            && !mbi.allocation_base.is_null()
        {
            let mut name_buf = [0u16; 260];
            let len = GetModuleFileNameW(mbi.allocation_base, name_buf.as_mut_ptr(), 260);
            if len > 0 {
                mod_name = String::from_utf16_lossy(&name_buf[..len as usize]);
                // Also log the offset within the module.
                let offset = rip - mbi.allocation_base as u64;
                mod_name = format!("{}+{:#x}", mod_name, offset);
            }
        }

        let msg = format!(
            "CRASH: code={:#010x} rip={:#018x} rsp={:#018x} addr={:#018x} module={}",
            code, rip, ctx.rsp,
            if record.number_parameters >= 2 { record.exception_information[1] } else { 0 },
            mod_name,
        );
        overlay::debug_log(&msg);
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

fn main() {
    // Log panics to debug.log so crashes during login are diagnosable.
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        overlay::debug_log(&msg);
    }));

    // Install a vectored exception handler to catch access violations and
    // log them before the process terminates.
    unsafe {
        AddVectoredExceptionHandler(1, crash_handler);
    }

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
