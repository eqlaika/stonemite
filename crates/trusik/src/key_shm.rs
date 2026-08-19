use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentProcessId;

use crate::log;

/// Shared memory layout — must match the app's definitions exactly.
#[repr(C)]
struct SharedKeyState {
    magic: u32,
    version: u32,
    /// Bitset of independent keyboard delivery owners.
    keyboard_active: AtomicU32,
    /// Independent Mouse Clutch activation (zero or one).
    mouse_active: AtomicU32,
    /// 1 = zero physical keyboard state before applying `keys`.
    suppress: u32,
    /// Sequence counter incremented by the app on every key change.
    seq: u32,
    /// DirectInput scan codes 0–254. Scan code 255 is reserved.
    keys: [u8; 255],
    /// Reverse acknowledgement owned by this proxy.
    proxy_ready: u8,
    /// Low 32 bits of GetTickCount64, refreshed by the controller.
    controller_heartbeat_ms: AtomicU32,
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const VERSION: u32 = 2;
const PROXY_READY: u8 = 0xA5;
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();
const KEYBOARD_CONTROLLER_MASK: u32 = (1 << 0) | (1 << 1);
const KEYBOARD_AUTO_TYPE: u32 = 1 << 2;
const CONTROLLER_HEARTBEAT_TIMEOUT_MS: u32 = 500;

static mut SHM_PTR: *mut SharedKeyState = std::ptr::null_mut();
static mut SHM_HANDLE: HANDLE = HANDLE(std::ptr::null_mut());
/// Countdown frames before retrying open (avoids allocation spam at 60fps).
static mut RETRY_COUNTDOWN: u32 = 0;

fn compatible_values(magic: u32, version: u32) -> bool {
    magic == MAGIC && version == VERSION
}

unsafe fn mapping_is_compatible(ptr: *const SharedKeyState) -> bool {
    compatible_values(
        std::ptr::read_volatile(&(*ptr).magic),
        std::ptr::read_volatile(&(*ptr).version),
    )
}

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

    let magic = std::ptr::read_volatile(&(*ptr).magic);
    let version = std::ptr::read_volatile(&(*ptr).version);
    if !compatible_values(magic, version) {
        log::write(&format!(
            "key_shm: rejected incompatible Local\\DI8_{pid} magic=0x{magic:08X} version={version}"
        ));
        let _ = UnmapViewOfFile(view);
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        return false;
    }

    SHM_HANDLE = handle;
    SHM_PTR = ptr;

    acknowledge_if_valid(ptr);
    let ready = std::ptr::read_volatile(&(*ptr).proxy_ready);
    log::write(&format!(
        "key_shm: opened Local\\DI8_{pid} magic=0x{magic:08X} version={version} ready=0x{ready:02X}",
    ));
    true
}

unsafe fn acknowledge_if_valid(ptr: *mut SharedKeyState) {
    if mapping_is_compatible(ptr) && std::ptr::read_volatile(&(*ptr).proxy_ready) != PROXY_READY {
        std::ptr::write_volatile(&mut (*ptr).proxy_ready, PROXY_READY);
        log::write("key_shm: acknowledged proxy readiness");
    }
}

unsafe fn get_compatible_state() -> Option<&'static SharedKeyState> {
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
    if !mapping_is_compatible(ptr) {
        return None;
    }
    acknowledge_if_valid(ptr);
    Some(&*ptr)
}

fn heartbeat_is_fresh(state: &SharedKeyState, now_ms: u32) -> bool {
    let heartbeat = state.controller_heartbeat_ms.load(Ordering::Acquire);
    heartbeat != 0 && now_ms.wrapping_sub(heartbeat) <= CONTROLLER_HEARTBEAT_TIMEOUT_MS
}

fn keyboard_active_at(state: &SharedKeyState, now_ms: u32) -> bool {
    let active = state.keyboard_active.load(Ordering::Acquire);
    active & KEYBOARD_AUTO_TYPE != 0
        || (active & KEYBOARD_CONTROLLER_MASK != 0 && heartbeat_is_fresh(state, now_ms))
}

fn mouse_active_at(state: &SharedKeyState, now_ms: u32) -> bool {
    state.mouse_active.load(Ordering::Acquire) != 0 && heartbeat_is_fresh(state, now_ms)
}

unsafe fn keyboard_active_in(state: &SharedKeyState) -> bool {
    keyboard_active_at(state, GetTickCount())
}

unsafe fn mouse_active_in(state: &SharedKeyState) -> bool {
    mouse_active_at(state, GetTickCount())
}

unsafe fn any_active_in(state: &SharedKeyState) -> bool {
    keyboard_active_in(state) || mouse_active_in(state)
}

/// Returns true when either keyboard delivery or Mouse Clutch needs EQ's
/// activation/focus spoof.
pub unsafe fn is_active() -> bool {
    get_compatible_state().is_some_and(|state| any_active_in(state))
}

/// Returns true only for synthetic keyboard delivery.
pub unsafe fn is_keyboard_active() -> bool {
    get_compatible_state().is_some_and(|state| keyboard_active_in(state))
}

/// Returns true only for physical mouse pass-through.
pub unsafe fn is_mouse_active() -> bool {
    get_compatible_state().is_some_and(|state| mouse_active_in(state))
}

/// Returns true when a compatible controller mapping is present, regardless of
/// whether an input path is active.
pub unsafe fn is_compatible() -> bool {
    get_compatible_state().is_some()
}

/// Returns true if the app identifies this as a background process whose real
/// keyboard must be suppressed. This remains fail-safe while activation drains,
/// the helper delays WM_ACTIVATEAPP(FALSE), or the controller disappears.
pub unsafe fn should_suppress() -> bool {
    get_compatible_state().is_some_and(|state| std::ptr::read_volatile(&state.suppress) != 0)
}

/// Returns true if the given scan code is marked as pressed in shared memory.
/// Called by the IAT-hooked GetAsyncKeyState (after VK->scan conversion).
pub unsafe fn is_key_pressed(scan: u8) -> bool {
    if scan == 255 {
        return false;
    }
    let Some(state) = get_compatible_state() else {
        return false;
    };
    keyboard_active_in(state) && std::ptr::read_volatile(&state.keys[scan as usize]) != 0
}

/// Read synthetic key states from shared memory and OR them into the
/// DirectInput keyboard buffer.
///
/// Returns `true` if any keys were injected.
pub unsafe fn inject_keys(buf: *mut u8, buf_len: u32) -> bool {
    let Some(state) = get_compatible_state().filter(|state| keyboard_active_in(state)) else {
        return false;
    };

    let len = (buf_len as usize).min(255);
    let mut injected = false;
    for i in 0..len {
        let key = std::ptr::read_volatile(&state.keys[i]);
        if key != 0 {
            *buf.add(i) |= key;
            injected = true;
        }
    }
    injected
}

/// Copy the current shared-memory key array into `out` (256 bytes).
/// Returns true if keyboard delivery is active and keys were read.
pub unsafe fn read_keys(out: &mut [u8; 256]) -> bool {
    let Some(state) = get_compatible_state().filter(|state| keyboard_active_in(state)) else {
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
            keyboard_active: AtomicU32::new(0),
            mouse_active: AtomicU32::new(0),
            suppress: 0,
            seq: 0,
            keys: [0; 255],
            proxy_ready: 0,
            controller_heartbeat_ms: AtomicU32::new(0),
        }
    }

    #[test]
    fn shared_memory_layout_matches_version_two_abi() {
        assert_eq!(VERSION, 2);
        assert_eq!(SHM_SIZE, 284);
        assert_eq!(std::mem::align_of::<SharedKeyState>(), 4);
        assert_eq!(std::mem::offset_of!(SharedKeyState, magic), 0);
        assert_eq!(std::mem::offset_of!(SharedKeyState, version), 4);
        assert_eq!(std::mem::offset_of!(SharedKeyState, keyboard_active), 8);
        assert_eq!(std::mem::offset_of!(SharedKeyState, mouse_active), 12);
        assert_eq!(std::mem::offset_of!(SharedKeyState, suppress), 16);
        assert_eq!(std::mem::offset_of!(SharedKeyState, seq), 20);
        assert_eq!(std::mem::offset_of!(SharedKeyState, keys), 24);
        assert_eq!(std::mem::offset_of!(SharedKeyState, proxy_ready), 279);
        assert_eq!(
            std::mem::offset_of!(SharedKeyState, controller_heartbeat_ms),
            280
        );
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

    #[test]
    fn keyboard_and_mouse_activation_are_independent_and_controller_bounded() {
        let shared = state(MAGIC, VERSION);
        assert!(!keyboard_active_at(&shared, 100));
        assert!(!mouse_active_at(&shared, 100));

        shared.keyboard_active.store(1, Ordering::Release);
        shared.controller_heartbeat_ms.store(100, Ordering::Release);
        assert!(keyboard_active_at(&shared, 100));
        assert!(!keyboard_active_at(
            &shared,
            100 + CONTROLLER_HEARTBEAT_TIMEOUT_MS + 1
        ));

        shared.keyboard_active.store(4, Ordering::Release);
        assert!(keyboard_active_at(
            &shared,
            100 + CONTROLLER_HEARTBEAT_TIMEOUT_MS + 1
        ));

        shared.keyboard_active.store(0, Ordering::Release);
        shared.mouse_active.store(1, Ordering::Release);
        assert!(mouse_active_at(&shared, 100));
        assert!(mouse_active_at(
            &shared,
            100 + CONTROLLER_HEARTBEAT_TIMEOUT_MS
        ));
        assert!(!mouse_active_at(
            &shared,
            100 + CONTROLLER_HEARTBEAT_TIMEOUT_MS + 1
        ));
    }
}
