use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, HMODULE, HWND, LPARAM, RECT, TRUE, UNICODE_STRING,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::ProcessStatus::{K32EnumProcesses, K32GetModuleFileNameExW};
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

/// Return lowercase account usernames from `/login:` arguments on running EQ processes.
pub fn find_eq_login_usernames() -> HashSet<String> {
    find_eq_process_ids()
        .into_iter()
        .filter_map(|pid| unsafe { process_command_line(pid) })
        .filter_map(|command_line| login_username_from_command_line(&command_line))
        .map(|username| username.to_ascii_lowercase())
        .collect()
}

fn find_eq_process_ids() -> Vec<u32> {
    let mut pids = vec![0u32; 1024];

    loop {
        let mut bytes_needed = 0u32;
        let buffer_bytes = (pids.len() * std::mem::size_of::<u32>()) as u32;
        if !unsafe { K32EnumProcesses(pids.as_mut_ptr(), buffer_bytes, &mut bytes_needed) }
            .as_bool()
        {
            return Vec::new();
        }

        let count = bytes_needed as usize / std::mem::size_of::<u32>();
        if count < pids.len() {
            pids.truncate(count);
            break;
        }
        pids.resize(pids.len() * 2, 0);
    }

    pids.retain(|pid| *pid != 0 && unsafe { is_eqgame_process(*pid) });
    pids
}

unsafe fn process_command_line(pid: u32) -> Option<String> {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let result = (|| {
        let mut bytes_needed = 0u32;
        let _ = NtQueryInformationProcess(
            handle,
            ProcessCommandLineInformation,
            std::ptr::null_mut(),
            0,
            &mut bytes_needed,
        );
        if bytes_needed < std::mem::size_of::<UNICODE_STRING>() as u32 {
            return None;
        }

        let mut buffer = vec![0u8; bytes_needed as usize];
        let status = NtQueryInformationProcess(
            handle,
            ProcessCommandLineInformation,
            buffer.as_mut_ptr().cast(),
            bytes_needed,
            &mut bytes_needed,
        );
        if status.is_err() {
            return None;
        }

        let command_line = std::ptr::read_unaligned(buffer.as_ptr().cast::<UNICODE_STRING>());
        let byte_length = usize::from(command_line.Length);
        if byte_length == 0 || byte_length % std::mem::size_of::<u16>() != 0 {
            return None;
        }

        let start = command_line.Buffer.as_ptr() as usize;
        let end = start.checked_add(byte_length)?;
        let buffer_start = buffer.as_ptr() as usize;
        let buffer_end = buffer_start.checked_add(buffer.len())?;
        if start < buffer_start || end > buffer_end {
            return None;
        }

        let wide = std::slice::from_raw_parts(
            command_line.Buffer.as_ptr(),
            byte_length / std::mem::size_of::<u16>(),
        );
        Some(String::from_utf16_lossy(wide))
    })();
    let _ = CloseHandle(handle);
    result
}

fn login_username_from_command_line(command_line: &str) -> Option<String> {
    command_line.split_ascii_whitespace().find_map(|argument| {
        let argument = argument.trim_matches('"');
        let (switch, username) = argument.split_once(':')?;
        if !switch.eq_ignore_ascii_case("/login") {
            return None;
        }
        let username = username.trim_matches('"').trim();
        (!username.is_empty()).then(|| username.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_login_username_from_eq_command_line() {
        assert_eq!(
            login_username_from_command_line(
                r#""C:\Program Files\EverQuest\eqgame.exe" patchme /login:TestAccount"#,
            ),
            Some("TestAccount".into())
        );
        assert_eq!(
            login_username_from_command_line("eqgame.exe patchme /LOGIN:testaccount"),
            Some("testaccount".into())
        );
    }

    #[test]
    fn reads_current_process_command_line() {
        let command_line = unsafe { process_command_line(std::process::id()) };

        assert!(command_line.is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn ignores_missing_or_empty_login_arguments() {
        assert_eq!(login_username_from_command_line("eqgame.exe patchme"), None);
        assert_eq!(
            login_username_from_command_line("eqgame.exe patchme /login:"),
            None
        );
        assert_eq!(
            login_username_from_command_line("eqgame.exe patchme /notlogin:TestAccount"),
            None
        );
    }
}
