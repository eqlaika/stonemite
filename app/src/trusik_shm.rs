use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ,
};

/// Shared memory layout — must match trusik's definition exactly.
#[repr(C)]
struct CharacterInfo {
    magic: u32,
    pid: u32,
    character: [u8; 64],
    server: [u8; 64],
}

const MAGIC: u32 = 0x53544D43; // "STMC"
const SHM_SIZE: usize = std::mem::size_of::<CharacterInfo>();

/// Try to read character info from the shared memory region for a given PID.
/// Returns (character, server) if the region exists and has valid data.
pub fn read_character(pid: u32) -> Option<(String, String)> {
    unsafe {
        let name = format!("Local\\Stonemite_{pid}\0");
        let wide: Vec<u16> = name.encode_utf16().collect();

        let handle = match OpenFileMappingW(
            FILE_MAP_READ.0,
            false,
            windows::core::PCWSTR(wide.as_ptr()),
        ) {
            Ok(h) => h,
            Err(_) => return None,
        };

        let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, SHM_SIZE);
        let ptr = view.Value as *const CharacterInfo;
        if ptr.is_null() {
            let _ = CloseHandle(handle);
            return None;
        }

        let magic = std::ptr::read_volatile(&(*ptr).magic);
        if magic != MAGIC {
            let _ = UnmapViewOfFile(view);
            let _ = CloseHandle(handle);
            return None;
        }

        // Read character name (null-terminated UTF-8).
        let character = read_null_terminated(&(*ptr).character);
        let server = read_null_terminated(&(*ptr).server);

        let _ = UnmapViewOfFile(view);
        let _ = CloseHandle(handle);

        if character.is_empty() {
            return None;
        }

        Some((character, server))
    }
}

fn read_null_terminated(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}
