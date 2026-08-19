use std::sync::Arc;

use tokio::sync::broadcast;

use eqlog::{
    LogSource, ParsedLogEvent, ParserRegistry, RawLogLine, TelemetryChange, TelemetryReducer,
};

use super::diagnostic::{DiagnosticKind, LogDiagnostic};
use super::triggers::{TriggerActivation, TriggerEngine};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LogEnvelope {
    pub sequence: u64,
    pub raw: RawLogLine,
    pub events: Arc<[ParsedLogEvent]>,
    pub telemetry_changes: Arc<[TelemetryChange]>,
    pub trigger_activations: Arc<[TriggerActivation]>,
}

pub(crate) struct PipelineOutcome {
    pub envelope: Arc<LogEnvelope>,
    pub diagnostics: Vec<LogDiagnostic>,
}

pub(crate) struct LogPipeline {
    parsers: ParserRegistry,
    telemetry: TelemetryReducer,
    triggers: TriggerEngine,
    next_sequence: u64,
    event_bus: broadcast::Sender<Arc<LogEnvelope>>,
}

impl LogPipeline {
    pub fn new(event_bus: broadcast::Sender<Arc<LogEnvelope>>) -> Self {
        Self {
            parsers: ParserRegistry::default(),
            telemetry: TelemetryReducer::new(),
            triggers: TriggerEngine::new(),
            next_sequence: 0,
            event_bus,
        }
    }

    pub fn process(&mut self, raw: RawLogLine) -> PipelineOutcome {
        let parsed = self.parsers.parse(&raw);
        let diagnostics = parsed
            .errors
            .iter()
            .map(|failure| {
                LogDiagnostic::new(
                    DiagnosticKind::Parser,
                    None,
                    format!("{} parser failed: {}", failure.parser, failure.message),
                )
            })
            .collect();
        let telemetry_changes: Vec<_> = parsed
            .events
            .iter()
            .filter_map(|event| self.telemetry.apply(event))
            .collect();
        let trigger_activations = self.triggers.evaluate(&raw, &parsed.events);

        let envelope = Arc::new(LogEnvelope {
            sequence: self.next_sequence,
            raw,
            events: parsed.events.into(),
            telemetry_changes: telemetry_changes.into(),
            trigger_activations: trigger_activations.into(),
        });
        self.next_sequence = self.next_sequence.wrapping_add(1);
        // The reducer, trigger engine, and UI delivery are the authoritative
        // synchronous path. Optional observers use a bounded broadcast queue;
        // lag is explicit through tokio's `RecvError::Lagged`.
        let _ = self.event_bus.send(envelope.clone());

        PipelineOutcome {
            envelope,
            diagnostics,
        }
    }

    pub fn reset_source(&mut self, source: &LogSource) {
        self.parsers.reset_source(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(body: &str) -> RawLogLine {
        RawLogLine {
            source: LogSource::new("client-1", "Bilka", "teek"),
            timestamp: None,
            body: Arc::from(body),
        }
    }

    #[test]
    fn unknown_lines_pass_through_without_becoming_parser_errors() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let outcome = pipeline.process(raw("an unknown future EQ line"));

        assert_eq!(
            outcome.envelope.raw.body.as_ref(),
            "an unknown future EQ line"
        );
        assert!(outcome.envelope.events.is_empty());
        assert!(outcome.diagnostics.is_empty());
    }

    #[test]
    fn event_ordering_is_monotonic_for_one_source() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let first = pipeline.process(raw("first")).envelope;
        let second = pipeline.process(raw("second")).envelope;
        let third = pipeline.process(raw("third")).envelope;

        assert_eq!([first.sequence, second.sequence, third.sequence], [0, 1, 2]);
        assert_eq!(
            [
                first.raw.body.as_ref(),
                second.raw.body.as_ref(),
                third.raw.body.as_ref()
            ],
            ["first", "second", "third"]
        );
    }
}
