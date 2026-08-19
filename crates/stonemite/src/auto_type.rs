//! Auto-type passwords into EQ login fields via trusik shared memory.
//!
//! Waits for DirectInput to initialize (signaled by trusik via a named event),
//! then writes key states to the DI8 shared memory. The trusik device proxy
//! injects these into DirectInput's keyboard buffer.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    OpenEventW, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC};

use crate::overlay::debug_log;

/// Shared memory layout — must match broadcast.rs / trusik key_shm.rs exactly.
#[repr(C)]
struct SharedKeyState {
    magic: u32,
    version: u32,
    /// Bitset shared by independent keyboard delivery owners.
    keyboard_active: AtomicU32,
    /// Independent Mouse Clutch activation; auto-type never writes it.
    mouse_active: AtomicU32,
    suppress: u32,
    seq: u32,
    keys: [u8; 255],
    proxy_ready: u8,
    controller_heartbeat_ms: AtomicU32,
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const VERSION: u32 = 2;
const PROXY_READY: u8 = 0xA5;
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();
const KEYBOARD_AUTO_TYPE: u32 = 1 << 2;

/// How long to hold each key down.
const KEY_DOWN_MS: u64 = 50;
/// Delay after releasing a key before pressing the next one.
const KEY_UP_MS: u64 = 50;
/// Maximum time to wait for DirectInput to initialize (ms).
const DI_WAIT_TIMEOUT_MS: u32 = 30_000;

/// Wrapper to send shm handles across threads.
struct Shm {
    handle: HANDLE,
    ptr: *mut SharedKeyState,
}
unsafe impl Send for Shm {}

/// Spawn a background thread that types `password` into the EQ process with the
/// given PID, then presses Enter to submit.
pub fn spawn(pid: u32, password: String) {
    let pw_len = password.len();
    debug_log(&format!("auto_type: spawn pid={pid} password_len={pw_len}"));

    // Create the shm immediately so trusik finds it on its first try_open().
    let shm = match open_or_create_shm(pid) {
        Ok((handle, ptr)) => Shm { handle, ptr },
        Err(e) => {
            debug_log(&format!(
                "auto_type: failed to create shm for pid={pid}: {e}"
            ));
            return;
        }
    };

    std::thread::spawn(move || {
        // Wait for trusik to signal that DirectInput is ready.
        debug_log(&format!("auto_type: waiting for DI ready event pid={pid}"));
        if !wait_for_di_ready(pid) {
            debug_log(&format!("auto_type: DI ready timeout pid={pid}"));
            cleanup_shm(shm);
            return;
        }
        debug_log(&format!(
            "auto_type: DI ready, starting type_password pid={pid}"
        ));

        // Brief pause to let DI fully settle.
        std::thread::sleep(std::time::Duration::from_millis(500));

        if let Err(e) = type_password(pid, &password, shm) {
            debug_log(&format!("auto_type: ERROR pid={pid}: {e}"));
        }
    });
}

/// Wait for the named event `Local\Stonemite_DI_{pid}` to be signaled.
/// Polls until the event exists, then waits on it.
fn wait_for_di_ready(pid: u32) -> bool {
    let name = format!("Local\\Stonemite_DI_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(DI_WAIT_TIMEOUT_MS as u64);

    loop {
        if start.elapsed() > timeout {
            debug_log(&format!(
                "auto_type: timed out waiting for DI event pid={pid}"
            ));
            return false;
        }

        let handle = unsafe {
            OpenEventW(
                SYNCHRONIZATION_SYNCHRONIZE,
                false,
                windows::core::PCWSTR(wide.as_ptr()),
            )
        };

        match handle {
            Ok(h) => {
                let remaining = timeout.saturating_sub(start.elapsed());
                debug_log(&format!("auto_type: found DI event, waiting pid={pid}"));
                let result = unsafe { WaitForSingleObject(h, remaining.as_millis() as u32) };
                unsafe {
                    let _ = CloseHandle(h);
                }
                return result.0 == 0; // WAIT_OBJECT_0
            }
            Err(_) => {
                // Event doesn't exist yet, trusik hasn't loaded.
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

fn type_password(pid: u32, password: &str, shm: Shm) -> Result<(), String> {
    let Shm { handle, ptr } = shm;
    unsafe {
        let magic = std::ptr::read_volatile(&(*ptr).magic);
        let version = std::ptr::read_volatile(&(*ptr).version);
        let proxy_ready = std::ptr::read_volatile(&(*ptr).proxy_ready);
        let mouse_active = (*ptr).mouse_active.load(Ordering::Acquire);
        let suppress = std::ptr::read_volatile(&(*ptr).suppress);
        if magic != MAGIC || version != VERSION || proxy_ready != PROXY_READY {
            close_shm(Shm { handle, ptr });
            return Err(format!(
                "incompatible trusik input mapping: magic={magic:#010x} version={version} proxy_ready={proxy_ready:#04x}"
            ));
        }

        // Auto-login owns only keyboard activation and never changes the
        // independent Mouse Clutch field (new mappings initialize it to zero).
        (*ptr)
            .keyboard_active
            .fetch_or(KEYBOARD_AUTO_TYPE, Ordering::AcqRel);
        debug_log(&format!(
            "auto_type: shm ready pid={pid} magic={magic:#010x} version={version} proxy_ready={proxy_ready:#04x} keyboard_auto_type=on mouse_active={mouse_active} suppress={suppress}"
        ));
    }

    // Give trusik's wm_activate_thread time to post WM_ACTIVATEAPP(1) and
    // EQ time to process it.  Background windows need this message to
    // enable keyboard_process before keystrokes arrive.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Type each character.
    for (i, ch) in password.chars().enumerate() {
        type_char(ptr, ch, i, pid);
    }

    // Press Enter to submit login.
    let enter_scan = vk_to_scan(0x0D);
    debug_log(&format!(
        "auto_type: pressing Enter (login) scan={enter_scan:#04x} pid={pid}"
    ));
    press_scancode(ptr, enter_scan, false);

    // Wait for the server select screen to appear, then press Enter
    // repeatedly to confirm the pre-selected server.
    debug_log(&format!(
        "auto_type: waiting 2s for server select pid={pid}"
    ));
    std::thread::sleep(std::time::Duration::from_millis(2000));

    for i in 0..3 {
        debug_log(&format!(
            "auto_type: pressing Enter (server select {}) pid={pid}",
            i + 1
        ));
        press_scancode(ptr, enter_scan, false);
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }

    debug_log(&format!("auto_type: done, deactivating shm pid={pid}"));

    // Deactivate and clean up.
    cleanup_shm(Shm { handle, ptr });

    Ok(())
}

fn cleanup_shm(shm: Shm) {
    unsafe {
        (*shm.ptr)
            .keyboard_active
            .fetch_and(!KEYBOARD_AUTO_TYPE, Ordering::AcqRel);
    }
    close_shm(shm);
}

fn close_shm(shm: Shm) {
    unsafe {
        let _ = windows::Win32::System::Memory::UnmapViewOfFile(
            windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                Value: shm.ptr as *mut c_void,
            },
        );
        let _ = CloseHandle(shm.handle);
    }
}

fn open_or_create_shm(pid: u32) -> Result<(HANDLE, *mut SharedKeyState), String> {
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    debug_log(&format!("auto_type: opening shm Local\\DI8_{pid}"));

    unsafe {
        let handle = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            SHM_SIZE as u32,
            windows::core::PCWSTR(wide.as_ptr()),
        )
        .map_err(|e| format!("CreateFileMappingW failed: {e}"))?;

        let last_error = windows::Win32::Foundation::GetLastError();
        let existed = last_error == windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        debug_log(&format!(
            "auto_type: CreateFileMappingW ok, existed={existed}"
        ));

        let view = MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, SHM_SIZE);
        let ptr = view.Value as *mut SharedKeyState;
        if ptr.is_null() {
            let _ = CloseHandle(handle);
            return Err("MapViewOfFile returned null".into());
        }

        if existed {
            // Preserve a live compatible mapping. Never zero shared state owned
            // by the broadcast controller or reinterpret an older ABI.
            let magic = std::ptr::read_volatile(&(*ptr).magic);
            let version = std::ptr::read_volatile(&(*ptr).version);
            if magic != MAGIC || version != VERSION {
                let _ = windows::Win32::System::Memory::UnmapViewOfFile(view);
                let _ = CloseHandle(handle);
                return Err(format!(
                    "existing input mapping is incompatible: magic={magic:#010x} version={version}"
                ));
            }
            debug_log("auto_type: reusing compatible shm from broadcast engine");
        } else {
            debug_log("auto_type: initializing new shm");
            std::ptr::write_bytes(ptr, 0, 1);
            std::ptr::write_volatile(&mut (*ptr).magic, MAGIC);
            std::ptr::write_volatile(&mut (*ptr).version, VERSION);
        }

        Ok((handle, ptr))
    }
}

/// Convert a VK code to a DirectInput scan code.
fn vk_to_scan(vk: u32) -> u8 {
    unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) as u8 }
}

/// Type a single character by resolving it to VK + shift state.
fn type_char(ptr: *mut SharedKeyState, ch: char, index: usize, pid: u32) {
    let result = unsafe { VkKeyScanW(ch as u16) };
    if result == -1i16 {
        debug_log(&format!(
            "auto_type: no VK mapping for char[{index}]='{ch}' pid={pid}"
        ));
        return;
    }
    let vk = (result & 0xFF) as u32;
    let shift_state = ((result >> 8) & 0xFF) as u8;
    let needs_shift = shift_state & 0x01 != 0;
    let scan = vk_to_scan(vk);
    if !is_usable_scan_code(scan) {
        debug_log(&format!(
            "auto_type: no usable scan code for char[{index}]='{ch}' vk={vk:#04x} pid={pid}"
        ));
        return;
    }

    debug_log(&format!(
        "auto_type: char[{index}] vk={vk:#04x} scan={scan:#04x} shift={needs_shift} pid={pid}"
    ));
    press_scancode(ptr, scan, needs_shift);
}

/// Press and release a scan code, optionally with Shift held.
fn press_scancode(ptr: *mut SharedKeyState, scan: u8, shift: bool) {
    if !is_usable_scan_code(scan) {
        return;
    }
    let shift_scan = vk_to_scan(0x10); // VK_SHIFT
    if shift && !is_usable_scan_code(shift_scan) {
        return;
    }

    unsafe {
        // Press Shift if needed.
        if shift {
            std::ptr::write_volatile(&mut (*ptr).keys[shift_scan as usize], 0x80);
            bump_seq(ptr);
        }

        // Press key.
        std::ptr::write_volatile(&mut (*ptr).keys[scan as usize], 0x80);
        bump_seq(ptr);

        let seq = std::ptr::read_volatile(&(*ptr).seq);
        debug_log(&format!("auto_type: key down scan={scan:#04x} seq={seq}"));

        std::thread::sleep(std::time::Duration::from_millis(KEY_DOWN_MS));

        // Release key.
        std::ptr::write_volatile(&mut (*ptr).keys[scan as usize], 0x00);
        bump_seq(ptr);

        // Release Shift.
        if shift {
            std::ptr::write_volatile(&mut (*ptr).keys[shift_scan as usize], 0x00);
            bump_seq(ptr);
        }

        // Give EQ time to see the release before the next key press.
        // Without this, repeated characters (same scan code) can be missed
        // if EQ doesn't poll between the release and re-press.
        std::thread::sleep(std::time::Duration::from_millis(KEY_UP_MS));
    }
}

fn is_usable_scan_code(scan: u8) -> bool {
    (1..=254).contains(&scan)
}

unsafe fn bump_seq(ptr: *mut SharedKeyState) {
    let seq = std::ptr::read_volatile(&(*ptr).seq);
    std::ptr::write_volatile(&mut (*ptr).seq, seq.wrapping_add(1));
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn auto_login_activation_keeps_mouse_off() {
        let shared = SharedKeyState {
            magic: MAGIC,
            version: VERSION,
            keyboard_active: AtomicU32::new(0),
            mouse_active: AtomicU32::new(0),
            suppress: 0,
            seq: 0,
            keys: [0; 255],
            proxy_ready: PROXY_READY,
            controller_heartbeat_ms: AtomicU32::new(0),
        };

        // Simulate an independently active keyboard-broadcast source.
        shared.keyboard_active.store(1, Ordering::Release);
        shared
            .keyboard_active
            .fetch_or(KEYBOARD_AUTO_TYPE, Ordering::AcqRel);
        assert_eq!(shared.keyboard_active.load(Ordering::Acquire), 5);
        assert_eq!(shared.mouse_active.load(Ordering::Acquire), 0);

        shared
            .keyboard_active
            .fetch_and(!KEYBOARD_AUTO_TYPE, Ordering::AcqRel);
        assert_eq!(shared.keyboard_active.load(Ordering::Acquire), 1);
        assert_eq!(shared.mouse_active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn shared_memory_scan_codes_exclude_reserved_boundaries() {
        assert!(!is_usable_scan_code(0));
        assert!(is_usable_scan_code(1));
        assert!(is_usable_scan_code(254));
        assert!(!is_usable_scan_code(255));
    }
}
