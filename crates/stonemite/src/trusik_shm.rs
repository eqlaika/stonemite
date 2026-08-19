use std::sync::atomic::{AtomicU32, Ordering};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ,
};

/// Shared memory layout used by identity-refresh-capable trusik DLLs.
#[repr(C)]
struct CharacterInfo {
    magic: u32,
    pid: u32,
    character: [u8; 64],
    server: [u8; 64],
    /// Seqlock generation. Odd while the identity fields are being replaced.
    generation: AtomicU32,
}

/// Prefix layout used by proxy DLLs released before identity refresh support.
#[repr(C)]
struct LegacyCharacterInfo {
    magic: u32,
    pid: u32,
    character: [u8; 64],
    server: [u8; 64],
}

const MAGIC: u32 = 0x53544D43; // "STMC"
const SHM_SIZE: usize = std::mem::size_of::<CharacterInfo>();
const LEGACY_SHM_SIZE: usize = std::mem::size_of::<LegacyCharacterInfo>();

/// Try to read character info from the shared memory region for a given PID.
/// Returns (character, server) if the region exists and has valid data.
pub fn read_character(pid: u32) -> Option<(String, String)> {
    unsafe {
        let name = format!("Local\\Stonemite_{pid}\0");
        let wide: Vec<u16> = name.encode_utf16().collect();

        let handle =
            match OpenFileMappingW(FILE_MAP_READ.0, false, windows::core::PCWSTR(wide.as_ptr())) {
                Ok(h) => h,
                Err(_) => return None,
            };

        // Mapping the extended size fails for an already-running legacy DLL,
        // so fall back to its original prefix layout during rolling updates.
        let versioned_view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, SHM_SIZE);
        if !versioned_view.Value.is_null() {
            let result = read_versioned_identity(versioned_view.Value as *const CharacterInfo, pid);
            let _ = UnmapViewOfFile(versioned_view);
            let _ = CloseHandle(handle);
            return result;
        }

        let legacy_view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, LEGACY_SHM_SIZE);
        let ptr = legacy_view.Value as *const LegacyCharacterInfo;
        if ptr.is_null() {
            let _ = CloseHandle(handle);
            return None;
        }
        let result = read_legacy_identity(ptr, pid);
        let _ = UnmapViewOfFile(legacy_view);
        let _ = CloseHandle(handle);
        result
    }
}

unsafe fn read_versioned_identity(
    ptr: *const CharacterInfo,
    expected_pid: u32,
) -> Option<(String, String)> {
    if std::ptr::read_volatile(std::ptr::addr_of!((*ptr).magic)) != MAGIC
        || std::ptr::read_volatile(std::ptr::addr_of!((*ptr).pid)) != expected_pid
    {
        return None;
    }

    for _ in 0..8 {
        let before = (*ptr).generation.load(Ordering::SeqCst);
        if before & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        let character =
            read_null_terminated_volatile(std::ptr::addr_of!((*ptr).character).cast::<u8>(), 64);
        let server =
            read_null_terminated_volatile(std::ptr::addr_of!((*ptr).server).cast::<u8>(), 64);
        let after = (*ptr).generation.load(Ordering::SeqCst);
        if before == after && after & 1 == 0 {
            return (!character.is_empty()).then_some((character, server));
        }
    }
    None
}

unsafe fn read_legacy_identity(
    ptr: *const LegacyCharacterInfo,
    expected_pid: u32,
) -> Option<(String, String)> {
    if std::ptr::read_volatile(std::ptr::addr_of!((*ptr).magic)) != MAGIC
        || std::ptr::read_volatile(std::ptr::addr_of!((*ptr).pid)) != expected_pid
    {
        return None;
    }
    let character =
        read_null_terminated_volatile(std::ptr::addr_of!((*ptr).character).cast::<u8>(), 64);
    let server = read_null_terminated_volatile(std::ptr::addr_of!((*ptr).server).cast::<u8>(), 64);
    (!character.is_empty()).then_some((character, server))
}

unsafe fn read_null_terminated_volatile(ptr: *const u8, length: usize) -> String {
    let mut bytes = Vec::with_capacity(length);
    for index in 0..length {
        let byte = std::ptr::read_volatile(ptr.add(index));
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: &str) -> [u8; 64] {
        let mut field = [0u8; 64];
        field[..value.len()].copy_from_slice(value.as_bytes());
        field
    }

    #[test]
    fn reads_consistent_versioned_and_legacy_identities() {
        let current = CharacterInfo {
            magic: MAGIC,
            pid: 42,
            character: field("Laika"),
            server: field("xegony"),
            generation: AtomicU32::new(2),
        };
        let legacy = LegacyCharacterInfo {
            magic: MAGIC,
            pid: 42,
            character: field("Orlov"),
            server: field("teek"),
        };

        unsafe {
            assert_eq!(
                read_versioned_identity(&current, 42),
                Some(("Laika".into(), "xegony".into()))
            );
            assert_eq!(
                read_legacy_identity(&legacy, 42),
                Some(("Orlov".into(), "teek".into()))
            );
        }
    }

    #[test]
    fn rejects_in_progress_or_wrong_process_identity() {
        let current = CharacterInfo {
            magic: MAGIC,
            pid: 42,
            character: field("Laika"),
            server: field("xegony"),
            generation: AtomicU32::new(3),
        };

        unsafe {
            assert_eq!(read_versioned_identity(&current, 42), None);
            current.generation.store(4, Ordering::SeqCst);
            assert_eq!(read_versioned_identity(&current, 7), None);
        }
    }
}
