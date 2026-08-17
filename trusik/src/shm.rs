use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::GetCurrentProcessId;

use crate::log;

/// Shared memory layout — must match app's reader definition exactly.
#[repr(C)]
pub struct CharacterInfo {
    /// Magic value: 0x53544D43 ("STMC")
    pub magic: u32,
    /// Process ID of this EQ instance.
    pub pid: u32,
    /// Character name, UTF-8, null-terminated.
    pub character: [u8; 64],
    /// Server name, UTF-8, null-terminated.
    pub server: [u8; 64],
    /// Seqlock generation. Odd while identity fields are being replaced.
    pub generation: AtomicU32,
}

const MAGIC: u32 = 0x53544D43; // "STMC"
const SHM_SIZE: usize = std::mem::size_of::<CharacterInfo>();

/// Wrapper to make a raw pointer Send + Sync. Writes are serialized and the
/// mapped region outlives every hook call in the process.
struct SendPtr(*mut CharacterInfo);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

static SHM_PTR: OnceLock<SendPtr> = OnceLock::new();
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Create the shared memory region on DLL_PROCESS_ATTACH.
/// Writes magic + PID immediately; character/server filled later.
pub fn create() {
    unsafe {
        let pid = GetCurrentProcessId();
        let name = format!("Local\\Stonemite_{pid}\0");
        let wide: Vec<u16> = name.encode_utf16().collect();

        let handle = CreateFileMappingW(
            windows::Win32::Foundation::INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            SHM_SIZE as u32,
            windows::core::PCWSTR(wide.as_ptr()),
        );
        let handle: windows::Win32::Foundation::HANDLE = match handle {
            Ok(h) => h,
            Err(e) => {
                log::write(&format!("shm: CreateFileMappingW failed: {e}"));
                return;
            }
        };

        let view = MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, SHM_SIZE);
        let ptr = view.Value as *mut CharacterInfo;
        if ptr.is_null() {
            log::write("shm: MapViewOfFile returned null");
            let _: Result<(), _> = CloseHandle(handle);
            return;
        }

        // Zero-initialize, then write magic + PID.
        std::ptr::write_bytes(ptr, 0, 1);
        std::ptr::write_volatile(&mut (*ptr).magic, MAGIC);
        std::ptr::write_volatile(&mut (*ptr).pid, pid);
        std::ptr::write(&mut (*ptr).generation, AtomicU32::new(0));

        let _ = SHM_PTR.set(SendPtr(ptr));
        log::write(&format!(
            "shm: created Local\\Stonemite_{pid} ({SHM_SIZE} bytes)"
        ));
    }
}

/// Write a newly detected character identity into shared memory.
/// Returns false when the published, truncated identity is already current.
pub fn write_character(character: &str, server: &str) -> bool {
    let Some(SendPtr(ptr)) = SHM_PTR.get() else {
        return false;
    };
    let ptr = *ptr;
    if ptr.is_null() {
        return false;
    }

    let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let character = truncated_bytes(character);
    let server = truncated_bytes(server);

    unsafe { publish_identity(ptr, character, server) }
}

unsafe fn publish_identity(ptr: *mut CharacterInfo, character: &[u8], server: &[u8]) -> bool {
    if field_matches(&(*ptr).character, character) && field_matches(&(*ptr).server, server) {
        return false;
    }

    // An odd generation tells readers not to accept the two identity
    // fields while either one may contain a partially replaced value.
    (*ptr).generation.fetch_add(1, Ordering::SeqCst);

    let dst = &mut (*ptr).character;
    dst.fill(0);
    dst[..character.len()].copy_from_slice(character);

    let dst = &mut (*ptr).server;
    dst.fill(0);
    dst[..server.len()].copy_from_slice(server);

    (*ptr).generation.fetch_add(1, Ordering::SeqCst);
    true
}

fn truncated_bytes(value: &str) -> &[u8] {
    let bytes = value.as_bytes();
    &bytes[..bytes.len().min(63)]
}

fn field_matches(field: &[u8; 64], value: &[u8]) -> bool {
    field[..value.len()] == *value && field[value.len()] == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_comparison_uses_the_published_truncated_value() {
        let mut field = [0u8; 64];
        let long = "a".repeat(80);
        let truncated = truncated_bytes(&long);
        field[..truncated.len()].copy_from_slice(truncated);

        assert!(field_matches(&field, truncated));
        assert!(!field_matches(&field, b"different"));
    }

    #[test]
    fn publishing_changes_identity_once_and_advances_an_even_generation() {
        let mut info = CharacterInfo {
            magic: MAGIC,
            pid: 42,
            character: [0; 64],
            server: [0; 64],
            generation: AtomicU32::new(0),
        };

        unsafe {
            assert!(publish_identity(&mut info, b"Orlov", b"teek"));
            assert_eq!(info.generation.load(Ordering::SeqCst), 2);
            assert!(!publish_identity(&mut info, b"Orlov", b"teek"));
            assert_eq!(info.generation.load(Ordering::SeqCst), 2);
            assert!(publish_identity(&mut info, b"Laika", b"xegony"));
            assert_eq!(info.generation.load(Ordering::SeqCst), 4);
            assert!(field_matches(&info.character, b"Laika"));
            assert!(field_matches(&info.server, b"xegony"));
        }
    }
}
