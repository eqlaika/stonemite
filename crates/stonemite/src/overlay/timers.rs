//! Pure timer-overlay state derived from passive log-trigger actions.
//!
//! Time is supplied by the owner thread so the model remains independent of
//! Win32 timers and deterministic in tests. A Win32 timer is only ever a wake
//! mechanism; remaining time and progress are derived from the stored start.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::log_watcher::{
    LogSourceId, PresentationAction, TimerRequest, TriggerActivation, TriggerScope,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TimerScope {
    Client { source_id: LogSourceId },
    Global,
}

impl TimerScope {
    fn from_activation(activation: &TriggerActivation) -> Self {
        match &activation.scope {
            TriggerScope::SourceClient { source_id } => Self::Client {
                source_id: source_id.clone(),
            },
            // An AllClients definition can activate independently from every
            // matching source. Resolve each activation back to that source so
            // its timer follows the corresponding character label.
            TriggerScope::AllClients => Self::Client {
                source_id: activation.source.id.clone(),
            },
            TriggerScope::Global => Self::Global,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct TimerOverlay {
    pub scope: TimerScope,
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub start_time: Instant,
    pub duration: Duration,
}

impl TimerOverlay {
    fn new(scope: TimerScope, request: &TimerRequest, start_time: Instant) -> Self {
        Self {
            scope,
            id: request.id.clone(),
            label: request.label.clone(),
            start_time,
            duration: request.duration,
        }
    }

    pub fn remaining_time(&self, now: Instant) -> Duration {
        self.duration
            .saturating_sub(now.saturating_duration_since(self.start_time))
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
        now.saturating_duration_since(self.start_time) >= self.duration
    }
}

#[derive(Debug, Default)]
pub(super) struct TimerOverlayState {
    timers: Vec<TimerOverlay>,
}

impl TimerOverlayState {
    pub fn apply_activations(&mut self, activations: &[TriggerActivation], now: Instant) -> bool {
        let mut changed = self.remove_expired(now);
        for activation in activations {
            for action in activation.presentation.iter() {
                if let PresentationAction::StartTimer(request) = action {
                    self.start(TimerScope::from_activation(activation), request, now);
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn start(&mut self, scope: TimerScope, request: &TimerRequest, now: Instant) {
        let replacement = TimerOverlay::new(scope, request, now);
        if let Some(timer) = self
            .timers
            .iter_mut()
            .find(|timer| timer.scope == replacement.scope && timer.id == replacement.id)
        {
            *timer = replacement;
        } else {
            self.timers.push(replacement);
        }
    }

    pub fn remove_expired(&mut self, now: Instant) -> bool {
        let previous_len = self.timers.len();
        self.timers.retain(|timer| !timer.is_expired(now));
        self.timers.len() != previous_len
    }

    pub fn visible_for(&self, source_id: Option<&str>, now: Instant) -> Option<&TimerOverlay> {
        self.timers.iter().find(|timer| {
            !timer.is_expired(now)
                && match &timer.scope {
                    TimerScope::Client {
                        source_id: timer_source,
                    } => source_id.is_some_and(|source| timer_source.as_str() == source),
                    TimerScope::Global => true,
                }
        })
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
    use crate::log_watcher::{LogSource, LogSourceId};

    fn request(id: &str, label: &str, seconds: u64) -> TimerRequest {
        TimerRequest {
            id: Arc::from(id),
            label: Arc::from(label),
            duration: Duration::from_secs(seconds),
        }
    }

    fn activation(
        source_id: &str,
        scope: TriggerScope,
        actions: Vec<PresentationAction>,
    ) -> TriggerActivation {
        TriggerActivation {
            trigger_id: Arc::from("test-trigger"),
            scope,
            source: LogSource::new(source_id, "Bilka", "teek"),
            presentation: actions.into(),
        }
    }

    #[test]
    fn remaining_time_and_progress_are_bounded() {
        let start = Instant::now();
        let timer = TimerOverlay::new(TimerScope::Global, &request("burn", "Burn", 10), start);

        assert_eq!(timer.remaining_time(start), Duration::from_secs(10));
        assert_eq!(timer.progress(start), 0.0);
        assert_eq!(
            timer.remaining_time(start + Duration::from_secs(4)),
            Duration::from_secs(6)
        );
        assert!((timer.progress(start + Duration::from_secs(4)) - 0.4).abs() < f32::EPSILON);
        assert_eq!(
            timer.remaining_time(start + Duration::from_secs(12)),
            Duration::ZERO
        );
        assert_eq!(timer.progress(start + Duration::from_secs(12)), 1.0);
    }

    #[test]
    fn expiry_is_inclusive_and_zero_duration_is_complete() {
        let start = Instant::now();
        let timer = TimerOverlay::new(TimerScope::Global, &request("short", "Short", 1), start);
        assert!(!timer.is_expired(start + Duration::from_millis(999)));
        assert!(timer.is_expired(start + Duration::from_secs(1)));

        let zero = TimerOverlay::new(TimerScope::Global, &request("zero", "Zero", 0), start);
        assert!(zero.is_expired(start));
        assert_eq!(zero.remaining_time(start), Duration::ZERO);
        assert_eq!(zero.progress(start), 1.0);
    }

    #[test]
    fn matching_scope_and_id_restart_without_reordering() {
        let start = Instant::now();
        let scope = TimerScope::Client {
            source_id: LogSourceId::new("client-1"),
        };
        let mut state = TimerOverlayState::default();
        state.start(scope.clone(), &request("disc", "First", 10), start);
        state.start(TimerScope::Global, &request("disc", "Global", 30), start);
        state.start(
            scope,
            &request("disc", "Restarted", 20),
            start + Duration::from_secs(2),
        );

        assert_eq!(state.timers().len(), 2);
        assert_eq!(state.timers()[0].label.as_ref(), "Restarted");
        assert_eq!(state.timers()[0].start_time, start + Duration::from_secs(2));
        assert_eq!(state.timers()[1].label.as_ref(), "Global");
    }

    #[test]
    fn start_timer_actions_flow_from_trigger_activations() {
        let start = Instant::now();
        let scope = TriggerScope::AllClients;
        let activation = activation(
            "client-1",
            scope.clone(),
            vec![
                PresentationAction::ShowText {
                    text: Arc::from("ignored here"),
                },
                PresentationAction::StartTimer(request("mez", "Mez", 18)),
            ],
        );
        let mut state = TimerOverlayState::default();

        assert!(state.apply_activations(&[activation], start));
        assert_eq!(state.timers().len(), 1);
        assert_eq!(
            state.timers()[0].scope,
            TimerScope::Client {
                source_id: LogSourceId::new("client-1")
            }
        );
        assert_eq!(state.timers()[0].label.as_ref(), "Mez");
        assert_eq!(state.timers()[0].duration, Duration::from_secs(18));
    }

    #[test]
    fn all_clients_activations_keep_same_id_timers_per_source() {
        let start = Instant::now();
        let actions = || {
            vec![PresentationAction::StartTimer(request(
                "shared", "Shared", 10,
            ))]
        };
        let mut state = TimerOverlayState::default();

        assert!(state.apply_activations(
            &[
                activation("pid:1", TriggerScope::AllClients, actions()),
                activation("pid:2", TriggerScope::AllClients, actions()),
            ],
            start,
        ));
        assert_eq!(state.timers().len(), 2);
        assert!(state.visible_for(Some("pid:1"), start).is_some());
        assert!(state.visible_for(Some("pid:2"), start).is_some());
    }

    #[test]
    fn source_timers_only_appear_for_the_matching_active_client() {
        let start = Instant::now();
        let mut state = TimerOverlayState::default();
        state.start(
            TimerScope::Client {
                source_id: LogSourceId::new("pid:42"),
            },
            &request("source", "Source", 10),
            start,
        );

        assert!(state.visible_for(Some("pid:7"), start).is_none());
        assert_eq!(
            state
                .visible_for(Some("pid:42"), start)
                .map(|timer| timer.label.as_ref()),
            Some("Source")
        );
    }

    #[test]
    fn expired_timers_are_removed_deterministically() {
        let start = Instant::now();
        let mut state = TimerOverlayState::default();
        state.start(TimerScope::Global, &request("one", "One", 1), start);
        state.start(TimerScope::Global, &request("two", "Two", 2), start);

        assert!(state.remove_expired(start + Duration::from_secs(1)));
        assert_eq!(state.timers().len(), 1);
        assert_eq!(state.timers()[0].id.as_ref(), "two");
        assert!(!state.remove_expired(start + Duration::from_secs(1)));
    }
}
