//! Key broadcast engine.
//!
//! Creates per-process shared memory regions (`Local\DI8_{pid}`) that the
//! trusik DLL reads to inject synthetic keystrokes into background EQ clients.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_SYSKEYDOWN,
};

use crate::config::Config;

/// Shared memory layout — must match trusik/key_shm.rs exactly.
#[repr(C)]
struct SharedKeyState {
    magic: u32,
    version: u32,
    active: u32,
    suppress: u32,
    seq: u32,
    /// DirectInput scan codes 0–254. Scan code 255 is reserved.
    keys: [u8; 255],
    /// Written by a compatible trusik proxy after it maps this region.
    proxy_ready: u8,
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const VERSION: u32 = 1;
const PROXY_READY: u8 = 0xA5;
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();

/// Per-process shared memory handle.
struct ProcessShm {
    #[allow(dead_code)] // Stored for identification/debugging.
    pid: u32,
    handle: HANDLE,
    ptr: *mut SharedKeyState,
    broadcast_keys: [bool; 256],
    targeted_keys: [bool; 256],
    targeted_active: bool,
}

impl Drop for ProcessShm {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                // Deactivate before unmapping.
                std::ptr::write_volatile(&mut (*self.ptr).active, 0);
                let _ = windows::Win32::System::Memory::UnmapViewOfFile(
                    windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.ptr as *mut c_void,
                    },
                );
            }
            if !self.handle.0.is_null() {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FilterMode {
    Blacklist,
    Whitelist,
}

/// All broadcast state, accessed only from the main (tray message loop) thread.
/// The LL keyboard hook also runs on this thread (Windows dispatches LL hooks
/// via the installing thread's message loop).
struct BroadcastState {
    targets: HashMap<u32, ProcessShm>,
    active_pid: Option<u32>,
    hook: HHOOK,
    broadcasting: bool,
    eq_pids: Vec<u32>,
    eq_was_foreground: bool,
    filter_mode: FilterMode,
    filter_scancodes: Vec<u8>,
}

struct BroadcastCell(UnsafeCell<Option<BroadcastState>>);
unsafe impl Sync for BroadcastCell {}

static STATE: BroadcastCell = BroadcastCell(UnsafeCell::new(None));

fn state() -> &'static mut Option<BroadcastState> {
    unsafe { &mut *STATE.0.get() }
}

/// Initialize the broadcast engine. Call once from main.
pub fn init() {
    let cfg = Config::load();
    let (filter_mode, filter_scancodes) = load_filter(&cfg);
    *state() = Some(BroadcastState {
        targets: HashMap::new(),
        active_pid: None,
        hook: HHOOK(std::ptr::null_mut()),
        broadcasting: false,
        eq_pids: Vec::new(),
        eq_was_foreground: false,
        filter_mode,
        filter_scancodes,
    });
}

/// Load filter configuration from a config.
fn load_filter(cfg: &Config) -> (FilterMode, Vec<u8>) {
    let mode = if cfg.broadcast_filter_mode == "whitelist" {
        FilterMode::Whitelist
    } else {
        FilterMode::Blacklist
    };
    let scancodes = cfg
        .broadcast_filter_keys
        .iter()
        .filter_map(|name| {
            crate::config::parse_vk_name(name).and_then(|vk| {
                let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) };
                if scan > 0 && scan < 256 {
                    Some(scan as u8)
                } else {
                    None
                }
            })
        })
        .collect();
    (mode, scancodes)
}

/// Check if a scan code passes the filter.
fn passes_filter(s: &BroadcastState, scan: u8) -> bool {
    let in_list = s.filter_scancodes.contains(&scan);
    match s.filter_mode {
        FilterMode::Blacklist => !in_list,
        FilterMode::Whitelist => in_list,
    }
}

/// Cleanup the broadcast engine. Call before exit.
pub fn cleanup() {
    let _ = set_active(false);
    *state() = None;
}

/// Toggle broadcasting on/off.
pub fn toggle() -> Result<bool, String> {
    let currently_active = is_active();
    set_active(!currently_active)?;
    Ok(!currently_active)
}

/// Returns whether the broadcast engine was initialized and can be enabled.
pub fn is_available() -> bool {
    state().is_some()
}

/// Returns whether broadcasting is currently active.
pub fn is_active() -> bool {
    state().as_ref().is_some_and(|s| s.broadcasting)
}

/// Returns whether the target process has acknowledged a compatible trusik proxy.
pub fn is_target_ready(pid: u32) -> bool {
    state()
        .as_ref()
        .and_then(|state| state.targets.get(&pid))
        .is_some_and(target_is_ready)
}

fn target_is_ready(target: &ProcessShm) -> bool {
    unsafe {
        std::ptr::read_volatile(&(*target.ptr).magic) == MAGIC
            && std::ptr::read_volatile(&(*target.ptr).version) == VERSION
            && std::ptr::read_volatile(&(*target.ptr).proxy_ready) == PROXY_READY
    }
}

/// Enable or disable broadcasting.
pub fn set_active(active: bool) -> Result<(), String> {
    let Some(s) = state().as_mut() else {
        return Err("broadcast engine is unavailable".to_owned());
    };
    if s.broadcasting == active {
        return Ok(());
    }
    unsafe {
        if active && s.hook.0.is_null() {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), None, 0);
            match hook {
                Ok(h) => s.hook = h,
                Err(e) => {
                    return Err(format!("failed to install keyboard hook: {e}"));
                }
            }
        } else if !active && !s.hook.0.is_null() {
            UnhookWindowsHookEx(s.hook)
                .map_err(|error| format!("failed to remove keyboard hook: {error}"))?;
            s.hook = HHOOK(std::ptr::null_mut());
            // Clear the physical-broadcast source while preserving any
            // target-specific input sequence in progress.
            for shm in s.targets.values_mut() {
                for scan in 0..255 {
                    shm.broadcast_keys[scan] = false;
                    write_combined_key(shm, scan);
                }
            }
        }
        s.broadcasting = active;
        // Write active flag to all shm regions.
        for shm in s.targets.values() {
            let flag = if active || shm.targeted_active {
                1u32
            } else {
                0u32
            };
            std::ptr::write_volatile(&mut (*shm.ptr).active, flag);
        }
    }
    Ok(())
}

/// Update the set of target processes. Called from overlay poll.
/// Creates/destroys shared memory regions as EQ processes come and go.
pub fn update_targets(pids: &[u32], active_pid: Option<u32>) {
    let Some(s) = state().as_mut() else { return };
    unsafe {
        // Update EQ PIDs for the LL hook foreground check.
        s.eq_pids.clear();
        s.eq_pids.extend_from_slice(pids);

        // Remove shm for processes that are gone.
        s.targets.retain(|pid, _| pids.contains(pid));

        // Create shm for new processes.
        for &pid in pids {
            if s.targets.contains_key(&pid) {
                continue;
            }
            if let Some(shm) = create_shm(pid) {
                if s.broadcasting {
                    std::ptr::write_volatile(&mut (*shm.ptr).active, 1);
                }
                s.targets.insert(pid, shm);
            }
        }

        // Update suppress flags: suppress physical keys on background targets.
        s.active_pid = active_pid;
        for (&pid, shm) in s.targets.iter() {
            let suppress = if Some(pid) == active_pid { 0u32 } else { 1u32 };
            std::ptr::write_volatile(&mut (*shm.ptr).suppress, suppress);
        }
    }
}

/// Update which process is the active (foreground) one.
pub fn set_active_pid(pid: u32) {
    let Some(s) = state().as_mut() else { return };
    s.active_pid = Some(pid);
    unsafe {
        for (&target_pid, shm) in s.targets.iter() {
            let suppress = if target_pid == pid { 0u32 } else { 1u32 };
            std::ptr::write_volatile(&mut (*shm.ptr).suppress, suppress);
        }
    }
}

/// Reload filter config (called on settings change).
pub fn on_settings_changed() {
    let Some(s) = state().as_mut() else { return };
    let cfg = Config::load();
    let (mode, scancodes) = load_filter(&cfg);
    s.filter_mode = mode;
    s.filter_scancodes = scancodes;
}

unsafe fn create_shm(pid: u32) -> Option<ProcessShm> {
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let handle = CreateFileMappingW(
        windows::Win32::Foundation::INVALID_HANDLE_VALUE,
        None,
        PAGE_READWRITE,
        0,
        SHM_SIZE as u32,
        windows::core::PCWSTR(wide.as_ptr()),
    )
    .ok()?;

    let view = MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, SHM_SIZE);
    let ptr = view.Value as *mut SharedKeyState;
    if ptr.is_null() {
        let _ = CloseHandle(handle);
        return None;
    }

    // Initialize.
    std::ptr::write_bytes(ptr, 0, 1);
    std::ptr::write_volatile(&mut (*ptr).magic, MAGIC);
    std::ptr::write_volatile(&mut (*ptr).version, VERSION);

    Some(ProcessShm {
        pid,
        handle,
        ptr,
        broadcast_keys: [false; 256],
        targeted_keys: [false; 256],
        targeted_active: false,
    })
}

#[derive(Debug, Eq, PartialEq)]
pub enum TargetedInputError {
    Unavailable(String),
    OperationFailed(String),
}

/// Activate direct input for one loaded process without enabling broadcasting.
/// All functions in this group are called only by the Win32 owner thread.
pub fn begin_targeted_input(pid: u32) -> Result<(), TargetedInputError> {
    let Some(s) = state().as_mut() else {
        return Err(TargetedInputError::Unavailable(
            "targeted input is unavailable because trusik is disabled".to_owned(),
        ));
    };
    let Some(shm) = s.targets.get_mut(&pid) else {
        return Err(TargetedInputError::Unavailable(
            "target process has no trusik shared-memory target".to_owned(),
        ));
    };
    if !target_is_ready(shm) {
        return Err(TargetedInputError::Unavailable(
            "target process has not acknowledged a compatible trusik proxy".to_owned(),
        ));
    }
    if shm.targeted_active {
        return Err(TargetedInputError::OperationFailed(
            "target process already has an input sequence in progress".to_owned(),
        ));
    }
    shm.targeted_active = true;
    unsafe {
        std::ptr::write_volatile(&mut (*shm.ptr).active, 1);
    }
    Ok(())
}

pub fn set_targeted_key(pid: u32, scan: u8, pressed: bool) -> Result<(), String> {
    if scan == 0 || scan == 255 {
        return Err("invalid DirectInput scan code".to_owned());
    }
    let Some(s) = state().as_mut() else {
        return Err("targeted input is unavailable because trusik is disabled".to_owned());
    };
    let Some(shm) = s.targets.get_mut(&pid) else {
        return Err("target process disappeared during input delivery".to_owned());
    };
    if !shm.targeted_active {
        return Err("targeted input sequence is not active".to_owned());
    }
    unsafe {
        shm.targeted_keys[scan as usize] = pressed;
        write_combined_key(shm, scan as usize);
    }
    Ok(())
}

pub fn finish_targeted_input(pid: u32) {
    let Some(s) = state().as_mut() else { return };
    let broadcasting = s.broadcasting;
    let Some(shm) = s.targets.get_mut(&pid) else {
        return;
    };
    unsafe {
        for scan in 0..255 {
            if shm.targeted_keys[scan] {
                shm.targeted_keys[scan] = false;
                write_combined_key(shm, scan);
            }
        }
        shm.targeted_active = false;
        std::ptr::write_volatile(&mut (*shm.ptr).active, if broadcasting { 1 } else { 0 });
    }
}

unsafe fn write_combined_key(shm: &ProcessShm, scan: usize) {
    let value = combined_key_value(shm.broadcast_keys[scan], shm.targeted_keys[scan]);
    std::ptr::write_volatile(&mut (*shm.ptr).keys[scan], value);
    let seq = std::ptr::read_volatile(&(*shm.ptr).seq);
    std::ptr::write_volatile(&mut (*shm.ptr).seq, seq.wrapping_add(1));
}

fn combined_key_value(broadcast: bool, targeted: bool) -> u8 {
    if broadcast || targeted {
        0x80
    } else {
        0x00
    }
}

/// Convert the physical scan code from the low-level hook to a DirectInput
/// keyboard offset. DirectInput marks E0-prefixed keys by setting bit 7;
/// without it, navigation keys are mistaken for their numpad counterparts.
fn direct_input_scan_code(scan_code: u32, extended: bool) -> Option<u8> {
    let scan = u8::try_from(scan_code).ok()?;
    let scan = if extended { scan | 0x80 } else { scan };
    (1..=254).contains(&scan).then_some(scan)
}

/// Low-level keyboard hook callback.
/// Converts physical scan codes to DIK offsets and writes to background targets.
unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(s) = state().as_mut() else {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    };

    if code >= 0 {
        // Only broadcast when an EQ window is foreground.
        let fg = GetForegroundWindow();
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        let eq_is_fg = fg_pid != 0 && s.eq_pids.contains(&fg_pid);
        if !eq_is_fg {
            // EQ lost focus — release all stuck keys in background targets.
            if s.eq_was_foreground {
                s.eq_was_foreground = false;
                let active_pid = s.active_pid;
                for (&pid, shm) in s.targets.iter_mut() {
                    if Some(pid) == active_pid {
                        continue;
                    }
                    for scan in 0..255 {
                        shm.broadcast_keys[scan] = false;
                        write_combined_key(shm, scan);
                    }
                }
            }
            return CallNextHookEx(s.hook, code, wparam, lparam);
        }
        s.eq_was_foreground = true;

        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let msg = wparam.0 as u32;

        if let Some(scan) = direct_input_scan_code(kb.scanCode, kb.flags.contains(LLKHF_EXTENDED))
            .filter(|&scan| passes_filter(s, scan))
        {
            let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let value: u8 = if pressed { 0x80 } else { 0x00 };

            let active_pid = s.active_pid;
            for (&pid, shm) in s.targets.iter_mut() {
                // Only broadcast to background windows (not the active one).
                if Some(pid) == active_pid {
                    continue;
                }
                shm.broadcast_keys[scan as usize] = value != 0;
                write_combined_key(shm, scan as usize);
            }
        }
    }
    CallNextHookEx(s.hook, code, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_navigation_keys_use_directinput_offsets() {
        assert_eq!(direct_input_scan_code(0x48, true), Some(0xc8));
        assert_eq!(direct_input_scan_code(0x50, true), Some(0xd0));
        assert_eq!(direct_input_scan_code(0x4b, true), Some(0xcb));
        assert_eq!(direct_input_scan_code(0x4d, true), Some(0xcd));
        assert_eq!(direct_input_scan_code(0x48, false), Some(0x48));
    }

    #[test]
    fn invalid_hook_scan_codes_are_ignored() {
        assert_eq!(direct_input_scan_code(0, false), None);
        assert_eq!(direct_input_scan_code(0x7f, true), None);
        assert_eq!(direct_input_scan_code(0x100, false), None);
    }

    #[test]
    fn broadcast_and_targeted_sources_cannot_release_each_other() {
        assert_eq!(SHM_SIZE, 276);
        assert_eq!(std::mem::offset_of!(SharedKeyState, proxy_ready), 275);
        assert_eq!(combined_key_value(false, false), 0x00);
        assert_eq!(combined_key_value(true, false), 0x80);
        assert_eq!(combined_key_value(false, true), 0x80);
        assert_eq!(combined_key_value(true, true), 0x80);
    }

    #[test]
    fn targeted_input_is_isolated_across_concurrent_mappings() {
        let target_pid = 0x7fff_ff01;
        let other_pid = 0x7fff_ff02;
        *state() = Some(BroadcastState {
            targets: HashMap::new(),
            active_pid: None,
            hook: HHOOK(std::ptr::null_mut()),
            broadcasting: false,
            eq_pids: Vec::new(),
            eq_was_foreground: false,
            filter_mode: FilterMode::Blacklist,
            filter_scancodes: Vec::new(),
        });
        update_targets(&[target_pid, other_pid], None);

        assert!(!is_target_ready(target_pid));
        assert!(matches!(
            begin_targeted_input(target_pid),
            Err(TargetedInputError::Unavailable(_))
        ));
        unsafe {
            for pid in [target_pid, other_pid] {
                std::ptr::write_volatile(
                    &mut (*state().as_mut().unwrap().targets[&pid].ptr).proxy_ready,
                    PROXY_READY,
                );
            }
        }
        assert!(is_target_ready(target_pid));
        assert!(is_target_ready(other_pid));

        begin_targeted_input(target_pid).unwrap();
        assert!(matches!(
            begin_targeted_input(target_pid),
            Err(TargetedInputError::OperationFailed(_))
        ));
        begin_targeted_input(other_pid).unwrap();
        set_targeted_key(target_pid, 0x1e, true).unwrap();
        set_targeted_key(other_pid, 0x1f, true).unwrap();
        let broadcast_state = state().as_ref().unwrap();
        let target = &broadcast_state.targets[&target_pid];
        let other = &broadcast_state.targets[&other_pid];
        unsafe {
            assert_eq!(std::ptr::read_volatile(&(*target.ptr).active), 1);
            assert_eq!(std::ptr::read_volatile(&(*target.ptr).keys[0x1e]), 0x80);
            assert_eq!(std::ptr::read_volatile(&(*target.ptr).keys[0x1f]), 0x00);
            assert_eq!(std::ptr::read_volatile(&(*other.ptr).active), 1);
            assert_eq!(std::ptr::read_volatile(&(*other.ptr).keys[0x1e]), 0x00);
            assert_eq!(std::ptr::read_volatile(&(*other.ptr).keys[0x1f]), 0x80);
        }

        set_targeted_key(target_pid, 0x1e, false).unwrap();
        finish_targeted_input(target_pid);
        let broadcast_state = state().as_ref().unwrap();
        let target = &broadcast_state.targets[&target_pid];
        let other = &broadcast_state.targets[&other_pid];
        unsafe {
            assert_eq!(std::ptr::read_volatile(&(*target.ptr).active), 0);
            assert_eq!(std::ptr::read_volatile(&(*target.ptr).keys[0x1e]), 0x00);
            assert_eq!(std::ptr::read_volatile(&(*other.ptr).active), 1);
            assert_eq!(std::ptr::read_volatile(&(*other.ptr).keys[0x1f]), 0x80);
        }
        set_targeted_key(other_pid, 0x1f, false).unwrap();
        finish_targeted_input(other_pid);
        *state() = None;
    }
}
