//! Auto-type passwords into EQ login fields via trusik shared memory.
//!
//! Waits for DirectInput to initialize (signaled by trusik via a named event),
//! then writes an independently leased key buffer. The trusik device proxy
//! combines this owner with controller-owned broadcast input.

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use trusik_protocol::{SharedKeyState, SHARED_KEY_STATE_SIZE};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    OpenEventW, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC};

use crate::diagnostics::debug_log;

/// How long to hold each key down.
const KEY_DOWN_MS: u64 = 50;
/// Delay after releasing a key before pressing the next one.
const KEY_UP_MS: u64 = 50;
/// Maximum time to wait for DirectInput to initialize (ms).
const DI_WAIT_TIMEOUT_MS: u32 = 30_000;
/// Refresh well inside the proxy's 500 ms lease timeout.
const HEARTBEAT_REFRESH_MS: u64 = 100;

/// Owned mapping view and handle. The mapping may outlive this view because the
/// target proxy also keeps it mapped.
struct Shm {
    handle: HANDLE,
    ptr: *mut SharedKeyState,
}
unsafe impl Send for Shm {}

impl Drop for Shm {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Memory::UnmapViewOfFile(
                windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.ptr as *mut c_void,
                },
            );
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Generation-checked RAII lease. A newer auto-type job can supersede this one
/// without an older thread clearing or writing the newer owner's keys.
struct AutoTypeSession {
    shm: Shm,
    generation: u32,
}

impl AutoTypeSession {
    fn start(shm: Shm, pid: u32) -> Result<Self, String> {
        let state = unsafe { &*shm.ptr };
        if !state.is_compatible() || !state.proxy_is_ready() {
            return Err(format!(
                "incompatible trusik input mapping: magic={:#010x} version={} proxy_ready={:#04x}",
                state.magic.load(Ordering::Acquire),
                state.version.load(Ordering::Acquire),
                state.proxy_ready.load(Ordering::Acquire)
            ));
        }

        let now_ms = unsafe { GetTickCount64() as u32 };
        let generation = state.begin_auto_type(now_ms);
        debug_log(&format!(
            "auto_type: shm ready pid={pid} version={} generation={generation} mouse_active={} suppress={}",
            trusik_protocol::VERSION,
            state.controller_mouse_active.load(Ordering::Acquire),
            state.suppress.load(Ordering::Acquire)
        ));
        Ok(Self { shm, generation })
    }

    fn state(&self) -> &SharedKeyState {
        unsafe { &*self.shm.ptr }
    }

    fn refresh_heartbeat(&self) -> Result<(), String> {
        let now_ms = unsafe { GetTickCount64() as u32 };
        self.state()
            .refresh_auto_type_lease(self.generation, now_ms)
            .then_some(())
            .ok_or_else(|| "auto-type job was superseded".to_owned())
    }

    fn set_key(&self, scan: u8, pressed: bool) -> Result<(), String> {
        self.state()
            .set_auto_type_key(
                self.generation,
                scan as usize,
                if pressed { 0x80 } else { 0 },
            )
            .then_some(())
            .ok_or_else(|| "auto-type job was superseded".to_owned())?;
        self.refresh_heartbeat()
    }

    fn sleep(&self, duration: std::time::Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + duration;
        loop {
            self.refresh_heartbeat()?;
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(
                remaining.min(std::time::Duration::from_millis(HEARTBEAT_REFRESH_MS)),
            );
        }
    }
}

impl Drop for AutoTypeSession {
    fn drop(&mut self) {
        self.state().retire_auto_type(self.generation);
    }
}

/// Spawn a background thread that types `password` into the EQ process with the
/// given PID, then presses Enter to submit.
pub fn spawn(pid: u32, password: String) {
    let pw_len = password.len();
    debug_log(&format!("auto_type: spawn pid={pid} password_len={pw_len}"));

    // Create the mapping immediately so trusik finds it on its first open.
    let shm = match open_or_create_shm(pid) {
        Ok(shm) => shm,
        Err(error) => {
            debug_log(&format!(
                "auto_type: failed to create shm for pid={pid}: {error}"
            ));
            return;
        }
    };

    std::thread::spawn(move || {
        debug_log(&format!("auto_type: waiting for DI ready event pid={pid}"));
        if !wait_for_di_ready(pid) {
            debug_log(&format!("auto_type: DI ready timeout pid={pid}"));
            return;
        }
        debug_log(&format!(
            "auto_type: DI ready, starting type_password pid={pid}"
        ));

        // Brief pause to let DI fully settle. The source is not active yet.
        std::thread::sleep(std::time::Duration::from_millis(500));

        if let Err(error) = type_password(pid, &password, shm) {
            debug_log(&format!("auto_type: ERROR pid={pid}: {error}"));
        }
    });
}

/// Wait for the named event `Local\Stonemite_DI_{pid}` to be signaled.
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
            Ok(handle) => {
                let remaining = timeout.saturating_sub(start.elapsed());
                debug_log(&format!("auto_type: found DI event, waiting pid={pid}"));
                let result = unsafe { WaitForSingleObject(handle, remaining.as_millis() as u32) };
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return result.0 == 0; // WAIT_OBJECT_0
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

fn type_password(pid: u32, password: &str, shm: Shm) -> Result<(), String> {
    let session = AutoTypeSession::start(shm, pid)?;

    // Give trusik's activation thread time to post WM_ACTIVATEAPP(1).
    session.sleep(std::time::Duration::from_millis(200))?;

    for (index, character) in password.chars().enumerate() {
        type_char(&session, character, index, pid)?;
    }

    let enter_scan = vk_to_scan(0x0D);
    debug_log(&format!(
        "auto_type: pressing Enter (login) scan={enter_scan:#04x} pid={pid}"
    ));
    press_scancode(&session, enter_scan, false)?;

    debug_log(&format!(
        "auto_type: waiting 2s for server select pid={pid}"
    ));
    session.sleep(std::time::Duration::from_millis(2000))?;

    for index in 0..3 {
        debug_log(&format!(
            "auto_type: pressing Enter (server select {}) pid={pid}",
            index + 1
        ));
        press_scancode(&session, enter_scan, false)?;
        session.sleep(std::time::Duration::from_millis(1000))?;
    }

    debug_log(&format!("auto_type: done, deactivating shm pid={pid}"));
    Ok(())
}

fn open_or_create_shm(pid: u32) -> Result<Shm, String> {
    let name = format!("Local\\DI8_{pid}\0");
    let wide: Vec<u16> = name.encode_utf16().collect();

    debug_log(&format!("auto_type: opening shm Local\\DI8_{pid}"));

    unsafe {
        let handle = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            SHARED_KEY_STATE_SIZE as u32,
            windows::core::PCWSTR(wide.as_ptr()),
        )
        .map_err(|error| format!("CreateFileMappingW failed: {error}"))?;

        let existed = windows::Win32::Foundation::GetLastError()
            == windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
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
            return Err("MapViewOfFile returned null".into());
        }

        if existed {
            if !(*ptr).is_compatible() {
                let magic = (*ptr).magic.load(Ordering::Acquire);
                let version = (*ptr).version.load(Ordering::Acquire);
                let _ = windows::Win32::System::Memory::UnmapViewOfFile(view);
                let _ = CloseHandle(handle);
                return Err(format!(
                    "existing input mapping is incompatible: magic={magic:#010x} version={version}"
                ));
            }
            debug_log("auto_type: reusing compatible shm from broadcast engine");
        } else {
            SharedKeyState::initialize(ptr);
            debug_log("auto_type: initialized new input mapping");
        }

        Ok(Shm { handle, ptr })
    }
}

/// Convert a VK code to a DirectInput scan code.
fn vk_to_scan(vk: u32) -> u8 {
    unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) as u8 }
}

/// Type a single character by resolving it to VK + shift state.
fn type_char(
    session: &AutoTypeSession,
    character: char,
    index: usize,
    pid: u32,
) -> Result<(), String> {
    let result = unsafe { VkKeyScanW(character as u16) };
    if result == -1i16 {
        debug_log(&format!(
            "auto_type: no VK mapping for char[{index}]='{character}' pid={pid}"
        ));
        return Ok(());
    }
    let vk = (result & 0xFF) as u32;
    let shift_state = ((result >> 8) & 0xFF) as u8;
    let needs_shift = shift_state & 0x01 != 0;
    let scan = vk_to_scan(vk);
    if !is_usable_scan_code(scan) {
        debug_log(&format!(
            "auto_type: no usable scan code for char[{index}]='{character}' vk={vk:#04x} pid={pid}"
        ));
        return Ok(());
    }

    debug_log(&format!(
        "auto_type: char[{index}] vk={vk:#04x} scan={scan:#04x} shift={needs_shift} pid={pid}"
    ));
    press_scancode(session, scan, needs_shift)
}

/// Press and release a scan code, optionally with Shift held.
fn press_scancode(session: &AutoTypeSession, scan: u8, shift: bool) -> Result<(), String> {
    if !is_usable_scan_code(scan) {
        return Ok(());
    }
    let shift_scan = vk_to_scan(0x10); // VK_SHIFT
    if shift && !is_usable_scan_code(shift_scan) {
        return Ok(());
    }

    if shift {
        session.set_key(shift_scan, true)?;
    }
    session.set_key(scan, true)?;
    debug_log(&format!("auto_type: key down scan={scan:#04x}"));
    session.sleep(std::time::Duration::from_millis(KEY_DOWN_MS))?;

    session.set_key(scan, false)?;
    if shift {
        session.set_key(shift_scan, false)?;
    }
    session.sleep(std::time::Duration::from_millis(KEY_UP_MS))
}

fn is_usable_scan_code(scan: u8) -> bool {
    (1..=254).contains(&scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SharedKeyState {
        let mut state = std::mem::MaybeUninit::<SharedKeyState>::uninit();
        unsafe {
            SharedKeyState::initialize(state.as_mut_ptr());
            state.assume_init()
        }
    }

    #[test]
    fn newer_auto_type_generation_cannot_be_cleared_by_the_old_job() {
        let shared = state();
        let old = shared.begin_auto_type(100);
        assert!(shared.set_auto_type_key(old, 0x1e, 0x80));
        let new = shared.begin_auto_type(101);
        assert_ne!(old, new);
        assert!(shared.set_auto_type_key(new, 0x30, 0x80));
        assert!(!shared.set_auto_type_key(old, 0x1e, 0));
        shared.retire_auto_type(old);

        let mut keys = [0; 256];
        assert!(shared.read_effective_keys(101, &mut keys));
        assert_eq!(keys[0x1e], 0);
        assert_eq!(keys[0x30], 0x80);
    }

    #[test]
    fn shared_memory_scan_codes_exclude_reserved_boundaries() {
        assert!(!is_usable_scan_code(0));
        assert!(is_usable_scan_code(1));
        assert!(is_usable_scan_code(254));
        assert!(!is_usable_scan_code(255));
    }
}
