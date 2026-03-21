//! IAT (Import Address Table) hook for CreateFileW.
//!
//! Intercepts file opens to detect EQ log files (eqlog_*_*.txt)
//! and report the character name + server via shared memory.

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};

use crate::log;

/// Whether we've already detected the character (skip redundant shm writes + logging).
static CHARACTER_DETECTED: AtomicBool = AtomicBool::new(false);

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
    // Fast path: skip entirely once character is already detected.
    if !CHARACTER_DETECTED.load(Ordering::Relaxed)
        && !lp_file_name.is_null()
        && wide_contains_eqlog(lp_file_name)
    {
        if let Some(path) = read_wide_string(lp_file_name) {
            if let Some((character, server)) = parse_eqlog_path(&path) {
                log::write(&format!("CreateFileW: detected {character} on {server}"));
                crate::shm::write_character(&character, &server);
                CHARACTER_DETECTED.store(true, Ordering::Relaxed);
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
            for j in 0..6 {
                let c = *ptr.add(i + j);
                // Compare lowercase
                let lower = if c >= 0x41 && c <= 0x5A { c + 0x20 } else { c };
                if lower != NEEDLE[j] {
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

    // Try CreateFileW first (most common).
    if let Some(real) = patch_iat(
        base,
        b"kernel32.dll",
        b"CreateFileW",
        hooked_create_file_w as *const c_void,
    ) {
        let func: CreateFileWFn = std::mem::transmute(real);
        let _ = REAL_CREATE_FILE_W.set(func);
        log::write("iat_hook: hooked CreateFileW");
        return;
    }

    // Fallback: try api-ms-win-core-file-l1-1-0.dll (apiset redirect).
    if let Some(real) = patch_iat(
        base,
        b"api-ms-win-core-file-l1-1-0.dll",
        b"CreateFileW",
        hooked_create_file_w as *const c_void,
    ) {
        let func: CreateFileWFn = std::mem::transmute(real);
        let _ = REAL_CREATE_FILE_W.set(func);
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
                        let fn_name =
                            std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
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
                        let fn_name =
                            std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
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
                        let fn_name =
                            std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        if fn_name.to_bytes() == target_fn {
                            let original = *thunk as *const c_void;
                            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
                            let _ = VirtualProtect(
                                thunk as *const c_void,
                                8,
                                PAGE_READWRITE,
                                &mut old_protect,
                            );
                            *thunk = new_fn as u64;
                            let _ = VirtualProtect(
                                thunk as *const c_void,
                                8,
                                old_protect,
                                &mut old_protect,
                            );
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
                        let fn_name =
                            std::ffi::CStr::from_ptr(hint_name_ptr.add(2) as *const i8);
                        if fn_name.to_bytes() == target_fn {
                            let original = *thunk as *const c_void;
                            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
                            let _ = VirtualProtect(
                                thunk as *const c_void,
                                4,
                                PAGE_READWRITE,
                                &mut old_protect,
                            );
                            *thunk = new_fn as u32;
                            let _ = VirtualProtect(
                                thunk as *const c_void,
                                4,
                                old_protect,
                                &mut old_protect,
                            );
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
