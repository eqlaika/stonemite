//! Key broadcast engine.
//!
//! Creates per-process shared memory regions (`Local\DI8_{pid}`) that the
//! trusik DLL reads to inject synthetic keystrokes into background EQ clients.

use std::cell::UnsafeCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use trushar::control::{MouseClutchOperation, MouseClutchOwner};
use trusik_protocol::{
    SharedKeyState, KEYBOARD_BROADCAST, KEYBOARD_TARGETED, SHARED_KEY_STATE_SIZE,
};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, LRESULT, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, MAPVK_VK_TO_VSC, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
    VK_XBUTTON1, VK_XBUTTON2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::config::Config;

/// Per-process shared memory handle.
struct ProcessShm {
    #[allow(dead_code)] // Stored for identification/debugging.
    pid: u32,
    handle: HANDLE,
    process_handle: Option<HANDLE>,
    ptr: *mut SharedKeyState,
    broadcast_keys: [bool; 256],
    targeted_keys: [bool; 256],
    targeted_active: bool,
    mouse_selected: bool,
}

impl Drop for ProcessShm {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                // Retire only controller-owned input; auto-type has an
                // independent buffer and lease.
                (*self.ptr).retire_controller();
                let _ = windows::Win32::System::Memory::UnmapViewOfFile(
                    windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.ptr as *mut c_void,
                    },
                );
            }
            if !self.handle.0.is_null() {
                let _ = CloseHandle(self.handle);
            }
            if let Some(process_handle) = self.process_handle {
                let _ = CloseHandle(process_handle);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FilterMode {
    Blacklist,
    Whitelist,
}

const MOUSE_LEFT: u8 = 1 << 0;
const MOUSE_RIGHT: u8 = 1 << 1;
const MOUSE_MIDDLE: u8 = 1 << 2;
const MOUSE_X1: u8 = 1 << 3;
const MOUSE_X2: u8 = 1 << 4;
/// Three background polls at the measured 33–36 Hz, with a small margin.
const MOUSE_DRAIN_MS: u64 = 90;
/// Remote controls renew every 500 ms; expire quickly after a lost connection.
pub const REMOTE_MOUSE_CLUTCH_LEASE_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClutchPhase {
    Inactive,
    Active,
    Releasing,
    Draining,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseClutchStatus {
    Inactive,
    Active,
    Releasing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseClutchAvailability {
    Ready,
    NoActiveClient,
    NoCompatibleTargets,
    InputUnavailable,
}

#[derive(Debug, Eq, PartialEq)]
pub enum MouseClutchControlError {
    Unavailable(String),
    NotReady(String),
    HoldExpired(String),
    OperationFailed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClutchKeyEffect {
    Pass,
    Swallow,
    Activate,
}

#[derive(Debug)]
struct MouseClutch {
    phase: ClutchPhase,
    buttons: u8,
    swallowed_vk: Option<u32>,
    local_owner_active: bool,
    source_pid: Option<u32>,
    drain_deadline_ms: u64,
}

impl MouseClutch {
    fn new(buttons: u8) -> Self {
        Self {
            phase: ClutchPhase::Inactive,
            buttons,
            swallowed_vk: None,
            local_owner_active: false,
            source_pid: None,
            drain_deadline_ms: 0,
        }
    }

    fn status(&self) -> MouseClutchStatus {
        match self.phase {
            ClutchPhase::Inactive => MouseClutchStatus::Inactive,
            ClutchPhase::Active => MouseClutchStatus::Active,
            ClutchPhase::Releasing | ClutchPhase::Draining => MouseClutchStatus::Releasing,
        }
    }

    fn on_key_down(&mut self, vk: u32, source_pid: u32, has_targets: bool) -> ClutchKeyEffect {
        if self.swallowed_vk == Some(vk) {
            return ClutchKeyEffect::Swallow;
        }
        self.swallowed_vk = Some(vk);
        if !self.activate(source_pid, has_targets) {
            return ClutchKeyEffect::Swallow;
        }
        self.local_owner_active = true;
        ClutchKeyEffect::Activate
    }

    fn on_key_up(&mut self, vk: u32, now_ms: u64, remote_owner_active: bool) -> ClutchKeyEffect {
        if self.swallowed_vk != Some(vk) {
            return ClutchKeyEffect::Pass;
        }
        self.swallowed_vk = None;
        self.local_owner_active = false;
        self.release_if_unowned(now_ms, remote_owner_active);
        ClutchKeyEffect::Swallow
    }

    fn activate(&mut self, source_pid: u32, has_targets: bool) -> bool {
        if self.phase == ClutchPhase::Active {
            return self.source_pid == Some(source_pid);
        }
        if self.buttons != 0 || !has_targets {
            return false;
        }
        self.phase = ClutchPhase::Active;
        self.source_pid = Some(source_pid);
        self.drain_deadline_ms = 0;
        true
    }

    fn release_if_unowned(&mut self, now_ms: u64, remote_owner_active: bool) {
        if self.local_owner_active || remote_owner_active || self.phase != ClutchPhase::Active {
            return;
        }
        if self.buttons == 0 {
            self.begin_drain(now_ms);
        } else {
            self.phase = ClutchPhase::Releasing;
        }
    }

    fn set_button(&mut self, button: u8, pressed: bool, now_ms: u64) {
        if pressed {
            self.buttons |= button;
        } else {
            self.buttons &= !button;
        }
        if self.phase == ClutchPhase::Releasing && self.buttons == 0 {
            self.begin_drain(now_ms);
        }
    }

    fn cancel_bounded(&mut self, now_ms: u64) {
        self.local_owner_active = false;
        if matches!(self.phase, ClutchPhase::Active | ClutchPhase::Releasing) {
            self.begin_drain(now_ms);
        }
    }

    fn begin_drain(&mut self, now_ms: u64) {
        self.phase = ClutchPhase::Draining;
        self.drain_deadline_ms = now_ms.saturating_add(MOUSE_DRAIN_MS);
    }

    fn finish_drain_if_due(&mut self, now_ms: u64) -> bool {
        if self.phase == ClutchPhase::Draining && now_ms >= self.drain_deadline_ms {
            self.force_inactive();
            return true;
        }
        false
    }

    fn force_inactive(&mut self) {
        self.phase = ClutchPhase::Inactive;
        self.local_owner_active = false;
        self.source_pid = None;
        self.drain_deadline_ms = 0;
    }
}

#[derive(Clone, Copy, Debug)]
struct RemoteMouseHold {
    source_pid: u32,
    deadline_ms: u64,
}

/// All broadcast state, accessed only from the main (tray message loop) thread.
/// The LL keyboard hook also runs on this thread (Windows dispatches LL hooks
/// via the installing thread's message loop).
struct BroadcastState {
    targets: HashMap<u32, ProcessShm>,
    active_pid: Option<u32>,
    keyboard_hook: HHOOK,
    mouse_hook: HHOOK,
    broadcasting: bool,
    clutch_binding_vk: Option<u32>,
    pending_clutch_binding: Option<Option<u32>>,
    clutch_key_is_down: bool,
    clutch: MouseClutch,
    remote_mouse_holds: HashMap<MouseClutchOwner, RemoteMouseHold>,
    remote_mouse_sequences: HashMap<MouseClutchOwner, u64>,
    closed_remote_sessions: HashMap<u64, u64>,
    closed_remote_session_order: VecDeque<u64>,
    clutch_status_dirty: bool,
    clutch_error: Option<String>,
    eq_pids: Vec<u32>,
    mouse_eligible_pids: Vec<u32>,
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
    let (clutch_binding_vk, clutch_error) = match cfg.mouse_clutch_vk() {
        Ok(binding) => (binding, None),
        Err(error) => (None, Some(error)),
    };
    *state() = Some(BroadcastState {
        targets: HashMap::new(),
        active_pid: None,
        keyboard_hook: HHOOK(std::ptr::null_mut()),
        mouse_hook: HHOOK(std::ptr::null_mut()),
        broadcasting: false,
        clutch_binding_vk,
        pending_clutch_binding: None,
        clutch_key_is_down: clutch_binding_vk.is_some_and(key_is_down),
        clutch: MouseClutch::new(sample_mouse_buttons()),
        remote_mouse_holds: HashMap::new(),
        remote_mouse_sequences: HashMap::new(),
        closed_remote_sessions: HashMap::new(),
        closed_remote_session_order: VecDeque::new(),
        clutch_status_dirty: false,
        clutch_error,
        eq_pids: Vec::new(),
        mouse_eligible_pids: Vec::new(),
        eq_was_foreground: false,
        filter_mode,
        filter_scancodes,
    });
    if let Some(s) = state().as_mut() {
        if let Err(error) = reconcile_hooks(s) {
            s.clutch_binding_vk = None;
            s.clutch_error = Some(error);
            let _ = reconcile_hooks(s);
        }
    }
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

fn key_is_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

fn sample_mouse_buttons() -> u8 {
    unsafe {
        let mut buttons = 0;
        for (vk, button) in [
            (VK_LBUTTON, MOUSE_LEFT),
            (VK_RBUTTON, MOUSE_RIGHT),
            (VK_MBUTTON, MOUSE_MIDDLE),
            (VK_XBUTTON1, MOUSE_X1),
            (VK_XBUTTON2, MOUSE_X2),
        ] {
            if GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 {
                buttons |= button;
            }
        }
        buttons
    }
}

fn keyboard_hook_needed(s: &BroadcastState) -> bool {
    s.broadcasting
        || s.clutch_binding_vk.is_some()
        || s.pending_clutch_binding.is_some()
        || s.clutch.swallowed_vk.is_some()
}

fn mouse_hook_needed(s: &BroadcastState) -> bool {
    s.clutch_binding_vk.is_some()
        || s.pending_clutch_binding.is_some()
        || !s.remote_mouse_holds.is_empty()
        || s.clutch.phase != ClutchPhase::Inactive
}

fn reconcile_hooks(s: &mut BroadcastState) -> Result<(), String> {
    unsafe {
        if keyboard_hook_needed(s) && s.keyboard_hook.0.is_null() {
            s.keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard_proc), None, 0)
                .map_err(|error| format!("failed to install keyboard hook: {error}"))?;
        }
        if mouse_hook_needed(s) && s.mouse_hook.0.is_null() {
            match SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse_proc), None, 0) {
                Ok(hook) => s.mouse_hook = hook,
                Err(error) => {
                    return Err(format!("failed to install mouse hook: {error}"));
                }
            }
            s.clutch.buttons = sample_mouse_buttons();
        }

        let mut removal_error = None;
        if !mouse_hook_needed(s) && !s.mouse_hook.0.is_null() {
            match UnhookWindowsHookEx(s.mouse_hook) {
                Ok(()) => s.mouse_hook = HHOOK(std::ptr::null_mut()),
                Err(error) => {
                    removal_error = Some(format!("failed to remove mouse hook: {error}"));
                }
            }
        }
        if !keyboard_hook_needed(s) && !s.keyboard_hook.0.is_null() {
            match UnhookWindowsHookEx(s.keyboard_hook) {
                Ok(()) => s.keyboard_hook = HHOOK(std::ptr::null_mut()),
                Err(error) => {
                    removal_error = Some(format!("failed to remove keyboard hook: {error}"));
                }
            }
        }
        removal_error.map_or(Ok(()), Err)
    }
}

unsafe fn set_keyboard_source(target: &ProcessShm, source: u32, active: bool) {
    if active {
        (*target.ptr)
            .controller_keyboard_active
            .fetch_or(source, Ordering::AcqRel);
    } else {
        (*target.ptr)
            .controller_keyboard_active
            .fetch_and(!source, Ordering::AcqRel);
    }
}

unsafe fn deactivate_mouse_targets(s: &mut BroadcastState) {
    for target in s.targets.values_mut() {
        if target.mouse_selected {
            target.mouse_selected = false;
            (*target.ptr)
                .controller_mouse_active
                .store(0, Ordering::Release);
        }
    }
}

fn has_ready_mouse_targets(s: &BroadcastState, source_pid: u32) -> bool {
    s.mouse_eligible_pids.contains(&source_pid)
        && s.targets.iter().any(|(&pid, target)| {
            pid != source_pid && s.mouse_eligible_pids.contains(&pid) && target_is_ready(target)
        })
}

fn cancel_all_clutch_owners(s: &mut BroadcastState, now_ms: u64) {
    if !s.remote_mouse_holds.is_empty() {
        s.remote_mouse_holds.clear();
    }
    s.clutch.cancel_bounded(now_ms);
    s.clutch_status_dirty = true;
}

unsafe fn activate_mouse_targets(s: &mut BroadcastState, source_pid: u32) -> bool {
    if !has_ready_mouse_targets(s, source_pid) {
        return false;
    }

    // Activate the new snapshot before retiring any previous drain snapshot so
    // a clutch re-press never creates a global inactive gap.
    for (&pid, target) in s.targets.iter_mut() {
        if pid != source_pid && s.mouse_eligible_pids.contains(&pid) && target_is_ready(target) {
            target.mouse_selected = true;
            (*target.ptr)
                .controller_mouse_active
                .store(1, Ordering::Release);
        }
    }
    for (&pid, target) in s.targets.iter_mut() {
        if target.mouse_selected
            && (pid == source_pid
                || !s.mouse_eligible_pids.contains(&pid)
                || !target_is_ready(target))
        {
            target.mouse_selected = false;
            (*target.ptr)
                .controller_mouse_active
                .store(0, Ordering::Release);
        }
    }
    true
}

/// Cleanup the broadcast engine. Call before exit.
pub fn cleanup() {
    let _ = set_active(false);
    if let Some(s) = state().as_mut() {
        unsafe { deactivate_mouse_targets(s) };
        s.remote_mouse_holds.clear();
        s.clutch.force_inactive();
        s.clutch.swallowed_vk = None;
        s.clutch_binding_vk = None;
        s.pending_clutch_binding = None;
        let _ = reconcile_hooks(s);
    }
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

pub fn mouse_clutch_status() -> MouseClutchStatus {
    state()
        .as_ref()
        .map_or(MouseClutchStatus::Inactive, |s| s.clutch.status())
}

pub fn mouse_clutch_error() -> Option<String> {
    state().as_ref().and_then(|s| s.clutch_error.clone())
}

fn mouse_clutch_availability_for(s: &BroadcastState) -> MouseClutchAvailability {
    let Some(source_pid) =
        real_foreground_pid().filter(|pid| s.active_pid == Some(*pid) && s.eq_pids.contains(pid))
    else {
        return MouseClutchAvailability::NoActiveClient;
    };
    if has_ready_mouse_targets(s, source_pid) {
        MouseClutchAvailability::Ready
    } else {
        MouseClutchAvailability::NoCompatibleTargets
    }
}

pub fn mouse_clutch_availability() -> MouseClutchAvailability {
    state().as_ref().map_or(
        MouseClutchAvailability::InputUnavailable,
        mouse_clutch_availability_for,
    )
}

pub fn update_remote_mouse_clutch_hold(
    owner: MouseClutchOwner,
    operation: MouseClutchOperation,
    sequence: u64,
) -> Result<bool, MouseClutchControlError> {
    let Some(s) = state().as_mut() else {
        return Err(MouseClutchControlError::Unavailable(
            "Mouse Clutch is unavailable because trusik is disabled".to_owned(),
        ));
    };
    let session_id = owner.session_id();
    if s.closed_remote_sessions.contains_key(&session_id) {
        return Err(MouseClutchControlError::HoldExpired(
            "the Mouse Clutch connection is no longer active".to_owned(),
        ));
    }
    let previous_sequence = s.remote_mouse_sequences.get(&owner).copied().unwrap_or(0);
    if sequence <= previous_sequence {
        return Ok(s.remote_mouse_holds.contains_key(&owner));
    }
    s.remote_mouse_sequences.insert(owner.clone(), sequence);

    let now_ms = unsafe { GetTickCount64() };
    match operation {
        MouseClutchOperation::Begin => {
            if let Some(hold) = s.remote_mouse_holds.get_mut(&owner) {
                hold.deadline_ms = now_ms.saturating_add(REMOTE_MOUSE_CLUTCH_LEASE_MS);
                return Ok(true);
            }
            let source_pid = match mouse_clutch_availability_for(s) {
                MouseClutchAvailability::Ready => real_foreground_pid().expect("ready source"),
                MouseClutchAvailability::InputUnavailable => {
                    return Err(MouseClutchControlError::Unavailable(
                        "Mouse Clutch is unavailable because trusik is disabled".to_owned(),
                    ));
                }
                MouseClutchAvailability::NoActiveClient => {
                    return Err(MouseClutchControlError::NotReady(
                        "Mouse Clutch needs a foreground EverQuest client".to_owned(),
                    ));
                }
                MouseClutchAvailability::NoCompatibleTargets => {
                    return Err(MouseClutchControlError::NotReady(
                        "Mouse Clutch needs a compatible input-ready background client".to_owned(),
                    ));
                }
            };
            let resuming_active_source =
                s.clutch.phase == ClutchPhase::Active && s.clutch.source_pid == Some(source_pid);
            s.remote_mouse_holds.insert(
                owner.clone(),
                RemoteMouseHold {
                    source_pid,
                    deadline_ms: now_ms.saturating_add(REMOTE_MOUSE_CLUTCH_LEASE_MS),
                },
            );
            if let Err(error) = reconcile_hooks(s) {
                s.remote_mouse_holds.remove(&owner);
                let _ = reconcile_hooks(s);
                return Err(MouseClutchControlError::OperationFailed(error));
            }
            if !resuming_active_source {
                s.clutch.buttons = sample_mouse_buttons();
                if !s.clutch.activate(source_pid, true) {
                    s.remote_mouse_holds.remove(&owner);
                    let _ = reconcile_hooks(s);
                    return Err(MouseClutchControlError::NotReady(
                        "release every mouse button before engaging Mouse Clutch".to_owned(),
                    ));
                }
                if !unsafe { activate_mouse_targets(s, source_pid) } {
                    s.remote_mouse_holds.remove(&owner);
                    s.clutch.release_if_unowned(now_ms, false);
                    let _ = reconcile_hooks(s);
                    return Err(MouseClutchControlError::NotReady(
                        "Mouse Clutch has no compatible input-ready background target".to_owned(),
                    ));
                }
            }
            s.clutch_status_dirty = true;
            Ok(true)
        }
        MouseClutchOperation::Renew => {
            let Some(hold) = s.remote_mouse_holds.get_mut(&owner) else {
                return Err(MouseClutchControlError::HoldExpired(
                    "the Mouse Clutch hold is no longer active".to_owned(),
                ));
            };
            if s.clutch.phase != ClutchPhase::Active || s.clutch.source_pid != Some(hold.source_pid)
            {
                s.remote_mouse_holds.remove(&owner);
                return Err(MouseClutchControlError::HoldExpired(
                    "the Mouse Clutch hold was canceled".to_owned(),
                ));
            }
            hold.deadline_ms = now_ms.saturating_add(REMOTE_MOUSE_CLUTCH_LEASE_MS);
            Ok(true)
        }
        MouseClutchOperation::End => {
            let removed = s.remote_mouse_holds.remove(&owner).is_some();
            if removed {
                let remote_owner_active = !s.remote_mouse_holds.is_empty();
                s.clutch.release_if_unowned(now_ms, remote_owner_active);
                s.clutch_status_dirty = true;
                let _ = reconcile_hooks(s);
            }
            Ok(false)
        }
    }
}

pub fn end_remote_mouse_clutch_session(session_id: u64, sequence: u64) {
    let Some(s) = state().as_mut() else { return };
    if !s.closed_remote_sessions.contains_key(&session_id) {
        s.closed_remote_session_order.push_back(session_id);
    }
    s.closed_remote_sessions.insert(session_id, sequence);
    let before = s.remote_mouse_holds.len();
    s.remote_mouse_holds
        .retain(|owner, _| owner.session_id() != session_id);
    s.remote_mouse_sequences
        .retain(|owner, _| owner.session_id() != session_id);
    while s.closed_remote_session_order.len() > 256 {
        if let Some(expired) = s.closed_remote_session_order.pop_front() {
            s.closed_remote_sessions.remove(&expired);
        }
    }
    if s.remote_mouse_holds.len() != before {
        let now_ms = unsafe { GetTickCount64() };
        let remote_owner_active = !s.remote_mouse_holds.is_empty();
        s.clutch.release_if_unowned(now_ms, remote_owner_active);
        s.clutch_status_dirty = true;
        let _ = reconcile_hooks(s);
    }
}

/// Returns whether the target process has acknowledged a compatible trusik proxy.
pub fn is_target_ready(pid: u32) -> bool {
    state()
        .as_ref()
        .and_then(|state| state.targets.get(&pid))
        .is_some_and(target_is_ready)
}

fn refresh_controller_heartbeat(s: &BroadcastState, now_ms: u64) {
    for target in s.targets.values() {
        unsafe {
            (*target.ptr).refresh_controller_heartbeat(now_ms as u32);
        }
    }
}

fn target_is_ready(target: &ProcessShm) -> bool {
    unsafe { (*target.ptr).proxy_is_ready() }
}

fn target_process_alive(target: &ProcessShm) -> bool {
    target
        .process_handle
        .is_none_or(|handle| unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT })
}

/// Enable or disable keyboard broadcasting.
pub fn set_active(active: bool) -> Result<(), String> {
    let Some(s) = state().as_mut() else {
        return Err("broadcast engine is unavailable".to_owned());
    };
    if s.broadcasting == active {
        return Ok(());
    }

    s.broadcasting = active;
    let hook_result = reconcile_hooks(s);
    if active {
        if let Err(error) = &hook_result {
            let error = error.clone();
            s.broadcasting = false;
            let _ = reconcile_hooks(s);
            return Err(error);
        }
    }

    unsafe {
        if !active {
            // Clear only the physical-broadcast source while preserving any
            // target-specific input sequence in progress.
            for shm in s.targets.values_mut() {
                for scan in 0..255 {
                    shm.broadcast_keys[scan] = false;
                    write_combined_key(shm, scan);
                }
            }
        }
        for shm in s.targets.values() {
            set_keyboard_source(shm, KEYBOARD_BROADCAST, active);
        }
    }
    hook_result
}

/// Update the set of target processes. Called from overlay poll.
/// Creates/destroys shared memory regions as EQ processes come and go.
pub fn update_targets(pids: &[u32], active_pid: Option<u32>) {
    let Some(s) = state().as_mut() else { return };
    unsafe {
        let source_disappeared = s.clutch.source_pid.is_some_and(|pid| !pids.contains(&pid));
        let selected_target_disappeared = s
            .targets
            .iter()
            .any(|(pid, target)| target.mouse_selected && !pids.contains(pid));
        if source_disappeared || selected_target_disappeared {
            cancel_all_clutch_owners(s, GetTickCount64());
        }

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
                    set_keyboard_source(&shm, KEYBOARD_BROADCAST, true);
                }
                s.targets.insert(pid, shm);
            }
        }

        // Update suppress flags: suppress physical keys on background targets.
        s.active_pid = active_pid;
        for (&pid, shm) in s.targets.iter() {
            let suppress = if Some(pid) == active_pid { 0u32 } else { 1u32 };
            (*shm.ptr).suppress.store(suppress, Ordering::Release);
        }
    }
}

/// Update the same-origin/client-size/DPI compatibility set supplied by the
/// overlay. A selected target leaving this set cancels the current clutch.
pub fn update_mouse_eligible_pids(pids: &[u32]) {
    let Some(s) = state().as_mut() else { return };
    let selected_became_ineligible = s
        .targets
        .iter()
        .any(|(pid, target)| target.mouse_selected && !pids.contains(pid));
    s.mouse_eligible_pids.clear();
    s.mouse_eligible_pids.extend_from_slice(pids);
    if selected_became_ineligible {
        cancel_all_clutch_owners(s, unsafe { GetTickCount64() });
    }
}

/// Update which process is the active (foreground) one.
pub fn set_active_pid(pid: u32) {
    let Some(s) = state().as_mut() else { return };
    if s.clutch.source_pid.is_some_and(|source| source != pid) {
        cancel_all_clutch_owners(s, unsafe { GetTickCount64() });
    }
    s.active_pid = Some(pid);
    unsafe {
        for (&target_pid, shm) in s.targets.iter() {
            let suppress = if target_pid == pid { 0u32 } else { 1u32 };
            (*shm.ptr).suppress.store(suppress, Ordering::Release);
        }
    }
}

fn real_foreground_pid() -> Option<u32> {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

fn apply_pending_clutch_binding(s: &mut BroadcastState) {
    if s.clutch.phase != ClutchPhase::Inactive || s.clutch.swallowed_vk.is_some() {
        return;
    }
    let Some(binding) = s.pending_clutch_binding.take() else {
        return;
    };
    s.clutch_binding_vk = binding;
    s.clutch_key_is_down = binding.is_some_and(key_is_down);
    if let Err(error) = reconcile_hooks(s) {
        s.clutch_binding_vk = None;
        s.clutch_error = Some(error);
        unsafe { deactivate_mouse_targets(s) };
        s.clutch.force_inactive();
        let _ = reconcile_hooks(s);
    }
}

/// Advance focus/lifecycle validation and the bounded release drain. Returns
/// true when an overlay/tray indicator should be refreshed.
pub fn tick() -> bool {
    let Some(s) = state().as_mut() else {
        return false;
    };
    let now_ms = unsafe { GetTickCount64() };
    refresh_controller_heartbeat(s, now_ms);
    let before = s.clutch.status();

    let hold_count = s.remote_mouse_holds.len();
    s.remote_mouse_holds
        .retain(|_, hold| now_ms < hold.deadline_ms);
    if s.remote_mouse_holds.len() != hold_count {
        let remote_owner_active = !s.remote_mouse_holds.is_empty();
        s.clutch.release_if_unowned(now_ms, remote_owner_active);
        s.clutch_status_dirty = true;
    }

    if s.clutch.phase != ClutchPhase::Inactive {
        let source_is_foreground = s.clutch.source_pid.is_some()
            && s.clutch.source_pid == real_foreground_pid()
            && s.clutch
                .source_pid
                .is_some_and(|pid| s.eq_pids.contains(&pid));
        let selected_targets_ready = s
            .targets
            .values()
            .filter(|target| target.mouse_selected)
            .all(|target| target_is_ready(target) && target_process_alive(target));
        let has_selected_target = s.targets.values().any(|target| target.mouse_selected);
        if !source_is_foreground || !selected_targets_ready || !has_selected_target {
            cancel_all_clutch_owners(s, now_ms);
        }
    }

    if s.clutch.finish_drain_if_due(now_ms) {
        unsafe { deactivate_mouse_targets(s) };
    }
    apply_pending_clutch_binding(s);
    if let Err(error) = reconcile_hooks(s) {
        s.clutch_error = Some(error);
    }

    let dirty = s.clutch_status_dirty || before != s.clutch.status();
    s.clutch_status_dirty = false;
    dirty
}

/// Reload filter and clutch configuration. An engaged clutch drains before
/// the new binding takes effect; the old swallowed key-up remains tracked.
pub fn on_settings_changed() {
    let Some(s) = state().as_mut() else { return };
    let cfg = Config::load();
    let (mode, scancodes) = load_filter(&cfg);
    s.filter_mode = mode;
    s.filter_scancodes = scancodes;

    let new_binding = match cfg.mouse_clutch_vk() {
        Ok(binding) => {
            s.clutch_error = None;
            binding
        }
        Err(error) => {
            s.clutch_error = Some(error);
            None
        }
    };
    s.pending_clutch_binding = Some(new_binding);
    cancel_all_clutch_owners(s, unsafe { GetTickCount64() });
    apply_pending_clutch_binding(s);
}

unsafe fn create_shm(pid: u32) -> Option<ProcessShm> {
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    let handle = CreateFileMappingW(
        windows::Win32::Foundation::INVALID_HANDLE_VALUE,
        None,
        PAGE_READWRITE,
        0,
        SHARED_KEY_STATE_SIZE as u32,
        windows::core::PCWSTR(wide.as_ptr()),
    )
    .ok()?;
    let existed = GetLastError() == ERROR_ALREADY_EXISTS;

    let view = MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, SHARED_KEY_STATE_SIZE);
    let ptr = view.Value as *mut SharedKeyState;
    if ptr.is_null() {
        let _ = CloseHandle(handle);
        return None;
    }

    if existed {
        // Reuse only the exact ABI and retire stale controller-owned state.
        // The separately leased auto-type source must remain untouched.
        if !(*ptr).is_compatible() {
            let _ = windows::Win32::System::Memory::UnmapViewOfFile(view);
            let _ = CloseHandle(handle);
            return None;
        }
        (*ptr).retire_controller();
    } else {
        SharedKeyState::initialize(ptr);
    }
    (*ptr).refresh_controller_heartbeat(GetTickCount64() as u32);

    Some(ProcessShm {
        pid,
        handle,
        process_handle: OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok(),
        ptr,
        broadcast_keys: [false; 256],
        targeted_keys: [false; 256],
        targeted_active: false,
        mouse_selected: false,
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
    unsafe { set_keyboard_source(shm, KEYBOARD_TARGETED, true) }
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
        set_keyboard_source(shm, KEYBOARD_TARGETED, false);
    }
}

unsafe fn write_combined_key(shm: &ProcessShm, scan: usize) {
    let value = combined_key_value(shm.broadcast_keys[scan], shm.targeted_keys[scan]);
    (*shm.ptr).set_controller_key(scan, value);
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

fn mouse_button_message(message: u32, mouse_data: u32) -> Option<(u8, bool)> {
    match message {
        WM_LBUTTONDOWN => Some((MOUSE_LEFT, true)),
        WM_LBUTTONUP => Some((MOUSE_LEFT, false)),
        WM_RBUTTONDOWN => Some((MOUSE_RIGHT, true)),
        WM_RBUTTONUP => Some((MOUSE_RIGHT, false)),
        WM_MBUTTONDOWN => Some((MOUSE_MIDDLE, true)),
        WM_MBUTTONUP => Some((MOUSE_MIDDLE, false)),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let button = match (mouse_data >> 16) & 0xffff {
                1 => MOUSE_X1,
                2 => MOUSE_X2,
                _ => return None,
            };
            Some((button, message == WM_XBUTTONDOWN))
        }
        _ => None,
    }
}

/// Low-level mouse hook used only for physical button level/lifecycle tracking.
/// It never suppresses, copies, logs, allocates, or synthesizes mouse input, and
/// intentionally accepts Moonlight/Vibepollo events marked as injected.
unsafe extern "system" fn ll_mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(s) = state().as_mut() else {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    };
    if code >= 0 {
        let mouse = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        if let Some((button, pressed)) = mouse_button_message(wparam.0 as u32, mouse.mouseData) {
            let before = s.clutch.status();
            s.clutch.set_button(button, pressed, GetTickCount64());
            s.clutch_status_dirty |= before != s.clutch.status();
        }
    }
    CallNextHookEx(s.mouse_hook, code, wparam, lparam)
}

/// Low-level keyboard hook callback. Mouse Clutch is handled before keyboard
/// Broadcast so the bound key never reaches EQ or synthetic keyboard delivery.
unsafe extern "system" fn ll_keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(s) = state().as_mut() else {
        return CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam);
    };

    if code >= 0 {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let message = wparam.0 as u32;
        let key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        let key_up = message == WM_KEYUP || message == WM_SYSKEYUP;

        // Matching repeats/up events stay swallowed even after focus,
        // settings, source, or target changes.
        if key_down && s.clutch.swallowed_vk == Some(kb.vkCode) {
            return LRESULT(1);
        }
        if key_up && s.clutch.swallowed_vk == Some(kb.vkCode) {
            s.clutch_key_is_down = false;
            let before = s.clutch.status();
            let remote_owner_active = !s.remote_mouse_holds.is_empty();
            let _ = s
                .clutch
                .on_key_up(kb.vkCode, GetTickCount64(), remote_owner_active);
            s.clutch_status_dirty |= before != s.clutch.status();
            return LRESULT(1);
        }

        let foreground_pid = real_foreground_pid();
        let eq_is_foreground = foreground_pid.is_some_and(|pid| s.eq_pids.contains(&pid));
        if key_up && s.clutch_binding_vk == Some(kb.vkCode) {
            s.clutch_key_is_down = false;
        }
        if key_down && s.pending_clutch_binding.is_none() && s.clutch_binding_vk == Some(kb.vkCode)
        {
            if s.clutch_key_is_down {
                return CallNextHookEx(s.keyboard_hook, code, wparam, lparam);
            }
            s.clutch_key_is_down = true;
            if eq_is_foreground {
                let source_pid = foreground_pid.unwrap_or_default();
                let has_targets = has_ready_mouse_targets(s, source_pid);
                let before = s.clutch.status();
                if s.clutch.on_key_down(kb.vkCode, source_pid, has_targets)
                    == ClutchKeyEffect::Activate
                {
                    let _ = activate_mouse_targets(s, source_pid);
                }
                s.clutch_status_dirty |= before != s.clutch.status();
                return LRESULT(1);
            }
        }

        if !eq_is_foreground {
            // EQ lost focus — release all stuck keyboard-broadcast keys.
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
            return CallNextHookEx(s.keyboard_hook, code, wparam, lparam);
        }
        s.eq_was_foreground = true;

        if s.broadcasting {
            if let Some(scan) =
                direct_input_scan_code(kb.scanCode, kb.flags.contains(LLKHF_EXTENDED))
                    .filter(|&scan| passes_filter(s, scan))
            {
                let pressed = key_down;
                for (&pid, shm) in s.targets.iter_mut() {
                    if Some(pid) == foreground_pid {
                        continue;
                    }
                    shm.broadcast_keys[scan as usize] = pressed;
                    write_combined_key(shm, scan as usize);
                }
            }
        }
    }
    CallNextHookEx(s.keyboard_hook, code, wparam, lparam)
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
            keyboard_hook: HHOOK(std::ptr::null_mut()),
            mouse_hook: HHOOK(std::ptr::null_mut()),
            broadcasting: false,
            clutch_binding_vk: None,
            pending_clutch_binding: None,
            clutch_key_is_down: false,
            clutch: MouseClutch::new(0),
            remote_mouse_holds: HashMap::new(),
            remote_mouse_sequences: HashMap::new(),
            closed_remote_sessions: HashMap::new(),
            closed_remote_session_order: VecDeque::new(),
            clutch_status_dirty: false,
            clutch_error: None,
            eq_pids: Vec::new(),
            mouse_eligible_pids: vec![target_pid, other_pid],
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
                (*state().as_mut().unwrap().targets[&pid].ptr)
                    .proxy_ready
                    .store(trusik_protocol::PROXY_READY, Ordering::Release);
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
            let now = GetTickCount64() as u32;
            let mut target_keys = [0; 256];
            let mut other_keys = [0; 256];
            assert_eq!(
                (*target.ptr)
                    .controller_keyboard_active
                    .load(Ordering::Acquire),
                KEYBOARD_TARGETED
            );
            assert_eq!(
                (*target.ptr)
                    .controller_mouse_active
                    .load(Ordering::Acquire),
                0
            );
            (*target.ptr).read_effective_keys(now, &mut target_keys);
            (*other.ptr).read_effective_keys(now, &mut other_keys);
            assert_eq!(target_keys[0x1e], 0x80);
            assert_eq!(target_keys[0x1f], 0x00);
            assert_eq!(other_keys[0x1e], 0x00);
            assert_eq!(other_keys[0x1f], 0x80);
        }

        set_targeted_key(target_pid, 0x1e, false).unwrap();
        finish_targeted_input(target_pid);
        let broadcast_state = state().as_ref().unwrap();
        let target = &broadcast_state.targets[&target_pid];
        let other = &broadcast_state.targets[&other_pid];
        unsafe {
            let now = GetTickCount64() as u32;
            let mut target_keys = [0; 256];
            let mut other_keys = [0; 256];
            assert_eq!(
                (*target.ptr)
                    .controller_keyboard_active
                    .load(Ordering::Acquire),
                0
            );
            assert_eq!(
                (*other.ptr)
                    .controller_keyboard_active
                    .load(Ordering::Acquire),
                KEYBOARD_TARGETED
            );
            (*target.ptr).read_effective_keys(now, &mut target_keys);
            (*other.ptr).read_effective_keys(now, &mut other_keys);
            assert_eq!(target_keys[0x1e], 0x00);
            assert_eq!(other_keys[0x1f], 0x80);
        }
        set_targeted_key(other_pid, 0x1f, false).unwrap();
        finish_targeted_input(other_pid);
        *state() = None;
    }

    #[test]
    fn clutch_ignores_repeated_down_and_drains_after_plain_release() {
        let mut clutch = MouseClutch::new(0);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        assert_eq!(clutch.phase, ClutchPhase::Active);
        assert_eq!(clutch.on_key_down(0x7c, 10, true), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Active);

        assert_eq!(clutch.on_key_up(0x7c, 100, false), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Draining);
        assert!(!clutch.finish_drain_if_due(100 + MOUSE_DRAIN_MS - 1));
        assert!(clutch.finish_drain_if_due(100 + MOUSE_DRAIN_MS));
        assert_eq!(clutch.phase, ClutchPhase::Inactive);
    }

    #[test]
    fn local_release_keeps_an_overlapping_remote_owner_active() {
        let mut clutch = MouseClutch::new(0);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        assert_eq!(clutch.on_key_up(0x7c, 100, true), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Active);

        clutch.release_if_unowned(120, false);
        assert_eq!(clutch.phase, ClutchPhase::Draining);
    }

    #[test]
    fn preheld_mouse_button_rejects_press_and_swallows_matching_up() {
        let mut clutch = MouseClutch::new(MOUSE_LEFT);
        assert_eq!(clutch.on_key_down(0x7c, 10, true), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Inactive);
        assert_eq!(clutch.on_key_up(0x7c, 50, false), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Inactive);
    }

    #[test]
    fn release_while_dragging_waits_for_button_up_then_drains() {
        let mut clutch = MouseClutch::new(0);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        clutch.set_button(MOUSE_LEFT, true, 10);
        assert_eq!(clutch.on_key_up(0x7c, 20, false), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Releasing);
        assert!(!clutch.finish_drain_if_due(10_000));

        clutch.set_button(MOUSE_LEFT, false, 30);
        assert_eq!(clutch.phase, ClutchPhase::Draining);
        assert!(!clutch.finish_drain_if_due(30 + MOUSE_DRAIN_MS - 1));
        assert!(clutch.finish_drain_if_due(30 + MOUSE_DRAIN_MS));
    }

    #[test]
    fn new_mouse_press_during_drain_cannot_extend_activation() {
        let mut clutch = MouseClutch::new(0);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        assert_eq!(clutch.on_key_up(0x7c, 20, false), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Draining);

        clutch.set_button(MOUSE_LEFT, true, 30);
        assert_eq!(clutch.phase, ClutchPhase::Draining);
        assert!(clutch.finish_drain_if_due(20 + MOUSE_DRAIN_MS));
    }

    #[test]
    fn clutch_repress_during_drain_resumes_without_deactivation() {
        let mut clutch = MouseClutch::new(0);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        assert_eq!(clutch.on_key_up(0x7c, 20, false), ClutchKeyEffect::Swallow);
        assert_eq!(clutch.phase, ClutchPhase::Draining);
        assert_eq!(
            clutch.on_key_down(0x7c, 10, true),
            ClutchKeyEffect::Activate
        );
        assert_eq!(clutch.phase, ClutchPhase::Active);
        assert!(!clutch.finish_drain_if_due(20 + MOUSE_DRAIN_MS));
    }

    #[test]
    fn focus_source_or_target_loss_cancels_bounded_and_late_up_is_swallowed() {
        for _reason in ["focus", "source", "target"] {
            let mut clutch = MouseClutch::new(0);
            assert_eq!(
                clutch.on_key_down(0x7c, 10, true),
                ClutchKeyEffect::Activate
            );
            clutch.set_button(MOUSE_RIGHT, true, 5);
            clutch.cancel_bounded(40);
            assert_eq!(clutch.phase, ClutchPhase::Draining);
            assert_eq!(clutch.on_key_up(0x7c, 50, false), ClutchKeyEffect::Swallow);
            assert_eq!(clutch.swallowed_vk, None);
            assert!(clutch.finish_drain_if_due(40 + MOUSE_DRAIN_MS));
            assert_eq!(clutch.phase, ClutchPhase::Inactive);
        }
    }
}
