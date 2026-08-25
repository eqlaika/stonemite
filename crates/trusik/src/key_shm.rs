use std::sync::Mutex;

use trusik_protocol::{SharedKeyState, PROXY_READY, SHARED_KEY_STATE_SIZE};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentProcessId;

use crate::log;

const RETRY_INTERVAL_MS: u32 = 100;

struct MappingCache {
    ptr: usize,
    _handle: isize,
    next_retry_ms: u32,
}

static MAPPING: Mutex<MappingCache> = Mutex::new(MappingCache {
    ptr: 0,
    _handle: 0,
    next_retry_ms: 0,
});
static INCOMPATIBLE_LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn retry_is_due(now_ms: u32, deadline_ms: u32) -> bool {
    deadline_ms == 0 || now_ms.wrapping_sub(deadline_ms) as i32 >= 0
}

unsafe fn try_open(cache: &mut MappingCache, now_ms: u32) -> bool {
    let pid = GetCurrentProcessId();
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let access = FILE_MAP_READ.0 | FILE_MAP_WRITE.0;
    let handle = match OpenFileMappingW(access, false, windows::core::PCWSTR(wide.as_ptr())) {
        Ok(handle) => handle,
        Err(_) => {
            cache.next_retry_ms = now_ms.wrapping_add(RETRY_INTERVAL_MS);
            return false;
        }
    };

    let view = MapViewOfFile(
        handle,
        FILE_MAP_READ | FILE_MAP_WRITE,
        0,
        0,
        SHARED_KEY_STATE_SIZE,
    );
    let ptr = view.Value as *mut SharedKeyState;
    if ptr.is_null() {
        let _ = CloseHandle(handle);
        cache.next_retry_ms = now_ms.wrapping_add(RETRY_INTERVAL_MS);
        return false;
    }

    let state = &*ptr;
    if !state.is_compatible() {
        let magic = state.magic.load(std::sync::atomic::Ordering::Acquire);
        let version = state.version.load(std::sync::atomic::Ordering::Acquire);
        if INCOMPATIBLE_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 3 {
            log::write(&format!(
                "key_shm: rejected incompatible Local\\DI8_{pid} magic=0x{magic:08X} version={version} expected_version={}",
                trusik_protocol::VERSION
            ));
        }
        let _ = UnmapViewOfFile(view);
        let _ = CloseHandle(handle);
        cache.next_retry_ms = now_ms.wrapping_add(RETRY_INTERVAL_MS);
        return false;
    }

    state.acknowledge_proxy();
    cache._handle = handle.0 as isize;
    cache.ptr = ptr as usize;
    cache.next_retry_ms = 0;
    log::write(&format!(
        "key_shm: opened Local\\DI8_{pid} version={} ready=0x{:02X}",
        trusik_protocol::VERSION,
        state.proxy_ready.load(std::sync::atomic::Ordering::Acquire)
    ));
    true
}

fn compatible_ptr() -> Option<*const SharedKeyState> {
    let mut cache = MAPPING.lock().unwrap_or_else(|error| error.into_inner());
    if cache.ptr == 0 {
        let now_ms = unsafe { GetTickCount() };
        if !retry_is_due(now_ms, cache.next_retry_ms) || !unsafe { try_open(&mut cache, now_ms) } {
            return None;
        }
    }

    let ptr = cache.ptr as *const SharedKeyState;
    let state = unsafe { &*ptr };
    if !state.is_compatible() {
        return None;
    }
    state.acknowledge_proxy();
    debug_assert_eq!(
        state.proxy_ready.load(std::sync::atomic::Ordering::Acquire),
        PROXY_READY
    );
    Some(ptr)
}

fn with_state<T>(operation: impl FnOnce(&SharedKeyState, u32) -> T) -> Option<T> {
    let ptr = compatible_ptr()?;
    let now_ms = unsafe { GetTickCount() };
    Some(operation(unsafe { &*ptr }, now_ms))
}

/// Returns true when either keyboard delivery or Mouse Clutch needs EQ's
/// activation/focus spoof.
pub fn is_active() -> bool {
    with_state(|state, now_ms| state.is_active(now_ms)).unwrap_or(false)
}

/// Snapshot every source used by the activation watcher with one timestamp.
pub fn activation_state() -> (bool, Option<u32>, bool) {
    with_state(|state, now_ms| {
        (
            state.is_active(now_ms),
            state.active_auto_type_generation(now_ms),
            state.mouse_is_active(now_ms),
        )
    })
    .unwrap_or((false, None, false))
}

/// Returns true only for synthetic keyboard delivery.
pub fn is_keyboard_active() -> bool {
    with_state(|state, now_ms| state.keyboard_is_active(now_ms)).unwrap_or(false)
}

/// Returns true only while the controller-owned keyboard source is fresh.
pub fn is_controller_keyboard_active() -> bool {
    with_state(|state, now_ms| state.controller_keyboard_is_active(now_ms)).unwrap_or(false)
}

/// Capture the current controller event head when a keyboard device is
/// created. Subsequent events are then retained even before its first poll.
pub fn controller_event_head() -> Option<u32> {
    with_state(|state, _| state.controller_event_head())
}

/// Advances a per-device cursor through every controller key edge.
pub fn drain_controller_events(cursor: &mut Option<u32>, visit: impl FnMut(u8, bool)) -> bool {
    with_state(|state, _| state.drain_controller_events(cursor, visit)).unwrap_or(false)
}

/// Snapshot auto-type levels independently so controller edge replay can keep
/// another owner holding the same key pressed.
pub fn read_auto_type_keys(out: &mut [u8; 256]) -> bool {
    let Some(active) = with_state(|state, now_ms| state.read_auto_type_keys(now_ms, out)) else {
        out.fill(0);
        return false;
    };
    active
}

/// Returns true only for physical mouse pass-through.
pub fn is_mouse_active() -> bool {
    with_state(|state, now_ms| state.mouse_is_active(now_ms)).unwrap_or(false)
}

/// Returns true when a compatible controller mapping is present, regardless of
/// whether an input path is active.
pub fn is_compatible() -> bool {
    compatible_ptr().is_some()
}

/// Returns true when a fresh controller identifies this as a background process
/// whose real keyboard must be suppressed. Controller failure is fail-open.
pub fn should_suppress() -> bool {
    with_state(|state, now_ms| state.should_suppress(now_ms)).unwrap_or(false)
}

/// Returns true if the given scan code is pressed by any fresh active owner.
pub fn is_key_pressed(scan: u8) -> bool {
    if scan == 255 {
        return false;
    }
    with_state(|state, now_ms| state.key_is_pressed(scan, now_ms)).unwrap_or(false)
}

/// OR current synthetic key levels into a DirectInput keyboard buffer.
/// Returns `true` if any keys were injected.
pub unsafe fn inject_keys(buf: *mut u8, buf_len: u32) -> bool {
    let mut keys = [0u8; 256];
    if !read_keys(&mut keys) {
        return false;
    }

    let mut injected = false;
    for (index, key) in keys
        .iter()
        .copied()
        .take((buf_len as usize).min(255))
        .enumerate()
    {
        if key != 0 {
            *buf.add(index) |= key;
            injected = true;
        }
    }
    injected
}

/// Copy the effective shared-memory key array into `out`.
/// Returns true while at least one keyboard owner is fresh and active.
pub fn read_keys(out: &mut [u8; 256]) -> bool {
    let Some(active) = with_state(|state, now_ms| state.read_effective_keys(now_ms, out)) else {
        out.fill(0);
        return false;
    };
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_deadline_handles_tick_count_wrap() {
        assert!(!retry_is_due(u32::MAX - 5, 10));
        assert!(retry_is_due(10, u32::MAX - 5));
        assert!(retry_is_due(10, 10));
        assert!(retry_is_due(10, 0));
    }

    #[test]
    fn reserved_scan_code_is_never_injected() {
        assert!(!is_key_pressed(255));
    }
}
