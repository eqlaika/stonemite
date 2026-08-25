use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    TOOLTIPS_CLASS, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTS_ALWAYSTIP, TTTOOLINFOW,
    WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::client_controller::restore_active_eq_if_owned;
use super::geometry::{dpi_scale, scale};
use super::presentation::StonemiteButtonDrag;
use super::runtime::{is_busy, try_with_state, try_with_state_mut};
use super::state::OverlayState;
use super::surfaces::{
    position_window_if_changed, render_stonemite_button_surface,
    render_stonemite_button_surface_for_size, request_redraw, surface_is_ready, update_visibility,
    validate_composition_paint,
};
use crate::{eq_windows, settings_dialog, tray};

pub(super) const LOGICAL_BUTTON_SIZE: i32 = 56;
const LOGICAL_BUTTON_GAP: i32 = 4;

pub(super) unsafe fn create_tooltip(button_hwnd: HWND) -> HWND {
    let Ok(tooltip_hwnd) = CreateWindowExW(
        Default::default(),
        TOOLTIPS_CLASS,
        w!(""),
        WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP),
        0,
        0,
        0,
        0,
        button_hwnd,
        None,
        None,
        None,
    ) else {
        return HWND::default();
    };
    let tooltip_text = w!("Stonemite — drag to move; left-click Settings; right-click menu.");
    let tool = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: button_hwnd,
        uId: button_hwnd.0 as usize,
        lpszText: PWSTR(tooltip_text.0 as *mut u16),
        ..Default::default()
    };
    let added = SendMessageW(
        tooltip_hwnd,
        TTM_ADDTOOLW,
        WPARAM(0),
        LPARAM((&tool as *const TTTOOLINFOW) as isize),
    );
    if added.0 == 0 {
        let _ = DestroyWindow(tooltip_hwnd);
        HWND::default()
    } else {
        tooltip_hwnd
    }
}

pub(super) fn button_rect(
    monitor: RECT,
    edge: crate::config::PipEdge,
    strip_width: i32,
    strip_height: i32,
    has_pips: bool,
    dpi_scale: f64,
) -> RECT {
    let size = scale(LOGICAL_BUTTON_SIZE, dpi_scale).max(1);
    let gap = scale(LOGICAL_BUTTON_GAP, dpi_scale).max(1);
    let strip_width = has_pips.then_some(strip_width.max(0)).unwrap_or(0);
    let strip_height = has_pips.then_some(strip_height.max(0)).unwrap_or(0);
    let (x, y) = match edge {
        crate::config::PipEdge::Right => (monitor.right - strip_width - gap - size, monitor.top),
        crate::config::PipEdge::Left => (monitor.left + strip_width + gap, monitor.top),
        crate::config::PipEdge::Top => (monitor.right - size, monitor.top + strip_height + gap),
        crate::config::PipEdge::Bottom => (
            monitor.right - size,
            monitor.bottom - strip_height - gap - size,
        ),
    };
    clamp_button_rect(monitor, size, x, y)
}

fn clamp_button_rect(monitor: RECT, size: i32, x: i32, y: i32) -> RECT {
    let size = size.max(1);
    let x = x.clamp(monitor.left, (monitor.right - size).max(monitor.left));
    let y = y.clamp(monitor.top, (monitor.bottom - size).max(monitor.top));
    RECT {
        left: x,
        top: y,
        right: x + size,
        bottom: y + size,
    }
}

fn positioned_button_rect(monitor: RECT, dpi_scale: f64, position: [f32; 2]) -> Option<RECT> {
    if !position.iter().all(|coordinate| coordinate.is_finite()) {
        return None;
    }
    let size = scale(LOGICAL_BUTTON_SIZE, dpi_scale).max(1);
    let available_x = (monitor.right - monitor.left - size).max(0);
    let available_y = (monitor.bottom - monitor.top - size).max(0);
    let x = monitor.left
        + (f64::from(available_x) * f64::from(position[0].clamp(0.0, 1.0))).round() as i32;
    let y = monitor.top
        + (f64::from(available_y) * f64::from(position[1].clamp(0.0, 1.0))).round() as i32;
    Some(clamp_button_rect(monitor, size, x, y))
}

fn normalized_button_position(monitor: RECT, rect: RECT) -> [f32; 2] {
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    let available_x = (monitor.right - monitor.left - width).max(0);
    let available_y = (monitor.bottom - monitor.top - height).max(0);
    let x = if available_x == 0 {
        0.0
    } else {
        ((rect.left - monitor.left) as f32 / available_x as f32).clamp(0.0, 1.0)
    };
    let y = if available_y == 0 {
        0.0
    } else {
        ((rect.top - monitor.top) as f32 / available_y as f32).clamp(0.0, 1.0)
    };
    [x, y]
}

fn drag_threshold_exceeded(dx: i32, dy: i32, dpi_scale: f64) -> bool {
    let threshold = scale(4, dpi_scale).max(1);
    dx.abs() >= threshold || dy.abs() >= threshold
}

fn rects_match(left: RECT, right: RECT) -> bool {
    left.left == right.left
        && left.top == right.top
        && left.right == right.right
        && left.bottom == right.bottom
}

fn desired_button_rect(s: &OverlayState) -> RECT {
    s.presentation
        .stonemite_button
        .position
        .and_then(|position| {
            positioned_button_rect(s.layout.monitor_rect, s.layout.dpi_scale, position)
        })
        .unwrap_or_else(|| {
            button_rect(
                s.layout.monitor_rect,
                s.layout.pip_edge,
                s.layout.strip_width,
                s.layout.strip_height,
                !s.clients.pips().is_empty(),
                s.layout.dpi_scale,
            )
        })
}

pub(super) unsafe fn update_layout(s: &mut OverlayState) {
    let reference = s
        .clients
        .active_pid()
        .and_then(|pid| s.clients.windows.iter().find(|window| window.pid == pid))
        .or_else(|| s.clients.windows.first())
        .map(|window| window.hwnd);
    if let Some(reference) = reference {
        s.layout.monitor_rect = eq_windows::get_monitor_work_area(Some(reference));
        s.layout.dpi_scale = dpi_scale(reference);
    }
    let rect = desired_button_rect(s);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let mut current_client = RECT::default();
    let size_changed = GetClientRect(s.presentation.stonemite_button.hwnd, &mut current_client)
        .is_err()
        || current_client.right - current_client.left != width
        || current_client.bottom - current_client.top != height;
    if (size_changed
        || !surface_is_ready(s, s.presentation.stonemite_button.hwnd)
        || super::surfaces::has_redraw_request(s.presentation.stonemite_button.hwnd))
        && !render_stonemite_button_surface_for_size(s, width, height)
    {
        return;
    }
    position_window_if_changed(
        s.presentation.stonemite_button.hwnd,
        HWND_TOPMOST,
        rect.left,
        rect.top,
        width,
        height,
    );
}

pub(super) fn visibility_policy(
    enabled: bool,
    has_client: bool,
    hidden_by_user: bool,
    menu_open: bool,
    foreground_owned: bool,
) -> bool {
    enabled && has_client && !hidden_by_user && (menu_open || foreground_owned)
}

fn point_is_inside(hwnd: HWND, lparam: LPARAM) -> bool {
    let point_x = (lparam.0 & 0xFFFF) as i16 as i32;
    let point_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client).is_err() } {
        return false;
    }
    point_x >= client.left
        && point_x < client.right
        && point_y >= client.top
        && point_y < client.bottom
}

unsafe fn set_hovered(s: &mut OverlayState, hovered: bool) {
    if s.presentation.stonemite_button.hovered != hovered {
        s.presentation.stonemite_button.hovered = hovered;
        request_redraw(s.presentation.stonemite_button.hwnd);
    }
}

unsafe fn set_pressed(s: &mut OverlayState, pressed: bool) {
    if s.presentation.stonemite_button.pressed != pressed {
        s.presentation.stonemite_button.pressed = pressed;
        request_redraw(s.presentation.stonemite_button.hwnd);
    }
}

/// Release capture without letting the synchronous WM_CAPTURECHANGED callback
/// mistake an expected button-up/cancel transition for unsolicited capture loss.
unsafe fn release_button_capture(hwnd: HWND) {
    let _ = try_with_state_mut(|s| {
        if s.presentation.stonemite_button.hwnd == hwnd {
            s.presentation.stonemite_button.releasing_capture = true;
        }
    });
    let _ = ReleaseCapture();
    let _ = try_with_state_mut(|s| {
        if s.presentation.stonemite_button.hwnd == hwnd {
            s.presentation.stonemite_button.releasing_capture = false;
        }
    });
}

unsafe fn update_button_drag(s: &mut OverlayState) -> bool {
    let Some(mut drag) = s.presentation.stonemite_button.drag else {
        return false;
    };
    let mut cursor = POINT::default();
    if GetCursorPos(&mut cursor).is_err() {
        return drag.dragging;
    }
    if !rects_match(drag.monitor_rect, s.layout.monitor_rect)
        || (drag.dpi_scale - s.layout.dpi_scale).abs() > 0.001
    {
        let mut current_rect = RECT::default();
        let rect = if GetWindowRect(s.presentation.stonemite_button.hwnd, &mut current_rect).is_ok()
        {
            current_rect
        } else {
            desired_button_rect(s)
        };
        drag.start_cursor = cursor;
        drag.start_rect = rect;
        drag.monitor_rect = s.layout.monitor_rect;
        drag.dpi_scale = s.layout.dpi_scale;
        s.presentation.stonemite_button.drag = Some(drag);
        return drag.dragging;
    }

    let dx = cursor.x - drag.start_cursor.x;
    let dy = cursor.y - drag.start_cursor.y;
    drag.dragging |= drag_threshold_exceeded(dx, dy, s.layout.dpi_scale);
    s.presentation.stonemite_button.drag = Some(drag);
    if !drag.dragging {
        return false;
    }

    let size = (drag.start_rect.right - drag.start_rect.left)
        .max(drag.start_rect.bottom - drag.start_rect.top)
        .max(1);
    let rect = clamp_button_rect(
        s.layout.monitor_rect,
        size,
        drag.start_rect.left + dx,
        drag.start_rect.top + dy,
    );
    s.presentation.stonemite_button.position =
        Some(normalized_button_position(s.layout.monitor_rect, rect));
    position_window_if_changed(
        s.presentation.stonemite_button.hwnd,
        HWND_TOPMOST,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    );
    true
}

fn finish_button_drag(s: &mut OverlayState) -> bool {
    let dragged = s
        .presentation
        .stonemite_button
        .drag
        .take()
        .is_some_and(|drag| drag.dragging);
    if dragged {
        let position = s.presentation.stonemite_button.position;
        if let Err(error) = crate::config::Config::update(|config| {
            config.stonemite_button_position = position;
        }) {
            crate::diagnostics::debug_log(&format!(
                "Could not save the in-game Stonemite logo position: {error}"
            ));
        }
    }
    dragged
}

unsafe fn open_settings(s: &mut OverlayState, button_hwnd: HWND) {
    let _ = settings_dialog::show();
    // A newly spawned settings process may not own foreground yet. Return EQ
    // immediately only while the button still owns it; an existing settings
    // window or another user-selected app is never displaced.
    restore_active_eq_if_owned(s, button_hwnd);
    update_visibility(s);
}

unsafe fn open_tray_menu(s: &mut OverlayState, button_hwnd: HWND) {
    s.presentation.stonemite_button.menu_open = true;
    update_visibility(s);
    if !tray::request_stonemite_menu(button_hwnd) {
        s.presentation.stonemite_button.menu_open = false;
        update_visibility(s);
        restore_active_eq_if_owned(s, button_hwnd);
    }
}

pub(super) fn menu_opened() {
    let _ = try_with_state_mut(|s| unsafe {
        s.presentation.stonemite_button.menu_open = true;
        update_visibility(s);
    });
}

pub(super) fn tray_menu_closed() {
    let _ = try_with_state_mut(|s| unsafe {
        s.presentation.stonemite_button.menu_open = false;
        update_visibility(s);
    });
}

pub(super) unsafe fn button_menu_closed(menu_owner_hwnd: HWND, source_hwnd: HWND) {
    let _ = try_with_state_mut(|s| unsafe {
        if source_hwnd != s.presentation.stonemite_button.hwnd {
            return;
        }
        s.presentation.stonemite_button.menu_open = false;
        restore_active_eq_if_owned(s, menu_owner_hwnd);
        update_visibility(s);
    });
}

pub(super) unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_busy() {
        return match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                validate_composition_paint(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        };
    }
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        WM_SETCURSOR => {
            let moving =
                try_with_state(|s| s.presentation.stonemite_button.drag.is_some()).unwrap_or(false);
            let cursor = if moving { IDC_SIZEALL } else { IDC_HAND };
            SetCursor(LoadCursorW(None, cursor).unwrap_or_default());
            LRESULT(1)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let _ = try_with_state_mut(|s| unsafe {
                if super::surfaces::take_redraw_request(hwnd) || !surface_is_ready(s, hwnd) {
                    render_stonemite_button_surface(s);
                }
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let moving = try_with_state_mut(|s| unsafe {
                set_hovered(s, true);
                let _ = update_button_drag(s);
                s.presentation.stonemite_button.drag.is_some()
            })
            .unwrap_or(false);
            if moving {
                SetCursor(LoadCursorW(None, IDC_SIZEALL).unwrap_or_default());
            }
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                ..Default::default()
            };
            let _ = TrackMouseEvent(&mut tracking);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let _ = try_with_state_mut(|s| unsafe {
                if !s.presentation.stonemite_button.pressed {
                    set_hovered(s, false);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let _ = SetCapture(hwnd);
            let _ = try_with_state_mut(|s| unsafe {
                let mut cursor = POINT::default();
                let mut rect = RECT::default();
                s.presentation.stonemite_button.drag = if GetCursorPos(&mut cursor).is_ok()
                    && GetWindowRect(hwnd, &mut rect).is_ok()
                {
                    Some(StonemiteButtonDrag {
                        start_cursor: cursor,
                        start_rect: rect,
                        monitor_rect: s.layout.monitor_rect,
                        dpi_scale: s.layout.dpi_scale,
                        dragging: false,
                    })
                } else {
                    None
                };
                set_pressed(s, true);
            });
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = SetCapture(hwnd);
            let _ = try_with_state_mut(|s| unsafe {
                s.presentation.stonemite_button.drag = None;
                set_pressed(s, true);
            });
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // ReleaseCapture synchronously sends WM_CAPTURECHANGED, so retain
            // the matched down-state before ending capture.
            let was_pressed =
                try_with_state(|s| s.presentation.stonemite_button.pressed).unwrap_or(false);
            release_button_capture(hwnd);
            let inside = point_is_inside(hwnd, lparam);
            let _ = try_with_state_mut(|s| unsafe {
                let dragged = finish_button_drag(s);
                set_pressed(s, false);
                if !inside {
                    set_hovered(s, false);
                }
                if dragged {
                    restore_active_eq_if_owned(s, hwnd);
                } else if was_pressed && inside {
                    open_settings(s, hwnd);
                } else if was_pressed {
                    restore_active_eq_if_owned(s, hwnd);
                }
            });
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let was_pressed =
                try_with_state(|s| s.presentation.stonemite_button.pressed).unwrap_or(false);
            release_button_capture(hwnd);
            let inside = point_is_inside(hwnd, lparam);
            let _ = try_with_state_mut(|s| unsafe {
                s.presentation.stonemite_button.drag = None;
                set_pressed(s, false);
                if !inside {
                    set_hovered(s, false);
                }
                if was_pressed && inside {
                    open_tray_menu(s, hwnd);
                } else if was_pressed {
                    restore_active_eq_if_owned(s, hwnd);
                }
            });
            LRESULT(0)
        }
        WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            let _ = SetCapture(hwnd);
            let _ = try_with_state_mut(|s| {
                s.presentation.stonemite_button.drag = None;
            });
            LRESULT(0)
        }
        WM_MBUTTONUP | WM_XBUTTONUP => {
            release_button_capture(hwnd);
            let _ = try_with_state_mut(|s| unsafe { restore_active_eq_if_owned(s, hwnd) });
            LRESULT(if msg == WM_XBUTTONUP { 1 } else { 0 })
        }
        WM_CAPTURECHANGED => {
            let _ = try_with_state_mut(|s| unsafe {
                let expected =
                    std::mem::take(&mut s.presentation.stonemite_button.releasing_capture);
                set_pressed(s, false);
                if !expected {
                    let _ = finish_button_drag(s);
                    restore_active_eq_if_owned(s, hwnd);
                }
            });
            LRESULT(0)
        }
        WM_CANCELMODE => {
            release_button_capture(hwnd);
            let _ = try_with_state_mut(|s| unsafe {
                let _ = finish_button_drag(s);
                set_pressed(s, false);
                restore_active_eq_if_owned(s, hwnd);
            });
            LRESULT(0)
        }
        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            let _ = try_with_state_mut(|s| unsafe {
                super::hosts::rebuild_thumbnails(s);
                update_visibility(s);
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PipEdge;

    fn monitor() -> RECT {
        RECT {
            left: 100,
            top: 200,
            right: 2020,
            bottom: 1280,
        }
    }

    #[test]
    fn button_anchors_inward_from_each_strip_edge() {
        assert_eq!(
            button_rect(monitor(), PipEdge::Right, 240, 500, true, 1.0),
            RECT {
                left: 1720,
                top: 200,
                right: 1776,
                bottom: 256,
            }
        );
        assert_eq!(
            button_rect(monitor(), PipEdge::Left, 240, 500, true, 1.0),
            RECT {
                left: 344,
                top: 200,
                right: 400,
                bottom: 256,
            }
        );
        assert_eq!(
            button_rect(monitor(), PipEdge::Top, 800, 160, true, 1.0),
            RECT {
                left: 1964,
                top: 364,
                right: 2020,
                bottom: 420,
            }
        );
        assert_eq!(
            button_rect(monitor(), PipEdge::Bottom, 800, 160, true, 1.0),
            RECT {
                left: 1964,
                top: 1060,
                right: 2020,
                bottom: 1116,
            }
        );
    }

    #[test]
    fn no_pip_fallback_stays_inside_the_configured_edge_at_scaled_dpi() {
        let right = button_rect(monitor(), PipEdge::Right, 0, 0, false, 1.5);
        assert_eq!(right.right, monitor().right - 6);
        assert_eq!(right.top, monitor().top);
        assert_eq!(right.right - right.left, 84);

        let bottom = button_rect(monitor(), PipEdge::Bottom, 0, 0, false, 1.5);
        assert_eq!(bottom.right, monitor().right);
        assert_eq!(bottom.bottom, monitor().bottom - 6);
    }

    #[test]
    fn dragged_position_is_relative_clamped_and_dpi_aware() {
        let rect = positioned_button_rect(monitor(), 1.0, [0.25, 0.75]).unwrap();
        assert_eq!(
            rect,
            RECT {
                left: 566,
                top: 968,
                right: 622,
                bottom: 1024,
            }
        );
        assert_eq!(normalized_button_position(monitor(), rect), [0.25, 0.75]);

        let clamped = positioned_button_rect(monitor(), 1.0, [2.0, -1.0]).unwrap();
        assert_eq!(clamped.right, monitor().right);
        assert_eq!(clamped.top, monitor().top);
        assert_eq!(clamped.right - clamped.left, 56);
        assert!(positioned_button_rect(monitor(), 1.0, [f32::NAN, 0.5]).is_none());

        let scaled = positioned_button_rect(monitor(), 1.5, [1.0, 1.0]).unwrap();
        assert_eq!(scaled.right, monitor().right);
        assert_eq!(scaled.bottom, monitor().bottom);
        assert_eq!(scaled.right - scaled.left, 84);
    }

    #[test]
    fn dragging_requires_a_scaled_movement_threshold() {
        assert!(!drag_threshold_exceeded(3, 0, 1.0));
        assert!(drag_threshold_exceeded(4, 0, 1.0));
        assert!(!drag_threshold_exceeded(5, 5, 1.5));
        assert!(drag_threshold_exceeded(0, 6, 1.5));
    }

    #[test]
    fn visibility_requires_setting_client_user_permission_and_owned_foreground() {
        assert!(visibility_policy(true, true, false, false, true));
        assert!(visibility_policy(true, true, false, true, false));
        assert!(!visibility_policy(false, true, false, true, true));
        assert!(!visibility_policy(true, false, false, true, true));
        assert!(!visibility_policy(true, true, true, true, true));
        assert!(!visibility_policy(true, true, false, false, false));
    }
}
