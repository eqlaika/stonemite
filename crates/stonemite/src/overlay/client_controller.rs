use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use super::activation::{
    reassert_eq_mouse_activation, request_error as foreground_request_error, request_foreground,
    target_has_keyboard_focus, ForegroundRequest,
};
use super::clients::{exchange_window_numbers, focused_foreground_pid, MAX_PIPS};
use super::control_bridge::{publish, sync_mouse_eligibility};
use super::hosts::rebuild_thumbnails;
use super::runtime::{try_with_state_mut, AccessError};
use super::state::OverlayState;
use super::surfaces::update_visibility;
use super::toast_controller::show_toast_inner;
use crate::config;

type CommandResult = Result<trushar::control::CommandOutcome, trushar::control::ControlError>;

fn runtime_command_error(error: AccessError) -> trushar::control::ControlError {
    let message = match error {
        AccessError::Busy => "overlay is already handling a window transition",
        AccessError::Unavailable => "overlay is unavailable",
    };
    trushar::control::ControlError::new(trushar::control::ErrorCode::InternalError, message)
}

// ---------------------------------------------------------------------------
// Foreground event hook
// ---------------------------------------------------------------------------

pub(super) unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dw_ms_event_time: u32,
) {
    let _ = try_with_state_mut(|state| unsafe { handle_foreground_change(state) });
}

unsafe fn handle_foreground_change(s: &mut OverlayState) {
    let fg = GetForegroundWindow();
    if let Some(fg_pid) = focused_foreground_pid(&s.clients.windows, fg, |hwnd| {
        target_has_keyboard_focus(hwnd)
    }) {
        s.notification_center.acknowledge(fg_pid);
        if s.clients.promote(fg_pid, MAX_PIPS) {
            if config::Config::load().auto_order {
                s.clients.apply_auto_order();
            }
            s.window_styles.apply(&s.clients);
            rebuild_thumbnails(s);
        }
        // Keep broadcast suppression synchronized with WinEvent foreground
        // changes immediately instead of waiting for the next process poll.
        crate::broadcast::set_active_pid(fg_pid);
        sync_mouse_eligibility(s);
    }

    update_visibility(s);
    s.clients.debug_assert_partition();
    publish(s);
}

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

/// Swap to the window with the given stable number (1-based).
/// Called from hotkey handlers.
pub(super) unsafe fn swap_to_number(number: usize) {
    let _ = try_with_state_mut(|state| unsafe {
        let target_pid = state
            .clients
            .windows
            .iter()
            .find(|window| window.number == number)
            .map(|window| window.pid);
        if let Some(target_pid) = target_pid {
            let _ = activate_pid_inner(state, target_pid);
        }
    });
}

/// Swap the selected client's stable window number with the active client's number.
/// The foreground client does not change.
pub(super) unsafe fn swap_active_window_numbers(target_pid: u32) -> CommandResult {
    match try_with_state_mut(|state| unsafe { swap_active_window_numbers_inner(state, target_pid) })
    {
        Ok(result) => result,
        Err(error) => Err(runtime_command_error(error)),
    }
}

unsafe fn swap_active_window_numbers_inner(s: &mut OverlayState, target_pid: u32) -> CommandResult {
    let Some(active_pid) = s.clients.active_pid() else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::WindowNumberSwapFailed,
            "there is no active EQ client whose window number can be swapped",
        ));
    };
    if !s
        .clients
        .windows
        .iter()
        .any(|window| window.pid == target_pid)
    {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the selected EQ window is no longer loaded",
        ));
    }
    let Some((active_previous_number, selected_previous_number)) =
        exchange_window_numbers(&mut s.clients.windows, active_pid, target_pid)
    else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::WindowNumberSwapFailed,
            "the active EQ window is no longer loaded",
        ));
    };

    if active_pid != target_pid {
        if config::Config::load().auto_order {
            s.clients.apply_auto_order();
        }
        rebuild_thumbnails(s);
        update_visibility(s);
        show_toast_inner(
            s,
            &format!(
                "Swapped window numbers #{} and #{}",
                active_previous_number, selected_previous_number
            ),
        );
        s.clients.debug_assert_partition();
        publish(s);
    }

    Ok(trushar::control::CommandOutcome::WindowNumbersSwapped {
        active_previous_number,
        selected_previous_number,
    })
}

/// Authoritative semantic activation operation used by local UI and trushar.
pub(super) unsafe fn activate_pid(target_pid: u32) -> CommandResult {
    match try_with_state_mut(|state| unsafe { activate_pid_inner(state, target_pid) }) {
        Ok(result) => result,
        Err(error) => Err(runtime_command_error(error)),
    }
}

unsafe fn activate_pid_inner(s: &mut OverlayState, target_pid: u32) -> CommandResult {
    let Some(target_window) = s
        .clients
        .windows
        .iter()
        .find(|window| window.pid == target_pid)
    else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target is no longer loaded",
        ));
    };
    if s.clients.active_pid() == Some(target_pid) {
        let result = reassert_active_foreground(target_window.hwnd);
        if result.is_ok() {
            s.notification_center.acknowledge(target_pid);
        }
        return result;
    }
    let Some(pip_index) = s.clients.pips().iter().position(|pid| *pid == target_pid) else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::ActivationFailed,
            "the loaded client is outside the supported activation set",
        ));
    };
    swap_to_inner(s, pip_index)
}

unsafe fn reassert_active_foreground(target_hwnd: HWND) -> CommandResult {
    let request = request_foreground(target_hwnd);
    if request != ForegroundRequest::Confirmed {
        return Err(foreground_request_error(request));
    }
    reassert_eq_mouse_activation(target_hwnd);
    Ok(trushar::control::CommandOutcome::Activated {
        status: trushar::control::ActivationStatus::AlreadyActive,
        foreground_confirmed: true,
    })
}

pub(super) unsafe fn swap_to_inner(s: &mut OverlayState, pip_index: usize) -> CommandResult {
    if pip_index >= s.clients.pips().len() {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared before activation",
        ));
    }
    if s.clients.active_pid().is_none() {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::ActivationFailed,
            "there is no active EQ client to exchange",
        ));
    }
    let new_active_pid = s
        .clients
        .pip_at(pip_index)
        .expect("PiP index was validated above");
    let Some(new_active_hwnd) = s
        .clients
        .windows
        .iter()
        .find(|window| window.pid == new_active_pid)
        .map(|window| window.hwnd)
    else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared before activation",
        ));
    };

    let request = request_foreground(new_active_hwnd);
    if request != ForegroundRequest::Confirmed {
        return Err(foreground_request_error(request));
    }

    let previous_partition = s.clients.snapshot_partition();
    if !s.clients.promote(new_active_pid, MAX_PIPS) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "the active/PiP partition changed during activation",
        ));
    }
    // Confirm again after the pure partition exchange but before any detached
    // style work. Rollback is then limited to in-memory state and cannot race
    // asynchronous Alt-Tab style changes.
    let final_request = request_foreground(new_active_hwnd);
    if final_request != ForegroundRequest::Confirmed {
        s.clients.restore_partition(previous_partition);
        return Err(foreground_request_error(final_request));
    }

    crate::broadcast::set_active_pid(new_active_pid);
    sync_mouse_eligibility(s);
    s.notification_center.acknowledge(new_active_pid);
    if config::Config::load().auto_order {
        s.clients.apply_auto_order();
    }
    s.window_styles.apply(&s.clients);
    rebuild_thumbnails(s);
    update_visibility(s);

    let toast_label = if let Some(window) = s
        .clients
        .windows
        .iter()
        .find(|window| window.pid == new_active_pid)
    {
        match &window.character {
            Some(name) => format!("Swapped to #{} {}", window.number, name),
            None => format!("Swapped to #{}", window.number),
        }
    } else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared during activation",
        ));
    };
    show_toast_inner(s, &toast_label);
    s.clients.debug_assert_partition();
    publish(s);
    // Reassert last, after all window churn, so EQ reacquires input while its
    // real foreground and focus state are stable.
    reassert_eq_mouse_activation(new_active_hwnd);
    Ok(trushar::control::CommandOutcome::Activated {
        status: trushar::control::ActivationStatus::Activated,
        foreground_confirmed: true,
    })
}
