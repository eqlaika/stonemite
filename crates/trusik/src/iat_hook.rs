//! IAT (Import Address Table) hook for CreateFileW.
//!
//! Intercepts file opens to detect EQ log files (eqlog_*_*.txt)
//! and report the character name + server via shared memory.

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use windows::Win32::Foundation::{BOOL, HANDLE};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};

use crate::log;

type RawProc = unsafe extern "system" fn() -> isize;

// CreateFileW signature.
type CreateFileWFn = unsafe extern "system" fn(
    lp_file_name: *const u16,
    dw_desired_access: u32,
    dw_share_mode: u32,
    lp_security_attributes: *const c_void,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
    h_template_file: *mut c_void,
) -> HANDLE;

static REAL_CREATE_FILE_W: OnceLock<CreateFileWFn> = OnceLock::new();

unsafe extern "system" fn hooked_create_file_w(
    lp_file_name: *const u16,
    dw_desired_access: u32,
    dw_share_mode: u32,
    lp_security_attributes: *const c_void,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
    h_template_file: *mut c_void,
) -> HANDLE {
    if !lp_file_name.is_null() && wide_contains_eqlog(lp_file_name) {
        if let Some(path) = read_wide_string(lp_file_name) {
            if let Some((character, server)) = parse_eqlog_path(&path) {
                if crate::shm::write_character(&character, &server) {
                    log::write(&format!("CreateFileW: detected {character} on {server}"));
                }
            }
        }
    }

    // Always call the real CreateFileW.
    if let Some(real) = REAL_CREATE_FILE_W.get() {
        real(
            lp_file_name,
            dw_desired_access,
            dw_share_mode,
            lp_security_attributes,
            dw_creation_disposition,
            dw_flags_and_attributes,
            h_template_file,
        )
    } else {
        HANDLE(std::ptr::null_mut())
    }
}

/// Fast check: scan the wide string for "eqlog_" without any allocation.
/// Looks for the substring anywhere in the path (handles full paths like
/// "C:\EQ\Logs\eqlog_Char_Server.txt").
unsafe fn wide_contains_eqlog(ptr: *const u16) -> bool {
    // "eqlog_" as u16: [0x65, 0x71, 0x6C, 0x6F, 0x67, 0x5F]
    const NEEDLE: [u16; 6] = [0x65, 0x71, 0x6C, 0x6F, 0x67, 0x5F];

    let mut i = 0usize;
    loop {
        let ch = *ptr.add(i);
        if ch == 0 {
            return false;
        }
        // Case-insensitive check for first char 'e'/'E'
        if (ch == 0x65 || ch == 0x45) && i < 4096 {
            let mut matched = true;
            for (j, expected) in NEEDLE.iter().enumerate() {
                let c = *ptr.add(i + j);
                // Compare lowercase
                let lower = if (0x41..=0x5A).contains(&c) {
                    c + 0x20
                } else {
                    c
                };
                if lower != *expected {
                    matched = false;
                    break;
                }
            }
            if matched {
                return true;
            }
        }
        i += 1;
        if i > 4096 {
            return false;
        }
    }
}

/// Read a null-terminated wide string into a Rust String.
unsafe fn read_wide_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
        if len > 4096 {
            return None; // sanity limit
        }
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    Some(String::from_utf16_lossy(slice))
}

/// Parse an EQ log file path to extract character and server.
/// Expected filename format: eqlog_CharName_ServerName.txt
fn parse_eqlog_path(path: &str) -> Option<(String, String)> {
    // Extract the filename from the full path.
    let filename = path.rsplit(['\\', '/']).next()?;

    if !filename.starts_with("eqlog_") || !filename.ends_with(".txt") {
        return None;
    }

    let stem = &filename["eqlog_".len()..filename.len() - ".txt".len()];
    let (character, server) = stem.rsplit_once('_')?;

    if character.is_empty() || server.is_empty() {
        return None;
    }

    Some((character.to_string(), server.to_string()))
}

/// Install the CreateFileW IAT hook. Call once after DllMain.
pub unsafe fn install() {
    let base = match windows::Win32::System::LibraryLoader::GetModuleHandleW(None) {
        Ok(h) => h.0 as *const u8,
        Err(_) => {
            log::write("iat_hook: GetModuleHandleW failed");
            return;
        }
    };

    log::write(&format!("iat_hook: base=0x{:X}", base as usize));

    // Dump kernel32 imports for diagnostics.
    dump_imports(base, b"kernel32.dll");

    // Publish the real target before replacing an IAT slot. Another thread can
    // enter an atomically replaced thunk immediately after the write.
    let kernel32 = match windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!(
        "kernel32.dll"
    )) {
        Ok(module) => module,
        Err(_) => {
            log::write("iat_hook: kernel32.dll is not loaded");
            return;
        }
    };
    let Some(real) = windows::Win32::System::LibraryLoader::GetProcAddress(
        kernel32,
        windows::core::s!("CreateFileW"),
    ) else {
        log::write("iat_hook: could not resolve the real CreateFileW");
        return;
    };
    let _ = REAL_CREATE_FILE_W.set(std::mem::transmute::<RawProc, CreateFileWFn>(real));

    // Try CreateFileW first (most common).
    if patch_iat(
        base,
        b"kernel32.dll",
        b"CreateFileW",
        hooked_create_file_w as *const c_void,
    )
    .is_some()
    {
        log::write("iat_hook: hooked CreateFileW");
        return;
    }

    // Fallback: try api-ms-win-core-file-l1-1-0.dll (apiset redirect).
    if patch_iat(
        base,
        b"api-ms-win-core-file-l1-1-0.dll",
        b"CreateFileW",
        hooked_create_file_w as *const c_void,
    )
    .is_some()
    {
        log::write("iat_hook: hooked CreateFileW (via api-ms-win-core-file-l1-1-0)");
        return;
    }

    log::write("iat_hook: WARNING - CreateFileW not found in IAT!");
}

/// Dump function names imported from a given DLL (for diagnostics).
unsafe fn dump_imports(base: *const u8, target_dll: &[u8]) {
    let e_lfanew = *(base.add(0x3C) as *const i32);
    let nt_headers = base.add(e_lfanew as usize);
    let opt_header = nt_headers.add(24);

    let magic = *(opt_header as *const u16);
    let (dd_offset, thunk_size) = match magic {
        0x010B => (96usize, 4usize),  // PE32
        0x020B => (112usize, 8usize), // PE32+
        _ => {
            log::write(&format!("iat_hook: unknown PE magic 0x{magic:04X}"));
            return;
        }
    };

    let import_dir_rva = *(opt_header.add(dd_offset + 8) as *const u32);
    if import_dir_rva == 0 {
        log::write("iat_hook: no import directory");
        return;
    }

    let mut desc = base.add(import_dir_rva as usize);
    loop {
        let name_rva = *(desc.add(12) as *const u32);
        if name_rva == 0 {
            break;
        }

        let dll_name = std::ffi::CStr::from_ptr(base.add(name_rva as usize) as *const i8);
        if dll_name.to_bytes().eq_ignore_ascii_case(target_dll) {
            let original_first_thunk = *(desc as *const u32);
            log::write(&format!(
                "iat_hook: found {} (OrigFirstThunk RVA=0x{original_first_thunk:X})",
                dll_name.to_string_lossy()
            ));

            let mut count = 0u32;
            if thunk_size == 8 {
                let mut p = base.add(original_first_thunk as usize) as *const u64;
                while *p != 0 {
                    if (*p & (1u64 << 63)) == 0 {
                        let hint_name_ptr = base.add(*p as usize);
                        let fn_name = std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        let name_str = fn_name.to_string_lossy();
                        let name_lower = name_str.to_ascii_lowercase();
                        if name_lower.contains("file") || name_lower.contains("write") {
                            log::write(&format!("iat_hook:   [file] {name_str}"));
                        }
                        count += 1;
                    }
                    p = p.add(1);
                }
            } else {
                let mut p = base.add(original_first_thunk as usize) as *const u32;
                while *p != 0 {
                    if (*p & (1u32 << 31)) == 0 {
                        let hint_name_ptr = base.add(*p as usize);
                        let fn_name = std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        let name_str = fn_name.to_string_lossy();
                        let name_lower = name_str.to_ascii_lowercase();
                        if name_lower.contains("file") || name_lower.contains("write") {
                            log::write(&format!("iat_hook:   [file] {name_str}"));
                        }
                        count += 1;
                    }
                    p = p.add(1);
                }
            }
            log::write(&format!("iat_hook:   ({count} total imports)"));
            return;
        }
        desc = desc.add(20);
    }
    log::write(&format!(
        "iat_hook: {} not found in import table",
        String::from_utf8_lossy(target_dll)
    ));
}

/// Patch a single IAT entry. Returns the original function pointer on success.
unsafe fn patch_iat(
    base: *const u8,
    target_dll: &[u8],
    target_fn: &[u8],
    new_fn: *const c_void,
) -> Option<*const c_void> {
    let e_lfanew = *(base.add(0x3C) as *const i32);
    let nt_headers = base.add(e_lfanew as usize);
    let opt_header = nt_headers.add(24);

    let magic = *(opt_header as *const u16);
    let (dd_offset, thunk_size) = match magic {
        0x010B => (96usize, 4usize),
        0x020B => (112usize, 8usize),
        _ => return None,
    };

    let import_dir_rva = *(opt_header.add(dd_offset + 8) as *const u32);
    let import_dir_size = *(opt_header.add(dd_offset + 12) as *const u32);
    if import_dir_size == 0 {
        return None;
    }

    let mut desc = base.add(import_dir_rva as usize);
    loop {
        let name_rva = *(desc.add(12) as *const u32);
        if name_rva == 0 {
            break;
        }

        let dll_name = std::ffi::CStr::from_ptr(base.add(name_rva as usize) as *const i8);
        if dll_name.to_bytes().eq_ignore_ascii_case(target_dll) {
            let original_first_thunk = *(desc as *const u32);
            let first_thunk_rva = *(desc.add(16) as *const u32);

            if thunk_size == 8 {
                let mut orig = base.add(original_first_thunk as usize) as *const u64;
                let mut thunk = base.add(first_thunk_rva as usize) as *mut u64;

                while *orig != 0 {
                    if (*orig & (1u64 << 63)) == 0 {
                        let hint_name_ptr = base.add(*orig as usize);
                        let fn_name = std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        if fn_name.to_bytes() == target_fn {
                            let original = *thunk as *const c_void;
                            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
                            if VirtualProtect(
                                thunk as *const c_void,
                                8,
                                PAGE_READWRITE,
                                &mut old_protect,
                            )
                            .is_err()
                            {
                                log::write(
                                    "iat_hook: VirtualProtect failed before 64-bit IAT write",
                                );
                                return None;
                            }
                            std::ptr::write_volatile(thunk, new_fn as u64);
                            if VirtualProtect(
                                thunk as *const c_void,
                                8,
                                old_protect,
                                &mut old_protect,
                            )
                            .is_err()
                            {
                                log::write("iat_hook: failed to restore 64-bit IAT protection");
                            }
                            return Some(original);
                        }
                    }
                    orig = orig.add(1);
                    thunk = thunk.add(1);
                }
            } else {
                let mut orig = base.add(original_first_thunk as usize) as *const u32;
                let mut thunk = base.add(first_thunk_rva as usize) as *mut u32;

                while *orig != 0 {
                    if (*orig & (1u32 << 31)) == 0 {
                        let hint_name_ptr = base.add(*orig as usize);
                        let fn_name = std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        if fn_name.to_bytes() == target_fn {
                            let original = *thunk as *const c_void;
                            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
                            if VirtualProtect(
                                thunk as *const c_void,
                                4,
                                PAGE_READWRITE,
                                &mut old_protect,
                            )
                            .is_err()
                            {
                                log::write(
                                    "iat_hook: VirtualProtect failed before 32-bit IAT write",
                                );
                                return None;
                            }
                            std::ptr::write_volatile(thunk, new_fn as u32);
                            if VirtualProtect(
                                thunk as *const c_void,
                                4,
                                old_protect,
                                &mut old_protect,
                            )
                            .is_err()
                            {
                                log::write("iat_hook: failed to restore 32-bit IAT protection");
                            }
                            return Some(original);
                        }
                    }
                    orig = orig.add(1);
                    thunk = thunk.add(1);
                }
            }
        }
        desc = desc.add(20);
    }

    None
}

// --- Keyboard state IAT hooks ---

type GetAsyncKeyStateFn = unsafe extern "system" fn(i32) -> i16;
type GetKeyStateFn = unsafe extern "system" fn(i32) -> i16;
type GetKeyboardStateFn = unsafe extern "system" fn(*mut u8) -> BOOL;
type GetForegroundWindowFn = unsafe extern "system" fn() -> isize;
type GetFocusFn = unsafe extern "system" fn() -> isize;
type GetActiveWindowFn = unsafe extern "system" fn() -> isize;

static REAL_ASYNC: OnceLock<GetAsyncKeyStateFn> = OnceLock::new();
static REAL_KEYSTATE: OnceLock<GetKeyStateFn> = OnceLock::new();
static REAL_KBSTATE: OnceLock<GetKeyboardStateFn> = OnceLock::new();
static REAL_GETFOREGROUNDWINDOW: OnceLock<GetForegroundWindowFn> = OnceLock::new();
static REAL_DETOURED_GFW: OnceLock<GetForegroundWindowFn> = OnceLock::new();
static REAL_GETFOCUS: OnceLock<GetFocusFn> = OnceLock::new();
static REAL_GETACTIVEWINDOW: OnceLock<GetActiveWindowFn> = OnceLock::new();

unsafe extern "system" fn hooked_get_async_key_state(vk: i32) -> i16 {
    if (0..=255).contains(&vk) {
        let scan = windows::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
            vk as u32,
            windows::Win32::UI::Input::KeyboardAndMouse::MAPVK_VK_TO_VSC,
        );
        if scan > 0 && scan < 256 && crate::key_shm::is_key_pressed(scan as u8) {
            return -32767; // 0x8001
        }
    }
    if crate::key_shm::should_suppress() {
        0
    } else if let Some(real) = REAL_ASYNC.get() {
        real(vk)
    } else {
        0
    }
}

unsafe extern "system" fn hooked_get_key_state(vk: i32) -> i16 {
    if (0..=255).contains(&vk) {
        let scan = windows::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
            vk as u32,
            windows::Win32::UI::Input::KeyboardAndMouse::MAPVK_VK_TO_VSC,
        );
        if scan > 0 && scan < 256 && crate::key_shm::is_key_pressed(scan as u8) {
            return -32767; // 0x8001
        }
    }
    if crate::key_shm::should_suppress() {
        0
    } else if let Some(real) = REAL_KEYSTATE.get() {
        real(vk)
    } else {
        0
    }
}

unsafe extern "system" fn hooked_get_keyboard_state(buf: *mut u8) -> BOOL {
    let ok = if crate::key_shm::should_suppress() {
        if !buf.is_null() {
            std::ptr::write_bytes(buf, 0, 256);
        }
        BOOL(1)
    } else if let Some(real) = REAL_KBSTATE.get() {
        real(buf)
    } else {
        BOOL(0)
    };
    if !buf.is_null() {
        let mut keys = [0u8; 256];
        if crate::key_shm::read_keys(&mut keys) {
            for vk in 0u16..=255 {
                let scan = windows::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
                    vk as u32,
                    windows::Win32::UI::Input::KeyboardAndMouse::MAPVK_VK_TO_VSC,
                );
                if scan > 0 && scan < 256 && keys[scan as usize] != 0 {
                    *buf.add(vk as usize) |= 0x80;
                }
            }
        }
    }
    ok
}

/// Counter to throttle GetForegroundWindow logging.
static GFW_LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

unsafe extern "system" fn hooked_get_foreground_window() -> isize {
    let hwnd = crate::device_proxy::eq_hwnd();
    let active = crate::key_shm::is_active();

    // Log first few calls regardless to confirm the hook fires.
    let count = GFW_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 5 || (active && count < 10) {
        crate::log::write(&format!(
            "GFW hook: eq_hwnd=0x{hwnd:X} active={active} count={count}"
        ));
    }

    if hwnd != 0 && active {
        return hwnd;
    }
    unspoofed_foreground_window()
}

unsafe fn unspoofed_foreground_window() -> isize {
    if let Some(real) = REAL_DETOURED_GFW.get() {
        real()
    } else if let Some(real) = REAL_GETFOREGROUNDWINDOW.get() {
        real()
    } else {
        0
    }
}

unsafe extern "system" fn hooked_get_focus() -> isize {
    let hwnd = crate::device_proxy::eq_hwnd();
    if hwnd != 0 && crate::key_shm::is_active() {
        return hwnd;
    }
    if let Some(real) = REAL_GETFOCUS.get() {
        real()
    } else {
        0
    }
}

unsafe extern "system" fn hooked_get_active_window() -> isize {
    let hwnd = crate::device_proxy::eq_hwnd();
    if hwnd != 0 && crate::key_shm::is_active() {
        return hwnd;
    }
    if let Some(real) = REAL_GETACTIVEWINDOW.get() {
        real()
    } else {
        0
    }
}

/// Install keyboard state IAT hooks. Call once from DirectInput8Create.
pub unsafe fn install_keyboard_hooks() {
    let base = match windows::Win32::System::LibraryLoader::GetModuleHandleW(None) {
        Ok(h) => h.0 as *const u8,
        Err(_) => {
            log::write("iat_hook: GetModuleHandleW failed (keyboard hooks)");
            return;
        }
    };

    // Resolve and publish every original before any IAT slot can point at a
    // hook. LoadLibrary is safe here because deferred initialization runs
    // outside DllMain.
    let user32 = match windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!(
        "user32.dll"
    )) {
        Ok(module) => module,
        Err(error) => {
            log::write(&format!("iat_hook: failed to load user32.dll: {error}"));
            return;
        }
    };
    if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
        user32,
        windows::core::s!("GetAsyncKeyState"),
    ) {
        let _ = REAL_ASYNC.set(std::mem::transmute::<RawProc, GetAsyncKeyStateFn>(proc));
    }
    if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
        user32,
        windows::core::s!("GetKeyState"),
    ) {
        let _ = REAL_KEYSTATE.set(std::mem::transmute::<RawProc, GetKeyStateFn>(proc));
    }
    if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
        user32,
        windows::core::s!("GetKeyboardState"),
    ) {
        let _ = REAL_KBSTATE.set(std::mem::transmute::<RawProc, GetKeyboardStateFn>(proc));
    }
    if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
        user32,
        windows::core::s!("GetForegroundWindow"),
    ) {
        let _ = REAL_GETFOREGROUNDWINDOW.set(proc);
    }
    if let Some(proc) =
        windows::Win32::System::LibraryLoader::GetProcAddress(user32, windows::core::s!("GetFocus"))
    {
        let _ = REAL_GETFOCUS.set(proc);
    }
    if let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
        user32,
        windows::core::s!("GetActiveWindow"),
    ) {
        let _ = REAL_GETACTIVEWINDOW.set(proc);
    }

    let mut hooked = 0u32;

    if let Some(_original) = patch_iat(
        base,
        b"user32.dll",
        b"GetAsyncKeyState",
        hooked_get_async_key_state as *const c_void,
    ) {
        hooked += 1;
        log::write("iat_hook: hooked GetAsyncKeyState");
    } else {
        log::write("iat_hook: FAILED GetAsyncKeyState");
    }

    if let Some(_original) = patch_iat(
        base,
        b"user32.dll",
        b"GetKeyState",
        hooked_get_key_state as *const c_void,
    ) {
        hooked += 1;
        log::write("iat_hook: hooked GetKeyState");
    } else {
        log::write("iat_hook: FAILED GetKeyState");
    }

    if let Some(_original) = patch_iat(
        base,
        b"user32.dll",
        b"GetKeyboardState",
        hooked_get_keyboard_state as *const c_void,
    ) {
        hooked += 1;
        log::write("iat_hook: hooked GetKeyboardState");
    } else {
        log::write("iat_hook: FAILED GetKeyboardState");
    }

    // Try user32.dll first, then apiset redirects.
    let fg_hook = patch_iat(
        base,
        b"user32.dll",
        b"GetForegroundWindow",
        hooked_get_foreground_window as *const c_void,
    )
    .or_else(|| {
        patch_iat(
            base,
            b"api-ms-win-ntuser-ia-l1-1-0.dll",
            b"GetForegroundWindow",
            hooked_get_foreground_window as *const c_void,
        )
    });
    if fg_hook.is_some() {
        hooked += 1;
        log::write("iat_hook: hooked GetForegroundWindow");
    } else {
        log::write("iat_hook: GetForegroundWindow not imported by the main executable");
    }

    let focus_hook = patch_iat(
        base,
        b"user32.dll",
        b"GetFocus",
        hooked_get_focus as *const c_void,
    );
    if focus_hook.is_some() {
        hooked += 1;
        log::write("iat_hook: hooked GetFocus");
    } else {
        log::write("iat_hook: FAILED GetFocus (may not be imported)");
    }

    let active_hook = patch_iat(
        base,
        b"user32.dll",
        b"GetActiveWindow",
        hooked_get_active_window as *const c_void,
    );
    if active_hook.is_some() {
        hooked += 1;
        log::write("iat_hook: hooked GetActiveWindow");
    } else {
        log::write("iat_hook: FAILED GetActiveWindow (may not be imported)");
    }

    log::write(&format!("iat_hook: {hooked} keyboard function(s) hooked"));

    // EQ resolves GetForegroundWindow through a path that bypasses its main
    // executable IAT. MinHook relocates the prologue into a trampoline and
    // suspends peer threads while publishing the detour, avoiding the torn
    // instruction window of the old hand-written live patch.
    install_foreground_detour();
}

unsafe fn install_foreground_detour() {
    let Some(target) = REAL_GETFOREGROUNDWINDOW.get().copied() else {
        log::write("gfw_detour: GetForegroundWindow was not resolved");
        return;
    };
    let target_ptr = target as *const () as *mut c_void;
    let result = std::panic::catch_unwind(|| unsafe {
        let trampoline = minhook::MinHook::create_hook(
            target_ptr,
            hooked_get_foreground_window as *const () as *mut c_void,
        )?;
        let trampoline: GetForegroundWindowFn = std::mem::transmute(trampoline);
        let _ = REAL_DETOURED_GFW.set(trampoline);
        minhook::MinHook::enable_hook(target_ptr)
    });
    match result {
        Ok(Ok(())) => log::write("gfw_detour: enabled safe GetForegroundWindow detour"),
        Ok(Err(error)) => log::write(&format!("gfw_detour: MinHook failed: {error}")),
        Err(_) => log::write("gfw_detour: MinHook panicked during installation"),
    }
}

/// Return the unspoofed system foreground window for proxy-internal gating.
pub unsafe fn real_foreground_window() -> isize {
    unspoofed_foreground_window()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_eqlog_path() {
        assert_eq!(
            parse_eqlog_path(r"C:\EQ\Logs\eqlog_Charname_servername.txt"),
            Some(("Charname".to_string(), "servername".to_string()))
        );
        assert_eq!(
            parse_eqlog_path(r"eqlog_MyChar_Bristlebane.txt"),
            Some(("MyChar".to_string(), "Bristlebane".to_string()))
        );
        assert_eq!(parse_eqlog_path(r"C:\EQ\somefile.txt"), None);
        assert_eq!(parse_eqlog_path(r"eqlog_.txt"), None);
        assert_eq!(parse_eqlog_path(r"eqlog_NoServer.txt"), None);
    }
}
