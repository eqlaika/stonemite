use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::geometry::scale;
use super::runtime::{is_busy, try_with_state_mut};
use super::scenes::UiTextRole;
use super::state::OverlayState;
use super::surfaces::{
    ensure_compositor, overlay_visibility_allowed, position_window_if_changed,
    render_toast_surface, render_toast_surface_for_size, set_composition_opacity,
    suppress_toast_publication, surface_is_ready, take_redraw_request, validate_composition_paint,
};
use super::toast::{
    advance_fade as advance_toast_fade, publication_allowed as toast_publication_allowed,
    FadeEffect as ToastFadeEffect, Phase as ToastPhase, FADE_STEP_MS as TOAST_FADE_STEP_MS,
    TIMER_ID as TIMER_TOAST_FADE,
};
use crate::diagnostics::debug_log;

pub(super) unsafe extern "system" fn toast_wnd_proc(
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
                        render_toast_surface(state);
                        if !toast_publication_allowed(
                            state.presentation.toast.phase,
                            state.presentation.toast.scene_ready,
                            overlay_visibility_allowed(state),
                            surface_is_ready(state, hwnd),
                        ) {
                            suppress_toast_publication(state);
                        }
                    }
                });
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_TOAST_FADE => {
            if try_with_state_mut(|state| unsafe { advance_toast(state, hwnd) }).is_err() {
                let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn advance_toast(s: &mut OverlayState, hwnd: HWND) {
    if !toast_publication_allowed(
        s.presentation.toast.phase,
        s.presentation.toast.scene_ready,
        overlay_visibility_allowed(s),
        surface_is_ready(s, hwnd),
    ) {
        s.presentation.toast.phase = ToastPhase::Hidden;
        suppress_toast_publication(s);
        return;
    }
    let now = windows::Win32::System::SystemInformation::GetTickCount64();
    let transition = advance_toast_fade(
        s.presentation.toast.phase,
        s.presentation.toast.alpha,
        s.presentation.toast.phase_start,
        s.presentation.toast.duration_ms,
        now,
    );
    s.presentation.toast.phase = transition.phase;
    s.presentation.toast.alpha = transition.alpha;
    s.presentation.toast.phase_start = transition.phase_start;
    match transition.effect {
        ToastFadeEffect::None => {}
        ToastFadeEffect::UpdateOpacity(alpha) => {
            set_composition_opacity(s, hwnd, alpha);
        }
        ToastFadeEffect::HideAndStop => {
            s.presentation.toast.scene_ready = false;
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
        }
    }
}

pub(super) unsafe fn show_toast_inner(s: &mut OverlayState, text: &str) {
    if !s.presentation.toast.enabled || !overlay_visibility_allowed(s) {
        return;
    }

    // Complete every fallible model/geometry preparation step while the
    // currently published toast remains authoritative.
    let active_hwnd = s
        .clients
        .active_pid()
        .and_then(|pid| s.clients.windows.iter().find(|w| w.pid == pid))
        .map(|w| w.hwnd);
    let Some(eq_hwnd) = active_hwnd else {
        return;
    };

    let mut eq_rect = RECT::default();
    if GetClientRect(eq_hwnd, &mut eq_rect).is_err() {
        return;
    }
    let mut top_left = POINT {
        x: eq_rect.left,
        y: eq_rect.top,
    };
    if !ClientToScreen(eq_hwnd, &mut top_left).as_bool() {
        return;
    }

    let d = s.layout.dpi_scale;
    let toast_h = scale(s.presentation.toast.height, d);
    if !ensure_compositor(s) {
        return;
    }
    let text_width = match s
        .presentation
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .measure_text(
            text,
            &UiTextRole::Toast.font(),
            UiTextRole::Toast.height(d, (s.presentation.toast.height - 12).max(12)),
        ) {
        Ok(width) => width,
        Err(error) => {
            debug_log(&format!("DirectWrite toast measurement failed: {error}"));
            return;
        }
    };
    let pad = scale(20, d);
    let toast_w = text_width + pad * 2;
    let eq_client_w = eq_rect.right - eq_rect.left;
    let toast_x = top_left.x + (eq_client_w - toast_w) / 2;
    let eq_client_h = eq_rect.bottom - eq_rect.top;
    let toast_y = top_left.y + eq_client_h / 3;

    // Replacement is fail-closed: no older attachment may remain visible once
    // the new toast starts staging.
    let _ = ShowWindow(s.presentation.toast.hwnd, SW_HIDE);
    let _ = KillTimer(s.presentation.toast.hwnd, TIMER_TOAST_FADE);
    s.presentation.toast.phase = ToastPhase::Hidden;
    s.presentation.toast.scene_ready = false;
    if !position_window_if_changed(
        s.presentation.toast.hwnd,
        HWND_TOPMOST,
        toast_x,
        toast_y,
        toast_w,
        toast_h,
    ) {
        return;
    }

    s.presentation.toast.text = text.to_string();
    s.presentation.toast.alpha = 0;
    s.presentation.toast.phase_start = 0;
    if !render_toast_surface_for_size(s, toast_w, toast_h) {
        s.presentation.toast.scene_ready = false;
        suppress_toast_publication(s);
        return;
    }

    // Foreground hooks can run while DirectComposition pumps Win32 messages.
    // Publish only this successfully presented scene and only while it is
    // still valid for the current overlay visibility policy.
    if !overlay_visibility_allowed(s) {
        s.presentation.toast.phase = ToastPhase::Hidden;
        suppress_toast_publication(s);
        return;
    }
    s.presentation.toast.phase = ToastPhase::FadingIn;
    s.presentation.toast.phase_start = windows::Win32::System::SystemInformation::GetTickCount64();
    if !toast_publication_allowed(
        s.presentation.toast.phase,
        s.presentation.toast.scene_ready,
        true,
        surface_is_ready(s, s.presentation.toast.hwnd),
    ) {
        s.presentation.toast.phase = ToastPhase::Hidden;
        suppress_toast_publication(s);
        return;
    }
    let _ = ShowWindow(s.presentation.toast.hwnd, SW_SHOWNOACTIVATE);
    let _ = SetTimer(
        s.presentation.toast.hwnd,
        TIMER_TOAST_FADE,
        TOAST_FADE_STEP_MS,
        None,
    );
}
