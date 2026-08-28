use std::sync::Arc;
use std::time::Instant;

use tokio::sync::broadcast;

use eqcombat::{
    CombatEngine, CombatPolicy, CombatRecord, EncounterBookSnapshot, EngineInput, EngineUpdate,
    GapReason, MonoTime, PublishUrgency,
};
use eqlog::{
    LogSource, ParsedLogEvent, ParserRegistry, RawLogLine, SourceRecordId, TelemetryChange,
    TelemetryReducer,
};

use super::diagnostic::{DiagnosticKind, LogDiagnostic};
use super::triggers::{TriggerActivation, TriggerEngine};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct LogEnvelope {
    pub sequence: u64,
    pub record_id: SourceRecordId,
    /// Monotonic receipt coordinate captured by the log worker.
    pub observed_at: Instant,
    pub raw: RawLogLine,
    pub events: Arc<[ParsedLogEvent]>,
    pub telemetry_changes: Arc<[TelemetryChange]>,
    pub trigger_activations: Arc<[TriggerActivation]>,
}

pub(crate) struct PipelineOutcome {
    pub envelope: Arc<LogEnvelope>,
    pub dps_snapshot: Option<Arc<EncounterBookSnapshot>>,
    pub diagnostics: Vec<LogDiagnostic>,
}

pub(crate) struct LogPipeline {
    parsers: ParserRegistry,
    telemetry: TelemetryReducer,
    triggers: TriggerEngine,
    combat: CombatEngine,
    monotonic_origin: Instant,
    next_sequence: u64,
    #[cfg(test)]
    legacy_source_sequences: std::collections::HashMap<eqlog::LogSourceId, u64>,
    pending_snapshot: Option<Arc<EncounterBookSnapshot>>,
    pending_urgency: PublishUrgency,
    last_snapshot_publication: Option<MonoTime>,
    last_combat_diagnostic: Option<Arc<str>>,
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
            combat: CombatEngine::new(CombatPolicy::mvp_v1())
                .expect("built-in combat policy is valid"),
            monotonic_origin: Instant::now(),
            next_sequence: 0,
            #[cfg(test)]
            legacy_source_sequences: std::collections::HashMap::new(),
            pending_snapshot: None,
            pending_urgency: PublishUrgency::None,
            last_snapshot_publication: None,
            last_combat_diagnostic: None,
            event_bus,
        }
    }

    /// Compatibility helper for focused pipeline tests and non-tailer callers.
    /// Production ingestion uses `process_record` with tailer-owned provenance.
    #[cfg(test)]
    pub fn process(&mut self, raw: RawLogLine) -> PipelineOutcome {
        let sequence = self
            .legacy_source_sequences
            .entry(raw.source.id.clone())
            .or_insert(0);
        let id = SourceRecordId::new(raw.source.id.clone(), 0, *sequence);
        *sequence = sequence.saturating_add(1);
        self.process_record(id, raw, Instant::now())
    }

    pub fn process_record(
        &mut self,
        record_id: SourceRecordId,
        raw: RawLogLine,
        receipt: Instant,
    ) -> PipelineOutcome {
        let parsed = self.parsers.parse(&raw);
        let mut diagnostics: Vec<_> = parsed
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
        let combat_events = parsed
            .events
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>();
        let now = self.mono(receipt);
        let update = self.combat.apply(
            now,
            EngineInput::Record(CombatRecord::new(
                record_id.clone(),
                raw.source.clone(),
                raw.timestamp.as_ref().and_then(eqlog::EqTimestamp::second),
                combat_events,
            )),
        );
        let dps_snapshot = self.accept_combat_update(update, now);

        let envelope = Arc::new(LogEnvelope {
            sequence: self.next_sequence,
            record_id,
            observed_at: receipt,
            raw,
            events: parsed.events.into(),
            telemetry_changes: telemetry_changes.into(),
            trigger_activations: trigger_activations.into(),
        });
        self.next_sequence = self.next_sequence.wrapping_add(1);
        // The reducers, trigger engine, combat engine, and UI delivery are the
        // authoritative synchronous path. Optional observers remain bounded
        // and explicitly lossy through broadcast lag.
        let _ = self.event_bus.send(envelope.clone());

        if let Some(message) = self.combat.diagnostics().last().cloned() {
            if self.last_combat_diagnostic.as_ref() != Some(&message) {
                diagnostics.push(LogDiagnostic::new(
                    DiagnosticKind::Combat,
                    None,
                    message.to_string(),
                ));
                self.last_combat_diagnostic = Some(message);
            }
        }
        PipelineOutcome {
            envelope,
            dps_snapshot,
            diagnostics,
        }
    }

    pub fn register_source(
        &mut self,
        source: LogSource,
        generation: u64,
        receipt: Instant,
    ) -> Option<Arc<EncounterBookSnapshot>> {
        let now = self.mono(receipt);
        let update = self
            .combat
            .apply(now, EngineInput::SourceRegistered { source, generation });
        self.accept_combat_update(update, now)
    }

    pub fn remove_source(
        &mut self,
        source: &LogSource,
        receipt: Instant,
    ) -> Option<Arc<EncounterBookSnapshot>> {
        self.parsers.reset_source(source);
        let now = self.mono(receipt);
        let update = self.combat.apply(
            now,
            EngineInput::SourceRemoved {
                source: source.id.clone(),
            },
        );
        self.accept_combat_update(update, now)
    }

    pub fn source_gap(
        &mut self,
        source: &LogSource,
        generation: u64,
        reason: GapReason,
        receipt: Instant,
    ) -> Option<Arc<EncounterBookSnapshot>> {
        self.parsers.reset_source(source);
        let now = self.mono(receipt);
        let update = self.combat.apply(
            now,
            EngineInput::SourceGap {
                source: source.id.clone(),
                generation,
                reason,
            },
        );
        self.accept_combat_update(update, now)
    }

    pub fn stable_eof(
        &mut self,
        source: eqlog::LogSourceId,
        generation: u64,
        receipt: Instant,
    ) -> Option<Arc<EncounterBookSnapshot>> {
        let now = self.mono(receipt);
        let update = self
            .combat
            .apply(now, EngineInput::SourceStableEof { source, generation });
        self.accept_combat_update(update, now)
    }

    pub fn tick(&mut self, receipt: Instant) -> Option<Arc<EncounterBookSnapshot>> {
        let now = self.mono(receipt);
        let update = self.combat.tick(now);
        self.accept_combat_update(update, now)
    }

    fn mono(&self, receipt: Instant) -> MonoTime {
        MonoTime::from_millis(
            receipt
                .saturating_duration_since(self.monotonic_origin)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        )
    }

    fn accept_combat_update(
        &mut self,
        update: EngineUpdate,
        now: MonoTime,
    ) -> Option<Arc<EncounterBookSnapshot>> {
        if let Some(snapshot) = update.snapshot {
            self.pending_snapshot = Some(snapshot);
            if update.urgency == PublishUrgency::Immediate
                || self.pending_urgency == PublishUrgency::None
            {
                self.pending_urgency = update.urgency;
            }
        }
        let due = self
            .last_snapshot_publication
            .is_none_or(|last| now.saturating_duration_since(last) >= 250);
        if self.pending_snapshot.is_some()
            && (self.pending_urgency == PublishUrgency::Immediate || due)
        {
            self.last_snapshot_publication = Some(now);
            self.pending_urgency = PublishUrgency::None;
            return self.pending_snapshot.take();
        }
        None
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

        let trade = pipeline
            .process(raw("Kafka is interested in making a trade."))
            .envelope;
        assert!(matches!(
            &trade.events[0].event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::TradeProposed { trader })
                if trader.as_ref() == "Kafka"
        ));

        let trade_cancelled = pipeline
            .process(raw("Kafka has cancelled the trade."))
            .envelope;
        assert!(matches!(
            &trade_cancelled.events[0].event,
            eqlog::LogEvent::Notification(eqlog::NotificationEvent::TradeCancelled)
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
    fn progression_events_keep_source_attribution() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);

        let level = pipeline
            .process(raw("You have gained a level! Welcome to level 125!"))
            .envelope;
        assert_eq!(level.raw.source.id.as_str(), "client-1");
        assert!(matches!(
            &level.events[0].event,
            eqlog::LogEvent::Progress(eqlog::ProgressEvent::LevelGained { level: 125 })
        ));

        let aa = pipeline
            .process(raw(
                "You have gained an ability point! You now have 250 ability points.",
            ))
            .envelope;
        assert!(matches!(
            &aa.events[0].event,
            eqlog::LogEvent::Progress(eqlog::ProgressEvent::AlternateAdvancementPointGained)
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

        let returned = pipeline
            .process(raw("You have entered The Nexus."))
            .envelope;
        assert_eq!(returned.telemetry_changes.len(), 1);
        assert!(!returned.telemetry_changes[0].telemetry.dead);
    }

    #[test]
    fn combat_activity_and_positioning_keep_source_attribution() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);

        let hit = pipeline
            .process(raw("You slash a wan ghoul knight for 28 points of damage."))
            .envelope;
        assert_eq!(hit.raw.source.id.as_str(), "client-1");
        assert!(matches!(
            hit.events[0].event,
            eqlog::LogEvent::Combat(eqlog::CombatEvent::WeaponDamageDealt)
        ));
        assert!(hit.events.iter().any(|event| matches!(
            event.event,
            eqlog::LogEvent::Combat(eqlog::CombatEvent::Damage(_))
        )));

        let blocked = pipeline
            .process(raw("You cannot see your target."))
            .envelope;
        assert!(matches!(
            blocked.events[0].event,
            eqlog::LogEvent::Combat(eqlog::CombatEvent::AttackBlocked(
                eqlog::AttackProblem::LineOfSight
            ))
        ));
        assert!(blocked.telemetry_changes.is_empty());
    }

    #[test]
    fn event_ordering_is_monotonic_and_source_record_identity_is_retained() {
        let (sender, _) = broadcast::channel(8);
        let mut pipeline = LogPipeline::new(sender);
        let first = pipeline.process(raw("first")).envelope;
        let second = pipeline.process(raw("second")).envelope;
        let third = pipeline.process(raw("third")).envelope;

        assert_eq!([first.sequence, second.sequence, third.sequence], [0, 1, 2]);
        assert_eq!(
            [
                first.record_id.sequence,
                second.record_id.sequence,
                third.record_id.sequence,
            ],
            [0, 1, 2]
        );
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
