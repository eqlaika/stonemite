use windows::Win32::UI::Accessibility::HWINEVENTHOOK;

use super::clients::ClientRegistry;
use super::combat_awareness::CombatAwarenessCenter;
use super::interaction::InteractionState;
use super::layout::LayoutState;
use super::notifications::NotificationCenter;
use super::presentation::PresentationState;
use super::telemetry::TelemetryState;
use super::timers::TimerOverlayState;
use super::window_styles::WindowStyleState;

/// Owner-thread state for one initialized overlay runtime.
pub(super) struct OverlayState {
    pub(super) presentation: PresentationState,
    /// Authoritative client identity and active/PiP partition.
    pub(super) clients: ClientRegistry,
    pub(super) event_hook: HWINEVENTHOOK,
    pub(super) layout: LayoutState,
    /// User has toggled overlay hidden via hotkey.
    pub(super) hidden_by_user: bool,
    pub(super) interaction: InteractionState,
    pub(super) window_styles: WindowStyleState,
    pub(super) telemetry: TelemetryState,
    /// High-frequency, recoverable combat presentation kept separate from unread events.
    pub(super) combat_awareness: CombatAwarenessCenter,
    pub(super) notification_center: NotificationCenter,
    /// Passive display-only timers started by log trigger activations.
    pub(super) timers: TimerOverlayState,
}
