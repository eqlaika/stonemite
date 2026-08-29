//! Virtual-clock timer state.
//!
//! Active timers are owned by the engine's per-character state; this module
//! defines the timer records, compiled early-enders, and the serializable
//! snapshots the presentation layer renders. All times are [`EngineMillis`]
//! on the caller-supplied clock.

use serde::{Deserialize, Serialize};

use crate::model::{OverlayId, PresentationTarget, TimerKind, TriggerId};
use crate::netregex::{CompatRegex, MatchMap};
use crate::pattern::NumberConstraint;
use crate::substitute;
use crate::EngineMillis;

/// A compiled end-early matcher captured at timer start (with the trigger's
/// match values already substituted into the pattern, like EQLP).
#[derive(Clone, Debug)]
pub(crate) enum CompiledEnder {
    Regex {
        regex: CompatRegex,
        constraints: Vec<NumberConstraint>,
    },
    Literal(String),
}

impl CompiledEnder {
    pub(crate) fn matches(&self, line: &str) -> Option<MatchMap> {
        match self {
            CompiledEnder::Regex { regex, constraints } => {
                let outcome = regex.snapshot_matches(line).ok().flatten()?;
                let (passed, _) = crate::pattern::check_constraints(constraints, &outcome.captures);
                passed.then_some(outcome.captures)
            }
            CompiledEnder::Literal(text) => (!text.is_empty()
                && crate::netregex::contains_ignore_case(line, text))
            .then(MatchMap::new),
        }
    }
}

/// One running timer instance.
#[derive(Clone, Debug)]
pub(crate) struct ActiveTimer {
    pub trigger_index: usize,
    pub trigger_id: TriggerId,
    pub kind: TimerKind,
    /// Fully resolved display name (variables applied at start).
    pub display_name: String,
    /// Pre-variable template so renders can re-resolve live variables.
    pub display_name_template: String,
    pub begin: EngineMillis,
    pub end: EngineMillis,
    pub duration_ms: u64,
    /// Cooldown window end for progress-style overlays.
    pub reset_at: Option<EngineMillis>,
    pub warning_at: Option<EngineMillis>,
    pub warned: bool,
    pub loop_count: u32,
    pub early_enders: Vec<CompiledEnder>,
    pub original_matches: MatchMap,
    pub previous_matches: MatchMap,
    /// Captured for looping timers so the repeat can re-fire the trigger.
    pub source_line: String,
    pub log_time: Option<String>,
    pub counter_count: i64,
    pub repeated_count: i64,
}

impl ActiveTimer {
    pub(crate) fn is_ended(&self, now: EngineMillis) -> bool {
        now >= self.end
    }

    pub(crate) fn warning_due(&self, now: EngineMillis) -> bool {
        !self.warned && self.warning_at.is_some_and(|at| now >= at)
    }
}

/// Serializable view of a running timer for overlays and the test bench.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerSnapshot {
    /// Log-source key of the character that started the timer.
    pub character: String,
    pub trigger_id: TriggerId,
    pub kind: TimerKind,
    pub display_name: String,
    pub begin_ms: EngineMillis,
    pub end_ms: EngineMillis,
    pub duration_ms: u64,
    pub reset_at_ms: Option<EngineMillis>,
    pub warned: bool,
    pub target: PresentationTarget,
    pub timer_overlays: Vec<OverlayId>,
    pub font_color: Option<String>,
    pub active_color: Option<String>,
    pub idle_color: Option<String>,
    pub reset_color: Option<String>,
}

impl TimerSnapshot {
    pub fn remaining_ms(&self, now: EngineMillis) -> u64 {
        self.end_ms.saturating_sub(now)
    }

    /// 0.0 at start → 1.0 at natural end.
    pub fn progress(&self, now: EngineMillis) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        ((now.saturating_sub(self.begin_ms)) as f64 / self.duration_ms as f64).clamp(0.0, 1.0)
            as f32
    }
}

/// Resolve a timer's live display name: built-in codes take precedence over
/// stored variables (EQLP `GetDisplayName`).
pub(crate) fn resolve_display_name(timer: &ActiveTimer, variables: &MatchMap) -> String {
    let mut name = timer.display_name_template.clone();
    if timer.counter_count >= 0 {
        name = substitute::replace_code(
            &name,
            substitute::COUNTER_CODE,
            &timer.counter_count.to_string(),
        );
    }
    if timer.repeated_count >= 0 {
        name = substitute::replace_code(
            &name,
            substitute::REPEATED_CODE,
            &timer.repeated_count.to_string(),
        );
    }
    if let Some(log_time) = &timer.log_time {
        name = substitute::replace_code(&name, substitute::LOGTIME_CODE, log_time);
    }
    substitute::replace_tokens(&name, |token| variables.get(token).map(str::to_owned))
}
