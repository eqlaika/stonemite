use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::appearance::{
    configured_label_theme, opacity_percent_to_alpha, DEFAULT_LABEL_HEIGHT, DEFAULT_LABEL_OPACITY,
    LABEL_COLORS,
};
use super::casting::{self, CastingCenter};
use super::client_controller::foreground_event_proc;
use super::clients::{apply_preferred_box_order, ClientRegistry};
use super::combat_awareness::{self, CombatAwarenessCenter};
use super::dps_overlay::{self, DpsOverlayController};
use super::geometry::{client_animations_enabled, dpi_scale};
use super::hosts::rebuild_thumbnails;
use super::in_game_button::{create_tooltip, wnd_proc as stonemite_button_wnd_proc};
use super::interaction::InteractionState;
use super::layout::LayoutState;
use super::menu::menu_owner_wnd_proc;
use super::notifications::{self, NotificationCenter};
use super::pip_interaction::pip_wnd_proc;
use super::pip_transition::force_finish as finish_pip_transition;
use super::presentation::{ComApartment, PresentationState, StonemiteButtonState};
use super::render::Compositor;
use super::runtime::{self, try_with_state_mut};
use super::state::OverlayState;
use super::surfaces::{suppress_toast_publication, update_visibility};
use super::telemetry::TelemetryState;
use super::timer_controller::TIMER_ID as TIMER_OVERLAY_TICK;
use super::timers::TimerOverlayState;
use super::toast::{
    Phase as ToastPhase, ToastState, DEFAULT_DURATION_MS as DEFAULT_TOAST_DURATION_MS,
    DEFAULT_HEIGHT as DEFAULT_TOAST_HEIGHT, TIMER_ID as TIMER_TOAST_FADE,
};
use super::toast_controller::toast_wnd_proc;
use super::window_procs::{broadcast_label_wnd_proc, label_wnd_proc, pip_label_wnd_proc};
use super::window_styles::{self, WindowStyleState};
use crate::config;
use crate::diagnostics::debug_log;

pub(super) unsafe fn init_inner() -> HWND {
    runtime::initialize(|| unsafe { initialize_state() })
        .expect("overlay runtime is already initialized or busy")
}

unsafe fn initialize_state() -> (OverlayState, HWND) {
    // Register per-PiP window class.
    let pip_class = w!("StonemitePipClass");
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(pip_wnd_proc),
        lpszClassName: pip_class,
        hCursor: cursor,
        style: CS_DBLCLKS,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Register PiP label overlay window class.
    let pip_label_class = w!("StonemitePipLabelClass");
    let pip_label_wc = WNDCLASSW {
        lpfnWndProc: Some(pip_label_wnd_proc),
        lpszClassName: pip_label_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&pip_label_wc);

    // Register a stable, activatable owner for context menus. Popup tracking
    // must not borrow a transient PiP HWND that can be rebuilt while open.
    let menu_owner_class = w!("StonemiteMenuOwnerClass");
    let menu_owner_wc = WNDCLASSW {
        lpfnWndProc: Some(menu_owner_wnd_proc),
        lpszClassName: menu_owner_class,
        ..Default::default()
    };
    RegisterClassW(&menu_owner_wc);
    let menu_owner_hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        menu_owner_class,
        w!("StonemiteMenuOwner"),
        Default::default(),
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create context-menu owner window");

    // Register the activatable in-game button. Taking foreground before the
    // button-up message prevents EQ's foreground-only DirectInput from seeing
    // the same physical click.
    let stonemite_button_class = w!("StonemiteInGameButtonClass");
    let stonemite_button_wc = WNDCLASSW {
        lpfnWndProc: Some(stonemite_button_wnd_proc),
        lpszClassName: stonemite_button_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&stonemite_button_wc);
    let stonemite_button_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
        stonemite_button_class,
        w!("Stonemite"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create in-game Stonemite button");
    let stonemite_tooltip_hwnd = create_tooltip(stonemite_button_hwnd);

    // Register label window class.
    let label_class = w!("StonemiteLabelClass");
    let label_wc = WNDCLASSW {
        lpfnWndProc: Some(label_wnd_proc),
        lpszClassName: label_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&label_wc);

    let label_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
        label_class,
        w!("StonemiteLabel"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create label window");

    let cfg = config::Config::load();
    let label_opacity = cfg
        .pip_label_opacity
        .unwrap_or(DEFAULT_LABEL_OPACITY)
        .min(100);
    let label_alpha = opacity_percent_to_alpha(label_opacity);

    // Register broadcast banner window class.
    let bc_class = w!("StonemiteBroadcastClass");
    let bc_wc = WNDCLASSW {
        lpfnWndProc: Some(broadcast_label_wnd_proc),
        lpszClassName: bc_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&bc_wc);

    let bc_hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
        bc_class,
        w!("StonemiteBroadcast"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create broadcast label window");

    // Register the passive, topmost DPS panel. Normal hit testing is
    // click-through; edit mode temporarily removes WS_EX_TRANSPARENT.
    let dps_class = w!("StonemiteDpsOverlayClass");
    let dps_wc = WNDCLASSW {
        lpfnWndProc: Some(dps_overlay::wnd_proc),
        lpszClassName: dps_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&dps_wc);
    let dps_hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
        dps_class,
        w!("Stonemite DPS"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create DPS overlay window");

    // Register toast notification window class.
    let toast_class = w!("StonemiteToastClass");
    let toast_wc = WNDCLASSW {
        lpfnWndProc: Some(toast_wnd_proc),
        lpszClassName: toast_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&toast_wc);

    let toast_hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
        toast_class,
        w!("StonemiteToast"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create toast window");

    let hook = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        None,
        Some(foreground_event_proc),
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    );

    let label_height = cfg
        .pip_label_height
        .map(|v| v as i32)
        .unwrap_or(DEFAULT_LABEL_HEIGHT);
    let label_theme = configured_label_theme(&cfg);
    let layout = LayoutState::new(&cfg, dpi_scale(label_hwnd));
    let com_apartment = ComApartment::initialize();
    let compositor = if com_apartment.usable {
        match Compositor::new() {
            Ok(compositor) => Some(compositor),
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition compositor initialization failed: {error}"
                ));
                None
            }
        }
    } else {
        None
    };

    let state = OverlayState {
        presentation: PresentationState {
            compositor,
            com_apartment,
            pip_windows: Vec::new(),
            pip_transition: None,
            pending_composition_destroys: Vec::new(),
            menu_owner_hwnd,
            stonemite_button: StonemiteButtonState {
                hwnd: stonemite_button_hwnd,
                tooltip_hwnd: stonemite_tooltip_hwnd,
                enabled: cfg.show_stonemite_button,
                position: cfg.stonemite_button_position,
                drag: None,
                hovered: false,
                pressed: false,
                releasing_capture: false,
                menu_open: false,
            },
            active_label_hwnd: label_hwnd,
            active_label_text: String::new(),
            active_label_class: None,
            active_label_color: LABEL_COLORS[0],
            active_label_number: 0,
            active_label_hovered: false,
            active_scene_key: None,
            banner_scene_key: None,
            broadcast_label_hwnd: bc_hwnd,
            dps_hwnd,
            dps_scene_ready: false,
            label_height,
            label_alpha,
            label_theme,
            thumbnail_alpha: opacity_percent_to_alpha(cfg.effective_pip_opacity()),
            toast: ToastState {
                hwnd: toast_hwnd,
                text: String::new(),
                phase: ToastPhase::Hidden,
                scene_ready: false,
                alpha: 0,
                phase_start: 0,
                duration_ms: cfg
                    .toast_duration
                    .map(|d| (d * 1000.0) as u32)
                    .unwrap_or(DEFAULT_TOAST_DURATION_MS),
                height: cfg
                    .toast_height
                    .map(|h| h as i32)
                    .unwrap_or(DEFAULT_TOAST_HEIGHT),
                enabled: cfg.toast_enabled,
            },
        },
        clients: ClientRegistry::new(cfg.box_order.clone(), cfg.box_cycles.clone()),
        event_hook: hook,
        layout,
        hidden_by_user: false,
        interaction: InteractionState::new(),
        window_styles: WindowStyleState::new(cfg.hide_from_alt_tab),
        telemetry: TelemetryState::new(&cfg),
        casting: CastingCenter::new(&cfg, client_animations_enabled()),
        combat_awareness: CombatAwarenessCenter::new(&cfg, client_animations_enabled()),
        dps: DpsOverlayController::new(&cfg),
        notification_center: NotificationCenter::new(&cfg, client_animations_enabled()),
        timers: TimerOverlayState::default(),
    };

    (state, label_hwnd)
}

/// Reload config into overlay state and rebuild the layout.
pub(super) fn force_rebuild() {
    let _ = try_with_state_mut(|s| unsafe {
        let cfg = config::Config::load();
        s.layout.pip_edge = cfg.pip_edge;
        s.presentation.stonemite_button.enabled = cfg.show_stonemite_button;
        s.presentation.stonemite_button.position = cfg.stonemite_button_position;
        s.clients.preferred_order = cfg.box_order.clone();
        s.clients.box_cycles = cfg.box_cycles.clone();
        apply_preferred_box_order(&mut s.clients.windows, &s.clients.preferred_order);
        if cfg.auto_order {
            s.clients.apply_auto_order();
        }
        s.layout.custom_strip_width = cfg.pip_strip_width.map(|v| v as i32);
        s.presentation.thumbnail_alpha = opacity_percent_to_alpha(cfg.effective_pip_opacity());
        s.layout.has_custom_positions = !cfg.pip_positions.is_empty();
        s.layout.snap_grid = cfg.snap_grid as i32;
        s.presentation.label_height = cfg
            .pip_label_height
            .map(|v| v as i32)
            .unwrap_or(DEFAULT_LABEL_HEIGHT);
        let opacity = cfg
            .pip_label_opacity
            .unwrap_or(DEFAULT_LABEL_OPACITY)
            .min(100);
        s.presentation.label_alpha = opacity_percent_to_alpha(opacity);
        s.presentation.label_theme = configured_label_theme(&cfg);
        s.dps.apply_config(&cfg);
        // Handle hide_from_alt_tab setting change.
        let was_hiding_background = s.window_styles.hide_background();
        s.window_styles.set_hide_background(cfg.hide_from_alt_tab);
        if was_hiding_background && !s.window_styles.hide_background() {
            s.window_styles.restore_all(&s.clients);
        } else if s.window_styles.hide_background() {
            s.window_styles.apply(&s.clients);
        }
        // Reload notification and combat-awareness config plus the EQ profile resolver.
        let notification_selection_changed = s
            .notification_center
            .apply_config(&cfg, client_animations_enabled());
        s.casting.apply_config(&cfg, client_animations_enabled());
        let _ = KillTimer(s.presentation.active_label_hwnd, casting::TIMER_ID);
        let combat_appearance_changed = s
            .combat_awareness
            .apply_config(&cfg, client_animations_enabled());
        if combat_appearance_changed && !cfg.combat_awareness_enabled {
            let _ = KillTimer(s.presentation.active_label_hwnd, combat_awareness::TIMER_ID);
        }
        s.telemetry.set_eq_dir(&cfg);
        if !s.notification_center.visual_enabled || notification_selection_changed {
            s.notification_center.entries.clear();
            let _ = KillTimer(s.presentation.active_label_hwnd, notifications::TIMER_ID);
        }
        // Reload toast config and withdraw an active toast immediately when disabled.
        let toast_was_enabled = s.presentation.toast.enabled;
        s.presentation.toast.enabled = cfg.toast_enabled;
        if toast_was_enabled && !s.presentation.toast.enabled {
            s.presentation.toast.phase = ToastPhase::Hidden;
            suppress_toast_publication(s);
        }
        s.presentation.toast.height = cfg
            .toast_height
            .map(|h| h as i32)
            .unwrap_or(DEFAULT_TOAST_HEIGHT);
        s.presentation.toast.duration_ms = cfg
            .toast_duration
            .map(|d| (d * 1000.0) as u32)
            .unwrap_or(DEFAULT_TOAST_DURATION_MS);
        rebuild_thumbnails(s);
        dps_overlay::reconcile(s);
        update_visibility(s);
    });
}

pub(super) fn cleanup() {
    let _ = runtime::shutdown(|mut state| unsafe {
        let s = &mut state;
        s.casting.save();
        // Restore original ex styles on all EQ windows before shutting down.
        s.window_styles.restore_all(&s.clients);
        if !window_styles::flush() {
            debug_log("timed out restoring EQ Alt-Tab window styles during shutdown");
        }
        if !s.event_hook.is_invalid() {
            let _ = UnhookWinEvent(s.event_hook);
        }
        finish_pip_transition(s);
        let pips = std::mem::take(&mut s.presentation.pip_windows);
        let pending_composition_destroys =
            std::mem::take(&mut s.presentation.pending_composition_destroys);
        if let Some(mut compositor) = s.presentation.compositor.take() {
            for pip in &pips {
                if let Err(error) = compositor.unregister_surface(pip.label_hwnd) {
                    debug_log(&format!("DirectComposition PiP cleanup failed: {error}"));
                }
            }
            for hwnd in pending_composition_destroys.iter().copied().chain([
                s.presentation.stonemite_button.hwnd,
                s.presentation.active_label_hwnd,
                s.presentation.broadcast_label_hwnd,
                s.presentation.dps_hwnd,
                s.presentation.toast.hwnd,
            ]) {
                if let Err(error) = compositor.unregister_surface(hwnd) {
                    debug_log(&format!("DirectComposition cleanup failed: {error}"));
                }
            }
            // shutdown detaches any registration whose individual commit
            // failed before its owning HWND is destroyed below.
            if let Err(error) = compositor.shutdown() {
                debug_log(&format!("DirectComposition shutdown failed: {error}"));
            }
        }
        for pw in pips {
            let _ = DwmUnregisterThumbnail(pw.thumb);
            let _ = DestroyWindow(pw.label_hwnd);
            let _ = DestroyWindow(pw.hwnd);
        }
        for hwnd in pending_composition_destroys {
            let _ = DestroyWindow(hwnd);
        }
        let _ = KillTimer(s.presentation.active_label_hwnd, notifications::TIMER_ID);
        let _ = KillTimer(s.presentation.active_label_hwnd, combat_awareness::TIMER_ID);
        let _ = KillTimer(s.presentation.active_label_hwnd, TIMER_OVERLAY_TICK);
        let _ = DestroyWindow(s.presentation.menu_owner_hwnd);
        if !s.presentation.stonemite_button.tooltip_hwnd.is_invalid() {
            let _ = DestroyWindow(s.presentation.stonemite_button.tooltip_hwnd);
        }
        let _ = DestroyWindow(s.presentation.stonemite_button.hwnd);
        let _ = DestroyWindow(s.presentation.active_label_hwnd);
        let _ = DestroyWindow(s.presentation.broadcast_label_hwnd);
        let _ = DestroyWindow(s.presentation.dps_hwnd);
        let _ = KillTimer(s.presentation.toast.hwnd, TIMER_TOAST_FADE);
        let _ = DestroyWindow(s.presentation.toast.hwnd);
    });
}
