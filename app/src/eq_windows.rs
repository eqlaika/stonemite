use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, BOOL, HMODULE, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Debug, Clone)]
pub struct EqWindow {
    pub hwnd: HWND,
    pub pid: u32,
    /// Stable user-visible number (1-based), auto-assigned or user-set.
    pub number: usize,
    pub character: Option<String>,
    pub server: Option<String>,
    pub class: Option<String>,
}

/// Find all visible top-level windows belonging to eqgame.exe.
/// Returns in z-order (topmost first). Numbers are NOT assigned here.
pub fn find_eq_windows() -> Vec<EqWindow> {
    let mut windows: Vec<EqWindow> = Vec::new();

    unsafe {
        let _ = EnumWindows(Some(enum_callback), LPARAM(&mut windows as *mut _ as isize));
    }

    windows
}

unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<EqWindow>);

    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 {
        return TRUE;
    }

    if is_eqgame_process(pid) {
        windows.push(EqWindow {
            hwnd,
            pid,
            number: 0, // assigned by overlay
            character: None,
            server: None,
            class: None,
        });
    }

    TRUE
}

unsafe fn is_eqgame_process(pid: u32) -> bool {
    process_executable_path(pid).is_some_and(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("eqgame.exe"))
    })
}

/// Return the installation directory for one live EQ process.
///
/// Semantic keymaps live beside that process's `eqgame.exe`, which can differ
/// from Stonemite's configured installation when several EQ installs are open.
pub fn process_eq_directory(pid: u32) -> Option<PathBuf> {
    unsafe { process_executable_path(pid) }
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
}

unsafe fn process_executable_path(pid: u32) -> Option<PathBuf> {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 32_768];
    let len = K32GetModuleFileNameExW(handle, HMODULE::default(), &mut buf);
    let _ = CloseHandle(handle);
    (len > 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
}

/// Get the work area of the primary monitor based on an existing EQ window,
/// or the primary monitor if no window is provided.
pub fn get_monitor_work_area(reference_hwnd: Option<HWND>) -> RECT {
    unsafe {
        let monitor = match reference_hwnd {
            Some(hwnd) => MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY),
            None => MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY),
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool()
            && info.rcWork.right > info.rcWork.left
            && info.rcWork.bottom > info.rcWork.top
        {
            return info.rcWork;
        }
        // Fallback: primary monitor via SystemMetrics.
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        }
    }
}
