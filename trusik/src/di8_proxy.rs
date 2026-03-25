use crate::device_proxy::DeviceProxy;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::core::{GUID, HRESULT};

/// GUID_SysKeyboard from dinput.h.
const GUID_SYS_KEYBOARD: GUID = GUID {
    data1: 0x6F1D2B61,
    data2: 0xD5A0,
    data3: 0x11CF,
    data4: [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00],
};

/// Raw COM vtable for IDirectInput8 (A or W — layouts are identical).
///
/// 3 IUnknown methods + 8 IDirectInput8 methods = 11 entries.
#[repr(C)]
struct IDirectInput8Vtbl {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut DI8Proxy, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DI8Proxy) -> u32,
    release: unsafe extern "system" fn(*mut DI8Proxy) -> u32,

    // IDirectInput8
    create_device: unsafe extern "system" fn(
        *mut DI8Proxy,
        *const GUID,
        *mut *mut c_void,
        *mut c_void,
    ) -> HRESULT,
    enum_devices: unsafe extern "system" fn(
        *mut DI8Proxy,
        u32,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT,
    get_device_status: unsafe extern "system" fn(*mut DI8Proxy, *const GUID) -> HRESULT,
    run_control_panel: unsafe extern "system" fn(*mut DI8Proxy, isize, u32) -> HRESULT,
    initialize: unsafe extern "system" fn(*mut DI8Proxy, isize, u32) -> HRESULT,
    find_device:
        unsafe extern "system" fn(*mut DI8Proxy, *const GUID, *const c_void, *mut GUID) -> HRESULT,
    enum_devices_by_semantics: unsafe extern "system" fn(
        *mut DI8Proxy,
        *const c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT,
    config_interface: unsafe extern "system" fn(*mut DI8Proxy, *mut c_void) -> HRESULT,
}

/// Shared static vtable for all DI8Proxy instances.
static VTBL: IDirectInput8Vtbl = IDirectInput8Vtbl {
    query_interface: di8_query_interface,
    add_ref: di8_add_ref,
    release: di8_release,
    create_device: di8_create_device,
    enum_devices: di8_enum_devices,
    get_device_status: di8_get_device_status,
    run_control_panel: di8_run_control_panel,
    initialize: di8_initialize,
    find_device: di8_find_device,
    enum_devices_by_semantics: di8_enum_devices_by_semantics,
    config_interface: di8_config_interface,
};

/// Our proxy for IDirectInput8. COM layout: vtable pointer first.
#[repr(C)]
pub struct DI8Proxy {
    vtbl: *const IDirectInput8Vtbl,
    real: *mut c_void, // The real IDirectInput8 interface
    ref_count: AtomicU32,
}

impl DI8Proxy {
    pub fn new(real: *mut c_void) -> Self {
        // AddRef on the real interface since we're holding a reference.
        unsafe {
            let real_vtbl = *(real as *const *const *const c_void);
            let add_ref: unsafe extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*real_vtbl.add(1));
            add_ref(real);
        }

        Self {
            vtbl: &VTBL,
            real,
            ref_count: AtomicU32::new(1),
        }
    }
}

/// Call a method on the real COM interface by vtable index.
/// Returns the raw function pointer cast to the caller's desired signature.
unsafe fn real_method<T>(real: *mut c_void, index: usize) -> T {
    let real_vtbl = *(real as *const *const *const c_void);
    std::mem::transmute_copy(&*real_vtbl.add(index))
}

// --- IUnknown ---

unsafe extern "system" fn di8_query_interface(
    this: *mut DI8Proxy,
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

unsafe extern "system" fn di8_add_ref(this: *mut DI8Proxy) -> u32 {
    (*this).ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn di8_release(this: *mut DI8Proxy) -> u32 {
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

// --- IDirectInput8 methods ---

unsafe extern "system" fn di8_create_device(
    this: *mut DI8Proxy,
    rguid: *const GUID,
    ppdev: *mut *mut c_void,
    punk_outer: *mut c_void,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
        *mut c_void,
    ) -> HRESULT = real_method(real, 3);
    let hr = method(real, rguid, ppdev, punk_outer);
    if hr.is_ok() {
        let real_device = *ppdev;
        let guid = *rguid;
        let is_keyboard = guid == GUID_SYS_KEYBOARD;
        let proxy = DeviceProxy::new(real_device, is_keyboard);
        let proxy_ptr = Box::into_raw(Box::new(proxy));
        *ppdev = proxy_ptr as *mut c_void;
    }
    hr
}

unsafe extern "system" fn di8_enum_devices(
    this: *mut DI8Proxy,
    dev_type: u32,
    callback: *mut c_void,
    pvref: *mut c_void,
    flags: u32,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT = real_method(real, 4);
    method(real, dev_type, callback, pvref, flags)
}

unsafe extern "system" fn di8_get_device_status(this: *mut DI8Proxy, rguid: *const GUID) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, *const GUID) -> HRESULT =
        real_method(real, 5);
    method(real, rguid)
}

unsafe extern "system" fn di8_run_control_panel(
    this: *mut DI8Proxy,
    hwnd_owner: isize,
    flags: u32,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT =
        real_method(real, 6);
    method(real, hwnd_owner, flags)
}

unsafe extern "system" fn di8_initialize(
    this: *mut DI8Proxy,
    hinst: isize,
    dwversion: u32,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT =
        real_method(real, 7);
    method(real, hinst, dwversion)
}

unsafe extern "system" fn di8_find_device(
    this: *mut DI8Proxy,
    rguid_class: *const GUID,
    name: *const c_void,
    pguid_instance: *mut GUID,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const GUID,
        *const c_void,
        *mut GUID,
    ) -> HRESULT = real_method(real, 8);
    method(real, rguid_class, name, pguid_instance)
}

unsafe extern "system" fn di8_enum_devices_by_semantics(
    this: *mut DI8Proxy,
    user_name: *const c_void,
    action_format: *mut c_void,
    callback: *mut c_void,
    pvref: *mut c_void,
    flags: u32,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
    ) -> HRESULT = real_method(real, 9);
    method(real, user_name, action_format, callback, pvref, flags)
}

unsafe extern "system" fn di8_config_interface(
    this: *mut DI8Proxy,
    params: *mut c_void,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT =
        real_method(real, 10);
    method(real, params)
}
