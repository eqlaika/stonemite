use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ValidateRect};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::appearance::{color_for_number, label_model, BORDER_WIDTH};
use super::control_bridge::owns_foreground;
use super::geometry::scale;
use super::labels::{Color, LabelStyle, Rect};
use super::presentation::{ComApartment, PipWindowEntry};
use super::render::Compositor;
use super::runtime;
use super::scenes::{
    ActiveLabelScene, LabelScene, PipInteractionScene, PipScene, StatusBannerScene,
    StonemiteButtonScene, TimerScene, ToastScene,
};
use super::state::OverlayState;
use super::timer_controller::{active_timer, format_remaining};
use super::timers;
use super::toast::{
    publication_allowed as toast_publication_allowed, Phase as ToastPhase,
    BACKGROUND_COLOR as TOAST_BG_COLOR, FADE_STEP_MS as TOAST_FADE_STEP_MS,
    SERVICE_COMPOSITOR_RECOVERY_MESSAGE as WM_SERVICE_COMPOSITOR_RECOVERY,
    TIMER_ID as TIMER_TOAST_FADE,
};
use crate::diagnostics::debug_log;

// ---------------------------------------------------------------------------
// DirectComposition scene publication and redraw scheduling
// ---------------------------------------------------------------------------

pub(super) unsafe fn position_window_pair(
    hwnd: HWND,
    label_hwnd: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let deferred = (|| -> windows::core::Result<()> {
        let batch = BeginDeferWindowPos(2)?;
        let batch = DeferWindowPos(
            batch,
            hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        let batch = DeferWindowPos(
            batch,
            label_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        )?;
        EndDeferWindowPos(batch)
    })();
    if deferred.is_err() {
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            label_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        );
    }
}

pub(super) unsafe fn position_pip_pair(
    pip: &PipWindowEntry,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    position_window_pair(pip.hwnd, pip.label_hwnd, x, y, width, height);
}

pub(super) unsafe fn position_window_if_changed(
    hwnd: HWND,
    insert_after: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> bool {
    let mut current = RECT::default();
    let matches = GetWindowRect(hwnd, &mut current).is_ok()
        && current.left == x
        && current.top == y
        && current.right - current.left == width
        && current.bottom - current.top == height;
    matches || SetWindowPos(hwnd, insert_after, x, y, width, height, SWP_NOACTIVATE).is_ok()
}

pub(super) fn client_scene_rect(hwnd: HWND) -> Option<Rect> {
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client).ok()? };
    let width = (client.right - client.left).max(0);
    let height = (client.bottom - client.top).max(0);
    (width > 0 && height > 0).then(|| Rect::new(0, 0, width, height))
}

pub(super) unsafe fn hide_unready_pip_pairs(s: &OverlayState) {
    if !surface_is_ready(s, s.presentation.stonemite_button.hwnd) {
        let _ = ShowWindow(s.presentation.stonemite_button.hwnd, SW_HIDE);
    }
    for pip in &s.presentation.pip_windows {
        if !surface_is_ready(s, pip.label_hwnd) {
            let _ = ShowWindow(pip.hwnd, SW_HIDE);
            let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
        }
    }
}

pub(super) unsafe fn hide_all_pip_pairs(s: &OverlayState) {
    let _ = ShowWindow(s.presentation.stonemite_button.hwnd, SW_HIDE);
    for pip in &s.presentation.pip_windows {
        let _ = ShowWindow(pip.hwnd, SW_HIDE);
        let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
    }
}

pub(super) unsafe fn schedule_compositor_recovery(s: &OverlayState) {
    let redraws = match s
        .presentation
        .compositor
        .as_ref()
        .map(|compositor| compositor.recovery_redraw_hwnds())
    {
        Some(Ok(redraws)) => redraws,
        Some(Err(error)) => {
            debug_log(&format!(
                "DirectComposition recovery scheduling failed: {error}"
            ));
            return;
        }
        None => return,
    };
    if redraws.is_empty() {
        return;
    }

    hide_unready_pip_pairs(s);
    for hwnd in redraws {
        request_redraw(hwnd);
    }
    if runtime::recovery_is_running() {
        return;
    }
    if runtime::claim_recovery_post()
        && PostMessageW(
            s.presentation.active_label_hwnd,
            WM_SERVICE_COMPOSITOR_RECOVERY,
            WPARAM(0),
            LPARAM(0),
        )
        .is_err()
    {
        runtime::clear_recovery_post();
    }
}

pub(super) unsafe fn ensure_compositor(s: &mut OverlayState) -> bool {
    if !s.presentation.com_apartment.usable {
        s.presentation.com_apartment = ComApartment::initialize();
    }
    if s.presentation.compositor.is_none() && s.presentation.com_apartment.usable {
        match Compositor::new() {
            Ok(compositor) => s.presentation.compositor = Some(compositor),
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition compositor retry failed: {error}"
                ));
                hide_all_pip_pairs(s);
                return false;
            }
        }
    }
    let Some(compositor) = s.presentation.compositor.as_mut() else {
        hide_all_pip_pairs(s);
        return false;
    };
    if let Err(error) = compositor.recover_if_needed() {
        debug_log(&format!("DirectComposition device retry failed: {error}"));
        hide_all_pip_pairs(s);
        return false;
    }
    schedule_compositor_recovery(s);
    true
}

pub(super) unsafe fn unregister_composition_surface(s: &mut OverlayState, hwnd: HWND) -> bool {
    let Some(compositor) = s.presentation.compositor.as_mut() else {
        return true;
    };
    match compositor.unregister_surface(hwnd) {
        Ok(()) => {
            clear_redraw_request(hwnd);
            true
        }
        Err(first_error) => {
            debug_log(&format!(
                "DirectComposition surface teardown retry: {first_error}"
            ));
            if let Err(error) = compositor.recover_device() {
                debug_log(&format!(
                    "DirectComposition teardown recovery failed: {error}"
                ));
                return false;
            }
            match compositor.unregister_surface(hwnd) {
                Ok(()) => {
                    clear_redraw_request(hwnd);
                    true
                }
                Err(error) => {
                    debug_log(&format!(
                        "DirectComposition surface teardown failed: {error}"
                    ));
                    false
                }
            }
        }
    }
}

pub(super) unsafe fn retry_pending_composition_destroys(s: &mut OverlayState) {
    let pending = std::mem::take(&mut s.presentation.pending_composition_destroys);
    for hwnd in pending {
        let detached = unregister_composition_surface(s, hwnd);
        if detached && runtime::is_busy() {
            // DestroyWindow pumps messages, so only destroy while the outer
            // overlay transaction guard blocks re-entrant state access.
            let _ = DestroyWindow(hwnd);
        } else {
            s.presentation.pending_composition_destroys.push(hwnd);
        }
    }
}

pub(super) unsafe fn ensure_surface(
    s: &mut OverlayState,
    hwnd: HWND,
    width: i32,
    height: i32,
    opacity: f32,
) -> bool {
    if !ensure_compositor(s) {
        retain_redraw_request(hwnd);
        return false;
    }
    let result = {
        let compositor = s
            .presentation
            .compositor
            .as_mut()
            .expect("compositor ensured");
        match compositor.has_surface(hwnd) {
            Ok(true) => Ok(()),
            Ok(false) => compositor.register_surface(
                hwnd,
                width.max(1) as u32,
                height.max(1) as u32,
                opacity,
            ),
            Err(error) => Err(error),
        }
    };
    match result {
        Ok(()) => true,
        Err(error) => {
            debug_log(&format!(
                "DirectComposition surface registration/lookup failed: {error}"
            ));
            retain_redraw_request(hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) unsafe fn set_composition_opacity(s: &mut OverlayState, hwnd: HWND, alpha: u8) {
    if !ensure_compositor(s) {
        retain_redraw_request(hwnd);
        return;
    }
    let result = {
        let compositor = s
            .presentation
            .compositor
            .as_mut()
            .expect("compositor ensured");
        compositor
            .set_surface_opacity(hwnd, f32::from(alpha) / 255.0)
            .and_then(|()| compositor.flush())
    };
    if let Err(error) = result {
        debug_log(&format!("DirectComposition opacity commit failed: {error}"));
        retain_redraw_request(hwnd);
        service_compositor_recovery(s);
    }
}

pub(super) fn surface_is_ready(s: &OverlayState, hwnd: HWND) -> bool {
    s.presentation
        .compositor
        .as_ref()
        .and_then(|compositor| compositor.surface_is_attached(hwnd).ok())
        .unwrap_or(false)
}

pub(super) unsafe fn request_redraw(hwnd: HWND) {
    if runtime::mark_redraw_requested(hwnd) {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn retain_redraw_request(hwnd: HWND) {
    runtime::retain_redraw_request(hwnd);
}

pub(super) fn has_redraw_request(hwnd: HWND) -> bool {
    runtime::has_redraw_request(hwnd)
}

pub(super) fn take_redraw_request(hwnd: HWND) -> bool {
    runtime::take_redraw_request(hwnd)
}

pub(super) fn clear_redraw_request(hwnd: HWND) {
    runtime::clear_redraw_request(hwnd);
}

pub(super) unsafe fn service_compositor_recovery(s: &mut OverlayState) {
    let _ = runtime::try_service_recovery(|| unsafe { service_compositor_recovery_guarded(s) });
}

pub(super) unsafe fn suppress_toast_publication(s: &mut OverlayState) {
    s.presentation.toast.scene_ready = false;
    let _ = ShowWindow(s.presentation.toast.hwnd, SW_HIDE);
    let _ = KillTimer(s.presentation.toast.hwnd, TIMER_TOAST_FADE);
}

pub(super) unsafe fn service_compositor_recovery_guarded(s: &mut OverlayState) {
    if !ensure_compositor(s) {
        suppress_toast_publication(s);
        return;
    }
    retry_pending_composition_destroys(s);
    let redraws = match s
        .presentation
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .recovery_redraw_hwnds()
    {
        Ok(redraws) => redraws,
        Err(error) => {
            debug_log(&format!(
                "DirectComposition recovery handshake failed: {error}"
            ));
            suppress_toast_publication(s);
            return;
        }
    };
    for hwnd in redraws {
        retain_redraw_request(hwnd);
    }

    // Each role gets at most one complete-frame attempt in this recovery pass.
    // A second device loss retains dirty work for the normal polling retry
    // rather than recursing indefinitely on persistent hardware failure.
    if has_redraw_request(s.presentation.active_label_hwnd) {
        render_active_label_surface(s);
    }
    if has_redraw_request(s.presentation.stonemite_button.hwnd) {
        render_stonemite_button_surface(s);
    }
    if has_redraw_request(s.presentation.broadcast_label_hwnd) {
        render_banner_surface(s);
    }
    if has_redraw_request(s.presentation.toast.hwnd) {
        render_toast_surface(s);
    }
    for index in 0..s.presentation.pip_windows.len() {
        if has_redraw_request(s.presentation.pip_windows[index].label_hwnd) {
            render_pip_surface(s, index);
        }
    }

    apply_pip_pair_visibility(s, overlay_visibility_allowed(s));
    apply_stonemite_button_visibility(s);
    if toast_publication_allowed(
        s.presentation.toast.phase,
        s.presentation.toast.scene_ready,
        overlay_visibility_allowed(s),
        surface_is_ready(s, s.presentation.toast.hwnd),
    ) {
        let _ = ShowWindow(s.presentation.toast.hwnd, SW_SHOWNOACTIVATE);
        let _ = SetTimer(
            s.presentation.toast.hwnd,
            TIMER_TOAST_FADE,
            TOAST_FADE_STEP_MS,
            None,
        );
    } else {
        let _ = ShowWindow(s.presentation.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.presentation.toast.hwnd, TIMER_TOAST_FADE);
    }
}

pub(super) fn timer_scene_values(
    timer: &timers::TimerOverlay,
    now: Instant,
) -> (String, String, f32) {
    (
        timer.label.to_string(),
        format_remaining(timer.remaining_time(now)),
        timer.progress(now),
    )
}

pub(super) unsafe fn render_active_label_surface(s: &mut OverlayState) {
    let Some(canvas) = client_scene_rect(s.presentation.active_label_hwnd) else {
        return;
    };
    let _ = render_active_label_surface_for_size(s, canvas.width(), canvas.height());
}

pub(super) unsafe fn render_active_label_surface_for_size(
    s: &mut OverlayState,
    width: i32,
    height: i32,
) -> bool {
    if s.presentation.active_label_text.is_empty() {
        return false;
    }
    let canvas = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(
        s,
        s.presentation.active_label_hwnd,
        canvas.width(),
        canvas.height(),
        1.0,
    ) {
        return false;
    }
    let now = Instant::now();
    let timer_values = active_timer(s, now).map(|timer| timer_scene_values(timer, now));
    let timer = timer_values
        .as_ref()
        .map(|(label, remaining, progress)| TimerScene {
            label,
            remaining_text: remaining,
            progress: *progress,
        });
    let model = label_model(
        &s.presentation.active_label_text,
        s.presentation.active_label_class.as_deref(),
        s.presentation.active_label_number,
        s.presentation.active_label_color,
    );
    let scene = ActiveLabelScene {
        canvas,
        label: LabelScene {
            model,
            style: LabelStyle::new(s.layout.dpi_scale, s.presentation.label_height),
            theme: &s.presentation.label_theme,
            alpha: if s.presentation.active_label_hovered {
                s.presentation.label_alpha / 2
            } else {
                s.presentation.label_alpha
            },
        },
        timer,
    };
    match s
        .presentation
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_active_label(s.presentation.active_label_hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.presentation.active_label_hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition active-label render failed: {error}"
            ));
            retain_redraw_request(s.presentation.active_label_hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) unsafe fn render_pip_surface(s: &mut OverlayState, pip_index: usize) {
    let Some(pip) = s.presentation.pip_windows.get(pip_index) else {
        return;
    };
    let Some(canvas) = client_scene_rect(pip.label_hwnd) else {
        return;
    };
    let _ = render_pip_surface_for_size(s, pip_index, canvas.width(), canvas.height());
}

pub(super) unsafe fn render_pip_surface_for_size(
    s: &mut OverlayState,
    pip_index: usize,
    width: i32,
    height: i32,
) -> bool {
    let Some(pip) = s.presentation.pip_windows.get(pip_index) else {
        return false;
    };
    let hwnd = pip.label_hwnd;
    let canvas = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(s, hwnd, canvas.width(), canvas.height(), 1.0) {
        return false;
    }
    let pip = &s.presentation.pip_windows[pip_index];
    let now = Instant::now();
    let source_id = format!("pid:{}", pip.pid);
    let timer_values = s
        .timers
        .visible_for(Some(&source_id), now)
        .map(|timer| timer_scene_values(timer, now));
    let timer = timer_values
        .as_ref()
        .map(|(label, remaining, progress)| TimerScene {
            label,
            remaining_text: remaining,
            progress: *progress,
        });
    let now_ms = windows::Win32::System::SystemInformation::GetTickCount64();
    let notification = s
        .notification_center
        .entries
        .get(&pip.pid)
        .map(|notification| {
            notification.visual_snapshot(now_ms, s.notification_center.animations_enabled)
        });
    let reorder_dragging = s
        .interaction
        .reorder_drag
        .as_ref()
        .is_some_and(|drag| drag.dragging);
    let drag_source = reorder_dragging
        && s.interaction
            .reorder_drag
            .as_ref()
            .map(|drag| drag.from_index)
            == Some(pip_index);
    let drop_target = reorder_dragging
        && s.interaction.drop_target == Some(pip_index)
        && s.interaction
            .reorder_drag
            .as_ref()
            .map(|drag| drag.from_index)
            != Some(pip_index);
    let model = label_model(
        &pip.label,
        pip.class.as_deref(),
        pip.number,
        color_for_number(pip.number),
    );
    let scene = PipScene {
        canvas,
        border_width: scale(BORDER_WIDTH, s.layout.dpi_scale),
        scale: s.layout.dpi_scale,
        label: LabelScene {
            model,
            style: LabelStyle::new(s.layout.dpi_scale, s.presentation.label_height),
            theme: &s.presentation.label_theme,
            alpha: s.presentation.label_alpha,
        },
        timer,
        notification,
        interaction: PipInteractionScene {
            hovered: pip.hovered,
            edit_mode: s.interaction.edit_mode,
            reorder_dragging,
            drag_source,
            drop_target,
        },
    };
    match s
        .presentation
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_pip_scene(hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition PiP scene render failed: {error}"
            ));
            retain_redraw_request(hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) fn input_indicator_text() -> Option<&'static str> {
    use crate::broadcast::MouseClutchStatus;
    match (
        crate::broadcast::is_active(),
        crate::broadcast::mouse_clutch_status(),
    ) {
        (false, MouseClutchStatus::Inactive) => None,
        (true, MouseClutchStatus::Inactive) => Some("Broadcasting"),
        (false, MouseClutchStatus::Active) => Some("Mouse Clutch"),
        (false, MouseClutchStatus::Releasing) => Some("Mouse Clutch · Releasing"),
        (true, MouseClutchStatus::Active) => Some("Broadcasting · Mouse Clutch"),
        (true, MouseClutchStatus::Releasing) => Some("Broadcasting · Mouse Clutch releasing"),
    }
}

pub(super) fn input_indicator_background() -> Color {
    let value = if crate::broadcast::is_active() {
        0x002030CC
    } else {
        match crate::broadcast::mouse_clutch_status() {
            crate::broadcast::MouseClutchStatus::Inactive => 0x002030CC,
            crate::broadcast::MouseClutchStatus::Active => 0x00906A28,
            crate::broadcast::MouseClutchStatus::Releasing => 0x002080C8,
        }
    };
    Color::from_colorref(value)
}

pub(super) unsafe fn render_banner_surface(s: &mut OverlayState) {
    let Some(bounds) = client_scene_rect(s.presentation.broadcast_label_hwnd) else {
        return;
    };
    let _ = render_banner_surface_for_size(s, bounds.width(), bounds.height());
}

pub(super) unsafe fn render_banner_surface_for_size(
    s: &mut OverlayState,
    width: i32,
    height: i32,
) -> bool {
    let Some(text) = input_indicator_text() else {
        return false;
    };
    let bounds = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(
        s,
        s.presentation.broadcast_label_hwnd,
        bounds.width(),
        bounds.height(),
        1.0,
    ) {
        return false;
    }
    let scene = StatusBannerScene {
        bounds,
        text,
        background: input_indicator_background(),
        alpha: s.presentation.label_alpha,
        scale: s.layout.dpi_scale,
        logical_label_height: s.presentation.label_height,
    };
    match s
        .presentation
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_status_banner(s.presentation.broadcast_label_hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.presentation.broadcast_label_hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition status-banner render failed: {error}"
            ));
            retain_redraw_request(s.presentation.broadcast_label_hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) unsafe fn render_stonemite_button_surface(s: &mut OverlayState) {
    let Some(bounds) = client_scene_rect(s.presentation.stonemite_button.hwnd) else {
        return;
    };
    let _ = render_stonemite_button_surface_for_size(s, bounds.width(), bounds.height());
}

pub(super) unsafe fn render_stonemite_button_surface_for_size(
    s: &mut OverlayState,
    width: i32,
    height: i32,
) -> bool {
    let bounds = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(
        s,
        s.presentation.stonemite_button.hwnd,
        bounds.width(),
        bounds.height(),
        1.0,
    ) {
        return false;
    }
    let icon_size = scale(52, s.layout.dpi_scale)
        .min(bounds.width())
        .min(bounds.height())
        .max(1);
    let icon_left = (bounds.width() - icon_size) / 2;
    let icon_top = (bounds.height() - icon_size) / 2;
    let scene = StonemiteButtonScene {
        bounds,
        icon_bounds: Rect::new(
            icon_left,
            icon_top,
            icon_left + icon_size,
            icon_top + icon_size,
        ),
        hovered: s.presentation.stonemite_button.hovered,
        pressed: s.presentation.stonemite_button.pressed,
    };
    match s
        .presentation
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_stonemite_button(s.presentation.stonemite_button.hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.presentation.stonemite_button.hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition Stonemite-button render failed: {error}"
            ));
            retain_redraw_request(s.presentation.stonemite_button.hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) unsafe fn render_toast_surface(s: &mut OverlayState) {
    s.presentation.toast.scene_ready = false;
    let Some(bounds) = client_scene_rect(s.presentation.toast.hwnd) else {
        suppress_toast_publication(s);
        return;
    };
    let _ = render_toast_surface_for_size(s, bounds.width(), bounds.height());
}

pub(super) unsafe fn render_toast_surface_for_size(
    s: &mut OverlayState,
    width: i32,
    height: i32,
) -> bool {
    // An attachment left by an older toast cannot make this staged scene
    // publishable. Only this render's successful Present1 restores readiness.
    s.presentation.toast.scene_ready = false;
    let bounds = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(
        s,
        s.presentation.toast.hwnd,
        bounds.width(),
        bounds.height(),
        1.0,
    ) {
        return false;
    }
    let scene = ToastScene {
        bounds,
        text: &s.presentation.toast.text,
        background: Color::from_colorref(TOAST_BG_COLOR),
        alpha: s.presentation.toast.alpha,
        scale: s.layout.dpi_scale,
        logical_height: s.presentation.toast.height,
    };
    match s
        .presentation
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_toast(s.presentation.toast.hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.presentation.toast.hwnd);
            s.presentation.toast.scene_ready = true;
            true
        }
        Err(error) => {
            debug_log(&format!("DirectComposition toast render failed: {error}"));
            retain_redraw_request(s.presentation.toast.hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

pub(super) unsafe fn validate_composition_paint(hwnd: HWND) {
    let _ = ValidateRect(hwnd, None);
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

pub(super) fn overlay_visibility_policy(
    hidden_by_user: bool,
    has_pip: bool,
    context_menu_open: bool,
    foreground_is_eq_or_ours: bool,
) -> bool {
    !hidden_by_user && has_pip && (context_menu_open || foreground_is_eq_or_ours)
}

pub(super) unsafe fn overlay_visibility_allowed(s: &OverlayState) -> bool {
    let has_pip = !s.clients.pips().is_empty();
    let foreground_matches = if s.hidden_by_user || !has_pip {
        false
    } else {
        owns_foreground(GetForegroundWindow(), s)
    };
    overlay_visibility_policy(
        s.hidden_by_user,
        has_pip,
        s.interaction.context_menu_open,
        foreground_matches,
    )
}

unsafe fn apply_one_pip_pair_visibility(s: &OverlayState, pip: &PipWindowEntry, visible: bool) {
    if visible && surface_is_ready(s, pip.label_hwnd) {
        let _ = ShowWindow(pip.hwnd, SW_SHOWNOACTIVATE);
        let _ = ShowWindow(pip.label_hwnd, SW_SHOWNOACTIVATE);
        // Interactions with a PiP can promote it above its sibling.
        let _ = SetWindowPos(
            pip.label_hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    } else {
        let _ = ShowWindow(pip.hwnd, SW_HIDE);
        let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
        if visible {
            request_redraw(pip.label_hwnd);
        }
    }
}

pub(super) unsafe fn apply_pip_pair_visibility(s: &OverlayState, visible: bool) {
    for pip in &s.presentation.pip_windows {
        apply_one_pip_pair_visibility(s, pip, visible);
    }
}

unsafe fn apply_stonemite_button_visibility(s: &OverlayState) {
    let button = &s.presentation.stonemite_button;
    let allowed = super::in_game_button::visibility_policy(
        button.enabled,
        !s.clients.windows.is_empty(),
        s.hidden_by_user,
        button.menu_open,
        owns_foreground(GetForegroundWindow(), s),
    );
    if allowed && surface_is_ready(s, button.hwnd) {
        let _ = ShowWindow(button.hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            button.hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    } else {
        let _ = ShowWindow(button.hwnd, SW_HIDE);
        if allowed {
            request_redraw(button.hwnd);
        }
    }
}

pub(super) unsafe fn update_visibility(s: &mut OverlayState) {
    if overlay_visibility_allowed(s) {
        // Publish any state accumulated while hidden before exposing a visual.
        for index in 0..s.presentation.pip_windows.len() {
            let hwnd = s.presentation.pip_windows[index].label_hwnd;
            if has_redraw_request(hwnd) {
                render_pip_surface(s, index);
            }
        }
        if has_redraw_request(s.presentation.active_label_hwnd) {
            render_active_label_surface(s);
        }
        if has_redraw_request(s.presentation.broadcast_label_hwnd) {
            render_banner_surface(s);
        }

        // A thumbnail host and its full-host authored sibling are one visible
        // pair. DWM remains registered while both wait for a complete frame.
        apply_pip_pair_visibility(s, true);
        if !s.presentation.active_label_text.is_empty()
            && surface_is_ready(s, s.presentation.active_label_hwnd)
        {
            let _ = ShowWindow(s.presentation.active_label_hwnd, SW_SHOWNOACTIVATE);
        }
        if input_indicator_text().is_some()
            && surface_is_ready(s, s.presentation.broadcast_label_hwnd)
        {
            let _ = ShowWindow(s.presentation.broadcast_label_hwnd, SW_SHOWNOACTIVATE);
        } else {
            let _ = ShowWindow(s.presentation.broadcast_label_hwnd, SW_HIDE);
        }
    } else {
        for pw in &mut s.presentation.pip_windows {
            if std::mem::take(&mut pw.hovered) {
                request_redraw(pw.label_hwnd);
            }
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
            let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
        }
        if std::mem::take(&mut s.presentation.active_label_hovered) {
            let alpha = s.presentation.label_alpha;
            set_composition_opacity(s, s.presentation.active_label_hwnd, alpha);
        }
        let _ = ShowWindow(s.presentation.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.presentation.broadcast_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.presentation.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.presentation.toast.hwnd, TIMER_TOAST_FADE);
        s.presentation.toast.phase = ToastPhase::Hidden;
        s.presentation.toast.scene_ready = false;
    }
    apply_stonemite_button_visibility(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_policy_requires_user_client_and_foreground_permission() {
        assert!(overlay_visibility_policy(false, true, false, true));
        assert!(overlay_visibility_policy(false, true, true, false));
        assert!(!overlay_visibility_policy(true, true, true, true));
        assert!(!overlay_visibility_policy(false, false, true, true));
        assert!(!overlay_visibility_policy(false, true, false, false));
    }
}
