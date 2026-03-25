//! Key broadcast engine.
//!
//! Creates per-process shared memory regions (`Local\DI8_{pid}`) that the
//! trusik DLL reads to inject synthetic keystrokes into background EQ clients.

#![allow(static_mut_refs)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, MAPVK_VK_TO_VSC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
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
    keys: [u8; 256],
}

const MAGIC: u32 = 0x53544D54; // "STMT"
const SHM_SIZE: usize = std::mem::size_of::<SharedKeyState>();

/// Per-process shared memory handle.
struct ProcessShm {
    #[allow(dead_code)]
    pid: u32,
    handle: HANDLE,
    ptr: *mut SharedKeyState,
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

/// Global broadcast state. Only accessed from the main (tray message loop) thread.
static mut TARGETS: Option<HashMap<u32, ProcessShm>> = None;
static mut ACTIVE_PID: Option<u32> = None;
static mut HOOK: HHOOK = HHOOK(std::ptr::null_mut());
static BROADCASTING: AtomicBool = AtomicBool::new(false);

/// EQ process IDs for foreground check in the LL hook.
static mut EQ_PIDS: Vec<u32> = Vec::new();
/// Whether EQ was foreground on the last hook call (for clearing stuck keys).
static mut EQ_WAS_FOREGROUND: bool = false;

/// Filter configuration cached from config.
static mut FILTER_MODE: FilterMode = FilterMode::Blacklist;
static mut FILTER_SCANCODES: Vec<u8> = Vec::new();

#[derive(Clone, Copy, PartialEq)]
enum FilterMode {
    Blacklist,
    Whitelist,
}

/// Initialize the broadcast engine. Call once from main.
pub fn init() {
    unsafe {
        TARGETS = Some(HashMap::new());
        reload_filter();
    }
}

/// Reload filter configuration from disk.
fn reload_filter() {
    let cfg = Config::load();
    unsafe {
        FILTER_MODE = if cfg.broadcast_filter_mode == "whitelist" {
            FilterMode::Whitelist
        } else {
            FilterMode::Blacklist
        };
        FILTER_SCANCODES = cfg
            .broadcast_filter_keys
            .iter()
            .filter_map(|name| {
                crate::config::parse_vk_name(name).and_then(|vk| {
                    let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
                    if scan > 0 && scan < 256 {
                        Some(scan as u8)
                    } else {
                        None
                    }
                })
            })
            .collect();
    }
}

/// Check if a scan code passes the filter.
unsafe fn passes_filter(scan: u8) -> bool {
    let in_list = FILTER_SCANCODES.contains(&scan);
    match FILTER_MODE {
        FilterMode::Blacklist => !in_list,
        FilterMode::Whitelist => in_list,
    }
}

/// Cleanup the broadcast engine. Call before exit.
pub fn cleanup() {
    set_active(false);
    unsafe {
        TARGETS = None;
    }
}

/// Toggle broadcasting on/off.
pub fn toggle() {
    let currently_active = BROADCASTING.load(Ordering::SeqCst);
    set_active(!currently_active);
}

/// Returns whether broadcasting is currently active.
pub fn is_active() -> bool {
    BROADCASTING.load(Ordering::SeqCst)
}

/// Enable or disable broadcasting.
pub fn set_active(active: bool) {
    unsafe {
        if active && HOOK.0.is_null() {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), None, 0);
            match hook {
                Ok(h) => HOOK = h,
                Err(_) => return,
            }
        } else if !active && !HOOK.0.is_null() {
            let _ = UnhookWindowsHookEx(HOOK);
            HOOK = HHOOK(std::ptr::null_mut());
            // Clear all keys in all targets.
            if let Some(targets) = TARGETS.as_ref() {
                for shm in targets.values() {
                    std::ptr::write_bytes(&mut (*shm.ptr).keys as *mut u8, 0, 256);
                    std::ptr::write_volatile(&mut (*shm.ptr).seq, (*shm.ptr).seq.wrapping_add(1));
                }
            }
        }
        BROADCASTING.store(active, Ordering::SeqCst);
        // Write active flag to all shm regions.
        if let Some(targets) = TARGETS.as_ref() {
            let flag = if active { 1u32 } else { 0u32 };
            for shm in targets.values() {
                std::ptr::write_volatile(&mut (*shm.ptr).active, flag);
            }
        }
    }
}

/// Update the set of target processes. Called from overlay poll.
/// Creates/destroys shared memory regions as EQ processes come and go.
/// `hwnds` is the list of EQ window handles for the foreground check.
pub fn update_targets(pids: &[u32], active_pid: Option<u32>) {
    unsafe {
        let Some(targets) = TARGETS.as_mut() else { return };
        let active = BROADCASTING.load(Ordering::SeqCst);

        // Update EQ PIDs for the LL hook foreground check.
        EQ_PIDS.clear();
        EQ_PIDS.extend_from_slice(pids);

        // Remove shm for processes that are gone.
        targets.retain(|pid, _| pids.contains(pid));

        // Create shm for new processes.
        for &pid in pids {
            if targets.contains_key(&pid) {
                continue;
            }
            if let Some(shm) = create_shm(pid) {
                if active {
                    std::ptr::write_volatile(&mut (*shm.ptr).active, 1);
                }
                targets.insert(pid, shm);
            }
        }

        // Update suppress flags: suppress physical keys on background targets.
        ACTIVE_PID = active_pid;
        for (&pid, shm) in targets.iter() {
            let suppress = if Some(pid) == active_pid { 0u32 } else { 1u32 };
            std::ptr::write_volatile(&mut (*shm.ptr).suppress, suppress);
        }
    }
}

/// Update which process is the active (foreground) one.
pub fn set_active_pid(pid: u32) {
    unsafe {
        ACTIVE_PID = Some(pid);
        if let Some(targets) = TARGETS.as_ref() {
            for (&target_pid, shm) in targets.iter() {
                let suppress = if target_pid == pid { 0u32 } else { 1u32 };
                std::ptr::write_volatile(&mut (*shm.ptr).suppress, suppress);
            }
        }
    }
}

/// Reload filter config (called on settings change).
pub fn on_settings_changed() {
    reload_filter();
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
    std::ptr::write_volatile(&mut (*ptr).version, 1);
    // Handshake marker: trusik checks keys[255] == 0xAB on open.
    std::ptr::write_volatile(&mut (*ptr).keys[255], 0xAB);

    Some(ProcessShm {
        pid,
        handle,
        ptr,
    })
}

/// Low-level keyboard hook callback.
/// Converts VK to DIK scan code, writes to all background target shm regions.
unsafe extern "system" fn ll_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        // Only broadcast when an EQ window is foreground.
        let fg = GetForegroundWindow();
        let mut fg_pid: u32 = 0;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        let eq_is_fg = fg_pid != 0 && EQ_PIDS.contains(&fg_pid);
        if !eq_is_fg {
            // EQ lost focus — release all stuck keys in background targets.
            if EQ_WAS_FOREGROUND {
                EQ_WAS_FOREGROUND = false;
                if let Some(targets) = TARGETS.as_ref() {
                    let active_pid = ACTIVE_PID;
                    for (&pid, shm) in targets.iter() {
                        if Some(pid) == active_pid {
                            continue;
                        }
                        std::ptr::write_bytes(&mut (*shm.ptr).keys as *mut u8, 0, 256);
                        let seq = std::ptr::read_volatile(&(*shm.ptr).seq);
                        std::ptr::write_volatile(&mut (*shm.ptr).seq, seq.wrapping_add(1));
                    }
                }
            }
            return CallNextHookEx(HOOK, code, wparam, lparam);
        }
        EQ_WAS_FOREGROUND = true;

        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;
        let msg = wparam.0 as u32;

        let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) as u8;
        if scan > 0 && scan < 255 && passes_filter(scan) {
            let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let value: u8 = if pressed { 0x80 } else { 0x00 };

            if let Some(targets) = TARGETS.as_ref() {
                let active_pid = ACTIVE_PID;
                for (&pid, shm) in targets.iter() {
                    // Only broadcast to background windows (not the active one).
                    if Some(pid) == active_pid {
                        continue;
                    }
                    std::ptr::write_volatile(&mut (*shm.ptr).keys[scan as usize], value);
                    let seq = std::ptr::read_volatile(&(*shm.ptr).seq);
                    std::ptr::write_volatile(&mut (*shm.ptr).seq, seq.wrapping_add(1));
                }
            }
        }
    }
    CallNextHookEx(HOOK, code, wparam, lparam)
}
