//! Reusable, platform-neutral EverQuest log parsing and telemetry primitives.
//!
//! `eqlog` accepts complete records supplied by an application, separates the
//! EQ timestamp envelope from the body, dispatches composable parsing domains,
//! emits typed semantic events, and reduces persistent per-character state.
//! It intentionally does not watch files, own an async runtime, render UI, or
//! perform gameplay input.

mod event;
pub mod parsers;
mod raw;
mod telemetry;

pub use event::{
    AttackProblem, CastKind, CastingEvent, CharacterEvent, ChatEvent, CombatAttempt, CombatEvent,
    DamageKind, DamageModifiers, DamageObservation, DamageOutcome, IdentityEvent, IncomingTell,
    LogEvent, LogEventDomain, NotificationEvent, ObservedCombatant, ParsedLogEvent,
    ParserProvenance, PersonaLoaded, Perspective, PetEvent, PetOwnershipObservation,
    PlayerEvidence, ProgressEvent, TargetSlainObservation, WhoResult, ZoneObservation,
};
pub use parsers::{DomainParser, ParseOutcome, ParserError, ParserFailure, ParserRegistry};
pub use raw::{
    DecodedRawLogLine, EqSecond, EqTimestamp, LogSource, LogSourceId, RawLogLine, SourceRecordId,
};
pub use telemetry::{CharacterKey, CharacterTelemetry, TelemetryChange, TelemetryReducer};
