use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicIsize, Ordering};
use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::SetEvent;

/// Event handle saved from SetEventNotification on the keyboard device.
/// The shm polling thread signals this to wake EQ's input loop.
static KB_EVENT_HANDLE: AtomicIsize = AtomicIsize::new(0);

/// HWND saved from SetCooperativeLevel on the keyboard device.
/// Used by the GetForegroundWindow IAT hook to trick EQ into processing keys.
static EQ_HWND: AtomicIsize = AtomicIsize::new(0);

/// Public accessor for the stored EQ HWND.
pub fn eq_hwnd() -> isize {
    EQ_HWND.load(Ordering::Acquire)
}

/// Spawned once to poll shm and signal the keyboard event.
static SHM_THREAD_STARTED: std::sync::Once = std::sync::Once::new();

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
    get_property:
        unsafe extern "system" fn(*mut DeviceProxy, *const GUID, *mut c_void) -> HRESULT,
    set_property:
        unsafe extern "system" fn(*mut DeviceProxy, *const GUID, *mut c_void) -> HRESULT,
    acquire: unsafe extern "system" fn(*mut DeviceProxy) -> HRESULT,
    unacquire: unsafe extern "system" fn(*mut DeviceProxy) -> HRESULT,
    get_device_state: unsafe extern "system" fn(*mut DeviceProxy, u32, *mut c_void) -> HRESULT,
    get_device_data: unsafe extern "system" fn(
        *mut DeviceProxy,
        u32,
        *mut c_void,
        *mut u32,
        u32,
    ) -> HRESULT,
    set_data_format: unsafe extern "system" fn(*mut DeviceProxy, *const c_void) -> HRESULT,
    set_event_notification: unsafe extern "system" fn(*mut DeviceProxy, isize) -> HRESULT,
    set_cooperative_level: unsafe extern "system" fn(*mut DeviceProxy, isize, u32) -> HRESULT,
    get_object_info:
        unsafe extern "system" fn(*mut DeviceProxy, *mut c_void, u32, u32) -> HRESULT,
    get_device_info: unsafe extern "system" fn(*mut DeviceProxy, *mut c_void) -> HRESULT,
    run_control_panel: unsafe extern "system" fn(*mut DeviceProxy, isize, u32) -> HRESULT,
    initialize:
        unsafe extern "system" fn(*mut DeviceProxy, isize, u32, *const GUID) -> HRESULT,
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

/// Our proxy for IDirectInputDevice8. COM layout: vtable pointer first.
#[repr(C)]
pub struct DeviceProxy {
    vtbl: *const IDirectInputDevice8Vtbl,
    real: *mut c_void,
    ref_count: AtomicU32,
    is_keyboard: bool,
}

impl DeviceProxy {
    pub fn new(real: *mut c_void, is_keyboard: bool) -> Self {
        // AddRef on the real device.
        unsafe {
            let real_vtbl = *(real as *const *const *const c_void);
            let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*real_vtbl.add(1));
            add_ref(real);
        }

        Self {
            vtbl: &DEV_VTBL,
            real,
            ref_count: AtomicU32::new(1),
            is_keyboard,
        }
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
    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT =
        real_method(real, 0);
    let hr = method(real, riid, ppv);
    if hr.is_ok() {
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = real_method(real, 2);
        release(*ppv);
        (*this).ref_count.fetch_add(1, Ordering::Relaxed);
        *ppv = this as *mut c_void;
    }
    hr
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
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT =
        real_method((*this).real, 7);
    let hr = method((*this).real);
    if hr.is_err() && (*this).is_keyboard && crate::key_shm::is_active() {
        return HRESULT(0); // DI_OK
    }
    hr
}

unsafe extern "system" fn dev_unacquire(this: *mut DeviceProxy) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT =
        real_method((*this).real, 8);
    method((*this).real)
}

unsafe extern "system" fn dev_get_device_state(
    this: *mut DeviceProxy,
    cbdata: u32,
    lpvdata: *mut c_void,
) -> HRESULT {
    let method: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT =
        real_method((*this).real, 9);
    let hr = method((*this).real, cbdata, lpvdata);

    if (*this).is_keyboard {
        if hr.is_ok() {
            if crate::key_shm::should_suppress() {
                std::ptr::write_bytes(lpvdata as *mut u8, 0, cbdata as usize);
            }
            crate::key_shm::inject_keys(lpvdata as *mut u8, cbdata);
        } else {
            std::ptr::write_bytes(lpvdata as *mut u8, 0, cbdata as usize);
            if crate::key_shm::inject_keys(lpvdata as *mut u8, cbdata) {
                return HRESULT(0); // DI_OK
            }
        }
    }

    hr
}

/// Previous shared-memory key state for generating press/release events.
static mut PREV_SHM_KEYS: [u8; 256] = [0u8; 256];
/// Sequence counter for synthetic events (start high to avoid collisions).
static mut SYNTH_SEQUENCE: u32 = 0x8000_0000;
/// DIDEVICEOBJECTDATA layout (matches the C struct on 64-bit).
#[repr(C)]
struct DiDeviceObjectData {
    dw_ofs: u32,
    dw_data: u32,
    dw_time_stamp: u32,
    dw_sequence: u32,
    u_app_data: usize,
}

unsafe extern "system" fn dev_get_device_data(
    this: *mut DeviceProxy,
    cbobjectdata: u32,
    rgdod: *mut c_void,
    pdwinout: *mut u32,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut u32,
        u32,
    ) -> HRESULT = real_method((*this).real, 10);

    if !(*this).is_keyboard {
        return method((*this).real, cbobjectdata, rgdod, pdwinout, flags);
    }

    let original_capacity = if pdwinout.is_null() { 0u32 } else { *pdwinout };

    const DIGDD_PEEK: u32 = 0x01;
    let peek = (flags & DIGDD_PEEK) != 0;

    let hr = method((*this).real, cbobjectdata, rgdod, pdwinout, flags);

    let mut real_count = if hr.is_ok() { *pdwinout } else { 0 };

    if crate::key_shm::should_suppress() && real_count > 0 {
        real_count = 0;
    }

    let mut cur_keys = [0u8; 256];
    let shm_active = crate::key_shm::read_keys(&mut cur_keys);

    if !shm_active {
        if !peek {
            PREV_SHM_KEYS = [0u8; 256];
        }
        *pdwinout = real_count;
        return hr;
    }

    let mut changes: [(u8, u8); 256] = [(0, 0); 256];
    let mut num_changes = 0usize;
    for i in 0..256 {
        let prev = PREV_SHM_KEYS[i];
        let cur = cur_keys[i];
        if prev != cur {
            changes[num_changes] = (i as u8, cur);
            num_changes += 1;
        }
    }

    if num_changes == 0 {
        *pdwinout = real_count;
        return hr;
    }

    if rgdod.is_null() {
        *pdwinout = real_count + num_changes as u32;
        if !peek {
            PREV_SHM_KEYS = cur_keys;
        }
        return hr;
    }

    let buf_start = rgdod as *mut u8;
    let available = original_capacity.saturating_sub(real_count) as usize;
    let to_inject = num_changes.min(available);
    let timestamp = windows::Win32::System::SystemInformation::GetTickCount();

    for j in 0..to_inject {
        let (scan, val) = changes[j];
        let offset = (real_count as usize + j) * cbobjectdata as usize;
        let entry = buf_start.add(offset) as *mut DiDeviceObjectData;
        (*entry).dw_ofs = scan as u32;
        (*entry).dw_data = if val != 0 { 0x80 } else { 0x00 };
        (*entry).dw_time_stamp = timestamp;
        (*entry).dw_sequence = SYNTH_SEQUENCE;
        SYNTH_SEQUENCE = SYNTH_SEQUENCE.wrapping_add(1);
        (*entry).u_app_data = 0;
    }

    *pdwinout = real_count + to_inject as u32;
    if !peek {
        PREV_SHM_KEYS = cur_keys;
    }
    hr
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

    if (*this).is_keyboard && hevent != 0 {
        crate::log::write(&format!(
            "SetEventNotification: keyboard event handle=0x{hevent:X}"
        ));
        KB_EVENT_HANDLE.store(hevent, Ordering::Release);

        // Start the shm polling thread (once).
        SHM_THREAD_STARTED.call_once(|| {
            std::thread::spawn(|| {
                let mut prev_any_keys = false;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(8)); // ~120Hz

                    let mut keys = [0u8; 256];
                    let active = crate::key_shm::read_keys(&mut keys);
                    let any_keys = active && keys.iter().any(|&k| k != 0);

                    if any_keys || (prev_any_keys && !any_keys) {
                        let h = KB_EVENT_HANDLE.load(Ordering::Acquire);
                        if h != 0 {
                            let _ = SetEvent(HANDLE(h as *mut c_void));
                        }
                    }
                    prev_any_keys = any_keys;
                }
            });
        });
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
    if (*this).is_keyboard {
        const DISCL_EXCLUSIVE: u32 = 0x01;
        const DISCL_FOREGROUND: u32 = 0x04;
        const DISCL_NONEXCLUSIVE: u32 = 0x02;
        const DISCL_BACKGROUND: u32 = 0x08;

        EQ_HWND.store(hwnd, Ordering::Release);
        crate::log::write(&format!(
            "SetCooperativeLevel: keyboard hwnd=0x{hwnd:X}"
        ));

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
    let method: unsafe extern "system" fn(*mut c_void) -> HRESULT =
        real_method((*this).real, 25);
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
    let method: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        u32,
    ) -> HRESULT = real_method((*this).real, 29);
    method((*this).real, lpdiactionformat, username, flags)
}

unsafe extern "system" fn dev_set_action_map(
    this: *mut DeviceProxy,
    lpdiactionformat: *mut c_void,
    username: *const c_void,
    flags: u32,
) -> HRESULT {
    let method: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        u32,
    ) -> HRESULT = real_method((*this).real, 30);
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
