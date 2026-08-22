use crate::device_proxy::{DeviceKind, DeviceProxy};
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

/// GUID_SysMouse from dinput.h.
const GUID_SYS_MOUSE: GUID = GUID {
    data1: 0x6F1D2B60,
    data2: 0xD5A0,
    data3: 0x11CF,
    data4: [0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00],
};

fn classify_device(guid: GUID) -> DeviceKind {
    if guid == GUID_SYS_KEYBOARD {
        DeviceKind::Keyboard
    } else if guid == GUID_SYS_MOUSE {
        DeviceKind::Mouse
    } else {
        DeviceKind::Other
    }
}

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
    enum_devices:
        unsafe extern "system" fn(*mut DI8Proxy, u32, *mut c_void, *mut c_void, u32) -> HRESULT,
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
    configure_devices: unsafe extern "system" fn(
        *mut DI8Proxy,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> HRESULT,
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
    configure_devices: di8_configure_devices,
};

/// Our proxy for IDirectInput8. COM layout: vtable pointer first.
#[repr(C)]
pub struct DI8Proxy {
    vtbl: *const IDirectInput8Vtbl,
    real: *mut c_void, // The real IDirectInput8 interface
    ref_count: AtomicU32,
    interface_iid: GUID,
}

impl DI8Proxy {
    /// Adopt the owned reference returned by `DirectInput8Create`.
    pub fn from_owned(real: *mut c_void, interface_iid: GUID) -> Self {
        debug_assert!(crate::com_ids::is_direct_input_8(interface_iid));
        Self {
            vtbl: &VTBL,
            real,
            ref_count: AtomicU32::new(1),
            interface_iid,
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
    const E_POINTER: HRESULT = HRESULT(0x8000_4003u32 as i32);
    if riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    *ppv = std::ptr::null_mut();

    let iid = *riid;
    if iid == crate::com_ids::IID_IUNKNOWN || iid == (*this).interface_iid {
        di8_add_ref(this);
        *ppv = this.cast();
        return HRESULT(0);
    }

    let real = (*this).real;
    let method: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT =
        real_method(real, 0);
    method(real, riid, ppv)
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
        if !real_device.is_null() {
            let guid = *rguid;
            let device_iid = crate::com_ids::device_iid_for((*this).interface_iid)
                .expect("DI8Proxy always stores an IDirectInput8 IID");
            let proxy = DeviceProxy::from_owned(real_device, classify_device(guid), device_iid);
            let proxy_ptr = Box::into_raw(Box::new(proxy));
            *ppdev = proxy_ptr as *mut c_void;
        }
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

unsafe extern "system" fn di8_get_device_status(
    this: *mut DI8Proxy,
    rguid: *const GUID,
) -> HRESULT {
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

unsafe extern "system" fn di8_configure_devices(
    this: *mut DI8Proxy,
    callback: *mut c_void,
    params: *mut c_void,
    flags: u32,
    ref_data: *mut c_void,
) -> HRESULT {
    let real = (*this).real;
    let method: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        u32,
        *mut c_void,
    ) -> HRESULT = real_method(real, 10);
    method(real, callback, params, flags, ref_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECONDARY_IID: GUID = GUID::from_u128(0xd29d40c8_a296_42ad_95b8_5fb8d2402682);

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
    fn classifies_system_keyboard_mouse_and_other_devices() {
        assert_eq!(classify_device(GUID_SYS_KEYBOARD), DeviceKind::Keyboard);
        assert_eq!(classify_device(GUID_SYS_MOUSE), DeviceKind::Mouse);
        assert_eq!(
            classify_device(GUID {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0; 8],
            }),
            DeviceKind::Other
        );
    }

    #[test]
    fn configure_devices_forwards_the_complete_abi() {
        #[repr(C)]
        struct ConfigureTarget {
            vtbl: *const *const c_void,
            callback: usize,
            params: usize,
            flags: u32,
            ref_data: usize,
        }

        unsafe extern "system" fn configure(
            this: *mut c_void,
            callback: *mut c_void,
            params: *mut c_void,
            flags: u32,
            ref_data: *mut c_void,
        ) -> HRESULT {
            let target = &mut *this.cast::<ConfigureTarget>();
            target.callback = callback as usize;
            target.params = params as usize;
            target.flags = flags;
            target.ref_data = ref_data as usize;
            HRESULT(7)
        }

        let mut vtable = [std::ptr::null::<c_void>(); 11];
        vtable[10] = configure as *const c_void;
        let mut target = ConfigureTarget {
            vtbl: vtable.as_ptr(),
            callback: 0,
            params: 0,
            flags: 0,
            ref_data: 0,
        };
        let mut proxy = DI8Proxy::from_owned(
            (&mut target as *mut ConfigureTarget).cast(),
            crate::com_ids::IID_IDIRECTINPUT8W,
        );
        let callback = 0x1111usize as *mut c_void;
        let params = 0x2222usize as *mut c_void;
        let ref_data = 0x3333usize as *mut c_void;

        let hr = unsafe { di8_configure_devices(&mut proxy, callback, params, 0x4444, ref_data) };
        assert_eq!(hr, HRESULT(7));
        assert_eq!(target.callback, callback as usize);
        assert_eq!(target.params, params as usize);
        assert_eq!(target.flags, 0x4444);
        assert_eq!(target.ref_data, ref_data as usize);
        // The stack proxy is not released through COM because this forwarding
        // target deliberately defines only the exercised slot.
    }

    #[test]
    fn adopts_one_real_reference_and_only_substitutes_implemented_iids() {
        let mut real = Box::new(FakeUnknown {
            vtbl: &FAKE_VTBL,
            refs: AtomicU32::new(1),
            release_calls: AtomicU32::new(0),
        });
        let real_ptr = (&mut *real as *mut FakeUnknown).cast::<c_void>();
        let proxy = Box::into_raw(Box::new(DI8Proxy::from_owned(
            real_ptr,
            crate::com_ids::IID_IDIRECTINPUT8W,
        )));
        assert_eq!(real.refs.load(Ordering::Relaxed), 1);

        unsafe {
            let mut same = std::ptr::null_mut();
            assert!(
                di8_query_interface(proxy, &crate::com_ids::IID_IDIRECTINPUT8W, &mut same).is_ok()
            );
            assert_eq!(same, proxy.cast());
            assert_eq!(real.refs.load(Ordering::Relaxed), 1);
            assert_eq!(di8_release(proxy), 1);

            let mut secondary = std::ptr::null_mut();
            assert!(di8_query_interface(proxy, &SECONDARY_IID, &mut secondary).is_ok());
            assert_eq!(secondary, real_ptr);
            assert_eq!(real.refs.load(Ordering::Relaxed), 2);
            assert_eq!(fake_release(secondary.cast()), 1);

            assert_eq!(di8_release(proxy), 0);
        }
        assert_eq!(real.refs.load(Ordering::Relaxed), 0);
        assert_eq!(real.release_calls.load(Ordering::Relaxed), 2);
    }
}
