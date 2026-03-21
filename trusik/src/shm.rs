use std::sync::OnceLock;
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
}

const MAGIC: u32 = 0x53544D43; // "STMC"
const SHM_SIZE: usize = std::mem::size_of::<CharacterInfo>();

/// Wrapper to make a raw pointer Send + Sync (safe because we only
/// write from DllMain's thread and the shm region outlives the process).
struct SendPtr(*mut CharacterInfo);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

static SHM_PTR: OnceLock<SendPtr> = OnceLock::new();

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

        let _ = SHM_PTR.set(SendPtr(ptr));
        log::write(&format!("shm: created Local\\Stonemite_{pid} ({SHM_SIZE} bytes)"));
    }
}

/// Write the detected character name and server into shared memory.
pub fn write_character(character: &str, server: &str) {
    let Some(SendPtr(ptr)) = SHM_PTR.get() else { return };
    let ptr = *ptr;
    if ptr.is_null() {
        return;
    }

    unsafe {
        // Write character name (null-terminated, truncated to fit).
        let char_bytes = character.as_bytes();
        let char_len = char_bytes.len().min(63);
        let dst = &mut (*ptr).character;
        dst.fill(0);
        dst[..char_len].copy_from_slice(&char_bytes[..char_len]);

        // Write server name (null-terminated, truncated to fit).
        let server_bytes = server.as_bytes();
        let server_len = server_bytes.len().min(63);
        let dst = &mut (*ptr).server;
        dst.fill(0);
        dst[..server_len].copy_from_slice(&server_bytes[..server_len]);
    }

    log::write(&format!("shm: character={character} server={server}"));
}
