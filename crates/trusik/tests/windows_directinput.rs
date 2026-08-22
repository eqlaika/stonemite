#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use trusik_protocol::{SharedKeyState, KEYBOARD_TARGETED, PROXY_READY, SHARED_KEY_STATE_SIZE};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HINSTANCE, INVALID_HANDLE_VALUE};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
    MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::GetCurrentProcessId;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDIRECTINPUT8W: GUID = GUID::from_u128(0xbf798031_483a_4da2_aa99_5d64ed369700);
const IID_IDIRECTINPUTDEVICE8W: GUID = GUID::from_u128(0x54d41081_dc15_4833_a41b_748f73a38179);
const GUID_SYS_KEYBOARD: GUID = GUID::from_u128(0x6f1d2b61_d5a0_11cf_bfc7_444553540000);
const UNKNOWN_IID: GUID = GUID::from_u128(0x4ba44c20_25e0_4f5d_8779_ec7f4d10eab8);
const DIRECT_INPUT_VERSION: u32 = 0x0800;
const DIGDD_PEEK: u32 = 0x01;
const DX3_RECORD_SIZE: usize = 16;

type DirectInput8CreateFn = unsafe extern "system" fn(
    HINSTANCE,
    u32,
    *const GUID,
    *mut *mut c_void,
    *mut c_void,
) -> HRESULT;
type QueryInterfaceFn =
    unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
type CreateDeviceFn =
    unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void, *mut c_void) -> HRESULT;
type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT;
type GetDeviceDataFn =
    unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut u32, u32) -> HRESULT;
type SetCooperativeLevelFn = unsafe extern "system" fn(*mut c_void, isize, u32) -> HRESULT;
type GetForegroundWindowFn = unsafe extern "system" fn() -> isize;

struct InputMapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    ptr: *mut SharedKeyState,
}

impl InputMapping {
    unsafe fn create() -> Self {
        let name = format!("Local\\DI8_{}\0", GetCurrentProcessId());
        let wide: Vec<u16> = name.encode_utf16().collect();
        let handle = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            SHARED_KEY_STATE_SIZE as u32,
            PCWSTR(wide.as_ptr()),
        )
        .expect("create integration-test input mapping");
        let view = MapViewOfFile(
            handle,
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            0,
            SHARED_KEY_STATE_SIZE,
        );
        let ptr = view.Value.cast::<SharedKeyState>();
        assert!(!ptr.is_null());
        SharedKeyState::initialize(ptr);
        let state = &*ptr;
        let now = GetTickCount().max(1);
        state.refresh_controller_heartbeat(now);
        state
            .controller_keyboard_active
            .store(KEYBOARD_TARGETED, Ordering::Release);
        state.suppress.store(1, Ordering::Release);
        Self { handle, view, ptr }
    }

    fn state(&self) -> &SharedKeyState {
        unsafe { &*self.ptr }
    }

    fn refresh(&self) {
        self.state()
            .refresh_controller_heartbeat(unsafe { GetTickCount() }.max(1));
    }

    fn set_key(&self, scan: usize, pressed: bool) {
        self.refresh();
        self.state()
            .set_controller_key(scan, if pressed { 0x80 } else { 0 });
    }
}

impl Drop for InputMapping {
    fn drop(&mut self) {
        self.state().retire_controller();
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

unsafe fn vtable_method<T>(interface: *mut c_void, index: usize) -> T {
    let vtable = *(interface as *const *const *const c_void);
    std::mem::transmute_copy(&*vtable.add(index))
}

unsafe fn release(interface: *mut c_void) -> u32 {
    let release: ReleaseFn = vtable_method(interface, 2);
    release(interface)
}

fn proxy_dll_path() -> PathBuf {
    let executable = std::env::current_exe().expect("integration-test executable path");
    let deps = executable.parent().expect("deps directory");
    let candidates = [
        deps.join("dinput8.dll"),
        deps.parent().unwrap().join("dinput8.dll"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("dinput8.dll not found beside {}", executable.display()))
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn field(record: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(record[offset..offset + 4].try_into().unwrap())
}

unsafe fn get_data(
    method: GetDeviceDataFn,
    device: *mut c_void,
    capacity: u32,
    flags: u32,
) -> (HRESULT, u32, [u8; DX3_RECORD_SIZE]) {
    let mut record = [0u8; DX3_RECORD_SIZE];
    let mut count = capacity;
    let hr = method(
        device,
        DX3_RECORD_SIZE as u32,
        record.as_mut_ptr().cast(),
        &mut count,
        flags,
    );
    (hr, count, record)
}

#[test]
fn loaded_proxy_wraps_real_direct_input_and_preserves_buffered_events() {
    assert_eq!(dinput8::integration_test_marker(), trusik_protocol::VERSION);
    unsafe {
        let mapping = InputMapping::create();
        let dll_path = proxy_dll_path();
        let wide = wide_path(&dll_path);
        let proxy_module = LoadLibraryW(PCWSTR(wide.as_ptr())).expect("load proxy dinput8.dll");
        let create_proc = GetProcAddress(proxy_module, windows::core::s!("DirectInput8Create"))
            .expect("proxy DirectInput8Create export");
        let create: DirectInput8CreateFn = std::mem::transmute(create_proc);

        let host_module = GetModuleHandleW(None).expect("test host module");
        let mut direct_input = std::ptr::null_mut();
        let hr = create(
            HINSTANCE(host_module.0),
            DIRECT_INPUT_VERSION,
            &IID_IDIRECTINPUT8W,
            &mut direct_input,
            std::ptr::null_mut(),
        );
        assert!(hr.is_ok(), "DirectInput8Create failed: {hr:?}");
        assert!(!direct_input.is_null());
        assert_eq!(
            mapping.state().proxy_ready.load(Ordering::Acquire),
            PROXY_READY
        );

        let query: QueryInterfaceFn = vtable_method(direct_input, 0);
        for iid in [IID_IUNKNOWN, IID_IDIRECTINPUT8W] {
            let mut queried = std::ptr::null_mut();
            assert!(query(direct_input, &iid, &mut queried).is_ok());
            assert_eq!(queried, direct_input, "proxy identity changed for {iid:?}");
            release(queried);
        }
        let mut unsupported = 1usize as *mut c_void;
        assert!(query(direct_input, &UNKNOWN_IID, &mut unsupported).is_err());
        assert!(unsupported.is_null());

        let create_device: CreateDeviceFn = vtable_method(direct_input, 3);
        let mut keyboard = std::ptr::null_mut();
        let hr = create_device(
            direct_input,
            &GUID_SYS_KEYBOARD,
            &mut keyboard,
            std::ptr::null_mut(),
        );
        assert!(hr.is_ok(), "CreateDevice(SysKeyboard) failed: {hr:?}");
        assert!(!keyboard.is_null());

        let device_query: QueryInterfaceFn = vtable_method(keyboard, 0);
        let mut queried_device = std::ptr::null_mut();
        assert!(device_query(keyboard, &IID_IDIRECTINPUTDEVICE8W, &mut queried_device).is_ok());
        assert_eq!(queried_device, keyboard);
        release(queried_device);

        // EQ obtains GetForegroundWindow through a path outside its main IAT.
        // Exercise the real COM SetCooperativeLevel path that captures EQ's
        // HWND, then resolve User32 dynamically to prove the safe process-wide
        // detour supplies that HWND while background delivery is active.
        let set_cooperative_level: SetCooperativeLevelFn = vtable_method(keyboard, 13);
        let synthetic_hwnd = 0x1234isize;
        let _ = set_cooperative_level(keyboard, synthetic_hwnd, 0x06);
        mapping.refresh();
        assert!(mapping
            .state()
            .controller_keyboard_is_active(GetTickCount()));
        let user32 = LoadLibraryW(windows::core::w!("user32.dll")).expect("load user32.dll");
        let foreground_proc = GetProcAddress(user32, windows::core::s!("GetForegroundWindow"))
            .expect("resolve GetForegroundWindow");
        let get_foreground_window: GetForegroundWindowFn = foreground_proc;
        assert_eq!(
            get_foreground_window(),
            synthetic_hwnd,
            "dynamic GetForegroundWindow bypassed the background-input detour"
        );

        let get_device_state: GetDeviceStateFn = vtable_method(keyboard, 9);
        assert!(get_device_state(keyboard, 256, std::ptr::null_mut()).is_err());

        let get_device_data: GetDeviceDataFn = vtable_method(keyboard, 10);

        // A real DirectInput device supplies the underlying call. Even though
        // it is not acquired/buffered in this isolated harness, the proxy must
        // deliver its synthetic stream safely.
        mapping.set_key(0x1e, true);
        let mut guarded = [0xCCu8; DX3_RECORD_SIZE + 8];
        let mut count = 1u32;
        let hr = get_device_data(
            keyboard,
            DX3_RECORD_SIZE as u32,
            guarded.as_mut_ptr().cast(),
            &mut count,
            0,
        );
        assert!(hr.is_ok(), "synthetic DX3 GetDeviceData failed: {hr:?}");
        assert_eq!(count, 1);
        assert_eq!(field(&guarded, 0), 0x1e);
        assert_eq!(field(&guarded, 4), 0x80);
        assert_eq!(&guarded[DX3_RECORD_SIZE..], &[0xCC; 8]);

        // Two transitions with capacity one must be delivered over two calls.
        mapping.set_key(0x1e, false);
        mapping.set_key(0x30, true);
        let (_, count, first) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&first, 0), 0x1e);
        assert_eq!(field(&first, 4), 0);
        let (_, count, second) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&second, 0), 0x30);
        assert_eq!(field(&second, 4), 0x80);

        // Peek is byte-stable and does not consume.
        mapping.set_key(0x20, true);
        let (_, count, peek_one) = get_data(get_device_data, keyboard, 1, DIGDD_PEEK);
        assert_eq!(count, 1);
        let (_, count, peek_two) = get_data(get_device_data, keyboard, 1, DIGDD_PEEK);
        assert_eq!(count, 1);
        assert_eq!(peek_one, peek_two);
        let (_, count, consumed) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(consumed, peek_one);

        // A null-buffer count query reports but does not consume.
        mapping.set_key(0x20, false);
        let mut pending_count = 0u32;
        let count_hr = get_device_data(
            keyboard,
            DX3_RECORD_SIZE as u32,
            std::ptr::null_mut(),
            &mut pending_count,
            0,
        );
        assert!(count_hr.is_ok());
        assert_eq!(pending_count, 1);
        let (_, count, release_record) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&release_record, 0), 0x20);
        assert_eq!(field(&release_record, 4), 0);

        // Unsupported record widths preserve the real error and never touch
        // caller storage; the observed event remains queued for a valid call.
        mapping.set_key(0x21, true);
        let mut invalid = [0xCCu8; DX3_RECORD_SIZE + 8];
        let mut invalid_count = 1u32;
        let invalid_hr = get_device_data(
            keyboard,
            (DX3_RECORD_SIZE - 1) as u32,
            invalid.as_mut_ptr().cast(),
            &mut invalid_count,
            0,
        );
        assert!(invalid_hr.is_err());
        assert_eq!(invalid, [0xCC; DX3_RECORD_SIZE + 8]);
        let (_, count, press_record) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&press_record, 0), 0x21);
        mapping.set_key(0x21, false);
        let (_, count, release_record) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&release_record, 0), 0x21);

        // A level-only transport loses the middle release/press when the same
        // key is typed twice between DirectInput polls (for example, /follow).
        // The controller event ring must preserve all four edges.
        mapping.set_key(0x26, true);
        mapping.set_key(0x26, false);
        mapping.set_key(0x26, true);
        mapping.set_key(0x26, false);
        let mut repeated = [0u8; DX3_RECORD_SIZE * 4];
        let mut repeated_count = 4u32;
        let repeated_hr = get_device_data(
            keyboard,
            DX3_RECORD_SIZE as u32,
            repeated.as_mut_ptr().cast(),
            &mut repeated_count,
            0,
        );
        assert!(repeated_hr.is_ok());
        assert_eq!(repeated_count, 4);
        for (index, expected_data) in [0x80, 0, 0x80, 0].into_iter().enumerate() {
            let record = &repeated[index * DX3_RECORD_SIZE..][..DX3_RECORD_SIZE];
            assert_eq!(field(record, 0), 0x26);
            assert_eq!(field(record, 4), expected_data);
        }

        // Owner expiry/deactivation generates release events instead of
        // silently forgetting a previously observed pressed key.
        mapping
            .state()
            .controller_keyboard_active
            .store(0, Ordering::Release);
        let (_, count, release_record) = get_data(get_device_data, keyboard, 1, 0);
        assert_eq!(count, 1);
        assert_eq!(field(&release_record, 0), 0x30);
        assert_eq!(field(&release_record, 4), 0);

        assert_eq!(release(keyboard), 0);
        assert_eq!(release(direct_input), 0);
        // The initialized proxy intentionally pins itself because hooks and
        // polling callbacks are process-lifetime resources.
    }
}
