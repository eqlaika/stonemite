use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::appearance::{color_for_number, format_label, label_model, BORDER_WIDTH, LABEL_COLORS};
use super::geometry::{dpi_scale, scale};
use super::interaction::take_reorder_cancellation;
use super::labels::LabelStyle;
use super::layout;
use super::presentation::{ActiveSceneKey, BannerSceneKey, PipWindowEntry};
use super::scene_layout::{TIMER_PANEL_GAP, TIMER_PANEL_HEIGHT};
use super::scenes::UiTextRole;
use super::state::OverlayState;
use super::surfaces::{
    ensure_compositor, input_indicator_background, input_indicator_text,
    position_window_if_changed, render_active_label_surface_for_size,
    render_banner_surface_for_size, render_pip_surface, request_redraw, surface_is_ready,
    unregister_composition_surface,
};
use super::timer_controller::active_timer;
use crate::diagnostics::debug_log;
use crate::{config, eq_windows};

const THUMB_GAP: i32 = 4;

pub(super) unsafe fn cancel_reorder_drag(state: &mut OverlayState) {
    let cancellation = take_reorder_cancellation(
        &mut state.interaction.reorder_drag,
        &mut state.interaction.drop_target,
    );
    if let Some(source) = cancellation.dimmed_source {
        if let Some(pip) = state.presentation.pip_windows.get(source) {
            let properties = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY,
                opacity: state.presentation.thumbnail_alpha,
                ..Default::default()
            };
            let _ = DwmUpdateThumbnailProperties(pip.thumb, &properties);
            request_redraw(pip.label_hwnd);
        }
    }
    if let Some(target) = cancellation.old_target {
        if Some(target) != cancellation.dimmed_source {
            if let Some(pip) = state.presentation.pip_windows.get(target) {
                request_redraw(pip.label_hwnd);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Position computation
// ---------------------------------------------------------------------------

pub(super) fn compute_positions(s: &OverlayState) -> (Vec<RECT>, i32, i32) {
    let configured_positions = s.layout.has_custom_positions.then(config::Config::load);
    let custom_positions = configured_positions
        .as_ref()
        .map_or(&[][..], |cfg| cfg.pip_positions.as_slice());
    let plan = layout::compute(layout::LayoutInput {
        dpi_scale: s.layout.dpi_scale,
        monitor_rect: s.layout.monitor_rect,
        pip_count: s.clients.pips().len(),
        pip_edge: s.layout.pip_edge,
        custom_strip_width: s.layout.custom_strip_width,
        custom_positions,
        gap: scale(THUMB_GAP, s.layout.dpi_scale),
        border: scale(BORDER_WIDTH, s.layout.dpi_scale),
    });
    (plan.rects, plan.strip_width, plan.strip_height)
}

// ---------------------------------------------------------------------------
// Rebuild
// ---------------------------------------------------------------------------

pub(super) unsafe fn rebuild_thumbnails(s: &mut OverlayState) {
    cancel_reorder_drag(s);
    // Destroy existing PiP windows only after authored composition surfaces
    // are detached. DWM thumbnail ownership remains on the host HWND.
    let previous = std::mem::take(&mut s.presentation.pip_windows);
    for pw in previous {
        let composition_detached = unregister_composition_surface(s, pw.label_hwnd);
        if pw.thumb != 0 {
            let _ = DwmUnregisterThumbnail(pw.thumb);
        }
        if !pw.label_hwnd.is_invalid() {
            if composition_detached {
                let _ = DestroyWindow(pw.label_hwnd);
            } else {
                let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
                if !s
                    .presentation
                    .pending_composition_destroys
                    .contains(&pw.label_hwnd)
                {
                    s.presentation
                        .pending_composition_destroys
                        .push(pw.label_hwnd);
                }
            }
        }
        let _ = DestroyWindow(pw.hwnd);
    }

    if s.clients.pips().is_empty() {
        let _ = ShowWindow(s.presentation.active_label_hwnd, SW_HIDE);
        return;
    }

    let reference = s.clients.windows.first().map(|w| w.hwnd);
    s.layout.monitor_rect = eq_windows::get_monitor_work_area(reference);
    // Get DPI from the same monitor as the EQ windows, so it stays consistent
    // with monitor_rect after display changes (unplug/replug, DPI change).
    let dpi_hwnd = reference.unwrap_or(s.presentation.active_label_hwnd);
    s.layout.dpi_scale = dpi_scale(dpi_hwnd);

    let (rects, sw, sh) = compute_positions(s);
    s.layout.strip_width = sw;
    s.layout.strip_height = sh;

    let d = s.layout.dpi_scale;
    let border = scale(BORDER_WIDTH, d);

    let pip_class = w!("StonemitePipClass");

    let pip_order = s.clients.pips().to_vec();
    for (i, pid) in pip_order.into_iter().enumerate() {
        let Some(eq_win) = s.clients.windows.iter().find(|w| w.pid == pid).cloned() else {
            continue;
        };
        if !IsWindow(eq_win.hwnd).as_bool() {
            continue;
        }
        let Some(rect) = rects.get(i) else { continue };

        let cw = rect.right - rect.left;
        let ch = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            pip_class,
            w!("StonemitePip"),
            WS_POPUP,
            rect.left,
            rect.top,
            cw,
            ch,
            None,
            None,
            None,
            None,
        )
        .expect("Failed to create PiP window");

        // Store 1-based index so 0 = uninitialized.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (i + 1) as isize);

        // Register DWM thumbnail filling the window (label overlays on top).
        let thumb_rect = RECT {
            left: border,
            top: border,
            right: cw - border,
            bottom: ch - border,
        };

        let thumb = match DwmRegisterThumbnail(hwnd, eq_win.hwnd) {
            Ok(t) => t,
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                continue;
            }
        };

        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: thumb_rect,
            fVisible: true.into(),
            opacity: s.presentation.thumbnail_alpha,
            fSourceClientAreaOnly: true.into(),
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);

        // One full-host transparent composition sibling owns every authored
        // PiP visual. The DWM host remains thumbnail-only beneath it.
        let label_text = format_label(&eq_win);
        let pip_label_class = w!("StonemitePipLabelClass");
        let lbl_hwnd = CreateWindowExW(
            WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | WS_EX_NOREDIRECTIONBITMAP,
            pip_label_class,
            w!("StonemitePipComposition"),
            WS_POPUP,
            rect.left,
            rect.top,
            cw,
            ch,
            None,
            None,
            None,
            None,
        )
        .expect("Failed to create PiP composition window");
        SetWindowLongPtrW(lbl_hwnd, GWLP_USERDATA, (i + 1) as isize);

        s.presentation.pip_windows.push(PipWindowEntry {
            hwnd,
            label_hwnd: lbl_hwnd,
            pid,
            thumb,
            label: label_text,
            class: eq_win.class.clone(),
            number: eq_win.number,
            hovered: false,
        });
        render_pip_surface(s, s.presentation.pip_windows.len() - 1);
    }

    update_active_label(s);
}

// ---------------------------------------------------------------------------
// Active label
// ---------------------------------------------------------------------------

pub(super) unsafe fn update_active_label(s: &mut OverlayState) {
    let active = s
        .clients
        .active_pid()
        .and_then(|pid| s.clients.windows.iter().find(|w| w.pid == pid));
    s.presentation.active_label_text = active.map(format_label).unwrap_or_default();
    s.presentation.active_label_class = active.and_then(|w| w.class.clone());
    s.presentation.active_label_color = active
        .map(|w| color_for_number(w.number))
        .unwrap_or(LABEL_COLORS[0]);
    s.presentation.active_label_number = active.map(|w| w.number).unwrap_or(0);

    if s.presentation.active_label_text.is_empty() {
        let _ = ShowWindow(s.presentation.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.presentation.broadcast_label_hwnd, SW_HIDE);
        return;
    }

    let active_hwnd = active.unwrap().hwnd;
    let mut rect = RECT::default();
    let _ = GetClientRect(active_hwnd, &mut rect);
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let _ = ClientToScreen(active_hwnd, &mut top_left);
    let mut top_right = POINT {
        x: rect.right,
        y: rect.top,
    };
    let _ = ClientToScreen(active_hwnd, &mut top_right);

    let d = s.layout.dpi_scale;
    let lh = s.presentation.label_height;
    let style = LabelStyle::new(d, lh);
    let label_h = style.height();
    let active_timer = active_timer(s, Instant::now());
    let timer_label = active_timer.map(|timer| timer.label.to_string());
    let timer_start = active_timer.map(|timer| timer.start_time);
    let timer_height = timer_label
        .as_ref()
        .map(|_| scale(TIMER_PANEL_GAP + TIMER_PANEL_HEIGHT, d))
        .unwrap_or(0);
    let active_scene_key = ActiveSceneKey {
        text: s.presentation.active_label_text.clone(),
        class: s.presentation.active_label_class.clone(),
        color: s.presentation.active_label_color,
        number: s.presentation.active_label_number,
        label_height: s.presentation.label_height,
        label_alpha: s.presentation.label_alpha,
        dpi_bits: s.layout.dpi_scale.to_bits(),
        theme: s.presentation.label_theme.clone(),
        timer_label,
        timer_start,
    };
    let active_scene_changed = s.presentation.active_scene_key.as_ref() != Some(&active_scene_key);
    if !ensure_compositor(s) {
        return;
    }
    let model = label_model(
        &s.presentation.active_label_text,
        s.presentation.active_label_class.as_deref(),
        s.presentation.active_label_number,
        s.presentation.active_label_color,
    );
    let text_width = match s
        .presentation
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .measure_label_width(&model, style, &s.presentation.label_theme, i32::MAX)
    {
        Ok(width) => width,
        Err(error) => {
            debug_log(&format!(
                "DirectWrite active-label measurement failed: {error}"
            ));
            return;
        }
    };

    // When PiP edge is left, anchor the label at top-right so the strip doesn't cover it.
    let label_x = if matches!(s.layout.pip_edge, config::PipEdge::Left) {
        top_right.x - text_width
    } else {
        top_left.x
    };

    let active_size = (text_width, label_h + timer_height);
    if (active_scene_changed || !surface_is_ready(s, s.presentation.active_label_hwnd))
        && !render_active_label_surface_for_size(s, active_size.0, active_size.1)
    {
        return;
    }
    s.presentation.active_scene_key = Some(active_scene_key);
    position_window_if_changed(
        s.presentation.active_label_hwnd,
        HWND_TOPMOST,
        label_x,
        top_left.y,
        active_size.0,
        active_size.1,
    );

    // Position the explicit keyboard/mouse input indicator next to the active label.
    if let Some(bc_text) = input_indicator_text() {
        let bc_width = match s
            .presentation
            .compositor
            .as_ref()
            .expect("compositor ensured")
            .measure_text(
                bc_text,
                &UiTextRole::StatusBanner.font(),
                UiTextRole::StatusBanner.height(d, (lh - 12).max(1)),
            ) {
            Ok(width) => width + scale(20, d),
            Err(error) => {
                debug_log(&format!(
                    "DirectWrite status-banner measurement failed: {error}"
                ));
                return;
            }
        };
        let bc_x = if matches!(s.layout.pip_edge, config::PipEdge::Left) {
            label_x - bc_width - scale(4, d)
        } else {
            label_x + text_width + scale(4, d)
        };
        let banner_scene_key = BannerSceneKey {
            text: bc_text.to_owned(),
            background: input_indicator_background(),
            label_height: s.presentation.label_height,
            label_alpha: s.presentation.label_alpha,
            dpi_bits: s.layout.dpi_scale.to_bits(),
        };
        let banner_scene_changed =
            s.presentation.banner_scene_key.as_ref() != Some(&banner_scene_key);
        if (banner_scene_changed || !surface_is_ready(s, s.presentation.broadcast_label_hwnd))
            && !render_banner_surface_for_size(s, bc_width, label_h)
        {
            return;
        }
        s.presentation.banner_scene_key = Some(banner_scene_key);
        position_window_if_changed(
            s.presentation.broadcast_label_hwnd,
            HWND_TOPMOST,
            bc_x,
            top_left.y,
            bc_width,
            label_h,
        );
    } else {
        s.presentation.banner_scene_key = None;
        let _ = ShowWindow(s.presentation.broadcast_label_hwnd, SW_HIDE);
    }
}
