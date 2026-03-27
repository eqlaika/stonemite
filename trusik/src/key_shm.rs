use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, FILE_MAP_READ};
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
    keys: [u8; 256],
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();

static mut SHM_PTR: *const SharedKeyState = std::ptr::null();
static mut SHM_HANDLE: HANDLE = HANDLE(std::ptr::null_mut());
/// Countdown frames before retrying open (avoids allocation spam at 60fps).
static mut RETRY_COUNTDOWN: u32 = 0;

unsafe fn try_open() -> bool {
    let pid = GetCurrentProcessId();
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let handle = match OpenFileMappingW(
        FILE_MAP_READ.0,
        false,
        windows::core::PCWSTR(wide.as_ptr()),
    ) {
        Ok(h) => h,
        Err(_) => return false,
    };

    let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, SHM_SIZE);
    let ptr = view.Value as *const SharedKeyState;
    if ptr.is_null() {
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        return false;
    }

    SHM_HANDLE = handle;
    SHM_PTR = ptr;

    let marker = std::ptr::read_volatile(&(*ptr).keys[255]);
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    log::write(&format!(
        "key_shm: opened Local\\DI8_{pid} magic=0x{magic:08X} marker=0x{marker:02X}",
    ));
    true
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
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let active = std::ptr::read_volatile(&(*ptr).active);
    if magic != MAGIC || active == 0 {
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
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let active = std::ptr::read_volatile(&(*ptr).active);
    magic == MAGIC && active != 0
}

/// Returns true if the app is telling this process to suppress physical keys.
pub unsafe fn should_suppress() -> bool {
    if SHM_PTR.is_null() {
        return false;
    }
    let ptr = SHM_PTR;
    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let active = std::ptr::read_volatile(&(*ptr).active);
    let suppress = std::ptr::read_volatile(&(*ptr).suppress);
    magic == MAGIC && active != 0 && suppress != 0
}

/// Returns true if the given scan code is marked as pressed in shared memory.
/// Called by the IAT-hooked GetAsyncKeyState (after VK->scan conversion).
pub unsafe fn is_key_pressed(scan: u8) -> bool {
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

    let len = (buf_len as usize).min(256);
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
    for i in 0..256 {
        out[i] = std::ptr::read_volatile(&state.keys[i]);
    }
    true
}
