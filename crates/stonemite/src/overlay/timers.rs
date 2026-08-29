//! Multi-entry trigger-timer overlay model.
//!
//! Timers are owned by the log worker's trigger engine; the worker publishes
//! immutable [`TimerFrame`] snapshots and this model turns them into what
//! each client label should display: deterministic ordering (soonest end
//! first, then name), replacement on refresh, expiry between frames, and
//! per-client scope derived from each trigger's presentation target.
//!
//! Time is supplied by the owner thread so the model remains independent of
//! Win32 timers and deterministic in tests. A Win32 timer is only ever a
//! wake mechanism; remaining time and progress derive from stored instants.

use std::time::{Duration, Instant};

use crate::log_watcher::{PresentationTarget, TimerFrame};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TimerOverlay {
    /// Log-source key of the character that started the timer.
    pub source_id: String,
    pub label: String,
    pub start_time: Instant,
    pub end_time: Instant,
    pub duration: Duration,
    pub target: PresentationTarget,
    pub warned: bool,
}

impl TimerOverlay {
    pub fn remaining_time(&self, now: Instant) -> Duration {
        self.end_time.saturating_duration_since(now)
    }

    /// Elapsed progress from 0.0 at start to 1.0 at expiry.
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.saturating_duration_since(self.start_time).as_secs_f64() / self.duration.as_secs_f64())
            .clamp(0.0, 1.0) as f32
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.end_time
    }

    /// Whether this timer shows on a label for `source_id`. `is_active`
    /// marks the focused client's label.
    fn visible_on(&self, source_id: Option<&str>, is_active: bool) -> bool {
        match self.target {
            PresentationTarget::Source => source_id.is_some_and(|source| self.source_id == source),
            PresentationTarget::ActiveClient | PresentationTarget::Global => is_active,
            PresentationTarget::AllClients => true,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct TimerOverlayState {
    timers: Vec<TimerOverlay>,
}

impl TimerOverlayState {
    /// Replace the model from a worker-published frame. Returns true when
    /// the visible set changed.
    pub fn replace_frame(&mut self, frame: &TimerFrame) -> bool {
        let mut timers: Vec<TimerOverlay> = frame
            .snapshots
            .iter()
            .map(|snapshot| TimerOverlay {
                source_id: snapshot.character.clone(),
                label: snapshot.display_name.clone(),
                start_time: frame.instant(snapshot.begin_ms),
                end_time: frame.instant(snapshot.end_ms),
                duration: Duration::from_millis(snapshot.duration_ms),
                target: snapshot.target,
                warned: snapshot.warned,
            })
            .collect();
        // Deterministic ordering: soonest end first, then label, then source.
        timers.sort_by(|a, b| {
            a.end_time
                .cmp(&b.end_time)
                .then_with(|| a.label.cmp(&b.label))
                .then_with(|| a.source_id.cmp(&b.source_id))
        });
        if timers == self.timers {
            return false;
        }
        self.timers = timers;
        true
    }

    pub fn remove_expired(&mut self, now: Instant) -> bool {
        let previous_len = self.timers.len();
        self.timers.retain(|timer| !timer.is_expired(now));
        self.timers.len() != previous_len
    }

    /// The single most urgent timer for a label (existing renderer slot).
    pub fn visible_for(
        &self,
        source_id: Option<&str>,
        is_active: bool,
        now: Instant,
    ) -> Option<&TimerOverlay> {
        self.visible_all_for(source_id, is_active, now)
            .into_iter()
            .next()
    }

    /// Every timer for a label, most urgent first (multi-row renderers).
    pub fn visible_all_for(
        &self,
        source_id: Option<&str>,
        is_active: bool,
        now: Instant,
    ) -> Vec<&TimerOverlay> {
        self.timers
            .iter()
            .filter(|timer| !timer.is_expired(now) && timer.visible_on(source_id, is_active))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    #[cfg(test)]
    fn timers(&self) -> &[TimerOverlay] {
        &self.timers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_watcher::TimerSnapshot;
    use eqtrigger::{TimerKind, TriggerId};

    fn snapshot(
        character: &str,
        name: &str,
        begin_ms: u64,
        end_ms: u64,
        target: PresentationTarget,
    ) -> TimerSnapshot {
        TimerSnapshot {
            character: character.to_owned(),
            trigger_id: TriggerId::new(),
            kind: TimerKind::Countdown,
            display_name: name.to_owned(),
            begin_ms,
            end_ms,
            duration_ms: end_ms - begin_ms,
            reset_at_ms: None,
            warned: false,
            target,
            timer_overlays: Vec::new(),
            font_color: None,
            active_color: None,
            idle_color: None,
            reset_color: None,
        }
    }

    fn frame(origin: Instant, snapshots: Vec<TimerSnapshot>) -> TimerFrame {
        TimerFrame { origin, snapshots }
    }

    #[test]
    fn frames_replace_the_model_with_deterministic_ordering() {
        let origin = Instant::now();
        let mut state = TimerOverlayState::default();
        assert!(state.replace_frame(&frame(
            origin,
            vec![
                snapshot("pid:1", "Slow", 0, 30_000, PresentationTarget::Source),
                snapshot("pid:1", "Fast", 0, 10_000, PresentationTarget::Source),
                snapshot("pid:1", "Also fast", 0, 10_000, PresentationTarget::Source),
            ],
        )));
        let labels: Vec<&str> = state.timers().iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["Also fast", "Fast", "Slow"]);
        // Identical frame → no change signal.
        assert!(!state.replace_frame(&frame(
            origin,
            vec![
                snapshot("pid:1", "Slow", 0, 30_000, PresentationTarget::Source),
                snapshot("pid:1", "Fast", 0, 10_000, PresentationTarget::Source),
                snapshot("pid:1", "Also fast", 0, 10_000, PresentationTarget::Source),
            ],
        )));
        // An empty frame clears everything.
        assert!(state.replace_frame(&frame(origin, Vec::new())));
        assert!(state.is_empty());
    }

    #[test]
    fn presentation_targets_control_label_visibility() {
        let origin = Instant::now();
        let mut state = TimerOverlayState::default();
        state.replace_frame(&frame(
            origin,
            vec![
                snapshot("pid:1", "Mine", 0, 10_000, PresentationTarget::Source),
                snapshot(
                    "pid:1",
                    "Focus",
                    0,
                    11_000,
                    PresentationTarget::ActiveClient,
                ),
                snapshot(
                    "pid:1",
                    "Everywhere",
                    0,
                    12_000,
                    PresentationTarget::AllClients,
                ),
                snapshot("pid:1", "World", 0, 13_000, PresentationTarget::Global),
            ],
        ));
        let now = origin;

        // The source's own label (also the active client here) sees all four.
        let labels: Vec<&str> = state
            .visible_all_for(Some("pid:1"), true, now)
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Mine", "Focus", "Everywhere", "World"]);

        // Another client's inactive label sees only the all-clients timer.
        let labels: Vec<&str> = state
            .visible_all_for(Some("pid:2"), false, now)
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Everywhere"]);

        // Another client's label while focused adds active/global timers.
        let labels: Vec<&str> = state
            .visible_all_for(Some("pid:2"), true, now)
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Focus", "Everywhere", "World"]);

        // The single-slot view picks the most urgent.
        assert_eq!(
            state
                .visible_for(Some("pid:1"), true, now)
                .map(|t| t.label.as_str()),
            Some("Mine")
        );
    }

    #[test]
    fn expiry_is_local_between_frames() {
        let origin = Instant::now();
        let mut state = TimerOverlayState::default();
        state.replace_frame(&frame(
            origin,
            vec![
                snapshot("pid:1", "Short", 0, 1_000, PresentationTarget::Source),
                snapshot("pid:1", "Long", 0, 5_000, PresentationTarget::Source),
            ],
        ));
        let later = origin + Duration::from_millis(1_500);
        assert!(state
            .visible_all_for(Some("pid:1"), true, later)
            .iter()
            .all(|timer| timer.label == "Long"));
        assert!(state.remove_expired(later));
        assert_eq!(state.timers().len(), 1);
        assert!(!state.remove_expired(later));
    }

    #[test]
    fn remaining_time_and_progress_are_bounded() {
        let origin = Instant::now();
        let mut state = TimerOverlayState::default();
        state.replace_frame(&frame(
            origin,
            vec![snapshot(
                "pid:1",
                "Burn",
                0,
                10_000,
                PresentationTarget::Source,
            )],
        ));
        let timer = &state.timers()[0];
        assert_eq!(timer.remaining_time(origin), Duration::from_secs(10));
        assert_eq!(timer.progress(origin), 0.0);
        assert_eq!(
            timer.remaining_time(origin + Duration::from_secs(4)),
            Duration::from_secs(6)
        );
        assert!((timer.progress(origin + Duration::from_secs(4)) - 0.4).abs() < f32::EPSILON);
        assert_eq!(
            timer.remaining_time(origin + Duration::from_secs(12)),
            Duration::ZERO
        );
        assert_eq!(timer.progress(origin + Duration::from_secs(12)), 1.0);
        assert!(timer.is_expired(origin + Duration::from_secs(10)));
    }
}
