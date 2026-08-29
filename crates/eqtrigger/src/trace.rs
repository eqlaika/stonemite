//! Deterministic evaluation traces for the test bench.

use serde::{Deserialize, Serialize};

use crate::model::TriggerId;

/// Everything that happened while evaluating one log line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LineTrace {
    pub line: String,
    pub entries: Vec<TriggerTrace>,
}

/// Per-trigger evaluation record. Only triggers whose pre-filter admitted the
/// line are recorded, so quiet lines stay cheap to trace.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TriggerTrace {
    pub trigger_id: Option<TriggerId>,
    pub trigger_name: String,
    pub matched: bool,
    /// Byte spans of the match within the line, for highlighting.
    pub match_spans: Vec<(usize, usize)>,
    pub captures: Vec<(String, String)>,
    /// `None` when the trigger has no previous-line requirement.
    pub previous_line_matched: Option<bool>,
    /// `None` when the trigger has no condition.
    pub condition_passed: Option<bool>,
    pub constraints_passed: bool,
    pub lockout_blocked: bool,
    /// Variable mutations: `(name, new value)`; `None` value = cleared.
    pub variable_mutations: Vec<(String, Option<String>)>,
    /// Human-readable summaries of generated actions.
    pub actions: Vec<String>,
}
