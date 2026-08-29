//! Multi-entry trigger text-overlay model.
//!
//! Display-text actions accumulate here with deterministic ordering
//! (newest first), bounded capacity, per-entry expiry, and per-client
//! scope. The renderer decides how many entries it can show; the model is
//! the single source of truth for what is visible where.

use std::time::{Duration, Instant};

use crate::log_watcher::{ActionEvent, PresentationTarget, TriggerAction};

/// Default visibility window when no preset overrides it (matches EQLP's
/// text-overlay fade default).
pub(super) const DEFAULT_VISIBLE: Duration = Duration::from_secs(10);
/// Bounded history so a trigger storm cannot grow without limit.
const MAX_ENTRIES: usize = 50;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TriggerTextEntry {
    pub source_id: String,
    pub text: String,
    pub font_color: Option<String>,
    pub target: PresentationTarget,
    pub added: Instant,
    pub expires: Instant,
}

impl TriggerTextEntry {
    fn visible_on(&self, source_id: Option<&str>, is_active: bool) -> bool {
        match self.target {
            PresentationTarget::Source => source_id.is_some_and(|source| self.source_id == source),
            PresentationTarget::ActiveClient | PresentationTarget::Global => is_active,
            PresentationTarget::AllClients => true,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TriggerTextState {
    entries: Vec<TriggerTextEntry>,
}

impl TriggerTextState {
    /// Record every display-text action from a batch of trigger events.
    /// Returns the number of entries added.
    pub fn apply_events(&mut self, events: &[ActionEvent], now: Instant) -> usize {
        let mut added = 0;
        for event in events {
            if let TriggerAction::DisplayText {
                text, font_color, ..
            } = &event.action
            {
                self.entries.push(TriggerTextEntry {
                    source_id: event.character.clone(),
                    text: text.clone(),
                    font_color: font_color.clone(),
                    target: event.target,
                    added: now,
                    expires: now + DEFAULT_VISIBLE,
                });
                added += 1;
            }
        }
        if added > 0 {
            // Newest first; stable within one batch by insertion order.
            self.entries.sort_by(|a, b| b.added.cmp(&a.added));
            self.entries.truncate(MAX_ENTRIES);
        }
        added
    }

    pub fn remove_expired(&mut self, now: Instant) -> bool {
        let previous = self.entries.len();
        self.entries.retain(|entry| now < entry.expires);
        self.entries.len() != previous
    }

    /// Entries visible on a label, newest first. The banner renderer shows
    /// only the newest entry today; multi-row label renderers consume the
    /// full scoped view.
    #[allow(dead_code)]
    pub fn visible_for(
        &self,
        source_id: Option<&str>,
        is_active: bool,
        now: Instant,
    ) -> Vec<&TriggerTextEntry> {
        self.entries
            .iter()
            .filter(|entry| now < entry.expires && entry.visible_on(source_id, is_active))
            .collect()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_watcher::ActionPhase;
    use eqtrigger::TriggerId;

    fn display(source: &str, text: &str, target: PresentationTarget) -> ActionEvent {
        ActionEvent {
            character: source.to_owned(),
            trigger_id: TriggerId::new(),
            trigger_name: "t".to_owned(),
            phase: ActionPhase::Initial,
            target,
            priority: 3,
            action: TriggerAction::DisplayText {
                text: text.to_owned(),
                overlays: Vec::new(),
                font_color: None,
            },
        }
    }

    fn speak(source: &str) -> ActionEvent {
        ActionEvent {
            character: source.to_owned(),
            trigger_id: TriggerId::new(),
            trigger_name: "t".to_owned(),
            phase: ActionPhase::Initial,
            target: PresentationTarget::Source,
            priority: 3,
            action: TriggerAction::Speak {
                text: "spoken".to_owned(),
                rate: None,
                volume: 4,
            },
        }
    }

    #[test]
    fn only_display_actions_become_entries() {
        let mut state = TriggerTextState::default();
        let now = Instant::now();
        let added = state.apply_events(
            &[
                display("pid:1", "hello", PresentationTarget::Source),
                speak("pid:1"),
            ],
            now,
        );
        assert_eq!(added, 1);
        assert_eq!(state.visible_for(Some("pid:1"), true, now).len(), 1);
    }

    #[test]
    fn scope_and_expiry_filter_the_view() {
        let mut state = TriggerTextState::default();
        let now = Instant::now();
        state.apply_events(
            &[
                display("pid:1", "mine", PresentationTarget::Source),
                display("pid:1", "everyone", PresentationTarget::AllClients),
                display("pid:1", "focus", PresentationTarget::ActiveClient),
            ],
            now,
        );
        let other = state.visible_for(Some("pid:2"), false, now);
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].text, "everyone");
        assert_eq!(state.visible_for(Some("pid:1"), true, now).len(), 3);

        let later = now + DEFAULT_VISIBLE + Duration::from_millis(1);
        assert!(state.visible_for(Some("pid:1"), true, later).is_empty());
        assert!(state.remove_expired(later));
        assert!(state.is_empty());
    }

    #[test]
    fn capacity_is_bounded_newest_first() {
        let mut state = TriggerTextState::default();
        let base = Instant::now();
        for index in 0..60 {
            state.apply_events(
                &[display(
                    "pid:1",
                    &format!("m{index}"),
                    PresentationTarget::Source,
                )],
                base + Duration::from_millis(index),
            );
        }
        let visible = state.visible_for(Some("pid:1"), true, base + Duration::from_millis(60));
        assert_eq!(visible.len(), 50);
        assert_eq!(visible[0].text, "m59");
    }
}
