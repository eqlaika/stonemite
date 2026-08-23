use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, IsChild,
    IsIconic, IsWindow, PostMessageW, SendMessageTimeoutW, SetForegroundWindow, ShowWindowAsync,
    GUITHREADINFO, SMTO_ABORTIFHUNG, SW_RESTORE, WM_ACTIVATEAPP, WM_NULL,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForegroundRequest {
    Confirmed,
    TargetDisappeared,
    TargetUnresponsive,
    ForegroundDenied,
    FocusDenied,
}

/// A short-lived connection between two GUI input queues.
///
/// Keeping this guard scoped is important: attached queues process input as one
/// queue, and Windows resets their keyboard state when they are attached.
struct InputQueueAttachment {
    current_thread: u32,
    target_thread: u32,
    attached: bool,
}

impl InputQueueAttachment {
    unsafe fn attach(current_thread: u32, target_thread: u32) -> Option<Self> {
        if current_thread == target_thread {
            return Some(Self {
                current_thread,
                target_thread,
                attached: false,
            });
        }
        if !AttachThreadInput(current_thread, target_thread, true).as_bool() {
            return None;
        }
        Some(Self {
            current_thread,
            target_thread,
            attached: true,
        })
    }
}

impl Drop for InputQueueAttachment {
    fn drop(&mut self) {
        if self.attached {
            unsafe {
                let _ = AttachThreadInput(self.current_thread, self.target_thread, false);
            }
        }
    }
}

unsafe fn window_is_responsive(hwnd: HWND) -> bool {
    let mut result = 0usize;
    SendMessageTimeoutW(
        hwnd,
        WM_NULL,
        WPARAM(0),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&mut result),
    )
    .0 != 0
}

pub(super) unsafe fn target_has_keyboard_focus(hwnd: HWND) -> bool {
    if GetForegroundWindow() != hwnd {
        return false;
    }
    let target_thread = GetWindowThreadProcessId(hwnd, None);
    if target_thread == 0 {
        return false;
    }
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if GetGUIThreadInfo(target_thread, &mut info).is_err() || info.hwndActive != hwnd {
        return false;
    }
    info.hwndFocus == hwnd
        || (!info.hwndFocus.is_invalid() && IsChild(hwnd, info.hwndFocus).as_bool())
}

/// Once the target is foreground, repair a missing keyboard-focus assignment.
unsafe fn repair_keyboard_focus(hwnd: HWND) -> bool {
    let target_thread = GetWindowThreadProcessId(hwnd, None);
    if target_thread == 0 {
        return false;
    }
    let Some(attachment) = InputQueueAttachment::attach(GetCurrentThreadId(), target_thread) else {
        return false;
    };
    let _ = SetFocus(hwnd);
    drop(attachment);
    target_has_keyboard_focus(hwnd)
}

/// Ask EQ to reacquire DirectInput mouse state after confirmed activation.
pub(super) unsafe fn reassert_eq_mouse_activation(hwnd: HWND) {
    let _ = PostMessageW(hwnd, WM_ACTIVATEAPP, WPARAM(1), LPARAM(0));
}

unsafe fn confirm_foreground_and_focus(hwnd: HWND) -> ForegroundRequest {
    if !IsWindow(hwnd).as_bool() {
        return ForegroundRequest::TargetDisappeared;
    }
    if GetForegroundWindow() != hwnd {
        return ForegroundRequest::ForegroundDenied;
    }
    if target_has_keyboard_focus(hwnd) {
        return ForegroundRequest::Confirmed;
    }
    if !window_is_responsive(hwnd) {
        return if IsWindow(hwnd).as_bool() {
            ForegroundRequest::TargetUnresponsive
        } else {
            ForegroundRequest::TargetDisappeared
        };
    }
    if repair_keyboard_focus(hwnd) {
        ForegroundRequest::Confirmed
    } else if !IsWindow(hwnd).as_bool() {
        ForegroundRequest::TargetDisappeared
    } else if GetForegroundWindow() != hwnd {
        ForegroundRequest::ForegroundDenied
    } else {
        ForegroundRequest::FocusDenied
    }
}

unsafe fn denied_or_disappeared(hwnd: HWND) -> ForegroundRequest {
    if IsWindow(hwnd).as_bool() {
        ForegroundRequest::ForegroundDenied
    } else {
        ForegroundRequest::TargetDisappeared
    }
}

/// Bring a live EQ HWND to the foreground without mutating overlay state.
pub(super) unsafe fn request_foreground(hwnd: HWND) -> ForegroundRequest {
    if !IsWindow(hwnd).as_bool() {
        return ForegroundRequest::TargetDisappeared;
    }
    if GetForegroundWindow() == hwnd {
        return confirm_foreground_and_focus(hwnd);
    }

    if IsIconic(hwnd).as_bool() {
        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
    }
    let _ = SetForegroundWindow(hwnd);
    if GetForegroundWindow() == hwnd {
        return confirm_foreground_and_focus(hwnd);
    }

    if !window_is_responsive(hwnd) {
        return if IsWindow(hwnd).as_bool() {
            ForegroundRequest::TargetUnresponsive
        } else {
            ForegroundRequest::TargetDisappeared
        };
    }
    let foreground = GetForegroundWindow();
    if foreground.is_invalid() || !window_is_responsive(foreground) {
        return denied_or_disappeared(hwnd);
    }
    let current_thread = GetCurrentThreadId();
    let foreground_thread = GetWindowThreadProcessId(foreground, None);
    if foreground_thread == 0 {
        return denied_or_disappeared(hwnd);
    }
    let Some(attachment) = InputQueueAttachment::attach(current_thread, foreground_thread) else {
        return denied_or_disappeared(hwnd);
    };

    let _ = BringWindowToTop(hwnd);
    let _ = SetForegroundWindow(hwnd);
    drop(attachment);

    if GetForegroundWindow() == hwnd {
        confirm_foreground_and_focus(hwnd)
    } else {
        denied_or_disappeared(hwnd)
    }
}

pub(super) fn request_error(request: ForegroundRequest) -> trushar::control::ControlError {
    let (code, message) = match request {
        ForegroundRequest::TargetDisappeared => (
            trushar::control::ErrorCode::TargetDisappeared,
            "the target EQ window is no longer loaded",
        ),
        ForegroundRequest::TargetUnresponsive => (
            trushar::control::ErrorCode::ActivationFailed,
            "the target EQ window is not responding",
        ),
        ForegroundRequest::ForegroundDenied => (
            trushar::control::ErrorCode::ActivationFailed,
            "Windows did not bring the target EQ window to the foreground",
        ),
        ForegroundRequest::FocusDenied => (
            trushar::control::ErrorCode::ActivationFailed,
            "Windows foregrounded the target EQ window but did not give it keyboard focus",
        ),
        ForegroundRequest::Confirmed => (
            trushar::control::ErrorCode::ActivationFailed,
            "the target EQ window activation failed",
        ),
    };
    trushar::control::ControlError::new(code, message)
}
