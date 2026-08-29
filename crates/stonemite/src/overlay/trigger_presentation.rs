//! Routes trigger action events into the presentation services: the audio
//! dispatcher (sounds and TTS) and the trigger text-overlay model. Timer
//! actions never appear here — running timers arrive as worker-published
//! frames consumed by [`super::timers::TimerOverlayState`].

use std::time::Instant;

use crate::log_watcher::{ActionEvent, TriggerAction};

use super::state::OverlayState;

/// Apply one batch of trigger events. Returns true when the trigger text
/// view changed and labels should re-render.
pub(super) fn apply_events(s: &mut OverlayState, events: &[ActionEvent], now: Instant) -> bool {
    let mut latest_text: Option<String> = None;
    for event in events {
        match &event.action {
            TriggerAction::PlaySound { sound, volume } => {
                crate::audio::play_trigger_sound(sound, *volume, event.priority);
            }
            TriggerAction::Speak { text, rate, volume } => {
                crate::audio::speak(text, *rate, *volume, event.priority);
            }
            TriggerAction::DisplayText { text, .. } => {
                latest_text = Some(text.clone());
            }
        }
    }
    let added = s.trigger_texts.apply_events(events, now);
    if let Some(text) = latest_text {
        // The banner surface shows the newest entry immediately; the model
        // retains the full bounded, scoped history for richer renderers.
        unsafe { super::toast_controller::show_toast_inner(s, &text) };
    }
    added > 0
}
