use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmFlush, DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::appearance::{color_for_number, format_label, label_model, BORDER_WIDTH, LABEL_COLORS};
use super::geometry::{client_animations_enabled, dpi_scale, scale};
use super::interaction::take_reorder_cancellation;
use super::labels::LabelStyle;
use super::layout;
use super::pip_transition::{
    destroy_pip_window, finish as finish_pip_transition, settle_now as settle_pip_transition,
    start as start_pip_transition, PipMotion, PipTransition, ThumbnailHandoff,
};
use super::presentation::{ActiveSceneKey, BannerSceneKey, PipWindowEntry};
use super::scene_layout::{TIMER_PANEL_GAP, TIMER_PANEL_HEIGHT};
use super::scenes::UiTextRole;
use super::state::OverlayState;
use super::surfaces::{
    apply_pip_pair_visibility, ensure_compositor, input_indicator_background, input_indicator_text,
    overlay_visibility_allowed, position_pip_pair, position_window_if_changed,
    render_active_label_surface_for_size, render_banner_surface_for_size, render_pip_surface,
    request_redraw, surface_is_ready,
};
use super::timer_controller::active_timer;
use crate::diagnostics::debug_log;
use crate::eq_windows::EqWindow;
use crate::{config, eq_windows};

const THUMB_GAP: i32 = 4;
const PRIMARY_HANDOFF_OFFSET: i32 = 18;

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

struct DesiredPip {
    window: EqWindow,
    rect: RECT,
}

struct ExistingPip {
    old_rect: RECT,
    entry: PipWindowEntry,
}

#[derive(Clone, Copy)]
struct HostReuseKey {
    pid: u32,
    source_hwnd: isize,
    reusable: bool,
}

fn plan_host_reuse(desired: &[(u32, isize)], existing: &[HostReuseKey]) -> Vec<Option<usize>> {
    let mut available = vec![true; existing.len()];
    let mut assignments = vec![None; desired.len()];

    // Preserve every live source relationship first.
    for (target_index, (pid, source_hwnd)) in desired.iter().copied().enumerate() {
        if let Some(existing_index) = existing.iter().enumerate().position(|(index, host)| {
            available[index] && host.reusable && host.pid == pid && host.source_hwnd == source_hwnd
        }) {
            available[existing_index] = false;
            assignments[target_index] = Some(existing_index);
        }
    }

    // Missing sources prefer the same physical slot, then any spare host.
    for target_index in 0..assignments.len() {
        if assignments[target_index].is_some() {
            continue;
        }
        let existing_index = existing
            .get(target_index)
            .filter(|host| available[target_index] && host.reusable)
            .map(|_| target_index)
            .or_else(|| {
                existing
                    .iter()
                    .enumerate()
                    .position(|(index, host)| available[index] && host.reusable)
            });
        if let Some(existing_index) = existing_index {
            available[existing_index] = false;
            assignments[target_index] = Some(existing_index);
        }
    }
    assignments
}

fn rect_size(rect: RECT) -> (i32, i32) {
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn rect_position_changed(first: RECT, second: RECT) -> bool {
    first.left != second.left || first.top != second.top
}

fn offset_primary_handoff(mut rect: RECT, edge: &config::PipEdge, distance: i32) -> RECT {
    let (dx, dy) = match edge {
        config::PipEdge::Right => (distance, 0),
        config::PipEdge::Left => (-distance, 0),
        config::PipEdge::Top => (0, -distance),
        config::PipEdge::Bottom => (0, distance),
    };
    rect.left += dx;
    rect.right += dx;
    rect.top += dy;
    rect.bottom += dy;
    rect
}

unsafe fn current_window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).is_ok().then_some(rect)
}

unsafe fn host_is_reusable(pip: &PipWindowEntry) -> bool {
    IsWindow(pip.hwnd).as_bool() && IsWindow(pip.label_hwnd).as_bool()
}

unsafe fn configure_thumbnail(
    thumbnail: isize,
    width: i32,
    height: i32,
    border: i32,
    alpha: u8,
) -> bool {
    if thumbnail == 0 {
        return false;
    }
    let properties = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_VISIBLE
            | DWM_TNP_OPACITY
            | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: RECT {
            left: border,
            top: border,
            right: width - border,
            bottom: height - border,
        },
        fVisible: true.into(),
        opacity: alpha,
        fSourceClientAreaOnly: true.into(),
        ..Default::default()
    };
    match DwmUpdateThumbnailProperties(thumbnail, &properties) {
        Ok(()) => true,
        Err(error) => {
            debug_log(&format!("DWM PiP thumbnail update failed: {error}"));
            false
        }
    }
}

unsafe fn create_pip_window(
    desired: &DesiredPip,
    index: usize,
    border: i32,
    initial_alpha: u8,
) -> Option<PipWindowEntry> {
    let (width, height) = rect_size(desired.rect);
    let hwnd = match CreateWindowExW(
        // The interactive host must take foreground before EQ polls its
        // foreground-only DirectInput mouse. The transparent composition
        // sibling remains non-activating and routes hit tests here.
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("StonemitePipClass"),
        w!("StonemitePip"),
        WS_POPUP,
        desired.rect.left,
        desired.rect.top,
        width,
        height,
        None,
        None,
        None,
        None,
    ) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            debug_log(&format!("PiP host creation failed: {error}"));
            return None;
        }
    };
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (index + 1) as isize);

    let thumbnail = match DwmRegisterThumbnail(hwnd, desired.window.hwnd) {
        Ok(thumbnail) => thumbnail,
        Err(error) => {
            debug_log(&format!("DWM PiP thumbnail registration failed: {error}"));
            let _ = DestroyWindow(hwnd);
            return None;
        }
    };
    if !configure_thumbnail(thumbnail, width, height, border, initial_alpha) {
        let _ = DwmUnregisterThumbnail(thumbnail);
        let _ = DestroyWindow(hwnd);
        return None;
    }

    // One full-host transparent composition sibling owns every authored PiP
    // visual. The DWM host remains thumbnail-only beneath it.
    let label_hwnd = match CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
        w!("StonemitePipLabelClass"),
        w!("StonemitePipComposition"),
        WS_POPUP,
        desired.rect.left,
        desired.rect.top,
        width,
        height,
        None,
        None,
        None,
        None,
    ) {
        Ok(hwnd) => hwnd,
        Err(error) => {
            debug_log(&format!("PiP composition host creation failed: {error}"));
            let _ = DwmUnregisterThumbnail(thumbnail);
            let _ = DestroyWindow(hwnd);
            return None;
        }
    };
    SetWindowLongPtrW(label_hwnd, GWLP_USERDATA, (index + 1) as isize);

    Some(PipWindowEntry {
        hwnd,
        label_hwnd,
        source_hwnd: desired.window.hwnd,
        pid: desired.window.pid,
        thumb: thumbnail,
        label: format_label(&desired.window),
        class: desired.window.class.clone(),
        number: desired.window.number,
        hovered: false,
    })
}

fn update_pip_identity(pip: &mut PipWindowEntry, desired: &DesiredPip) {
    pip.source_hwnd = desired.window.hwnd;
    pip.pid = desired.window.pid;
    pip.label = format_label(&desired.window);
    pip.class = desired.window.class.clone();
    pip.number = desired.window.number;
    pip.hovered = false;
}

pub(super) unsafe fn rebuild_thumbnails(s: &mut OverlayState) {
    cancel_reorder_drag(s);
    if !finish_pip_transition(s) {
        return;
    }

    let visibility_allowed = overlay_visibility_allowed(s);
    let animate = visibility_allowed && client_animations_enabled();
    let normal_alpha = s.presentation.thumbnail_alpha;
    let previous = std::mem::take(&mut s.presentation.pip_windows);

    if s.clients.pips().is_empty() {
        for pip in previous {
            destroy_pip_window(s, pip);
        }
        let _ = ShowWindow(s.presentation.active_label_hwnd, SW_HIDE);
        return;
    }

    let reference = s.clients.windows.first().map(|window| window.hwnd);
    s.layout.monitor_rect = eq_windows::get_monitor_work_area(reference);
    // Get DPI from the same monitor as the EQ windows, so it stays consistent
    // with monitor_rect after display changes (unplug/replug, DPI change).
    let dpi_hwnd = reference.unwrap_or(s.presentation.active_label_hwnd);
    s.layout.dpi_scale = dpi_scale(dpi_hwnd);

    let (rects, strip_width, strip_height) = compute_positions(s);
    s.layout.strip_width = strip_width;
    s.layout.strip_height = strip_height;
    let border = scale(BORDER_WIDTH, s.layout.dpi_scale);

    let desired = s
        .clients
        .pips()
        .iter()
        .copied()
        .zip(rects)
        .filter_map(|(pid, rect)| {
            s.clients
                .windows
                .iter()
                .find(|window| window.pid == pid && IsWindow(window.hwnd).as_bool())
                .cloned()
                .map(|window| DesiredPip { window, rect })
        })
        .collect::<Vec<_>>();

    let existing_keys = previous
        .iter()
        .map(|entry| HostReuseKey {
            pid: entry.pid,
            source_hwnd: entry.source_hwnd.0 as isize,
            reusable: host_is_reusable(entry),
        })
        .collect::<Vec<_>>();
    let desired_keys = desired
        .iter()
        .map(|target| (target.window.pid, target.window.hwnd.0 as isize))
        .collect::<Vec<_>>();
    let reuse_plan = plan_host_reuse(&desired_keys, &existing_keys);
    let mut available = previous
        .into_iter()
        .map(|entry| {
            let old_rect = current_window_rect(entry.hwnd).unwrap_or_default();
            Some(ExistingPip { old_rect, entry })
        })
        .collect::<Vec<_>>();
    let assignments = reuse_plan
        .into_iter()
        .map(|existing_index| existing_index.and_then(|index| available[index].take()))
        .collect::<Vec<_>>();

    let mut transition = PipTransition::new(normal_alpha);
    let mut retired = Vec::new();
    let mut next = Vec::with_capacity(desired.len());
    for (index, (target, assigned)) in desired.iter().zip(assignments).enumerate() {
        // Even reduced-motion swaps stage the incoming relationship at zero;
        // settle_now publishes the hard cut as one DWM handoff.
        let handoff_initial_alpha = 0;
        let mut old_rect = None;
        let mut source_handoff = false;
        let entry = if let Some(mut existing) = assigned {
            old_rect = Some(existing.old_rect);
            let (target_width, target_height) = rect_size(target.rect);
            let source_unchanged = existing.entry.pid == target.window.pid
                && existing.entry.source_hwnd == target.window.hwnd;
            if source_unchanged {
                let _ = configure_thumbnail(
                    existing.entry.thumb,
                    target_width,
                    target_height,
                    border,
                    normal_alpha,
                );
                update_pip_identity(&mut existing.entry, target);
                Some(existing.entry)
            } else {
                match DwmRegisterThumbnail(existing.entry.hwnd, target.window.hwnd) {
                    Ok(incoming) => {
                        let _ = configure_thumbnail(
                            existing.entry.thumb,
                            target_width,
                            target_height,
                            border,
                            normal_alpha,
                        );
                        if configure_thumbnail(
                            incoming,
                            target_width,
                            target_height,
                            border,
                            handoff_initial_alpha,
                        ) {
                            transition.handoffs.push(ThumbnailHandoff {
                                outgoing: existing.entry.thumb,
                                incoming,
                                switched: false,
                            });
                            source_handoff = true;
                            existing.entry.thumb = incoming;
                            update_pip_identity(&mut existing.entry, target);
                        } else {
                            // Preserve the known-visible relationship and retry
                            // from the poll loop instead of creating a gap.
                            let _ = DwmUnregisterThumbnail(incoming);
                        }
                        Some(existing.entry)
                    }
                    Err(error) => {
                        // Keep the known-visible relationship and let the poll
                        // loop retry this identity mismatch. Four unaffected
                        // PiPs remain fully live, and this slot never goes blank.
                        debug_log(&format!("DWM PiP thumbnail retarget failed: {error}"));
                        Some(existing.entry)
                    }
                }
            }
        } else {
            create_pip_window(target, index, border, normal_alpha)
        };
        let Some(entry) = entry else { continue };

        let (width, height) = rect_size(target.rect);
        if animate
            && old_rect.is_some_and(|rect| {
                rect_size(rect) == (width, height)
                    && (source_handoff || rect_position_changed(rect, target.rect))
            })
        {
            let old_rect = old_rect.expect("animated PiP movement has a source rectangle");
            let from = if source_handoff && !rect_position_changed(old_rect, target.rect) {
                offset_primary_handoff(
                    target.rect,
                    &s.layout.pip_edge,
                    scale(PRIMARY_HANDOFF_OFFSET, s.layout.dpi_scale),
                )
            } else {
                old_rect
            };
            position_pip_pair(&entry, from.left, from.top, width, height);
            transition.motions.push(PipMotion {
                hwnd: entry.hwnd,
                label_hwnd: entry.label_hwnd,
                from,
                to: target.rect,
            });
        } else {
            position_pip_pair(&entry, target.rect.left, target.rect.top, width, height);
        }
        SetWindowLongPtrW(entry.hwnd, GWLP_USERDATA, (next.len() + 1) as isize);
        SetWindowLongPtrW(entry.label_hwnd, GWLP_USERDATA, (next.len() + 1) as isize);
        next.push(entry);
    }

    for existing in available.into_iter().flatten() {
        SetWindowLongPtrW(existing.entry.hwnd, GWLP_USERDATA, 0);
        SetWindowLongPtrW(existing.entry.label_hwnd, GWLP_USERDATA, 0);
        retired.push(existing.entry);
    }

    s.presentation.pip_windows = next;
    for index in 0..s.presentation.pip_windows.len() {
        render_pip_surface(s, index);
    }
    update_active_label(s);

    // Existing hosts never leave the screen. Newly staged pairs are exposed
    // only after their complete DirectComposition frame has been presented.
    if visibility_allowed {
        apply_pip_pair_visibility(s, true);
    }
    if !retired.is_empty() {
        // Replacement pairs are already complete and visible. Flush their DWM
        // work before removing stale hosts, but never keep closed clients on
        // screen merely to make an exit animation decorative.
        if DwmFlush().is_err() {
            debug_log("DWM flush failed while retiring stale PiP hosts");
        }
        for pip in retired {
            destroy_pip_window(s, pip);
        }
    }
    if animate {
        start_pip_transition(s, transition);
    } else {
        settle_pip_transition(s, transition);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn host(pid: u32, source_hwnd: isize) -> HostReuseKey {
        HostReuseKey {
            pid,
            source_hwnd,
            reusable: true,
        }
    }

    #[test]
    fn active_swap_preserves_unchanged_sources_and_reuses_exchanged_slot() {
        let existing = [host(2, 20), host(3, 30), host(4, 40)];
        let desired = [(1, 10), (3, 30), (4, 40)];
        assert_eq!(
            plan_host_reuse(&desired, &existing),
            vec![Some(0), Some(1), Some(2)]
        );
    }

    #[test]
    fn auto_order_moves_stable_sources_before_assigning_the_spare_host() {
        let existing = [
            host(2, 20),
            host(3, 30),
            host(4, 40),
            host(5, 50),
            host(6, 60),
        ];
        let desired = [(1, 10), (2, 20), (3, 30), (4, 40), (6, 60)];
        assert_eq!(
            plan_host_reuse(&desired, &existing),
            vec![Some(3), Some(0), Some(1), Some(2), Some(4)]
        );
    }

    #[test]
    fn invalid_hosts_are_not_reassigned() {
        let existing = [
            HostReuseKey {
                reusable: false,
                ..host(2, 20)
            },
            host(3, 30),
        ];
        assert_eq!(
            plan_host_reuse(&[(1, 10), (3, 30)], &existing),
            vec![None, Some(1)]
        );
    }

    #[test]
    fn primary_handoff_starts_just_outside_the_configured_edge() {
        let rect = RECT {
            left: 100,
            top: 200,
            right: 300,
            bottom: 320,
        };
        let right = offset_primary_handoff(rect, &config::PipEdge::Right, 18);
        assert_eq!(
            (right.left, right.top, right.right, right.bottom),
            (118, 200, 318, 320)
        );
        let top = offset_primary_handoff(rect, &config::PipEdge::Top, 18);
        assert_eq!(
            (top.left, top.top, top.right, top.bottom),
            (100, 182, 300, 302)
        );
    }
}
