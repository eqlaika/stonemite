use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::appearance::BORDER_WIDTH;
use super::client_controller::{activate_pid_inner, restore_active_eq_if_owned};
use super::control_bridge::publish;
use super::geometry::scale;
use super::hosts::{
    cancel_reorder_drag, compute_positions, rebuild_thumbnails, update_active_label,
};
use super::interaction::{
    MoveDragState, PipResizeDragState, ReorderDragState, StripResizeDragState,
};
use super::layout::{
    resize_edge_hit_test as edit_resize_edge_hit_test, snap_point, snap_resize,
    strip_resize_hit_test, MoveSnapInput, ResizeEdge, MAX_STRIP_WIDTH_FRACTION,
    MIN_STRIP_WIDTH_FRACTION,
};
use super::menu::queue_char_menu;
use super::notifications;
use super::runtime::{is_busy, try_with_state_mut};
use super::state::OverlayState;
use super::surfaces::{
    position_pip_pair, render_pip_surface_for_size, request_redraw, update_visibility,
    validate_composition_paint,
};
use super::toast::CLEAR_INVITE_CAPTURE_MESSAGE as WM_CLEAR_INVITE_CAPTURE;
use crate::config;
use crate::diagnostics::debug_log;

const RESIZE_HANDLE_WIDTH: i32 = 12;
const THUMB_OPACITY_HOVER: u8 = 255;
const THUMB_OPACITY_DRAG_MAX: u8 = 80;
const DRAG_THRESHOLD: i32 = 8;
const RESIZE_ZONE: i32 = 8;
const VK_SHIFT_CODE: i32 = 0x10;

fn reorder_thumbnail_alpha(normal_alpha: u8) -> u8 {
    normal_alpha.min(THUMB_OPACITY_DRAG_MAX)
}

fn client_reorder_indices(
    client_pips: &[u32],
    presented_from_pid: u32,
    presented_to_pid: u32,
) -> Option<(usize, usize)> {
    client_pips
        .iter()
        .position(|pid| *pid == presented_from_pid)
        .zip(client_pips.iter().position(|pid| *pid == presented_to_pid))
}

fn cursor_for_resize_edge(edge: ResizeEdge) -> *const u16 {
    match edge {
        ResizeEdge::N | ResizeEdge::S => IDC_SIZENS.0,
        ResizeEdge::E | ResizeEdge::W => IDC_SIZEWE.0,
        ResizeEdge::NW | ResizeEdge::SE => IDC_SIZENWSE.0,
        ResizeEdge::NE | ResizeEdge::SW => IDC_SIZENESW.0,
    }
}

// ---------------------------------------------------------------------------
// PiP window proc
// ---------------------------------------------------------------------------

pub(super) unsafe extern "system" fn pip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Skip state access during rebuilds while still validating paint without
    // asking the host to erase or author pixels.
    if is_busy() {
        return unavailable_message_result(hwnd, msg, wparam, lparam);
    }

    // Decode pip index from GWLP_USERDATA (1-based, 0 = not yet set).
    let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    if raw_idx == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let pip_idx = raw_idx - 1;

    match try_with_state_mut(|state| unsafe {
        pip_wnd_proc_inner(state, hwnd, msg, wparam, lparam, pip_idx)
    }) {
        Ok(result) => result,
        Err(_) => unavailable_message_result(hwnd, msg, wparam, lparam),
    }
}

unsafe fn unavailable_message_result(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn pip_wnd_proc_inner(
    s: &mut OverlayState,
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    pip_idx: usize,
) -> LRESULT {
    match msg {
        // Foreground the interaction host before delivering the initiating
        // button message. Otherwise the still-foreground EQ client consumes
        // the same physical click through DirectInput.
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),

        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == 1
            /* HTCLIENT */
            {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let mut client_pt = pt;
                let _ = ScreenToClient(hwnd, &mut client_pt);

                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);

                if s.interaction.edit_mode {
                    let zone = scale(RESIZE_ZONE, s.layout.dpi_scale);
                    if let Some(edge) =
                        edit_resize_edge_hit_test(client_pt, cr.right, cr.bottom, zone)
                    {
                        let cursor_id = windows::core::PCWSTR(cursor_for_resize_edge(edge));
                        let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
                    // Body → move cursor.
                    let cursor = LoadCursorW(None, IDC_SIZEALL).unwrap_or_default();
                    SetCursor(cursor);
                    return LRESULT(1);
                }
                if notifications::has_invite_action_at(s, pip_idx, client_pt) {
                    let cursor = LoadCursorW(None, IDC_HAND).unwrap_or_default();
                    SetCursor(cursor);
                    return LRESULT(1);
                }
                if !s.layout.has_custom_positions {
                    // Strip resize cursor on interior edge.
                    let handle_w = scale(RESIZE_HANDLE_WIDTH, s.layout.dpi_scale);
                    if strip_resize_hit_test(
                        client_pt,
                        cr.right,
                        cr.bottom,
                        s.layout.pip_edge,
                        handle_w,
                    ) {
                        let cursor_id = if matches!(
                            s.layout.pip_edge,
                            config::PipEdge::Right | config::PipEdge::Left
                        ) {
                            IDC_SIZEWE
                        } else {
                            IDC_SIZENS
                        };
                        let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_ERASEBKGND => LRESULT(1),

        WM_PAINT => {
            // The host is exclusively a DWM thumbnail target. Authored pixels
            // are presented by its aligned composition sibling.
            validate_composition_paint(hwnd);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            // --- Edit mode move/resize drag ---
            if s.interaction.edit_mode {
                if let Some(ref md) = s.interaction.move_drag {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let dx = cursor.x - md.start_cursor.x;
                    let dy = cursor.y - md.start_cursor.y;
                    let new_x = md.start_rect.left + dx;
                    let new_y = md.start_rect.top + dy;
                    let w = md.start_rect.right - md.start_rect.left;
                    let h = md.start_rect.bottom - md.start_rect.top;

                    // Collect other pip rects for snapping.
                    let idx = md.pip_index;
                    let others: Vec<RECT> = s
                        .presentation
                        .pip_windows
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != idx)
                        .map(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            r
                        })
                        .collect();

                    let (sx, sy) = snap_point(
                        MoveSnapInput {
                            x: new_x,
                            y: new_y,
                            width: w,
                            height: h,
                            monitor: s.layout.monitor_rect,
                            grid: s.layout.snap_grid,
                            bypass: GetKeyState(VK_SHIFT_CODE) < 0,
                        },
                        &others,
                    );

                    if let Some(pw) = s.presentation.pip_windows.get(idx) {
                        position_pip_pair(pw, sx, sy, w, h);
                    }
                    return LRESULT(0);
                }

                if let Some(ref rd) = s.interaction.pip_resize_drag {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let dx = cursor.x - rd.start_cursor.x;
                    let dy = cursor.y - rd.start_cursor.y;
                    let idx = rd.pip_index;
                    let edge = rd.edge;

                    let d = s.layout.dpi_scale;
                    let border = scale(BORDER_WIDTH, d);
                    let bypass_snap = GetKeyState(VK_SHIFT_CODE) < 0;
                    let new_rect = snap_resize(
                        edge,
                        rd.start_rect,
                        dx,
                        dy,
                        s.layout.snap_grid,
                        border,
                        bypass_snap,
                    );
                    let nw = new_rect.right - new_rect.left;
                    let nh = new_rect.bottom - new_rect.top;

                    if render_pip_surface_for_size(s, idx, nw, nh) {
                        let pw = &s.presentation.pip_windows[idx];
                        position_pip_pair(pw, new_rect.left, new_rect.top, nw, nh);

                        // Update DWM thumbnail destination.
                        let thumb_rect = RECT {
                            left: border,
                            top: border,
                            right: nw - border,
                            bottom: nh - border,
                        };
                        let props = DWM_THUMBNAIL_PROPERTIES {
                            dwFlags: DWM_TNP_RECTDESTINATION,
                            rcDestination: thumb_rect,
                            ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    }
                    return LRESULT(0);
                }

                // Track mouse for leave.
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                return LRESULT(0);
            }

            // --- Use mode: strip resize drag ---
            if let Some(ref srd) = s.interaction.strip_resize_drag {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                let is_vertical = matches!(
                    s.layout.pip_edge,
                    config::PipEdge::Right | config::PipEdge::Left
                );
                let new_size = if is_vertical {
                    let delta = cursor.x - srd.start_pt.x;
                    let sign = if matches!(s.layout.pip_edge, config::PipEdge::Right) {
                        -1
                    } else {
                        1
                    };
                    let mon_w = s.layout.monitor_rect.right - s.layout.monitor_rect.left;
                    let min_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_w, max_w)
                } else {
                    let delta = cursor.y - srd.start_pt.y;
                    let sign = if matches!(s.layout.pip_edge, config::PipEdge::Bottom) {
                        -1
                    } else {
                        1
                    };
                    let mon_h = s.layout.monitor_rect.bottom - s.layout.monitor_rect.top;
                    let min_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_h, max_h)
                };
                if Some(new_size) != s.layout.custom_strip_width {
                    s.layout.custom_strip_width = Some(new_size);
                    let (rects, sw, sh) = compute_positions(s);
                    s.layout.strip_width = sw;
                    s.layout.strip_height = sh;
                    let d = s.layout.dpi_scale;
                    let border = scale(BORDER_WIDTH, d);
                    for i in 0..s.presentation.pip_windows.len() {
                        if let Some(rect) = rects.get(i).copied() {
                            let cw = rect.right - rect.left;
                            let ch = rect.bottom - rect.top;
                            if render_pip_surface_for_size(s, i, cw, ch) {
                                let pw = &s.presentation.pip_windows[i];
                                position_pip_pair(pw, rect.left, rect.top, cw, ch);
                                let thumb_rect = RECT {
                                    left: border,
                                    top: border,
                                    right: cw - border,
                                    bottom: ch - border,
                                };
                                let props = DWM_THUMBNAIL_PROPERTIES {
                                    dwFlags: DWM_TNP_RECTDESTINATION,
                                    rcDestination: thumb_rect,
                                    ..Default::default()
                                };
                                let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            }
                        }
                    }
                    update_active_label(s);
                }
                return LRESULT(0);
            }

            // --- Use mode: reorder drag ---
            if let Some(ref mut drag) = s.interaction.reorder_drag {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);

                if !drag.dragging {
                    let dx = (cursor.x - drag.start_pt.x).abs();
                    let dy = (cursor.y - drag.start_pt.y).abs();
                    let threshold = scale(DRAG_THRESHOLD, s.layout.dpi_scale);
                    if dx > threshold || dy > threshold {
                        drag.dragging = true;
                        let _ = SetCapture(hwnd);
                        // Dim the source thumbnail.
                        if let Some(pw) = s.presentation.pip_windows.get(drag.from_index) {
                            let props = DWM_THUMBNAIL_PROPERTIES {
                                dwFlags: DWM_TNP_OPACITY,
                                opacity: reorder_thumbnail_alpha(s.presentation.thumbnail_alpha),
                                ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            request_redraw(pw.label_hwnd);
                        }
                    }
                }

                if drag.dragging {
                    // Find which pip is under cursor (screen coords).
                    let new_target = s
                        .presentation
                        .pip_windows
                        .iter()
                        .enumerate()
                        .find(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            cursor.x >= r.left
                                && cursor.x < r.right
                                && cursor.y >= r.top
                                && cursor.y < r.bottom
                        })
                        .map(|(i, _)| i);

                    if s.interaction.drop_target != new_target {
                        // Invalidate old and new target.
                        if let Some(old_t) = s.interaction.drop_target {
                            if let Some(pw) = s.presentation.pip_windows.get(old_t) {
                                request_redraw(pw.label_hwnd);
                            }
                        }
                        s.interaction.drop_target = new_target;
                        if let Some(new_t) = new_target {
                            if let Some(pw) = s.presentation.pip_windows.get(new_t) {
                                request_redraw(pw.label_hwnd);
                            }
                        }
                    }
                }

                return LRESULT(0);
            }

            // --- Use mode: notification action and PiP hover ---
            let point = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            notifications::update_invite_hover(s, pip_idx, point);
            if let Some(pw) = s.presentation.pip_windows.get_mut(pip_idx) {
                if !pw.hovered {
                    pw.hovered = true;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY,
                        opacity: THUMB_OPACITY_HOVER,
                        ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    request_redraw(pw.label_hwnd);
                }
            }

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
            // Clear notification and PiP hover.
            notifications::clear_invite_interaction(s, pip_idx);
            if let Some(pw) = s.presentation.pip_windows.get_mut(pip_idx) {
                if pw.hovered {
                    pw.hovered = false;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY,
                        opacity: s.presentation.thumbnail_alpha,
                        ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    request_redraw(pw.label_hwnd);
                }
            }

            // A potential click that never became a captured drag is no
            // longer active. Right/middle/X-button presses also have no
            // gesture state, so restore EQ whenever no captured interaction
            // still owns the pointer.
            if s.interaction
                .reorder_drag
                .as_ref()
                .is_some_and(|drag| !drag.dragging)
            {
                cancel_reorder_drag(s);
            }
            let captured_interaction = notifications::invite_action_pressed(s, pip_idx)
                || s.interaction.move_drag.is_some()
                || s.interaction.pip_resize_drag.is_some()
                || s.interaction.strip_resize_drag.is_some()
                || s.interaction
                    .reorder_drag
                    .as_ref()
                    .is_some_and(|drag| drag.dragging);
            if !captured_interaction {
                restore_active_eq_if_owned(s, hwnd);
            }

            LRESULT(0)
        }

        WM_CANCELMODE => {
            notifications::clear_invite_interaction(s, pip_idx);
            s.interaction.move_drag = None;
            s.interaction.pip_resize_drag = None;
            s.interaction.strip_resize_drag = None;
            cancel_reorder_drag(s);
            let _ = ReleaseCapture();
            restore_active_eq_if_owned(s, hwnd);
            LRESULT(0)
        }

        WM_CAPTURECHANGED => {
            // Capture APIs can send this message synchronously while another
            // branch still owns `&mut OverlayState`. Defer state access until
            // the outer window-procedure invocation has returned.
            let _ = PostMessageW(hwnd, WM_CLEAR_INVITE_CAPTURE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }

        WM_CLEAR_INVITE_CAPTURE => {
            notifications::clear_invite_interaction(s, pip_idx);
            s.interaction.move_drag = None;
            s.interaction.pip_resize_drag = None;
            s.interaction.strip_resize_drag = None;
            cancel_reorder_drag(s);
            restore_active_eq_if_owned(s, hwnd);
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };

            if !s.interaction.edit_mode && notifications::press_invite_action(s, pip_idx, pt) {
                cancel_reorder_drag(s);
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

            cancel_reorder_drag(s);
            if s.interaction.edit_mode {
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);
                let zone = scale(RESIZE_ZONE, s.layout.dpi_scale);

                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                let mut win_rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut win_rect);

                if let Some(edge) = edit_resize_edge_hit_test(pt, cr.right, cr.bottom, zone) {
                    // Start resize drag.
                    s.interaction.pip_resize_drag = Some(PipResizeDragState {
                        pip_index: pip_idx,
                        edge,
                        start_cursor: cursor,
                        start_rect: win_rect,
                    });
                    let _ = SetCapture(hwnd);
                } else {
                    // Start move drag.
                    s.interaction.move_drag = Some(MoveDragState {
                        pip_index: pip_idx,
                        start_cursor: cursor,
                        start_rect: win_rect,
                    });
                    let _ = SetCapture(hwnd);
                }
            } else {
                // Use mode.
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);

                if !s.layout.has_custom_positions {
                    // Check for strip resize hit.
                    let handle_w = scale(RESIZE_HANDLE_WIDTH, s.layout.dpi_scale);
                    if strip_resize_hit_test(pt, cr.right, cr.bottom, s.layout.pip_edge, handle_w) {
                        let mut cursor = POINT::default();
                        let _ = GetCursorPos(&mut cursor);
                        let is_vertical = matches!(
                            s.layout.pip_edge,
                            config::PipEdge::Right | config::PipEdge::Left
                        );
                        let start_size = if is_vertical {
                            s.layout.strip_width
                        } else {
                            s.layout.strip_height
                        };
                        s.interaction.strip_resize_drag = Some(StripResizeDragState {
                            start_pt: cursor,
                            start_size,
                        });
                        let _ = SetCapture(hwnd);
                        return LRESULT(0);
                    }
                }

                // Start potential reorder drag.
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                s.interaction.reorder_drag = Some(ReorderDragState {
                    from_index: pip_idx,
                    start_pt: cursor,
                    dragging: false,
                });
            }

            LRESULT(0)
        }

        WM_LBUTTONUP => {
            if !s.interaction.edit_mode {
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let had_invite_press = notifications::invite_action_pressed(s, pip_idx);
                let invite_action = notifications::release_invite_action(s, pip_idx, point);
                if had_invite_press {
                    cancel_reorder_drag(s);
                    let _ = ReleaseCapture();
                    if let Some((pid, action)) = invite_action {
                        notifications::execute_invite_action(s, pid, action);
                    }
                    restore_active_eq_if_owned(s, hwnd);
                    return LRESULT(0);
                }
            }
            let _ = ReleaseCapture();

            // --- Edit mode: finalize move/resize ---
            if s.interaction.move_drag.take().is_some() {
                restore_active_eq_if_owned(s, hwnd);
                return LRESULT(0);
            }
            if s.interaction.pip_resize_drag.take().is_some() {
                restore_active_eq_if_owned(s, hwnd);
                return LRESULT(0);
            }

            // --- Strip resize finalize ---
            if s.interaction.strip_resize_drag.take().is_some() {
                let mut cfg = config::Config::load();
                cfg.pip_strip_width = s.layout.custom_strip_width.map(|v| v as u32);
                let _ = cfg.save();
                restore_active_eq_if_owned(s, hwnd);
                return LRESULT(0);
            }

            // --- Reorder drag finalize ---
            let drag = s.interaction.reorder_drag.take();
            let old_drop_target = s.interaction.drop_target.take();

            if let Some(drag) = drag {
                if drag.dragging {
                    // Restore source thumbnail opacity.
                    if let Some(pw) = s.presentation.pip_windows.get(drag.from_index) {
                        let props = DWM_THUMBNAIL_PROPERTIES {
                            dwFlags: DWM_TNP_OPACITY,
                            opacity: s.presentation.thumbnail_alpha,
                            ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                        request_redraw(pw.label_hwnd);
                    }
                    if let Some(target) = old_drop_target {
                        if let Some(pw) = s.presentation.pip_windows.get(target) {
                            request_redraw(pw.label_hwnd);
                        }
                    }
                    // Perform the swap by client identity, not by presentation
                    // index. A transient host-creation failure may compact the
                    // rendered list, but it must never redirect an interaction
                    // to a different EQ client.
                    if let Some(to_index) = old_drop_target {
                        let pids = s
                            .presentation
                            .pip_windows
                            .get(drag.from_index)
                            .zip(s.presentation.pip_windows.get(to_index))
                            .map(|(from, to)| (from.pid, to.pid));
                        if let Some((pid_a, pid_b)) = pids.filter(|(a, b)| a != b) {
                            if let Some((from_client_index, to_client_index)) =
                                client_reorder_indices(s.clients.pips(), pid_a, pid_b)
                            {
                                // When auto-order is on, swap window numbers so
                                // the sort keeps the user's intended arrangement.
                                if config::Config::load().auto_order {
                                    let num_a = s
                                        .clients
                                        .windows
                                        .iter()
                                        .find(|w| w.pid == pid_a)
                                        .map(|w| w.number);
                                    let num_b = s
                                        .clients
                                        .windows
                                        .iter()
                                        .find(|w| w.pid == pid_b)
                                        .map(|w| w.number);
                                    if let (Some(na), Some(nb)) = (num_a, num_b) {
                                        if let Some(wa) =
                                            s.clients.windows.iter_mut().find(|w| w.pid == pid_a)
                                        {
                                            wa.number = nb;
                                        }
                                        if let Some(wb) =
                                            s.clients.windows.iter_mut().find(|w| w.pid == pid_b)
                                        {
                                            wb.number = na;
                                        }
                                    }
                                }
                                let swapped =
                                    s.clients.swap_pips(from_client_index, to_client_index);
                                debug_assert!(swapped);
                                rebuild_thumbnails(s);
                                update_visibility(s);
                                publish(s);
                            }
                        }
                    }
                } else {
                    // Simple click → activate the presented identity.
                    if let Some((pid, target_hwnd)) = s
                        .presentation
                        .pip_windows
                        .get(drag.from_index)
                        .map(|pip| (pip.pid, pip.source_hwnd))
                    {
                        if let Err(error) = activate_pid_inner(s, pid) {
                            debug_log(&format!(
                                "PiP click activation failed: {} ({})",
                                error.message,
                                error.code.as_str()
                            ));
                            let foreground = GetForegroundWindow();
                            if foreground == hwnd || foreground == target_hwnd {
                                restore_active_eq_if_owned(s, foreground);
                            }
                        }
                    } else {
                        restore_active_eq_if_owned(s, hwnd);
                    }
                    return LRESULT(0);
                }
            }

            restore_active_eq_if_owned(s, hwnd);
            LRESULT(0)
        }

        WM_LBUTTONDBLCLK => {
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };

            if !s.interaction.edit_mode && !s.layout.has_custom_positions {
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);
                let handle_w = scale(RESIZE_HANDLE_WIDTH, s.layout.dpi_scale);
                if strip_resize_hit_test(pt, cr.right, cr.bottom, s.layout.pip_edge, handle_w) {
                    s.layout.custom_strip_width = None;
                    let mut cfg = config::Config::load();
                    cfg.pip_strip_width = None;
                    let _ = cfg.save();
                    rebuild_thumbnails(s);
                    update_visibility(s);
                }
            }
            restore_active_eq_if_owned(s, hwnd);
            LRESULT(0)
        }

        WM_MBUTTONDOWN | WM_XBUTTONDOWN => LRESULT(0),

        WM_MBUTTONUP => {
            restore_active_eq_if_owned(s, hwnd);
            LRESULT(0)
        }

        WM_XBUTTONUP => {
            restore_active_eq_if_owned(s, hwnd);
            LRESULT(1)
        }

        WM_RBUTTONUP => {
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };

            if let Some(pw) = s.presentation.pip_windows.get(pip_idx) {
                let pid = pw.pid;
                let mut screen_pt = pt;
                let _ = ClientToScreen(hwnd, &mut screen_pt);
                queue_char_menu(s, pid, screen_pt, hwnd);
            }
            LRESULT(0)
        }

        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            // Don't set dpi_scale from this PiP window — it may be on the
            // wrong monitor after a display change. rebuild_thumbnails derives
            // DPI from the EQ window (same source as monitor_rect).
            rebuild_thumbnails(s);
            update_visibility(s);
            LRESULT(0)
        }

        WM_DESTROY => {
            // Individual PiP cleanup is handled by rebuild_thumbnails / cleanup.
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_never_brightens_a_thumbnail() {
        assert_eq!(reorder_thumbnail_alpha(25), 25);
        assert_eq!(reorder_thumbnail_alpha(204), THUMB_OPACITY_DRAG_MAX);
    }

    #[test]
    fn compacted_presentation_reorders_by_identity_not_rendered_index() {
        let client_pips = [10, 20, 30, 40];
        assert_eq!(client_reorder_indices(&client_pips, 30, 10), Some((2, 0)));
        assert_eq!(client_reorder_indices(&client_pips, 99, 10), None);
    }
}
