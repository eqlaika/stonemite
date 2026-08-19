use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE,
};
use windows::Win32::System::Threading::GetCurrentProcessId;

use crate::log;

/// Shared memory layout — must match app's definition exactly.
#[repr(C)]
struct SharedKeyState {
    magic: u32,
    version: u32,
    active: u32,
    /// 1 = zero physical keyboard state before applying `keys`.
    suppress: u32,
    /// Sequence counter incremented by the app on every key change.
    seq: u32,
    /// DirectInput scan codes 0–254. Scan code 255 is reserved.
    keys: [u8; 255],
    /// Reverse acknowledgement owned by this proxy.
    proxy_ready: u8,
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const VERSION: u32 = 1;
const PROXY_READY: u8 = 0xA5;
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();

static mut SHM_PTR: *mut SharedKeyState = std::ptr::null_mut();
static mut SHM_HANDLE: HANDLE = HANDLE(std::ptr::null_mut());
/// Countdown frames before retrying open (avoids allocation spam at 60fps).
static mut RETRY_COUNTDOWN: u32 = 0;

unsafe fn try_open() -> bool {
    let pid = GetCurrentProcessId();
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let access = FILE_MAP_READ.0 | FILE_MAP_WRITE.0;
    let handle = match OpenFileMappingW(access, false, windows::core::PCWSTR(wide.as_ptr())) {
        Ok(h) => h,
        Err(_) => return false,
    };

    let view = MapViewOfFile(handle, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, SHM_SIZE);
    let ptr = view.Value as *mut SharedKeyState;
    if ptr.is_null() {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        return false;
    }

    SHM_HANDLE = handle;
    SHM_PTR = ptr;

    acknowledge_if_valid(ptr);
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    let ready = std::ptr::read_volatile(&(*ptr).proxy_ready);
    log::write(&format!(
        "key_shm: opened Local\\DI8_{pid} magic=0x{magic:08X} version={version} ready=0x{ready:02X}",
    ));
    true
}

unsafe fn acknowledge_if_valid(ptr: *mut SharedKeyState) {
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    if magic == MAGIC
        && version == VERSION
        && std::ptr::read_volatile(&(*ptr).proxy_ready) != PROXY_READY
    {
        std::ptr::write_volatile(&mut (*ptr).proxy_ready, PROXY_READY);
        log::write("key_shm: acknowledged proxy readiness");
    }
}

unsafe fn get_state() -> Option<&'static SharedKeyState> {
    if SHM_PTR.is_null() {
        if RETRY_COUNTDOWN > 0 {
            RETRY_COUNTDOWN -= 1;
            return None;
        }
        if !try_open() {
            RETRY_COUNTDOWN = 4;
            return None;
        }
    }
    let ptr = SHM_PTR;
    acknowledge_if_valid(ptr);
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    let active = std::ptr::read_volatile(&(*ptr).active);
    if magic != MAGIC || version != VERSION || active == 0 {
        return None;
    }
    Some(&*ptr)
}

/// Returns true if shared memory is open and active.
pub unsafe fn is_active() -> bool {
    if SHM_PTR.is_null() {
        let _ = get_state();
        if SHM_PTR.is_null() {
            return false;
        }
    }
    let ptr = SHM_PTR;
    acknowledge_if_valid(ptr);
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    let active = std::ptr::read_volatile(&(*ptr).active);
    magic == MAGIC && version == VERSION && active != 0
}

/// Returns true if the app is telling this process to suppress physical keys.
pub unsafe fn should_suppress() -> bool {
    if SHM_PTR.is_null() {
        return false;
    }
    let ptr = SHM_PTR;
    acknowledge_if_valid(ptr);
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    let active = std::ptr::read_volatile(&(*ptr).active);
    let suppress = std::ptr::read_volatile(&(*ptr).suppress);
    magic == MAGIC && version == VERSION && active != 0 && suppress != 0
}

/// Returns true if the given scan code is marked as pressed in shared memory.
/// Called by the IAT-hooked GetAsyncKeyState (after VK->scan conversion).
pub unsafe fn is_key_pressed(scan: u8) -> bool {
    if scan == 255 {
        return false;
    }
    if SHM_PTR.is_null() {
        let _ = get_state();
        if SHM_PTR.is_null() {
            return false;
        }
    }

    let ptr = SHM_PTR;
    let active = std::ptr::read_volatile(&(*ptr).active);
    if active == 0 {
        return false;
    }

    let key_val = std::ptr::read_volatile(&(*ptr).keys[scan as usize]);
    key_val != 0
}

/// Read synthetic key states from shared memory and OR them into the
/// DirectInput keyboard buffer.
///
/// Returns `true` if any keys were injected.
pub unsafe fn inject_keys(buf: *mut u8, buf_len: u32) -> bool {
    let Some(state) = get_state() else {
        return false;
    };

    let len = (buf_len as usize).min(255);
    let mut injected = false;
    for i in 0..len {
        if state.keys[i] != 0 {
            *buf.add(i) |= state.keys[i];
            injected = true;
        }
    }
    injected
}

/// Copy the current shared-memory key array into `out` (256 bytes).
/// Returns true if shared memory is active and keys were read.
pub unsafe fn read_keys(out: &mut [u8; 256]) -> bool {
    let Some(state) = get_state() else {
        *out = [0u8; 256];
        return false;
    };
    *out = [0u8; 256];
    for (output, key) in out.iter_mut().zip(state.keys.iter()) {
        *output = std::ptr::read_volatile(key);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(magic: u32, version: u32) -> SharedKeyState {
        SharedKeyState {
            magic,
            version,
            active: 0,
            suppress: 0,
            seq: 0,
            keys: [0; 255],
            proxy_ready: 0,
        }
    }

    #[test]
    fn readiness_reuses_the_reserved_scan_byte_without_changing_the_abi() {
        assert_eq!(SHM_SIZE, 276);
        assert_eq!(std::mem::offset_of!(SharedKeyState, proxy_ready), 275);
        assert!(!unsafe { is_key_pressed(255) });
    }

    #[test]
    fn acknowledges_only_a_compatible_mapping_and_can_acknowledge_again() {
        let mut compatible = state(MAGIC, VERSION);
        let mut wrong_version = state(MAGIC, VERSION + 1);
        unsafe {
            acknowledge_if_valid(&mut compatible);
            acknowledge_if_valid(&mut wrong_version);
        }
        assert_eq!(compatible.proxy_ready, PROXY_READY);
        assert_eq!(wrong_version.proxy_ready, 0);

        compatible.proxy_ready = 0;
        unsafe { acknowledge_if_valid(&mut compatible) };
        assert_eq!(compatible.proxy_ready, PROXY_READY);
    }
}
