//! Platform-neutral, event-sourced EverQuest combat encounter engine.
//!
//! Callers supply source lifecycle facts, complete-record identities, parsed EQ
//! seconds, and monotonic receipt times. The engine owns encounter projection,
//! correlation, whole-participant source election, metrics, lifecycle, and
//! immutable snapshots. It performs no filesystem, wall-clock, async, Windows,
//! or rendering work.

mod engine;
mod metrics;
mod model;

pub use engine::CombatEngine;
pub use metrics::format_grouped_ascii;
pub use model::{
    project_visible_rows, select_presented, CandidateExplanation, CanonicalTargetId, CombatPolicy,
    CombatRecord, DpsRowSnapshot, EncounterBookSnapshot, EncounterExplanation, EncounterId,
    EncounterPhase, EncounterSnapshot, EndReason, EngineInput, EngineUpdate, GapReason, MonoTime,
    ParticipantId, PolicyError, PublishUrgency, SourceQuality,
};
pub use model::{EqSecond, LogEvent, LogSource, LogSourceId, SourceRecordId};
