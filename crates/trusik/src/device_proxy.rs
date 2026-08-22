use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows::Win32::System::Threading::{GetCurrentProcess, SetEvent};

/// HWND saved from SetCooperativeLevel on the keyboard device.
/// Used by the GetForegroundWindow IAT hook to trick EQ into processing keys.
static EQ_HWND: AtomicIsize = AtomicIsize::new(0);

/// Public accessor for the stored EQ HWND.
pub fn eq_hwnd() -> isize {
    EQ_HWND.load(Ordering::Acquire)
}

fn should_post_activation(
    active: bool,
    activation_asserted: bool,
    mouse_active: bool,
    was_mouse_active: bool,
) -> bool {
    active && (!activation_asserted || (mouse_active && !was_mouse_active))
}

/// Thread that watches shared-memory state and posts WM_ACTIVATEAPP(TRUE) to
/// the EQ window when auto-login begins.  EQ's main loop only calls
/// keyboard_process when an internal "active" flag ([obj+5E4h]) is set — that
/// flag is driven by WM_ACTIVATEAPP.  By posting this message we trick EQ into
/// running keyboard_process for background windows.
fn wm_activate_thread() {
    const WM_ACTIVATEAPP: u32 = 0x001C;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn PostMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn GetForegroundWindow() -> isize;
    }

    const DEACTIVATE_STABLE_FRAMES: u8 = 3;
    let mut activation_asserted = false;
    let mut was_mouse_active = false;
    let mut inactive_frames = 0u8;

    loop {
        std::thread::sleep(std::time::Duration::from_millis(16));

        let active = crate::key_shm::is_active();
        let mouse_active = crate::key_shm::is_mouse_active();
        let hwnd = EQ_HWND.load(Ordering::Acquire);

        if should_post_activation(active, activation_asserted, mouse_active, was_mouse_active)
            && hwnd != 0
        {
            // Reassert activation on an overall transition and independently on
            // each Mouse Clutch rising edge. Keyboard Broadcast may already be
            // holding the process active while the real mouse needs reacquire.
            unsafe {
                PostMessageW(hwnd, WM_ACTIVATEAPP, 1, 0);
            }
            activation_asserted = true;
            crate::log::write(&format!(
                "wm_activate: posted WM_ACTIVATEAPP(1) hwnd=0x{hwnd:X} mouse_active={mouse_active}"
            ));
        }

        if active {
            inactive_frames = 0;
        } else if activation_asserted {
            inactive_frames = inactive_frames.saturating_add(1);
            if inactive_frames >= DEACTIVATE_STABLE_FRAMES {
                // Recheck after the stable interval. A rapid clutch re-press
                // must not receive a stale WM_ACTIVATEAPP(FALSE).
                let still_inactive = !crate::key_shm::is_active();
                let fg = unsafe { GetForegroundWindow() };
                if still_inactive && hwnd != 0 && fg != hwnd {
                    unsafe {
                        PostMessageW(hwnd, WM_ACTIVATEAPP, 0, 0);
                    }
                    crate::log::write(&format!(
                        "wm_activate: posted WM_ACTIVATEAPP(0) hwnd=0x{hwnd:X}"
                    ));
                }
                if still_inactive {
                    activation_asserted = false;
                }
                inactive_frames = 0;
            }
        }
        was_mouse_active = mouse_active;
    }
}

/// Raw COM vtable for IDirectInputDevice8 (A or W).
///
/// 3 IUnknown + 29 IDirectInputDevice8 = 32 entries.
#[repr(C)]
struct IDirectInputDevice8Vtbl {
    // IUnknown (0-2)
    query_interface:
        unsafe extern "system" fn(*mut DeviceProxy, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DeviceProxy) -> u32,
    release: unsafe extern "system" fn(*mut DeviceProxy) -> u32,

    // IDirectInputDevice8 (3-31)
    get_capabilities: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void) -> HRESULT,
    enum_objects:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *mut c_void, u32) -> HRESULT,
    get_property: unsafe extern "system" fn(*mut DeviceProxy, *const GUID, *mut c_void) -> HRESULT,
    set_property: unsafe extern "system" fn(*mut DeviceProxy, *const GUID, *mut c_void) -> HRESULT,
    acquire: unsafe extern "system" fn(*mut DeviceProxy) -> HRESULT,
    unacquire: unsafe extern "system" fn(*mut DeviceProxy) -> HRESULT,
    get_device_state: unsafe extern "system" fn(*mut DeviceProxy, u32, *mut c_void) -> HRESULT,
    get_device_data:
        unsafe extern "system" fn(*mut DeviceProxy, u32, *mut c_void, *mut u32, u32) -> HRESULT,
    set_data_format: unsafe extern "system" fn(*mut DeviceProxy, *const c_void) -> HRESULT,
    set_event_notification: unsafe extern "system" fn(*mut DeviceProxy, isize) -> HRESULT,
    set_cooperative_level: unsafe extern "system" fn(*mut DeviceProxy, isize, u32) -> HRESULT,
    get_object_info: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, u32, u32) -> HRESULT,
    get_device_info: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void) -> HRESULT,
    run_control_panel: unsafe extern "system" fn(*mut DeviceProxy, isize, u32) -> HRESULT,
    initialize: unsafe extern "system" fn(*mut DeviceProxy, isize, u32, *const GUID) -> HRESULT,
    create_effect: unsafe extern "system" fn(
        *mut DeviceProxy,
        *const GUID,
        *const c_void,
        *mut *mut c_void,
        *mut c_void,
    ) -> HRESULT,
    enum_effects:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *mut c_void, u32) -> HRESULT,
    get_effect_info:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *const GUID) -> HRESULT,
    get_force_feedback_state: unsafe extern "system" fn(*mut DeviceProxy, *mut u32) -> HRESULT,
    send_force_feedback_command: unsafe extern "system" fn(*mut DeviceProxy, u32) -> HRESULT,
    enum_created_effect_objects:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *mut c_void, u32) -> HRESULT,
    escape: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void) -> HRESULT,
    poll: unsafe extern "system" fn(*mut DeviceProxy) -> HRESULT,
    send_device_data:
        unsafe extern "system" fn(*mut DeviceProxy, u32, *const c_void, *mut u32, u32) -> HRESULT,
    enum_effects_in_file: unsafe extern "system" fn(
        *mut DeviceProxy,
        *const c_void,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT,
    write_effect_to_file: unsafe extern "system" fn(
        *mut DeviceProxy,
        *const c_void,
        u32,
        *mut c_void,
        u32,
    ) -> HRESULT,
    build_action_map:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *const c_void, u32) -> HRESULT,
    set_action_map:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, *const c_void, u32) -> HRESULT,
    get_image_info: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void) -> HRESULT,
}

static DEV_VTBL: IDirectInputDevice8Vtbl = IDirectInputDevice8Vtbl {
    query_interface: dev_query_interface,
    add_ref: dev_add_ref,
    release: dev_release,
    get_capabilities: dev_get_capabilities,
    enum_objects: dev_enum_objects,
    get_property: dev_get_property,
    set_property: dev_set_property,
    acquire: dev_acquire,
    unacquire: dev_unacquire,
    get_device_state: dev_get_device_state,
    get_device_data: dev_get_device_data,
    set_data_format: dev_set_data_format,
    set_event_notification: dev_set_event_notification,
    set_cooperative_level: dev_set_cooperative_level,
    get_object_info: dev_get_object_info,
    get_device_info: dev_get_device_info,
    run_control_panel: dev_run_control_panel,
    initialize: dev_initialize,
    create_effect: dev_create_effect,
    enum_effects: dev_enum_effects,
    get_effect_info: dev_get_effect_info,
    get_force_feedback_state: dev_get_force_feedback_state,
    send_force_feedback_command: dev_send_force_feedback_command,
    enum_created_effect_objects: dev_enum_created_effect_objects,
    escape: dev_escape,
    poll: dev_poll,
    send_device_data: dev_send_device_data,
    enum_effects_in_file: dev_enum_effects_in_file,
    write_effect_to_file: dev_write_effect_to_file,
    build_action_map: dev_build_action_map,
    set_action_map: dev_set_action_map,
    get_image_info: dev_get_image_info,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Other,
}

struct KeyboardNotification {
    handle: Mutex<Option<isize>>,
    worker_started: AtomicBool,
    stopping: AtomicBool,
}

impl KeyboardNotification {
    fn new() -> Self {
        Self {
            handle: Mutex::new(None),
            worker_started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        }
    }

    unsafe fn replace(self: &Arc<Self>, event: isize) {
        let replacement = if event == 0 {
            None
        } else {
            let process = GetCurrentProcess();
            let mut duplicate = HANDLE(std::ptr::null_mut());
            match DuplicateHandle(
                process,
                HANDLE(event as *mut c_void),
                process,
                &mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            ) {
                Ok(()) => Some(duplicate.0 as isize),
                Err(error) => {
                    crate::log::write(&format!(
                        "SetEventNotification: failed to duplicate keyboard event: {error}"
                    ));
                    None
                }
            }
        };

        let old = {
            let mut handle = self
                .handle
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::replace(&mut *handle, replacement)
        };
        if let Some(old) = old {
            let _ = CloseHandle(HANDLE(old as *mut c_void));
        }

        if event != 0
            && self
                .worker_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            let notification = Arc::clone(self);
            std::thread::spawn(move || notification.run());
        }
    }

    fn run(&self) {
        let mut previously_active = false;
        while !self.stopping.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(8));

            let mut keys = [0u8; 256];
            let active = crate::key_shm::read_keys(&mut keys);
            let any_keys = active && keys.iter().any(|&key| key != 0);
            if any_keys || previously_active {
                let handle = self
                    .handle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if let Some(handle) = *handle {
                    unsafe {
                        let _ = SetEvent(HANDLE(handle as *mut c_void));
                    }
                }
            }
            previously_active = any_keys;
        }
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        let old = self
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(old) = old {
            unsafe {
                let _ = CloseHandle(HANDLE(old as *mut c_void));
            }
        }
    }
}

/// Our proxy for IDirectInputDevice8. COM layout: vtable pointer first.
#[repr(C)]
pub struct DeviceProxy {
    vtbl: *const IDirectInputDevice8Vtbl,
    real: *mut c_void,
    ref_count: AtomicU32,
    kind: DeviceKind,
    interface_iid: GUID,
    buffered_keyboard: Mutex<BufferedKeyboardState>,
    notification: Arc<KeyboardNotification>,
}

impl DeviceProxy {
    /// Adopt the owned reference returned by `IDirectInput8::CreateDevice`.
    pub fn from_owned(real: *mut c_void, kind: DeviceKind, interface_iid: GUID) -> Self {
        debug_assert!(matches!(
            interface_iid,
            crate::com_ids::IID_IDIRECTINPUTDEVICE8A | crate::com_ids::IID_IDIRECTINPUTDEVICE8W
        ));
        let controller_event_cursor = if kind == DeviceKind::Keyboard {
            crate::key_shm::controller_event_head()
        } else {
            None
        };
        Self {
            vtbl: &DEV_VTBL,
            real,
            ref_count: AtomicU32::new(1),
            kind,
            interface_iid,
            buffered_keyboard: Mutex::new(BufferedKeyboardState::new_with_cursor(
                controller_event_cursor,
            )),
            notification: Arc::new(KeyboardNotification::new()),
        }
    }
}

impl Drop for DeviceProxy {
    fn drop(&mut self) {
        self.notification.stop();
    }
}

/// Call a method on the real COM interface by vtable index.
unsafe fn real_method<T>(real: *mut c_void, index: usize) -> T {
    let real_vtbl = *(real as *const *const *const c_void);
    std::mem::transmute_copy(&*real_vtbl.add(index))
}

// --- IUnknown ---

unsafe extern "system" fn dev_query_interface(
    this: *mut DeviceProxy,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);
    if riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    *ppv = std::ptr::null_mut();

    let iid = *riid;
    if iid == crate::com_ids::IID_IUNKNOWN || iid == (*this).interface_iid {
        dev_add_ref(this);
        *ppv = this.cast();
        return HRESULT(0);
    }

    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT =
        real_method(real, 0);
    method(real, riid, ppv)
}

unsafe extern "system" fn dev_add_ref(this: *mut DeviceProxy) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn dev_release(this: *mut DeviceProxy) -> u32 {
    let prev = (*this).ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        let real = (*this).real;
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = real_method(real, 2);
        release(real);
        drop(Box::from_raw(this));
        return 0;
    }
    prev - 1
}

// --- IDirectInputDevice8 methods (slots 3-31) ---

unsafe extern "system" fn dev_get_capabilities(
    this: *mut DeviceProxy,
    caps: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
        real_method((*this).real, 3);
    method((*this).real, caps)
}

unsafe extern "system" fn dev_enum_objects(
    this: *mut DeviceProxy,
    callback: *mut c_void,
    pvref: *mut c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) -> HRESULT =
        real_method((*this).real, 4);
    method((*this).real, callback, pvref, flags)
}

unsafe extern "system" fn dev_get_property(
    this: *mut DeviceProxy,
    rguid: *const GUID,
    pdipropheader: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *const GUID, *mut c_void) -> HRESULT =
        real_method((*this).real, 5);
    method((*this).real, rguid, pdipropheader)
}

unsafe extern "system" fn dev_set_property(
    this: *mut DeviceProxy,
    rguid: *const GUID,
    pdipropheader: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *const GUID, *mut c_void) -> HRESULT =
        real_method((*this).real, 6);
    method((*this).real, rguid, pdipropheader)
}

unsafe extern "system" fn dev_acquire(this: *mut DeviceProxy) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT = real_method((*this).real, 7);
    let hr = method((*this).real);
    if hr.is_err() && (*this).kind == DeviceKind::Keyboard && crate::key_shm::is_keyboard_active() {
        return HRESULT(0); // DI_OK
    }
    hr
}

unsafe extern "system" fn dev_unacquire(this: *mut DeviceProxy) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT = real_method((*this).real, 8);
    method((*this).real)
}

/// Keyboard activation still requires the process-wide foreground spoof. While
/// that spoof is active, explicitly discard system-mouse data in a real
/// background EQ process unless Mouse Clutch selected it. Foreground EQ and
/// other DirectInput devices remain natural pass-through.
unsafe fn should_block_background_mouse() -> bool {
    if !crate::key_shm::is_compatible() || crate::key_shm::is_mouse_active() {
        return false;
    }
    let hwnd = EQ_HWND.load(Ordering::Acquire);
    let foreground = crate::iat_hook::real_foreground_window();
    hwnd == 0 || foreground != hwnd
}

/// Counter to throttle GetDeviceState logging.
static GDS_LOG_COUNT: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn dev_get_device_state(
    this: *mut DeviceProxy,
    cbdata: u32,
    lpvdata: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT =
        real_method((*this).real, 9);
    let hr = method((*this).real, cbdata, lpvdata);

    if (*this).kind == DeviceKind::Mouse {
        if hr.is_ok() && should_block_background_mouse() {
            std::ptr::write_bytes(lpvdata as *mut u8, 0, cbdata as usize);
        }
        return hr;
    }

    if (*this).kind == DeviceKind::Keyboard {
        // Synthetic keyboard state uses DirectInput's standard 256-byte
        // c_dfDIKeyboard layout. Preserve the real result for custom/invalid
        // buffers rather than trusting an error-path pointer and length.
        if lpvdata.is_null() || cbdata != 256 {
            return hr;
        }

        // Log first few calls to confirm EQ is polling.
        let count = GDS_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
        if count < 5 {
            let active = crate::key_shm::is_active();
            crate::log::write(&format!(
                "GetDeviceState: hr=0x{:08X} shm_active={active} cbdata={cbdata}",
                hr.0 as u32
            ));
        }

        if hr.is_ok() {
            if crate::key_shm::should_suppress() {
                std::ptr::write_bytes(lpvdata as *mut u8, 0, cbdata as usize);
            }
            crate::key_shm::inject_keys(lpvdata as *mut u8, cbdata);
        } else if matches!(hr, DIERR_INPUTLOST | DIERR_NOTACQUIRED) {
            std::ptr::write_bytes(lpvdata as *mut u8, 0, cbdata as usize);
            if crate::key_shm::inject_keys(lpvdata as *mut u8, cbdata) {
                return HRESULT(0); // DI_OK
            }
        }
    }

    hr
}

const DX3_OBJECT_DATA_SIZE: usize = 4 * std::mem::size_of::<u32>();
const DIERR_INPUTLOST: HRESULT = HRESULT(0x8007_001Eu32 as i32);
const DIERR_NOTACQUIRED: HRESULT = HRESULT(0x8007_000Cu32 as i32);
const DIERR_NOTBUFFERED: HRESULT = HRESULT(-2_147_220_985);

fn synthetic_data_can_replace(error: HRESULT) -> bool {
    matches!(
        error,
        DIERR_INPUTLOST | DIERR_NOTACQUIRED | DIERR_NOTBUFFERED
    )
}

fn real_event_count(hr: HRESULT, reported: u32, capacity: u32, count_query: bool) -> u32 {
    if hr.is_err() {
        0
    } else if count_query {
        reported
    } else {
        reported.min(capacity)
    }
}

/// Native DIDEVICEOBJECTDATA. Callers may instead request the 16-byte DX3
/// prefix, so records are copied as bytes and capped to the caller's stride.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
struct SyntheticEvent {
    dw_ofs: u32,
    dw_data: u32,
    dw_time_stamp: u32,
    dw_sequence: u32,
    u_app_data: usize,
}

struct BufferedKeyboardState {
    observed_keys: [u8; 256],
    controller_event_cursor: Option<u32>,
    pending: VecDeque<SyntheticEvent>,
    next_sequence: u32,
}

impl BufferedKeyboardState {
    #[cfg(test)]
    fn new() -> Self {
        Self::new_with_cursor(None)
    }

    fn new_with_cursor(controller_event_cursor: Option<u32>) -> Self {
        Self {
            observed_keys: [0; 256],
            controller_event_cursor,
            pending: VecDeque::new(),
            next_sequence: 0x8000_0000,
        }
    }

    fn push_transition(&mut self, scan: usize, pressed: bool, timestamp: u32) {
        self.pending.push_back(SyntheticEvent {
            dw_ofs: scan as u32,
            dw_data: if pressed { 0x80 } else { 0 },
            dw_time_stamp: timestamp,
            dw_sequence: self.next_sequence,
            u_app_data: 0,
        });
        self.next_sequence = self.next_sequence.wrapping_add(1);
    }

    fn observe_controller_events(
        &mut self,
        events: &[(u8, bool)],
        auto_type_keys: &[u8; 256],
        timestamp: u32,
    ) {
        for &(scan, controller_pressed) in events {
            let index = scan as usize;
            let effective_pressed = controller_pressed || auto_type_keys[index] != 0;
            if (self.observed_keys[index] != 0) == effective_pressed {
                continue;
            }
            self.push_transition(index, effective_pressed, timestamp);
            self.observed_keys[index] = if effective_pressed { 0x80 } else { 0 };
        }
    }

    fn observe(&mut self, keys: [u8; 256], timestamp: u32) {
        for (scan, current) in keys.into_iter().enumerate() {
            if self.observed_keys[scan] == current {
                continue;
            }
            self.push_transition(scan, current != 0, timestamp);
            self.observed_keys[scan] = current;
        }
    }

    unsafe fn copy_to(
        &mut self,
        destination: *mut u8,
        stride: usize,
        capacity: usize,
        peek: bool,
    ) -> usize {
        let count = capacity.min(self.pending.len());
        for (index, event) in self.pending.iter().take(count).enumerate() {
            std::ptr::copy_nonoverlapping(
                (event as *const SyntheticEvent).cast::<u8>(),
                destination.add(index * stride),
                stride.min(std::mem::size_of::<SyntheticEvent>()),
            );
        }
        if !peek {
            self.pending.drain(..count);
        }
        count
    }
}

unsafe extern "system" fn dev_get_device_data(
    this: *mut DeviceProxy,
    cbobjectdata: u32,
    rgdod: *mut c_void,
    pdwinout: *mut u32,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32, u32) -> HRESULT =
        real_method((*this).real, 10);

    if (*this).kind != DeviceKind::Keyboard {
        let hr = method((*this).real, cbobjectdata, rgdod, pdwinout, flags);
        if (*this).kind == DeviceKind::Mouse
            && should_block_background_mouse()
            && !pdwinout.is_null()
        {
            *pdwinout = 0;
        }
        return hr;
    }

    // A null count pointer is invalid and cannot be safely merged. Preserve the
    // real implementation's exact error behavior without touching the buffer.
    if pdwinout.is_null() {
        return method((*this).real, cbobjectdata, rgdod, pdwinout, flags);
    }

    static GDD_ACTIVE_LOG: AtomicU32 = AtomicU32::new(0);
    if crate::key_shm::is_active() {
        let count = GDD_ACTIVE_LOG.fetch_add(1, Ordering::Relaxed);
        if count < 5 {
            crate::log::write(&format!("GetDeviceData: ACTIVE call #{count}"));
        }
    }

    let stride = cbobjectdata as usize;
    let native_stride = std::mem::size_of::<SyntheticEvent>();
    if stride != DX3_OBJECT_DATA_SIZE && stride != native_stride {
        return method((*this).real, cbobjectdata, rgdod, pdwinout, flags);
    }

    let original_capacity = *pdwinout;
    let count_query = rgdod.is_null();
    let legacy_stride = stride == DX3_OBJECT_DATA_SIZE;
    let mut translated_real = Vec::new();
    if legacy_stride && !count_query && original_capacity != 0 {
        let capacity = original_capacity as usize;
        if translated_real.try_reserve_exact(capacity).is_err() {
            *pdwinout = 0;
            return HRESULT(0x8007_000Eu32 as i32); // E_OUTOFMEMORY
        }
        translated_real.resize(
            capacity,
            SyntheticEvent {
                dw_ofs: 0,
                dw_data: 0,
                dw_time_stamp: 0,
                dw_sequence: 0,
                u_app_data: 0,
            },
        );
    }

    let mut reported_real = original_capacity;
    let real_buffer = if legacy_stride && !count_query {
        translated_real.as_mut_ptr().cast::<c_void>()
    } else {
        rgdod
    };
    let real_stride = if legacy_stride {
        native_stride as u32
    } else {
        cbobjectdata
    };
    let hr = method(
        (*this).real,
        real_stride,
        real_buffer,
        &mut reported_real,
        flags,
    );
    let suppress_real = crate::key_shm::should_suppress();
    let mut real_count = real_event_count(hr, reported_real, original_capacity, count_query);
    if suppress_real {
        real_count = 0;
    } else if legacy_stride && !count_query && hr.is_ok() {
        for (index, event) in translated_real.iter().take(real_count as usize).enumerate() {
            std::ptr::copy_nonoverlapping(
                (event as *const SyntheticEvent).cast::<u8>(),
                (rgdod as *mut u8).add(index * stride),
                DX3_OBJECT_DATA_SIZE,
            );
        }
    }

    let timestamp = windows::Win32::System::SystemInformation::GetTickCount();
    let mut buffered = (*this)
        .buffered_keyboard
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut controller_events = Vec::new();
    let overflowed = crate::key_shm::drain_controller_events(
        &mut buffered.controller_event_cursor,
        |scan, pressed| controller_events.push((scan, pressed)),
    );
    let mut auto_type_keys = [0u8; 256];
    let _ = crate::key_shm::read_auto_type_keys(&mut auto_type_keys);
    if crate::key_shm::is_controller_keyboard_active() {
        buffered.observe_controller_events(&controller_events, &auto_type_keys, timestamp);
    }
    let mut current_keys = [0u8; 256];
    let _ = crate::key_shm::read_keys(&mut current_keys);
    buffered.observe(current_keys, timestamp);
    if overflowed {
        static EVENT_OVERFLOW_LOG: AtomicU32 = AtomicU32::new(0);
        if EVENT_OVERFLOW_LOG.fetch_add(1, Ordering::Relaxed) < 5 {
            crate::log::write(
                "GetDeviceData: controller event ring overflowed; reconciled current key levels",
            );
        }
    }

    if rgdod.is_null() {
        *pdwinout =
            real_count.saturating_add(u32::try_from(buffered.pending.len()).unwrap_or(u32::MAX));
        return if synthetic_data_can_replace(hr) && !buffered.pending.is_empty() {
            HRESULT(0)
        } else {
            hr
        };
    }

    const DIGDD_PEEK: u32 = 0x01;
    let peek = flags & DIGDD_PEEK != 0;
    let available = original_capacity.saturating_sub(real_count) as usize;
    let destination = (rgdod as *mut u8).add(real_count as usize * stride);
    let synthetic_count = buffered.copy_to(destination, stride, available, peek);
    *pdwinout = real_count + synthetic_count as u32;

    if synthetic_data_can_replace(hr) && synthetic_count != 0 {
        HRESULT(0)
    } else {
        hr
    }
}

unsafe extern "system" fn dev_set_data_format(
    this: *mut DeviceProxy,
    lpdf: *const c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT =
        real_method((*this).real, 11);
    method((*this).real, lpdf)
}

unsafe extern "system" fn dev_set_event_notification(
    this: *mut DeviceProxy,
    hevent: isize,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, isize) -> HRESULT =
        real_method((*this).real, 12);
    let hr = method((*this).real, hevent);

    if hr.is_ok() && (*this).kind == DeviceKind::Keyboard {
        crate::log::write(&format!(
            "SetEventNotification: keyboard event handle=0x{hevent:X}"
        ));
        (*this).notification.replace(hevent);
    }

    hr
}

unsafe extern "system" fn dev_set_cooperative_level(
    this: *mut DeviceProxy,
    hwnd: isize,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT =
        real_method((*this).real, 13);

    let mut actual_flags = flags;
    if matches!((*this).kind, DeviceKind::Keyboard | DeviceKind::Mouse) {
        EQ_HWND.store(hwnd, Ordering::Release);
    }
    if (*this).kind == DeviceKind::Keyboard {
        const DISCL_EXCLUSIVE: u32 = 0x01;
        const DISCL_FOREGROUND: u32 = 0x04;
        const DISCL_NONEXCLUSIVE: u32 = 0x02;
        const DISCL_BACKGROUND: u32 = 0x08;

        EQ_HWND.store(hwnd, Ordering::Release);
        crate::log::write(&format!("SetCooperativeLevel: keyboard hwnd=0x{hwnd:X}"));

        // Posts WM_ACTIVATEAPP(1) when shm becomes active so the game's
        // "active" flag is set, allowing keyboard_process to run.
        static WM_ACTIVATE_THREAD: std::sync::Once = std::sync::Once::new();
        WM_ACTIVATE_THREAD.call_once(|| {
            std::thread::spawn(wm_activate_thread);
        });

        if actual_flags & DISCL_FOREGROUND != 0 {
            actual_flags = (actual_flags & !(DISCL_EXCLUSIVE | DISCL_FOREGROUND))
                | DISCL_NONEXCLUSIVE
                | DISCL_BACKGROUND;
        }
    }

    method((*this).real, hwnd, actual_flags)
}

unsafe extern "system" fn dev_get_object_info(
    this: *mut DeviceProxy,
    pdidoi: *mut c_void,
    dwobj: u32,
    dwhow: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) -> HRESULT =
        real_method((*this).real, 14);
    method((*this).real, pdidoi, dwobj, dwhow)
}

unsafe extern "system" fn dev_get_device_info(
    this: *mut DeviceProxy,
    pdidi: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
        real_method((*this).real, 15);
    method((*this).real, pdidi)
}

unsafe extern "system" fn dev_run_control_panel(
    this: *mut DeviceProxy,
    hwnd_owner: isize,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT =
        real_method((*this).real, 16);
    method((*this).real, hwnd_owner, flags)
}

unsafe extern "system" fn dev_initialize(
    this: *mut DeviceProxy,
    hinst: isize,
    dwversion: u32,
    rguid: *const GUID,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, isize, u32, *const GUID) -> HRESULT =
        real_method((*this).real, 17);
    method((*this).real, hinst, dwversion, rguid)
}

unsafe extern "system" fn dev_create_effect(
    this: *mut DeviceProxy,
    rguid: *const GUID,
    lpeff: *const c_void,
    ppeff: *mut *mut c_void,
    punk_outer: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const c_void,
        *mut *mut c_void,
        *mut c_void,
    ) -> HRESULT = real_method((*this).real, 18);
    method((*this).real, rguid, lpeff, ppeff, punk_outer)
}

unsafe extern "system" fn dev_enum_effects(
    this: *mut DeviceProxy,
    callback: *mut c_void,
    pvref: *mut c_void,
    efftype: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) -> HRESULT =
        real_method((*this).real, 19);
    method((*this).real, callback, pvref, efftype)
}

unsafe extern "system" fn dev_get_effect_info(
    this: *mut DeviceProxy,
    pdei: *mut c_void,
    rguid: *const GUID,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID) -> HRESULT =
        real_method((*this).real, 20);
    method((*this).real, pdei, rguid)
}

unsafe extern "system" fn dev_get_force_feedback_state(
    this: *mut DeviceProxy,
    pdwout: *mut u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT =
        real_method((*this).real, 21);
    method((*this).real, pdwout)
}

unsafe extern "system" fn dev_send_force_feedback_command(
    this: *mut DeviceProxy,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT =
        real_method((*this).real, 22);
    method((*this).real, flags)
}

unsafe extern "system" fn dev_enum_created_effect_objects(
    this: *mut DeviceProxy,
    callback: *mut c_void,
    pvref: *mut c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) -> HRESULT =
        real_method((*this).real, 23);
    method((*this).real, callback, pvref, flags)
}

unsafe extern "system" fn dev_escape(this: *mut DeviceProxy, pesc: *mut c_void) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
        real_method((*this).real, 24);
    method((*this).real, pesc)
}

unsafe extern "system" fn dev_poll(this: *mut DeviceProxy) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT = real_method((*this).real, 25);
    method((*this).real)
}

unsafe extern "system" fn dev_send_device_data(
    this: *mut DeviceProxy,
    cbobjectdata: u32,
    rgdod: *const c_void,
    pdwinout: *mut u32,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const c_void,
        *mut u32,
        u32,
    ) -> HRESULT = real_method((*this).real, 26);
    method((*this).real, cbobjectdata, rgdod, pdwinout, flags)
}

unsafe extern "system" fn dev_enum_effects_in_file(
    this: *mut DeviceProxy,
    filename: *const c_void,
    callback: *mut c_void,
    pvref: *mut c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT = real_method((*this).real, 27);
    method((*this).real, filename, callback, pvref, flags)
}

unsafe extern "system" fn dev_write_effect_to_file(
    this: *mut DeviceProxy,
    filename: *const c_void,
    nentries: u32,
    rgdifileeff: *mut c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        u32,
        *mut c_void,
        u32,
    ) -> HRESULT = real_method((*this).real, 28);
    method((*this).real, filename, nentries, rgdifileeff, flags)
}

unsafe extern "system" fn dev_build_action_map(
    this: *mut DeviceProxy,
    lpdiactionformat: *mut c_void,
    username: *const c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void, u32) -> HRESULT =
        real_method((*this).real, 29);
    method((*this).real, lpdiactionformat, username, flags)
}

unsafe extern "system" fn dev_set_action_map(
    this: *mut DeviceProxy,
    lpdiactionformat: *mut c_void,
    username: *const c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void, u32) -> HRESULT =
        real_method((*this).real, 30);
    method((*this).real, lpdiactionformat, username, flags)
}

unsafe extern "system" fn dev_get_image_info(
    this: *mut DeviceProxy,
    pdidevimageinfo: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
        real_method((*this).real, 31);
    method((*this).real, pdidevimageinfo)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECONDARY_IID: GUID = GUID::from_u128(0x544f63e8_bdb9_41f1_b5b4_8be6fb438e44);

    #[repr(C)]
    struct FakeUnknownVtbl {
        query_interface:
            unsafe extern "system" fn(*mut FakeUnknown, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut FakeUnknown) -> u32,
        release: unsafe extern "system" fn(*mut FakeUnknown) -> u32,
    }

    #[repr(C)]
    struct FakeUnknown {
        vtbl: *const FakeUnknownVtbl,
        refs: AtomicU32,
        release_calls: AtomicU32,
    }

    static FAKE_VTBL: FakeUnknownVtbl = FakeUnknownVtbl {
        query_interface: fake_query_interface,
        add_ref: fake_add_ref,
        release: fake_release,
    };

    unsafe extern "system" fn fake_query_interface(
        this: *mut FakeUnknown,
        iid: *const GUID,
        output: *mut *mut c_void,
    ) -> HRESULT {
        *output = std::ptr::null_mut();
        if *iid != SECONDARY_IID {
            return HRESULT(0x8000_4002u32 as i32);
        }
        fake_add_ref(this);
        *output = this.cast();
        HRESULT(0)
    }

    unsafe extern "system" fn fake_add_ref(this: *mut FakeUnknown) -> u32 {
        (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn fake_release(this: *mut FakeUnknown) -> u32 {
        (*this).release_calls.fetch_add(1, Ordering::Relaxed);
        (*this).refs.fetch_sub(1, Ordering::Relaxed) - 1
    }

    #[test]
    fn mouse_rising_edge_reasserts_activation_during_keyboard_delivery() {
        assert!(should_post_activation(true, false, false, false));
        assert!(should_post_activation(true, true, true, false));
        assert!(!should_post_activation(true, true, true, true));
        assert!(!should_post_activation(true, true, false, true));
        assert!(!should_post_activation(false, true, true, false));
    }

    #[test]
    fn adopts_one_device_reference_and_passes_secondary_iids_through() {
        let mut real = Box::new(FakeUnknown {
            vtbl: &FAKE_VTBL,
            refs: AtomicU32::new(1),
            release_calls: AtomicU32::new(0),
        });
        let real_ptr = (&mut *real as *mut FakeUnknown).cast::<c_void>();
        let proxy = Box::into_raw(Box::new(DeviceProxy::from_owned(
            real_ptr,
            DeviceKind::Keyboard,
            crate::com_ids::IID_IDIRECTINPUTDEVICE8W,
        )));
        assert_eq!(real.refs.load(Ordering::Relaxed), 1);

        unsafe {
            let mut secondary = std::ptr::null_mut();
            assert!(dev_query_interface(proxy, &SECONDARY_IID, &mut secondary).is_ok());
            assert_eq!(secondary, real_ptr);
            assert_eq!(real.refs.load(Ordering::Relaxed), 2);
            assert_eq!(fake_release(secondary.cast()), 1);
            assert_eq!(dev_release(proxy), 0);
        }
        assert_eq!(real.refs.load(Ordering::Relaxed), 0);
        assert_eq!(real.release_calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn null_buffer_count_queries_preserve_the_real_event_count() {
        assert_eq!(real_event_count(HRESULT(0), 7, 0, true), 7);
        assert_eq!(real_event_count(HRESULT(0), 7, 2, false), 2);
        assert_eq!(real_event_count(DIERR_NOTACQUIRED, 7, 7, true), 0);
    }

    #[test]
    fn dx3_records_do_not_overwrite_the_caller_stride() {
        let mut buffered = BufferedKeyboardState::new();
        let mut keys = [0u8; 256];
        keys[0x1e] = 0x80;
        buffered.observe(keys, 1234);

        let mut bytes = [0xCCu8; DX3_OBJECT_DATA_SIZE + 8];
        let copied =
            unsafe { buffered.copy_to(bytes.as_mut_ptr(), DX3_OBJECT_DATA_SIZE, 1, false) };
        assert_eq!(copied, 1);
        assert_eq!(u32::from_ne_bytes(bytes[0..4].try_into().unwrap()), 0x1e);
        assert_eq!(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()), 0x80);
        assert_eq!(u32::from_ne_bytes(bytes[8..12].try_into().unwrap()), 1234);
        assert_eq!(&bytes[DX3_OBJECT_DATA_SIZE..], &[0xCC; 8]);
    }

    #[test]
    fn repeated_controller_key_edges_survive_one_observation() {
        let mut buffered = BufferedKeyboardState::new();
        let auto_type_keys = [0u8; 256];
        buffered.observe_controller_events(
            &[(0x26, true), (0x26, false), (0x26, true), (0x26, false)],
            &auto_type_keys,
            42,
        );

        assert_eq!(buffered.pending.len(), 4);
        let events: Vec<_> = buffered.pending.iter().collect();
        assert_eq!(
            events.iter().map(|event| event.dw_ofs).collect::<Vec<_>>(),
            vec![0x26; 4]
        );
        assert_eq!(
            events.iter().map(|event| event.dw_data).collect::<Vec<_>>(),
            vec![0x80, 0, 0x80, 0]
        );
    }

    #[test]
    fn partial_and_peeked_events_remain_queued_in_order() {
        let mut buffered = BufferedKeyboardState::new();
        let mut pressed = [0u8; 256];
        pressed[0x1e] = 0x80;
        pressed[0x30] = 0x80;
        buffered.observe(pressed, 10);

        let mut first = [0u8; DX3_OBJECT_DATA_SIZE];
        unsafe {
            assert_eq!(
                buffered.copy_to(first.as_mut_ptr(), DX3_OBJECT_DATA_SIZE, 1, false),
                1
            );
        }
        assert_eq!(buffered.pending.len(), 1);

        let mut peek_one = [0u8; DX3_OBJECT_DATA_SIZE];
        let mut peek_two = [0u8; DX3_OBJECT_DATA_SIZE];
        unsafe {
            assert_eq!(
                buffered.copy_to(peek_one.as_mut_ptr(), DX3_OBJECT_DATA_SIZE, 1, true),
                1
            );
            assert_eq!(
                buffered.copy_to(peek_two.as_mut_ptr(), DX3_OBJECT_DATA_SIZE, 1, true),
                1
            );
        }
        assert_eq!(peek_one, peek_two);
        assert_eq!(buffered.pending.len(), 1);

        let mut released = [0u8; 256];
        released[0x1e] = 0x80;
        buffered.observe(released, 20);
        assert_eq!(buffered.pending.len(), 2);

        let mut rest = [0u8; DX3_OBJECT_DATA_SIZE * 2];
        unsafe {
            assert_eq!(
                buffered.copy_to(rest.as_mut_ptr(), DX3_OBJECT_DATA_SIZE, 2, false),
                2
            );
        }
        assert!(buffered.pending.is_empty());
        assert_eq!(u32::from_ne_bytes(rest[0..4].try_into().unwrap()), 0x30);
        assert_eq!(u32::from_ne_bytes(rest[16..20].try_into().unwrap()), 0x30);
        assert_eq!(u32::from_ne_bytes(rest[20..24].try_into().unwrap()), 0);
    }
}
