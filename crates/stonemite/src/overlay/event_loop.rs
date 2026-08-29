use std::collections::HashSet;
use std::time::Instant;

use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, KillTimer, SetTimer};

use super::activation::target_has_keyboard_focus;
use super::casting;
use super::clients::{
    apply_preferred_box_order, focused_foreground_pid, next_available_number, MAX_PIPS,
};
use super::combat_awareness;
use super::control_bridge::{publish, sync_mouse_eligibility};
use super::geometry::dpi_scale;
use super::hosts::{rebuild_thumbnails, update_active_label};
use super::notifications;
use super::runtime::{try_with_state, try_with_state_mut, AccessError};
use super::state::OverlayState;
use super::surfaces::{service_compositor_recovery, update_visibility};
use super::telemetry;
use super::timer_controller::{
    invalidate_labels, INTERVAL_MS as TIMER_OVERLAY_INTERVAL_MS, TIMER_ID as TIMER_OVERLAY_TICK,
};
use super::toast_controller::show_toast_inner;
use crate::diagnostics::debug_log;
use crate::eq_windows::EqWindow;
use crate::{config, eq_windows, log_watcher};

// ---------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------

fn should_disable_broadcast_when_clients_exit(
    setting_enabled: bool,
    previous_client_count: usize,
    current_client_count: usize,
) -> bool {
    setting_enabled && previous_client_count > 0 && current_client_count == 0
}

pub(super) fn poll() {
    let _ = try_with_state_mut(|state| unsafe { poll_inner(state) });
}

unsafe fn poll_inner(s: &mut OverlayState) {
    let new_windows = eq_windows::find_eq_windows();
    let old_pids: HashSet<u32> = s.clients.windows.iter().map(|w| w.pid).collect();
    let new_pids: HashSet<u32> = new_windows.iter().map(|w| w.pid).collect();

    if old_pids == new_pids {
        let mut hwnd_changed = false;
        for nw in &new_windows {
            if let Some(ow) = s.clients.windows.iter_mut().find(|w| w.pid == nw.pid) {
                if ow.hwnd != nw.hwnd {
                    ow.hwnd = nw.hwnd;
                    hwnd_changed = true;
                }
            }
        }
        // A foreground WinEvent can be suppressed while an overlay transaction
        // is guarded. Reconcile on every poll so that skipped callbacks cannot
        // leave the active/PiP partition stale indefinitely.
        let foreground_pid =
            focused_foreground_pid(&s.clients.windows, GetForegroundWindow(), |hwnd| {
                target_has_keyboard_focus(hwnd)
            });
        let foreground_changed = foreground_pid.is_some_and(|pid| {
            let changed = s.clients.promote(pid, MAX_PIPS);
            if changed && config::Config::load().auto_order {
                s.clients.apply_auto_order();
            }
            crate::broadcast::set_active_pid(pid);
            sync_mouse_eligibility(s);
            s.notification_center.acknowledge(pid);
            changed
        });
        // Poll trusik shared memory for character names.
        if s.telemetry.trusik_enabled {
            trusik_poll_characters(s);
        }
        // Publish identity changes to the event-driven log worker. No log
        // filesystem I/O occurs on the UI polling path.
        publish_log_sources_inner(s);
        // Update broadcast targets and the strict identical-geometry Mouse Clutch set.
        let pids: Vec<u32> = s.clients.windows.iter().map(|w| w.pid).collect();
        crate::broadcast::update_targets(&pids, s.clients.active_pid());
        sync_mouse_eligibility(s);
        // Re-derive DPI from the EQ window; if it changed (e.g. monitor
        // reconnect moved EQ to a different-DPI display), rebuild everything.
        // Also rebuild if any HWND changed (e.g. EQ recreated its window
        // during login), since DWM thumbnails are bound to specific HWNDs.
        let reference = s
            .clients
            .active_pid()
            .and_then(|pid| s.clients.windows.iter().find(|window| window.pid == pid))
            .or_else(|| s.clients.windows.first())
            .map(|window| window.hwnd);
        let dpi_hwnd = reference.unwrap_or(s.presentation.active_label_hwnd);
        let new_dpi = dpi_scale(dpi_hwnd);
        let new_monitor_rect = eq_windows::get_monitor_work_area(reference);
        let monitor_changed = new_monitor_rect.left != s.layout.monitor_rect.left
            || new_monitor_rect.top != s.layout.monitor_rect.top
            || new_monitor_rect.right != s.layout.monitor_rect.right
            || new_monitor_rect.bottom != s.layout.monitor_rect.bottom;
        let presentation_incomplete = s.presentation.pip_transition.is_some()
            || s.presentation.pip_windows.iter().map(|pip| pip.pid).ne(s
                .clients
                .pips()
                .iter()
                .copied())
            || s.presentation.pip_windows.iter().any(|pip| pip.thumb == 0);
        if foreground_changed
            || hwnd_changed
            || presentation_incomplete
            || monitor_changed
            || (new_dpi - s.layout.dpi_scale).abs() > 0.001
        {
            s.layout.dpi_scale = new_dpi;
            rebuild_thumbnails(s);
        } else {
            update_active_label(s);
            super::in_game_button::update_layout(s);
        }
        s.window_styles.apply(&s.clients);
        super::dps_overlay::reconcile(s);
        service_compositor_recovery(s);
        update_visibility(s);
        s.clients.debug_assert_partition();
        publish(s);
        return;
    }

    let added: Vec<u32> = new_pids.difference(&old_pids).copied().collect();
    let removed: Vec<u32> = old_pids.difference(&new_pids).copied().collect();
    let mut last_closed_label = None;
    for pid in &removed {
        // Capture info before removing for toast.
        if let Some(w) = s.clients.windows.iter().find(|w| w.pid == *pid) {
            last_closed_label = Some(format!("Window #{} closed", w.number));
        }
        s.clients.remove(*pid);
        s.casting.remove(*pid);
        s.combat_awareness.remove(*pid);
        s.notification_center.remove_client(*pid);
    }
    if let Some(label) = last_closed_label {
        show_toast_inner(s, &label);
    }

    let cfg = config::Config::load();
    if should_disable_broadcast_when_clients_exit(
        cfg.disable_broadcast_when_clients_exit,
        old_pids.len(),
        new_pids.len(),
    ) {
        let _ = crate::broadcast::set_active(false);
    }

    let fg_hwnd = GetForegroundWindow();
    let fg_pid = focused_foreground_pid(&new_windows, fg_hwnd, |hwnd| {
        target_has_keyboard_focus(hwnd)
    });

    for pid in &added {
        let nw = new_windows.iter().find(|w| w.pid == *pid).unwrap();
        let number = next_available_number(&s.clients.windows);
        let prefer_active =
            s.clients.active_pid().is_none() && (fg_pid == Some(nw.pid) || fg_pid.is_none());
        s.clients.add(
            EqWindow {
                hwnd: nw.hwnd,
                pid: nw.pid,
                number,
                character: None,
                server: None,
                class: None,
            },
            prefer_active,
        );
    }

    s.clients.ensure_active();

    if let Some(fg) = fg_pid {
        if s.clients.promote(fg, MAX_PIPS) && cfg.auto_order {
            s.clients.apply_auto_order();
        }
        crate::broadcast::set_active_pid(fg);
        sync_mouse_eligibility(s);
        s.notification_center.acknowledge(fg);
    }

    s.clients.truncate_pips(MAX_PIPS);

    for nw in &new_windows {
        if let Some(ow) = s.clients.windows.iter_mut().find(|w| w.pid == nw.pid) {
            ow.hwnd = nw.hwnd;
        }
    }

    // Poll trusik shared memory for character names.
    if s.telemetry.trusik_enabled {
        trusik_poll_characters(s);
    }
    // Publish identity changes to the event-driven log worker. No log
    // filesystem I/O occurs on the UI polling path.
    publish_log_sources_inner(s);

    // Update broadcast targets and the strict identical-geometry Mouse Clutch set.
    let pids: Vec<u32> = s.clients.windows.iter().map(|w| w.pid).collect();
    crate::broadcast::update_targets(&pids, s.clients.active_pid());
    sync_mouse_eligibility(s);

    apply_preferred_box_order(&mut s.clients.windows, &s.clients.preferred_order);
    if cfg.auto_order {
        s.clients.apply_auto_order();
    }

    s.window_styles.apply(&s.clients);
    rebuild_thumbnails(s);
    super::dps_overlay::reconcile(s);
    service_compositor_recovery(s);
    update_visibility(s);
    s.clients.debug_assert_partition();
    publish(s);
}

/// Check trusik shared memory for a newly published identity for each process.
fn trusik_poll_characters(s: &mut OverlayState) {
    if s.telemetry.poll_characters(&mut s.clients) {
        if apply_preferred_box_order(&mut s.clients.windows, &s.clients.preferred_order)
            && config::Config::load().auto_order
        {
            s.clients.apply_auto_order();
        }
        unsafe { rebuild_thumbnails(s) };
    }
}

pub(super) fn publish_log_sources_inner(s: &OverlayState) {
    let logs_dir = config::Config::load().eq_directory().join("Logs");
    telemetry::publish_log_sources(&s.clients, logs_dir);
}

/// Publish the current identity snapshot after the log worker starts or the EQ
/// directory changes. This is last-write-wins and performs no filesystem I/O.
pub(super) fn publish_log_sources() {
    let _ = try_with_state(publish_log_sources_inner);
}

/// Drain a bounded number of parsed log batches on the owner thread. Returns
/// false when a re-entrant Win32 callback should repost the wake message.
pub(super) fn drain_log_events() -> bool {
    match try_with_state_mut(|state| {
        let batches = log_watcher::drain_ready();
        let dps_snapshot = log_watcher::take_dps_snapshot();
        let timer_frame = log_watcher::take_timer_frame();
        apply_log_batches(state, batches, timer_frame);
        if let Some(snapshot) = dps_snapshot {
            unsafe {
                super::dps_overlay::apply_book(state, snapshot);
                update_visibility(state);
            }
        }
    }) {
        Ok(()) => true,
        Err(AccessError::Busy) => false,
        Err(AccessError::Unavailable) => {
            let _ = log_watcher::drain_ready();
            let _ = log_watcher::take_dps_snapshot();
            let _ = log_watcher::take_timer_frame();
            true
        }
    }
}

fn apply_log_batches(
    s: &mut OverlayState,
    batches: Vec<log_watcher::LogBatch>,
    timer_frame: Option<log_watcher::TimerFrame>,
) {
    let mut class_changed = false;
    let mut timers_changed = false;
    let now = Instant::now();
    for batch in batches {
        for diagnostic in batch.diagnostics {
            debug_log(&format!("eq_logs: {diagnostic}"));
        }
        // Timer-stage actions fired by the worker clock (warnings, ends).
        super::trigger_presentation::apply_events(s, &batch.trigger_events, now);
        for envelope in batch.envelopes {
            super::trigger_presentation::apply_events(s, &envelope.trigger_actions, now);
            // Persona changes must update the current class before a later cast
            // in the same drained batch is keyed and estimated.
            for change in envelope.telemetry_changes.iter() {
                class_changed |= s.telemetry.apply_change(&mut s.clients, change);
            }
            casting::apply_log_envelope(s, &envelope);
            for event in envelope.events.iter() {
                notifications::apply_log_event(s, event);
                combat_awareness::apply_log_event(s, event);
            }
        }
    }
    if let Some(frame) = timer_frame {
        timers_changed |= s.timers.replace_frame(&frame);
    }
    s.telemetry.save();
    s.casting.save();

    if class_changed {
        s.casting.reconcile_clients(&s.clients.windows);
        unsafe { rebuild_thumbnails(s) };
    }
    if timers_changed {
        unsafe {
            if s.timers.is_empty() {
                let _ = KillTimer(s.presentation.active_label_hwnd, TIMER_OVERLAY_TICK);
            } else {
                let _ = SetTimer(
                    s.presentation.active_label_hwnd,
                    TIMER_OVERLAY_TICK,
                    TIMER_OVERLAY_INTERVAL_MS,
                    None,
                );
            }
            update_active_label(s);
            invalidate_labels(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_disable_broadcast_when_clients_exit;

    #[test]
    fn broadcast_shutoff_only_runs_when_the_last_client_exits() {
        assert!(should_disable_broadcast_when_clients_exit(true, 1, 0));
        assert!(should_disable_broadcast_when_clients_exit(true, 6, 0));
        assert!(!should_disable_broadcast_when_clients_exit(false, 1, 0));
        assert!(!should_disable_broadcast_when_clients_exit(true, 2, 1));
        assert!(!should_disable_broadcast_when_clients_exit(true, 0, 0));
    }
}
