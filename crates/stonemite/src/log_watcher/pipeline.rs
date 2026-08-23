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
        #[allow(unused_mut)]
        let mut triggers = TriggerEngine::new();
        #[cfg(debug_assertions)]
        {
            let errors = triggers.replace_definitions(vec![super::triggers::qa_timer_definition()]);
            debug_assert!(errors.is_empty());
        }
        Self {
            parsers: ParserRegistry::default(),
            telemetry: TelemetryReducer::new(),
            triggers,
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

    #[cfg(debug_assertions)]
    #[test]
    fn development_qa_phrase_starts_a_visible_timer_action() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let envelope = pipeline.process(raw("You say, 'Stonemite timer'")).envelope;

        assert_eq!(envelope.trigger_activations.len(), 1);
        assert_eq!(
            envelope.trigger_activations[0].scope,
            super::super::TriggerScope::AllClients
        );
        assert_eq!(
            envelope.trigger_activations[0].source.id.as_str(),
            "client-1"
        );
        assert!(matches!(
            &envelope.trigger_activations[0].presentation[0],
            super::super::PresentationAction::StartTimer(request)
                if request.label.as_ref() == "QA timer"
                    && request.duration == std::time::Duration::from_secs(10)
        ));

        let remote = pipeline
            .process(raw("Kafka says, 'Stonemite timer'"))
            .envelope;
        assert!(remote.trigger_activations.is_empty());
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
    fn incoming_tell_keeps_source_attribution_and_structured_content() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let envelope = pipeline
            .process(raw("Laika tells you, 'I'm busy right now.'"))
            .envelope;

        assert_eq!(envelope.raw.source.id.as_str(), "client-1");
        assert!(matches!(
            &envelope.events[0].event,
            eqlog::LogEvent::Chat(eqlog::ChatEvent::IncomingTell(tell))
                if tell.sender.as_ref() == "Laika" && tell.message.as_ref() == "I'm busy right now."
        ));
        assert!(envelope.telemetry_changes.is_empty());
    }

    #[test]
    fn invitation_and_resurrection_notifications_keep_source_attribution() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);

        let group = pipeline
            .process(raw("Laika invites you to join a group."))
            .envelope;
        assert_eq!(group.raw.source.id.as_str(), "client-1");
        assert!(matches!(
            &group.events[0].event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::GroupInvite { inviter })
                if inviter.as_ref() == "Laika"
        ));

        let accepted = pipeline.process(raw("You have joined the group.")).envelope;
        assert!(matches!(
            &accepted.events[0].event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::GroupInviteAccepted)
        ));

        let declined = pipeline
            .process(raw("You cancel the invitation to join Laika's group."))
            .envelope;
        assert!(matches!(
            &declined.events[0].event,
            eqlog::LogEvent::Notification(
                eqlog::NotificationEvent::GroupInviteDeclined { inviter }
            ) if inviter.as_ref() == "Laika"
        ));

        let resurrection = pipeline
            .process(raw("You have been offered a resurrection."))
            .envelope;
        assert!(matches!(
            &resurrection.events[0].event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::ResurrectionOffered)
        ));
    }

    #[test]
    fn death_notifies_and_updates_persistent_character_state() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let envelope = pipeline
            .process(raw("You have been slain by a War Swarm invader!"))
            .envelope;

        assert!(envelope.events.iter().any(|event| matches!(
            &event.event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::CharacterSlain { killer })
                if killer.as_ref() == "a War Swarm invader"
        )));
        assert_eq!(envelope.telemetry_changes.len(), 1);
        assert!(envelope.telemetry_changes[0].telemetry.dead);
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
