//! Adapter between the portable `eqtrigger` engine and the log worker.
//!
//! The worker owns a [`TriggerEvaluator`]: an `eqtrigger::TriggerEngine`
//! driven by the worker's monotonic clock. Libraries are loaded from the
//! on-disk trigger store, compiled into an immutable snapshot, and swapped
//! in without restarting the worker (the existing `WM_SETTINGS_CHANGED`
//! path). All outputs are presentation-only `ActionEvent`s and timer
//! snapshots — there is intentionally no route from here to input
//! broadcasting or control APIs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use eqlog::RawLogLine;
use eqtrigger::store::TriggerStore;
use eqtrigger::TriggerEngine;
pub use eqtrigger::{
    ActionBatch, ActionEvent, ActionPhase, CharacterContext, CompiledLibrary, PresentationTarget,
    TimerSnapshot, TriggerAction,
};

/// A point-in-time view of every running trigger timer, with the Instant
/// that corresponds to engine-millisecond zero so consumers can convert.
#[derive(Clone, Debug)]
pub struct TimerFrame {
    pub origin: Instant,
    pub snapshots: Vec<TimerSnapshot>,
}

impl TimerFrame {
    pub fn instant(&self, engine_millis: u64) -> Instant {
        self.origin + Duration::from_millis(engine_millis)
    }
}

/// Managed sound catalog shared with the UI-side audio dispatcher:
/// asset name → playable path. Refreshed on every library reload.
static SOUND_CATALOG: Mutex<Option<Arc<HashMap<String, PathBuf>>>> = Mutex::new(None);

pub fn sound_catalog() -> Option<Arc<HashMap<String, PathBuf>>> {
    SOUND_CATALOG
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Root of the on-disk trigger store: `%APPDATA%\Stonemite\triggers`.
pub fn store_root() -> Option<PathBuf> {
    crate::config::Config::dir().map(|dir| dir.join("triggers"))
}

/// Development-only QA trigger: saying "stonemite timer" starts a visible
/// 10-second timer on the originating client.
#[cfg(debug_assertions)]
pub(crate) const QA_TIMER_PHRASE: &str = "stonemite timer";

#[cfg(debug_assertions)]
fn qa_timer_trigger() -> eqtrigger::Trigger {
    eqtrigger::Trigger {
        name: "QA timer".to_owned(),
        enabled: true,
        // Nearby clients also log normal /say text. Restrict the QA hook to
        // the originating client's local echo so only that box starts.
        pattern: eqtrigger::Pattern::regex(format!("^You say, .*{QA_TIMER_PHRASE}")),
        timer: Some(eqtrigger::TimerBehavior {
            duration_seconds: 10.0,
            timer_name: "QA timer".to_owned(),
            ..eqtrigger::TimerBehavior::default()
        }),
        ..eqtrigger::Trigger::default()
    }
}

/// Load the library from disk and compile it. Returns human-readable
/// diagnostics for anything salvaged, quarantined, or failed to compile.
pub fn load_compiled() -> (Arc<CompiledLibrary>, Vec<String>) {
    let mut diagnostics = Vec::new();
    let mut library = match store_root() {
        Some(root) => {
            let store = TriggerStore::new(root);
            let outcome = store.load();
            for issue in &outcome.report.issues {
                diagnostics.push(format!(
                    "trigger library: {} [{}] {}",
                    issue.subject, issue.code, issue.detail
                ));
            }
            let catalog: HashMap<String, PathBuf> = outcome
                .library
                .assets
                .iter()
                .map(|asset| {
                    (
                        asset.name.to_ascii_lowercase(),
                        store.assets_dir().join(&asset.file_name),
                    )
                })
                .collect();
            *SOUND_CATALOG
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(catalog));
            outcome.library
        }
        None => eqtrigger::TriggerLibrary::new(),
    };
    #[cfg(debug_assertions)]
    library.triggers.push(qa_timer_trigger());
    #[cfg(not(debug_assertions))]
    let library = library;

    let compiled = Arc::new(CompiledLibrary::compile(&library));
    for (id, error) in &compiled.compile_errors {
        let name = library
            .trigger(*id)
            .map(|trigger| trigger.name.as_str())
            .unwrap_or("unknown");
        diagnostics.push(format!("trigger '{name}' failed to compile: {error}"));
    }
    (compiled, diagnostics)
}

/// Compiled library holding only the QA trigger, for deterministic tests.
#[cfg(all(test, debug_assertions))]
pub(super) fn qa_only_compiled() -> Arc<CompiledLibrary> {
    let mut library = eqtrigger::TriggerLibrary::new();
    library.triggers.push(qa_timer_trigger());
    Arc::new(CompiledLibrary::compile(&library))
}

/// Default compiled library used until the first load completes.
pub(super) fn empty_compiled() -> Arc<CompiledLibrary> {
    static EMPTY: OnceLock<Arc<CompiledLibrary>> = OnceLock::new();
    EMPTY
        .get_or_init(|| Arc::new(CompiledLibrary::compile(&eqtrigger::TriggerLibrary::new())))
        .clone()
}

/// The worker-owned evaluator: engine plus the monotonic origin that maps
/// `Instant`s onto the engine's virtual clock.
pub(super) struct TriggerEvaluator {
    engine: TriggerEngine,
    origin: Instant,
    has_timers: bool,
}

impl TriggerEvaluator {
    pub fn new(compiled: Arc<CompiledLibrary>, origin: Instant) -> Self {
        Self {
            engine: TriggerEngine::new(compiled),
            origin,
            has_timers: false,
        }
    }

    /// Swap in a new compiled library. Per-character state (variables,
    /// counters, timers) resets: definitions changed underneath it.
    pub fn replace(&mut self, compiled: Arc<CompiledLibrary>) {
        self.engine = TriggerEngine::new(compiled);
        self.has_timers = false;
    }

    fn millis(&self, receipt: Instant) -> u64 {
        receipt
            .saturating_duration_since(self.origin)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }

    pub fn process(&mut self, raw: &RawLogLine, receipt: Instant) -> ActionBatch {
        let context = CharacterContext {
            key: raw.source.id.as_str().to_owned(),
            character: raw.source.character.to_string(),
            server: raw.source.server.to_string(),
        };
        let log_time = raw.timestamp.as_ref().map(|timestamp| timestamp.as_str());
        let now = self.millis(receipt);
        let batch = self
            .engine
            .process_line(&context, &raw.body, log_time, now, None);
        if batch.timers_changed {
            self.has_timers = !self.engine.timer_snapshots().is_empty();
        }
        batch
    }

    /// Fire due timer warnings/ends. Cheap when no timers run.
    pub fn advance(&mut self, receipt: Instant) -> ActionBatch {
        if !self.has_timers {
            return ActionBatch::default();
        }
        let batch = self.engine.advance(self.millis(receipt));
        if batch.timers_changed {
            self.has_timers = !self.engine.timer_snapshots().is_empty();
        }
        batch
    }

    pub fn frame(&self) -> TimerFrame {
        TimerFrame {
            origin: self.origin,
            snapshots: self.engine.timer_snapshots(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eqlog::LogSource;

    fn line(source_id: &str, body: &str) -> RawLogLine {
        RawLogLine {
            source: LogSource::new(source_id, "Bilka", "teek"),
            timestamp: None,
            body: std::sync::Arc::from(body),
        }
    }

    fn evaluator_with(triggers: Vec<eqtrigger::Trigger>) -> TriggerEvaluator {
        let mut library = eqtrigger::TriggerLibrary::new();
        library.triggers = triggers;
        TriggerEvaluator::new(Arc::new(CompiledLibrary::compile(&library)), Instant::now())
    }

    #[test]
    fn source_identity_flows_into_action_events() {
        let mut evaluator = evaluator_with(vec![eqtrigger::Trigger {
            name: "Tell".to_owned(),
            enabled: true,
            pattern: eqtrigger::Pattern::literal("tells you"),
            display_text: Some("Incoming tell for {c}".to_owned()),
            ..eqtrigger::Trigger::default()
        }]);
        let batch = evaluator.process(&line("pid:7", "Kafka tells you, 'hi'"), Instant::now());
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].character, "pid:7");
        assert!(matches!(
            &batch.events[0].action,
            TriggerAction::DisplayText { text, .. } if text == "Incoming tell for Bilka"
        ));
    }

    #[test]
    fn timers_advance_on_the_worker_clock_and_expose_frames() {
        let mut evaluator = evaluator_with(vec![eqtrigger::Trigger {
            name: "Buff".to_owned(),
            enabled: true,
            pattern: eqtrigger::Pattern::literal("buff on"),
            timer: Some(eqtrigger::TimerBehavior {
                duration_seconds: 1.0,
                end: eqtrigger::TimerStageActions {
                    speak_text: Some("buff over".to_owned()),
                    ..eqtrigger::TimerStageActions::default()
                },
                ..eqtrigger::TimerBehavior::default()
            }),
            ..eqtrigger::Trigger::default()
        }]);
        let start = Instant::now();
        let batch = evaluator.process(&line("pid:1", "buff on"), start);
        assert!(batch.timers_changed);
        let frame = evaluator.frame();
        assert_eq!(frame.snapshots.len(), 1);
        assert_eq!(frame.snapshots[0].character, "pid:1");

        // Nothing due yet.
        assert!(evaluator
            .advance(start + Duration::from_millis(500))
            .is_empty());
        let ended = evaluator.advance(start + Duration::from_millis(1_001));
        assert!(ended.timers_changed);
        assert!(matches!(
            &ended.events[0].action,
            TriggerAction::Speak { text, .. } if text == "buff over"
        ));
        assert!(evaluator.frame().snapshots.is_empty());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn qa_trigger_only_matches_the_originating_clients_local_echo() {
        let mut library = eqtrigger::TriggerLibrary::new();
        library.triggers.push(qa_timer_trigger());
        let mut evaluator =
            TriggerEvaluator::new(Arc::new(CompiledLibrary::compile(&library)), Instant::now());
        let now = Instant::now();
        let own = evaluator.process(&line("pid:1", "You say, 'Stonemite timer'"), now);
        assert!(own.timers_changed);
        let remote = evaluator.process(&line("pid:2", "Kafka says, 'Stonemite timer'"), now);
        assert!(remote.is_empty());
    }
}
