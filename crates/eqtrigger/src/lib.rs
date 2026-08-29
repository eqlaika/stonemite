//! Portable EverQuest trigger domain for Stonemite.
//!
//! This crate is deliberately platform-independent: it owns the native
//! trigger schema, EQLP (`.tgf.gz`/`.ogf.gz`) and GINA (`.gtp`) codecs,
//! pattern-macro expansion, a bounded .NET-regex compatibility adapter,
//! the condition language, variable/counter/lockout state, the
//! virtual-clock timer state machine, and presentation-action generation.
//!
//! Nothing in here performs I/O against a live EverQuest client, reads
//! Win32 state, or exposes an escape hatch into input broadcasting. The
//! engine consumes log lines and a caller-supplied clock and produces
//! presentation-only actions.

pub mod conditions;
pub mod engine;
pub mod eqlp;
pub mod gina;
pub mod model;
pub mod netregex;
pub mod package;
pub mod pattern;
pub mod report;
pub mod store;
pub mod substitute;
pub mod timers;
pub mod trace;

pub use engine::{
    ActionBatch, ActionEvent, ActionPhase, CharacterContext, CompiledLibrary, TriggerAction,
    TriggerEngine,
};
pub use model::*;
pub use report::{CompatIssue, CompatReport, CompatSeverity};
pub use timers::TimerSnapshot;
pub use trace::{LineTrace, TriggerTrace};

/// Milliseconds on the caller-supplied virtual clock. The engine never reads
/// wall time itself; deterministic tests and the test bench drive this
/// directly.
pub type EngineMillis = u64;
