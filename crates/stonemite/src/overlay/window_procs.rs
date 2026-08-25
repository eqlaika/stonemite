use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::combat_awareness;
use super::hosts::update_active_label;
use super::menu::queue_char_menu;
use super::notifications;
use super::pip_transition::{tick as tick_pip_transition, TIMER_ID as TIMER_PIP_TRANSITION};
use super::runtime::{self, is_busy, try_with_state, try_with_state_mut};
use super::state::OverlayState;
use super::surfaces::{
    render_active_label_surface, render_banner_surface, render_pip_surface, request_redraw,
    service_compositor_recovery, set_composition_opacity, surface_is_ready, take_redraw_request,
    validate_composition_paint,
};
use super::timer_controller::{
    owner_hwnds as timer_owner_hwnds, redraw_targets as timer_tick_redraw_targets,
    TIMER_ID as TIMER_OVERLAY_TICK,
};
use super::toast::SERVICE_COMPOSITOR_RECOVERY_MESSAGE as WM_SERVICE_COMPOSITOR_RECOVERY;

// ---------------------------------------------------------------------------
// Composition window procedures
// ---------------------------------------------------------------------------

pub(super) unsafe extern "system" fn pip_label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_busy()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !is_busy() && take_redraw_request(hwnd);
            if !is_busy() {
                let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
                if raw_idx > 0 {
                    let _ = try_with_state_mut(|state| unsafe {
                        if requested || !surface_is_ready(state, hwnd) {
                            render_pip_surface(state, raw_idx - 1);
                        }
                    });
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Broadcast banner window proc
// ---------------------------------------------------------------------------

pub(super) unsafe extern "system" fn broadcast_label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_busy()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !is_busy() && take_redraw_request(hwnd);
            if !is_busy() {
                let _ = try_with_state_mut(|state| unsafe {
                    if requested || !surface_is_ready(state, hwnd) {
                        render_banner_surface(state);
                    }
                });
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Label window proc
// ---------------------------------------------------------------------------

pub(super) fn forwards_active_label_mouse_message(message: u32) -> bool {
    matches!(message, WM_LBUTTONDOWN | WM_LBUTTONUP)
}

unsafe fn tick_notification_animation(timer_hwnd: HWND) {
    if try_with_state_mut(|state| unsafe { notifications::tick(state, timer_hwnd) }).is_err() {
        let _ = KillTimer(timer_hwnd, notifications::TIMER_ID);
    }
}

unsafe fn tick_combat_awareness(timer_hwnd: HWND) {
    if try_with_state_mut(|state| unsafe { combat_awareness::tick(state, timer_hwnd) }).is_err() {
        let _ = KillTimer(timer_hwnd, combat_awareness::TIMER_ID);
    }
}

unsafe fn tick_timer_overlay(timer_hwnd: HWND) {
    if try_with_state_mut(|state| unsafe { tick_timer_overlay_inner(state, timer_hwnd) }).is_err() {
        let _ = KillTimer(timer_hwnd, TIMER_OVERLAY_TICK);
    }
}

unsafe fn tick_pip_motion(timer_hwnd: HWND) {
    if try_with_state_mut(|state| unsafe { tick_pip_transition(state, timer_hwnd) }).is_err() {
        let _ = KillTimer(timer_hwnd, TIMER_PIP_TRANSITION);
    }
}

unsafe fn tick_timer_overlay_inner(s: &mut OverlayState, timer_hwnd: HWND) {
    let now = Instant::now();
    let previous_owners = timer_owner_hwnds(s, now);
    let expired = s.timers.remove_expired(now);
    if s.timers.is_empty() {
        let _ = KillTimer(timer_hwnd, TIMER_OVERLAY_TICK);
    }
    if expired {
        // Timer appearance/expiry is structural for the active-label HWND;
        // ordinary countdown ticks never move or resize any HWND.
        update_active_label(s);
    }
    let current_owners = timer_owner_hwnds(s, now);
    let pip_label_hwnds = s
        .presentation
        .pip_windows
        .iter()
        .map(|pip| pip.label_hwnd)
        .collect::<Vec<_>>();
    for hwnd in timer_tick_redraw_targets(
        expired,
        s.presentation.active_label_hwnd,
        &pip_label_hwnds,
        &previous_owners,
        &current_owners,
    ) {
        request_redraw(hwnd);
    }
}

pub(super) unsafe extern "system" fn label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_busy() {
        return match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                validate_composition_paint(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        };
    }
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SERVICE_COMPOSITOR_RECOVERY => {
            if try_with_state_mut(|state| unsafe { service_compositor_recovery(state) }).is_err() {
                runtime::clear_recovery_post();
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == notifications::TIMER_ID => {
            tick_notification_animation(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == combat_awareness::TIMER_ID => {
            tick_combat_awareness(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_OVERLAY_TICK => {
            tick_timer_overlay(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_PIP_TRANSITION => {
            tick_pip_motion(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = take_redraw_request(hwnd);
            let _ = try_with_state_mut(|state| unsafe {
                if requested || !surface_is_ready(state, hwnd) {
                    render_active_label_surface(state);
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let _ = try_with_state_mut(|state| unsafe {
                if !state.presentation.active_label_hovered {
                    state.presentation.active_label_hovered = true;
                    let alpha = state.presentation.label_alpha / 2;
                    set_composition_opacity(state, hwnd, alpha);
                }
            });
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let _ = try_with_state_mut(|state| unsafe {
                state.presentation.active_label_hovered = false;
                let alpha = state.presentation.label_alpha;
                set_composition_opacity(state, hwnd, alpha);
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let _ = try_with_state_mut(|state| unsafe {
                if let Some(active_pid) = state.clients.active_pid() {
                    let mut point = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = ClientToScreen(hwnd, &mut point);
                    queue_char_menu(state, active_pid, point, hwnd);
                }
            });
            LRESULT(0)
        }
        message if forwards_active_label_mouse_message(message) => {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ClientToScreen(hwnd, &mut pt);
            let target = try_with_state(|state| {
                let pid = state.clients.active_pid()?;
                state
                    .clients
                    .windows
                    .iter()
                    .find(|window| window.pid == pid)
                    .map(|window| window.hwnd)
            })
            .ok()
            .flatten();
            if let Some(target) = target.filter(|target| IsWindow(*target).as_bool()) {
                let mut client = pt;
                let _ = ScreenToClient(target, &mut client);
                let packed =
                    (client.x as i16 as u16 as isize) | ((client.y as i16 as u16 as isize) << 16);
                let _ = PostMessageW(target, msg, wparam, LPARAM(packed));
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_label_forwards_only_matched_left_button_messages() {
        assert!(forwards_active_label_mouse_message(WM_LBUTTONDOWN));
        assert!(forwards_active_label_mouse_message(WM_LBUTTONUP));
        assert!(!forwards_active_label_mouse_message(WM_RBUTTONDOWN));
        assert!(!forwards_active_label_mouse_message(WM_RBUTTONUP));
    }
}
