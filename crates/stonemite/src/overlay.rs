mod activation;
mod appearance;
mod client_controller;
mod clients;
mod control_bridge;
mod edit_mode;
mod event_loop;
mod geometry;
mod hosts;
mod interaction;
mod labels;
mod layout;
mod lifecycle;
mod menu;
mod notifications;
mod pip_interaction;
mod presentation;
mod render;
mod runtime;
mod scene_layout;
mod scenes;
mod state;
mod surfaces;
mod telemetry;
mod timer_controller;
mod timers;
mod toast;
mod toast_controller;
mod window_procs;
mod window_styles;

use runtime::{try_with_state, try_with_state_mut};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// Initialize owner-thread overlay resources.
pub(crate) fn initialize() {
    unsafe {
        lifecycle::init_inner();
    }
}

/// Release all overlay resources on the owner thread.
pub(crate) fn shutdown() {
    lifecycle::cleanup();
}

/// Reconcile EQ clients and advance overlay presentation state.
pub(crate) fn tick() {
    event_loop::poll();
}

/// Publish the current client identities to the log watcher.
pub(crate) fn sync_log_sources() {
    event_loop::publish_log_sources();
}

/// Try to drain pending log events. A false result asks the owner message loop
/// to repost the wake because an overlay transaction is already in progress.
pub(crate) fn try_drain_log_events() -> bool {
    event_loop::drain_log_events()
}

pub(crate) fn swap_to_number(number: usize) {
    unsafe {
        client_controller::swap_to_number(number);
    }
}

pub(crate) fn swap_active_window_numbers(
    target_pid: u32,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    unsafe { client_controller::swap_active_window_numbers(target_pid) }
}

pub(crate) fn activate_pid(
    target_pid: u32,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    unsafe { client_controller::activate_pid(target_pid) }
}

pub(crate) fn toggle_edit_mode() {
    edit_mode::toggle();
}

pub(crate) fn is_edit_mode() -> bool {
    edit_mode::is_active()
}

pub(crate) fn show_toast(text: &str) {
    let _ = try_with_state_mut(|state| unsafe { toast_controller::show_toast_inner(state, text) });
}

/// Return whether EQ, an overlay surface, or the settings window owns the
/// foreground interaction context.
pub(crate) fn is_app_foreground() -> bool {
    try_with_state(|state| unsafe { control_bridge::owns_foreground(GetForegroundWindow(), state) })
        .unwrap_or(false)
}

pub(crate) fn is_visible() -> bool {
    try_with_state(|state| !state.hidden_by_user).unwrap_or(false)
}

pub(crate) fn toggle_hidden() {
    let _ = try_with_state_mut(|state| unsafe {
        state.hidden_by_user = !state.hidden_by_user;
        surfaces::update_visibility(state);
    });
}

/// Apply a broadcast-state change to all overlay and control projections.
pub(crate) fn broadcast_state_changed() {
    let _ = try_with_state_mut(|state| unsafe {
        hosts::update_active_label(state);
        surfaces::update_visibility(state);
        control_bridge::publish(state);
    });
}

/// Reload overlay-owned configuration, rebuild presentation resources, and
/// republish the effective log-source directory.
pub(crate) fn reload_config() {
    lifecycle::force_rebuild();
    event_loop::publish_log_sources();
}
