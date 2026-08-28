use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::Arc;

use eqlog::{
    CombatEvent, DamageKind, DamageObservation, DamageOutcome, IdentityEvent, LogEvent,
    ObservedCombatant, Perspective, PetEvent,
};

use crate::metrics::{inclusive_seconds, ratio_millionths, round_rate, union_range_seconds};
use crate::model::*;

#[derive(Clone)]
struct SourceState {
    source: LogSource,
    generation: u64,
    present: bool,
    registered_at: MonoTime,
    continuity: u64,
    observed: Option<EqSecond>,
    closed_through: Option<EqSecond>,
    history_floor: Option<EqSecond>,
    last_progress_at: MonoTime,
    last_sequence: Option<u64>,
    eof_second: Option<EqSecond>,
    eof_first_at: Option<MonoTime>,
    eof_passes: u8,
    zone: Option<Arc<str>>,
}

#[derive(Clone)]
struct OwnershipClaim {
    server: Arc<str>,
    pet: Arc<str>,
    owner: ParticipantId,
    owner_display: Arc<str>,
    valid_from: EqSecond,
    source: LogSourceId,
}

#[derive(Clone)]
struct LedgerFact {
    _record: SourceRecordId,
    source: LogSource,
    eq_time: EqSecond,
    receipt: MonoTime,
    event: FactEvent,
}

#[derive(Clone)]
enum FactEvent {
    Damage(DamageObservation),
    Attempt(eqlog::CombatAttempt),
    Slain(eqlog::TargetSlainObservation),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Fingerprint {
    eq_time: EqSecond,
    attacker: Arc<str>,
    defender: Arc<str>,
    amount: u64,
    kind: DamageKind,
    ability: Option<Arc<str>>,
}

#[derive(Clone)]
struct TargetState {
    display: Arc<str>,
    terminal: bool,
    first_damage: EqSecond,
    last_damage: EqSecond,
    terminal_at: Option<EqSecond>,
}

#[derive(Clone)]
enum PhaseState {
    Active,
    Ending {
        reason: EndReason,
        entered_at: MonoTime,
        cutoff: EqSecond,
        closure_sources: BTreeSet<LogSourceId>,
    },
    Held {
        reason: EndReason,
        until: MonoTime,
    },
}

#[derive(Clone)]
struct EncounterState {
    id: EncounterId,
    server: Arc<str>,
    phase: PhaseState,
    sources: BTreeSet<LogSourceId>,
    source_continuity: BTreeMap<LogSourceId, u64>,
    partial_sources: BTreeSet<LogSourceId>,
    managed: BTreeSet<ParticipantId>,
    targets: BTreeMap<CanonicalTargetId, TargetState>,
    facts: Vec<usize>,
    fingerprints: BTreeSet<Fingerprint>,
    start_eq: EqSecond,
    end_eq: EqSecond,
    last_sustain_eq: EqSecond,
    start_receipt: MonoTime,
    last_sustain_receipt: MonoTime,
    revision: u64,
    last_snapshot: Option<Arc<EncounterSnapshot>>,
    frozen: Option<Arc<EncounterSnapshot>>,
}

#[derive(Clone)]
struct Candidate {
    participant: ParticipantId,
    display: Arc<str>,
    source: LogSourceId,
    complete: bool,
    personal: bool,
    provisional_pet: bool,
    has_pet: bool,
    direct_damage: u128,
    pet_damage: u128,
    damage: u128,
    events: u64,
    ranges: BTreeMap<CanonicalTargetId, (EqSecond, EqSecond)>,
    target_damage: BTreeMap<CanonicalTargetId, u128>,
    target_first: BTreeMap<CanonicalTargetId, EqSecond>,
    active_seconds: u64,
}

#[derive(Clone)]
struct Election {
    winner: Candidate,
    quality: SourceQuality,
    losers: Vec<(LogSourceId, u128)>,
}

pub struct CombatEngine {
    policy: CombatPolicy,
    sources: BTreeMap<LogSourceId, SourceState>,
    accepted: HashSet<SourceRecordId>,
    accepted_order: VecDeque<SourceRecordId>,
    occurrences: BTreeMap<(LogSourceId, u64, EqSecond, Arc<str>), u64>,
    ledger: Vec<LedgerFact>,
    assignments: BTreeMap<usize, EncounterId>,
    encounters: BTreeMap<EncounterId, EncounterState>,
    source_active: BTreeMap<LogSourceId, EncounterId>,
    verified_players: BTreeSet<ParticipantId>,
    ownership: Vec<OwnershipClaim>,
    current_book: Arc<EncounterBookSnapshot>,
    now: MonoTime,
    diagnostics: VecDeque<Arc<str>>,
}

impl CombatEngine {
    pub fn new(policy: CombatPolicy) -> Result<Self, PolicyError> {
        policy.validate()?;
        Ok(Self {
            policy,
            sources: BTreeMap::new(),
            accepted: HashSet::new(),
            accepted_order: VecDeque::new(),
            occurrences: BTreeMap::new(),
            ledger: Vec::new(),
            assignments: BTreeMap::new(),
            encounters: BTreeMap::new(),
            source_active: BTreeMap::new(),
            verified_players: BTreeSet::new(),
            ownership: Vec::new(),
            current_book: Arc::new(EncounterBookSnapshot::default()),
            now: MonoTime::ZERO,
            diagnostics: VecDeque::new(),
        })
    }

    pub fn apply(&mut self, now: MonoTime, input: EngineInput) -> EngineUpdate {
        self.now = self.now.max(now);
        let lifecycle_before = self.lifecycle_signature();
        let mut urgency = PublishUrgency::Coalescible;
        match input {
            EngineInput::SourceRegistered { source, generation } => {
                urgency = PublishUrgency::Immediate;
                self.register_source(source, generation, now);
            }
            EngineInput::SourceRemoved { source } => {
                urgency = PublishUrgency::Immediate;
                if let Some(state) = self.sources.get_mut(&source) {
                    state.present = false;
                }
                for encounter in self.encounters.values_mut() {
                    if encounter.sources.contains(&source)
                        && !matches!(encounter.phase, PhaseState::Held { .. })
                    {
                        encounter.partial_sources.insert(source.clone());
                    }
                }
            }
            EngineInput::SourceGap {
                source,
                generation,
                reason,
            } => {
                urgency = PublishUrgency::Immediate;
                self.mark_gap(&source, generation, reason);
            }
            EngineInput::SourceStableEof { source, generation } => {
                self.note_stable_eof(&source, generation, now);
            }
            EngineInput::Record(record) => {
                if !self.apply_record(record, now) {
                    urgency = PublishUrgency::None;
                }
            }
        }
        self.advance_time(self.now, &mut urgency);
        if self.lifecycle_signature() != lifecycle_before {
            urgency = PublishUrgency::Immediate;
        }
        let snapshot = self.rebuild_book();
        if snapshot.is_none() && urgency != PublishUrgency::None {
            urgency = PublishUrgency::None;
        }
        EngineUpdate {
            snapshot,
            urgency,
            next_deadline: self.next_deadline(),
        }
    }

    pub fn tick(&mut self, now: MonoTime) -> EngineUpdate {
        self.now = self.now.max(now);
        let mut urgency = PublishUrgency::None;
        self.advance_time(self.now, &mut urgency);
        let snapshot = self.rebuild_book();
        if snapshot.is_some() && urgency == PublishUrgency::None {
            urgency = PublishUrgency::Coalescible;
        }
        EngineUpdate {
            snapshot,
            urgency,
            next_deadline: self.next_deadline(),
        }
    }

    pub fn snapshot(&self) -> Arc<EncounterBookSnapshot> {
        self.current_book.clone()
    }

    pub fn next_deadline(&self) -> Option<MonoTime> {
        let encounter_deadlines = self
            .encounters
            .values()
            .filter_map(|encounter| match &encounter.phase {
                PhaseState::Active => Some(
                    encounter
                        .last_sustain_receipt
                        .saturating_add(self.policy.inactivity_ms),
                ),
                PhaseState::Ending { entered_at, .. } => {
                    Some(entered_at.saturating_add(self.policy.ending_grace_ms))
                }
                PhaseState::Held { until, .. } => Some(*until),
            });
        let eof_deadlines = self.sources.values().filter_map(|source| {
            (source.eof_passes >= 2)
                .then(|| {
                    source
                        .eof_first_at
                        .map(|at| at.saturating_add(self.policy.stable_eof_ms))
                })
                .flatten()
        });
        encounter_deadlines.chain(eof_deadlines).min()
    }

    pub fn has_ending_grace(&self) -> bool {
        self.encounters
            .values()
            .any(|encounter| matches!(encounter.phase, PhaseState::Ending { .. }))
    }

    pub fn diagnostics(&self) -> Arc<[Arc<str>]> {
        self.diagnostics.iter().cloned().collect::<Vec<_>>().into()
    }

    pub fn explain(&self, encounter_id: &EncounterId) -> Option<EncounterExplanation> {
        let encounter = self.encounters.get(encounter_id)?;
        let elections = self.elect(encounter)?;
        let candidates = elections
            .into_values()
            .map(|election| CandidateExplanation {
                participant: election.winner.participant,
                elected_source: election.winner.source,
                quality: election.quality,
                damage: election.winner.damage,
                direct_damage: election.winner.direct_damage,
                pet_damage: election.winner.pet_damage,
                losing_sources: election.losers.into(),
            })
            .collect::<Vec<_>>();
        Some(EncounterExplanation {
            encounter: encounter_id.clone(),
            candidates: candidates.into(),
            diagnostics: self.diagnostics(),
        })
    }

    fn lifecycle_signature(&self) -> Vec<(EncounterId, u8)> {
        self.encounters
            .values()
            .map(|encounter| {
                let phase = match encounter.phase {
                    PhaseState::Active => 0,
                    PhaseState::Ending { .. } => 1,
                    PhaseState::Held { .. } => 2,
                };
                (encounter.id.clone(), phase)
            })
            .collect()
    }

    fn register_source(&mut self, source: LogSource, generation: u64, now: MonoTime) {
        let managed = ParticipantId::new(&source.server, &source.character);
        self.verified_players.insert(managed.clone());
        match self.sources.get_mut(&source.id) {
            Some(existing) => {
                let discontinuity = existing.generation != generation
                    || existing.source != source
                    || !existing.present;
                if discontinuity {
                    existing.history_floor = existing.history_floor.max(existing.closed_through);
                    existing.continuity = existing.continuity.wrapping_add(1);
                    existing.generation = generation;
                    existing.last_sequence = None;
                    existing.observed = None;
                    existing.closed_through = None;
                    existing.eof_second = None;
                    existing.eof_first_at = None;
                    existing.eof_passes = 0;
                    existing.registered_at = now;
                    existing.last_progress_at = now;
                }
                existing.source = source;
                existing.present = true;
            }
            None => {
                self.sources.insert(
                    source.id.clone(),
                    SourceState {
                        source,
                        generation,
                        present: true,
                        registered_at: now,
                        continuity: 0,
                        observed: None,
                        closed_through: None,
                        history_floor: None,
                        last_progress_at: now,
                        last_sequence: None,
                        eof_second: None,
                        eof_first_at: None,
                        eof_passes: 0,
                        zone: None,
                    },
                );
            }
        }
        for encounter in self.encounters.values_mut() {
            if !matches!(encounter.phase, PhaseState::Held { .. })
                && encounter.server.eq_ignore_ascii_case(&managed.server)
            {
                encounter.managed.insert(managed.clone());
            }
        }
    }

    fn mark_gap(&mut self, source: &LogSourceId, generation: u64, reason: GapReason) {
        if let Some(state) = self.sources.get_mut(source) {
            state.history_floor = state.history_floor.max(state.closed_through);
            state.continuity = state.continuity.wrapping_add(1);
            state.generation = generation;
            state.last_sequence = None;
            state.eof_second = None;
            state.eof_first_at = None;
            state.eof_passes = 0;
            if matches!(
                reason,
                GapReason::FileRecreated
                    | GapReason::FileTruncated
                    | GapReason::BoundaryChanged
                    | GapReason::SourceReassociated
                    | GapReason::GenerationChanged
            ) {
                state.observed = None;
                state.closed_through = None;
            }
        }
        for encounter in self.encounters.values_mut() {
            if encounter.sources.contains(source)
                && !matches!(encounter.phase, PhaseState::Held { .. })
            {
                encounter.partial_sources.insert(source.clone());
            }
        }
        self.diagnose(format!(
            "source {} became partial: {reason:?}",
            source.as_str()
        ));
    }

    fn note_stable_eof(&mut self, source: &LogSourceId, generation: u64, now: MonoTime) {
        let Some(state) = self.sources.get_mut(source) else {
            return;
        };
        if state.generation != generation {
            return;
        }
        let Some(observed) = state.observed else {
            return;
        };
        if state.eof_second == Some(observed) {
            state.eof_passes = state.eof_passes.saturating_add(1);
        } else {
            state.eof_second = Some(observed);
            state.eof_first_at = Some(now);
            state.eof_passes = 1;
        }
    }

    fn apply_record(&mut self, record: CombatRecord, now: MonoTime) -> bool {
        if record.id.source != record.source.id {
            self.diagnose("rejected record whose source ID disagreed with its descriptor");
            return false;
        }
        if self.accepted.contains(&record.id) {
            self.diagnose(format!(
                "ignored duplicate record {}:{}:{}",
                record.id.source.as_str(),
                record.id.generation,
                record.id.sequence
            ));
            return false;
        }
        if !self.sources.contains_key(&record.source.id) {
            self.register_source(record.source.clone(), record.id.generation, now);
        }

        let mut gap = None;
        let mut replayed_sequence = false;
        {
            let state = self
                .sources
                .get(&record.source.id)
                .expect("registered source");
            if record.id.generation < state.generation {
                self.diagnose("ignored a record from an obsolete source generation");
                return false;
            }
            if record.id.generation > state.generation {
                gap = Some(GapReason::GenerationChanged);
            } else if state
                .last_sequence
                .is_some_and(|sequence| record.id.sequence <= sequence)
            {
                replayed_sequence = true;
            }
            if let Some(eq_time) = record.eq_time {
                if state.observed.is_some_and(|observed| eq_time < observed) {
                    gap = Some(GapReason::CalendarRegression);
                } else if state.closed_through.is_some_and(|closed| eq_time <= closed) {
                    gap = Some(GapReason::OutOfOrderClosedSecond);
                }
            }
        }
        if replayed_sequence {
            self.diagnose("ignored replayed or out-of-order source sequence");
            return false;
        }
        if let Some(reason) = gap {
            self.mark_gap(&record.source.id, record.id.generation, reason);
        }

        self.accepted.insert(record.id.clone());
        self.accepted_order.push_back(record.id.clone());
        while self.accepted_order.len() > self.policy.max_record_ids {
            if let Some(expired) = self.accepted_order.pop_front() {
                self.accepted.remove(&expired);
            }
        }
        {
            let state = self
                .sources
                .get_mut(&record.source.id)
                .expect("registered source");
            state.generation = record.id.generation;
            state.last_sequence = Some(record.id.sequence);
            state.present = true;
            if let Some(eq_time) = record.eq_time {
                if state.observed.is_none_or(|observed| eq_time > observed) {
                    if let Some(previous) = state.observed {
                        let close = eq_time.checked_add(-1).unwrap_or(previous);
                        state.closed_through = Some(
                            state
                                .closed_through
                                .map_or(close, |current| current.max(close)),
                        );
                    }
                    state.observed = Some(eq_time);
                    state.last_progress_at = now;
                }
                state.eof_second = None;
                state.eof_first_at = None;
                state.eof_passes = 0;
            }
        }

        let Some(eq_time) = record.eq_time else {
            if record
                .events
                .iter()
                .any(|event| matches!(event, LogEvent::Combat(CombatEvent::Damage(_))))
            {
                self.diagnose("timestamp-free damage was retained diagnostically and excluded");
            }
            return true;
        };
        if self
            .sources
            .get(&record.source.id)
            .and_then(|source| source.history_floor)
            .is_some_and(|floor| eq_time <= floor)
        {
            self.diagnose("ignored replay history at or before the prior closed-through second");
            return true;
        }

        let incoming_facts = record
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    LogEvent::Combat(
                        CombatEvent::Damage(_)
                            | CombatEvent::Attempt(_)
                            | CombatEvent::TargetSlain(_)
                    )
                )
            })
            .count();
        if self.ledger.len().saturating_add(incoming_facts) > self.policy.max_ledger_facts {
            self.diagnose("combat ledger bound reached; withdrew live encounters fail-closed");
            self.encounters.clear();
            self.source_active.clear();
            self.ledger.clear();
            self.assignments.clear();
        }

        for event in record.events.iter() {
            if matches!(
                event,
                LogEvent::Combat(
                    CombatEvent::Damage(_) | CombatEvent::Attempt(_) | CombatEvent::TargetSlain(_)
                )
            ) && self.is_replayed_occurrence(&record.id, eq_time, event)
            {
                continue;
            }
            match event {
                LogEvent::Identity(IdentityEvent::WhoResult(result)) => {
                    self.verified_players
                        .insert(ParticipantId::new(&record.source.server, &result.character));
                }
                LogEvent::Pet(PetEvent::OwnershipClaimed { pet, owner }) => {
                    self.add_ownership(
                        &record.source,
                        pet,
                        &ObservedCombatant::named(owner.clone()),
                        eq_time,
                    );
                }
                LogEvent::Combat(CombatEvent::PetOwnership(ownership)) => {
                    self.add_ownership(
                        &record.source,
                        &ownership.pet.name,
                        &ownership.owner,
                        eq_time,
                    );
                }
                LogEvent::Combat(CombatEvent::PlayerEvidence(evidence)) => {
                    let (_, player) = self.resolve_observed(&record.source, &evidence.player);
                    self.verified_players.insert(player);
                }
                LogEvent::Combat(CombatEvent::ZoneChanged(zone)) => {
                    if let Some(state) = self.sources.get_mut(&record.source.id) {
                        state.zone = Some(zone.zone.clone());
                    }
                }
                LogEvent::Combat(CombatEvent::Damage(damage)) => {
                    let index = self.ledger.len();
                    self.ledger.push(LedgerFact {
                        _record: record.id.clone(),
                        source: record.source.clone(),
                        eq_time,
                        receipt: now,
                        event: FactEvent::Damage(damage.clone()),
                    });
                    self.process_damage(index);
                }
                LogEvent::Combat(CombatEvent::Attempt(attempt)) => {
                    let index = self.ledger.len();
                    self.ledger.push(LedgerFact {
                        _record: record.id.clone(),
                        source: record.source.clone(),
                        eq_time,
                        receipt: now,
                        event: FactEvent::Attempt(attempt.clone()),
                    });
                    self.process_attempt(index);
                }
                LogEvent::Combat(CombatEvent::TargetSlain(slain)) => {
                    let index = self.ledger.len();
                    self.ledger.push(LedgerFact {
                        _record: record.id.clone(),
                        source: record.source.clone(),
                        eq_time,
                        receipt: now,
                        event: FactEvent::Slain(slain.clone()),
                    });
                    self.process_slain(index);
                }
                _ => {}
            }
        }
        true
    }

    fn is_replayed_occurrence(
        &mut self,
        record: &SourceRecordId,
        eq_time: EqSecond,
        event: &LogEvent,
    ) -> bool {
        let content: Arc<str> = Arc::from(format!("{event:?}"));
        let key = (
            record.source.clone(),
            record.generation,
            eq_time,
            content.clone(),
        );
        if self.occurrences.len() >= self.policy.max_record_ids
            && !self.occurrences.contains_key(&key)
        {
            self.occurrences.clear();
        }
        let occurrence = self.occurrences.entry(key).or_insert(0);
        *occurrence = occurrence.saturating_add(1);
        let occurrence = *occurrence;
        let prior_max = self
            .occurrences
            .iter()
            .filter(|((source, generation, second, candidate), _)| {
                source == &record.source
                    && *generation < record.generation
                    && *second == eq_time
                    && candidate.as_ref() == content.as_ref()
            })
            .map(|(_, count)| *count)
            .max()
            .unwrap_or(0);
        occurrence <= prior_max
    }

    fn add_ownership(
        &mut self,
        source: &LogSource,
        pet: &str,
        owner: &ObservedCombatant,
        eq_time: EqSecond,
    ) {
        let (owner_display, owner) = self.resolve_observed(source, owner);
        self.verified_players.insert(owner.clone());
        self.ownership.push(OwnershipClaim {
            server: Arc::from(source.server.to_ascii_lowercase()),
            pet: Arc::from(pet.to_ascii_lowercase()),
            owner,
            owner_display: Arc::from(owner_display),
            valid_from: eq_time,
            source: source.id.clone(),
        });
        self.ownership.sort_by(|left, right| {
            (
                left.server.as_ref(),
                left.pet.as_ref(),
                left.valid_from,
                left.source.as_str(),
            )
                .cmp(&(
                    right.server.as_ref(),
                    right.pet.as_ref(),
                    right.valid_from,
                    right.source.as_str(),
                ))
        });
    }

    fn process_damage(&mut self, fact_index: usize) {
        let fact = self.ledger[fact_index].clone();
        let FactEvent::Damage(damage) = &fact.event else {
            return;
        };
        let (attacker_display, attacker) = self.resolve_observed(&fact.source, &damage.attacker);
        let (defender_display, defender_participant) =
            self.resolve_observed(&fact.source, &damage.defender);
        let explicit_owner = damage
            .explicit_owner
            .as_ref()
            .map(|owner| self.resolve_observed(&fact.source, owner));
        let owner = explicit_owner
            .as_ref()
            .map(|(_, owner)| owner.clone())
            .or_else(|| self.claimed_owner(&fact.source, &attacker_display, fact.eq_time));
        let participant = owner.clone().unwrap_or_else(|| attacker.clone());
        let attacker_verified = owner.is_some()
            || self.verified_players.contains(&attacker)
            || damage.attacker.perspective != Perspective::Named;
        let target_id = CanonicalTargetId::new(&defender_display);
        let defender_is_player = self.verified_players.contains(&defender_participant);
        let self_attack = participant == defender_participant;
        let positive = damage.amount > 0 && damage.outcome == DamageOutcome::Hit;
        let start_qualified = positive && attacker_verified && !defender_is_player && !self_attack;
        let fingerprint = Fingerprint {
            eq_time: fact.eq_time,
            attacker: Arc::from(attacker.canonical_name.as_ref()),
            defender: target_id.0.clone(),
            amount: damage.amount,
            kind: damage.kind,
            ability: damage
                .ability
                .as_ref()
                .map(|ability| Arc::from(ability.to_ascii_lowercase())),
        };

        let current = self.source_active.get(&fact.source.id).cloned();
        let correlated = self
            .encounters
            .iter()
            .find(|(_, encounter)| {
                !matches!(encounter.phase, PhaseState::Held { .. })
                    && encounter.server.eq_ignore_ascii_case(&fact.source.server)
                    && encounter.fingerprints.contains(&fingerprint)
            })
            .map(|(id, _)| id.clone());
        let mut encounter_id = match (current, correlated) {
            (Some(current), Some(correlated)) if current != correlated => {
                Some(self.merge_encounters(current, correlated))
            }
            (Some(current), _) => Some(current),
            (None, Some(correlated)) => Some(correlated),
            (None, None) => None,
        };

        let mut excluded_from_closure = false;
        if let Some(id) = encounter_id.clone() {
            let phase = self
                .encounters
                .get(&id)
                .map(|encounter| encounter.phase.clone());
            if let Some(PhaseState::Ending {
                reason,
                cutoff,
                closure_sources,
                ..
            }) = phase
            {
                let known_target = self
                    .encounters
                    .get(&id)
                    .is_some_and(|encounter| encounter.targets.contains_key(&target_id));
                let closure_source = closure_sources.contains(&fact.source.id);
                let resumes = closure_source
                    && match reason {
                        EndReason::Inactivity => true,
                        EndReason::AllTargetsTerminal => known_target && fact.eq_time > cutoff,
                    };
                if resumes {
                    if let Some(encounter) = self.encounters.get_mut(&id) {
                        encounter.phase = PhaseState::Active;
                        if let Some(target) = encounter.targets.get_mut(&target_id) {
                            target.terminal = false;
                            target.terminal_at = None;
                        }
                    }
                } else if !closure_source || fact.eq_time > cutoff {
                    excluded_from_closure = !closure_source && fact.eq_time <= cutoff;
                    encounter_id = None;
                }
            }
        }

        if excluded_from_closure {
            return;
        }
        if encounter_id.is_none()
            && self.encounters.values().any(|encounter| {
                matches!(encounter.phase, PhaseState::Held { .. })
                    && encounter.sources.contains(&fact.source.id)
                    && encounter.server.eq_ignore_ascii_case(&fact.source.server)
                    && fact.eq_time <= encounter.end_eq
            })
        {
            self.diagnose("ignored LateAfterClosure combat for an immutable held encounter");
            return;
        }
        if encounter_id.is_none() && start_qualified {
            let id = self.encounter_id(&fact, &target_id, &participant);
            if !self.encounters.contains_key(&id)
                && self.encounters.len() >= self.policy.max_encounters
            {
                self.diagnose("combat encounter bound reached; ignored a new unrelated encounter");
                return;
            }
            if !self.encounters.contains_key(&id) {
                let continuity = self
                    .sources
                    .get(&fact.source.id)
                    .map_or(0, |source| source.continuity);
                let mut sources = BTreeSet::new();
                sources.insert(fact.source.id.clone());
                let mut source_continuity = BTreeMap::new();
                source_continuity.insert(fact.source.id.clone(), continuity);
                let managed = self.managed_identities(&fact.source.server);
                self.encounters.insert(
                    id.clone(),
                    EncounterState {
                        id: id.clone(),
                        server: fact.source.server.clone(),
                        phase: PhaseState::Active,
                        sources,
                        source_continuity,
                        partial_sources: BTreeSet::new(),
                        managed,
                        targets: BTreeMap::new(),
                        facts: Vec::new(),
                        fingerprints: BTreeSet::new(),
                        start_eq: fact.eq_time,
                        end_eq: fact.eq_time,
                        last_sustain_eq: fact.eq_time,
                        start_receipt: fact.receipt,
                        last_sustain_receipt: fact.receipt,
                        revision: 0,
                        last_snapshot: None,
                        frozen: None,
                    },
                );
            }
            encounter_id = Some(id);
        }

        let Some(encounter_id) = encounter_id else {
            return;
        };
        let known_target = self
            .encounters
            .get(&encounter_id)
            .is_some_and(|encounter| encounter.targets.contains_key(&target_id));
        let possible_participant = attacker_verified
            || self.verified_players.contains(&participant)
            || possible_player_name(&attacker_display);
        let contributes = positive
            && !defender_is_player
            && !self_attack
            && (start_qualified || (known_target && possible_participant));
        let sustains = contributes || known_target;
        if !contributes && !sustains {
            return;
        }

        self.add_source_member(&encounter_id, &fact.source);
        if let Some(encounter) = self.encounters.get_mut(&encounter_id) {
            if contributes {
                let target = encounter
                    .targets
                    .entry(target_id.clone())
                    .or_insert(TargetState {
                        display: Arc::from(defender_display.as_str()),
                        terminal: false,
                        first_damage: fact.eq_time,
                        last_damage: fact.eq_time,
                        terminal_at: None,
                    });
                target.last_damage = target.last_damage.max(fact.eq_time);
                if target.terminal {
                    target.terminal = false;
                    target.terminal_at = None;
                }
                encounter.fingerprints.insert(fingerprint);
                encounter.start_eq = encounter.start_eq.min(fact.eq_time);
                encounter.start_receipt = encounter.start_receipt.min(fact.receipt);
                encounter.end_eq = encounter.end_eq.max(fact.eq_time);
            }
            if sustains {
                encounter.last_sustain_eq = encounter.last_sustain_eq.max(fact.eq_time);
                encounter.last_sustain_receipt = encounter.last_sustain_receipt.max(fact.receipt);
            }
            encounter.facts.push(fact_index);
        }
        self.assignments.insert(fact_index, encounter_id.clone());
        self.source_active
            .insert(fact.source.id.clone(), encounter_id.clone());
        if start_qualified {
            self.promote_prior_facts(&encounter_id, &fact.source.id, &target_id, fact.eq_time);
        }
    }

    fn process_attempt(&mut self, fact_index: usize) {
        let fact = self.ledger[fact_index].clone();
        let FactEvent::Attempt(attempt) = &fact.event else {
            return;
        };
        let (attacker_name, _) = self.resolve_observed(&fact.source, &attempt.attacker);
        let (defender_name, _) = self.resolve_observed(&fact.source, &attempt.defender);
        let Some(id) = self.source_active.get(&fact.source.id).cloned() else {
            return;
        };
        let target = CanonicalTargetId::new(&defender_name);
        let incoming_target = CanonicalTargetId::new(&attacker_name);
        let known = self.encounters.get(&id).is_some_and(|encounter| {
            encounter.targets.contains_key(&target)
                || encounter.targets.contains_key(&incoming_target)
        });
        if !known {
            return;
        }
        if let Some(encounter) = self.encounters.get_mut(&id) {
            if matches!(
                encounter.phase,
                PhaseState::Ending {
                    reason: EndReason::Inactivity,
                    ..
                }
            ) {
                encounter.phase = PhaseState::Active;
            }
            encounter.last_sustain_eq = encounter.last_sustain_eq.max(fact.eq_time);
            encounter.last_sustain_receipt = encounter.last_sustain_receipt.max(fact.receipt);
            encounter.facts.push(fact_index);
        }
        self.assignments.insert(fact_index, id);
    }

    fn process_slain(&mut self, fact_index: usize) {
        let fact = self.ledger[fact_index].clone();
        let FactEvent::Slain(slain) = &fact.event else {
            return;
        };
        let (target_name, _) = self.resolve_observed(&fact.source, &slain.target);
        let target_id = CanonicalTargetId::new(&target_name);
        let encounter_id = self
            .source_active
            .get(&fact.source.id)
            .filter(|id| {
                self.encounters
                    .get(*id)
                    .is_some_and(|encounter| encounter.targets.contains_key(&target_id))
            })
            .cloned()
            .or_else(|| {
                self.encounters
                    .iter()
                    .find(|(_, encounter)| {
                        !matches!(encounter.phase, PhaseState::Held { .. })
                            && encounter.server.eq_ignore_ascii_case(&fact.source.server)
                            && encounter.targets.contains_key(&target_id)
                    })
                    .map(|(id, _)| id.clone())
            });
        let Some(id) = encounter_id else {
            return;
        };
        self.add_source_member(&id, &fact.source);
        let mut all_terminal = false;
        if let Some(encounter) = self.encounters.get_mut(&id) {
            if let Some(target) = encounter.targets.get_mut(&target_id) {
                target.terminal = true;
                target.terminal_at = Some(fact.eq_time);
            }
            encounter.end_eq = encounter.end_eq.max(fact.eq_time);
            encounter.facts.push(fact_index);
            all_terminal = !encounter.targets.is_empty()
                && encounter.targets.values().all(|target| target.terminal);
        }
        self.assignments.insert(fact_index, id.clone());
        if all_terminal {
            self.enter_ending(
                &id,
                EndReason::AllTargetsTerminal,
                fact.receipt,
                fact.eq_time,
            );
        }
    }

    fn enter_ending(
        &mut self,
        id: &EncounterId,
        reason: EndReason,
        now: MonoTime,
        cutoff: EqSecond,
    ) {
        if let Some(encounter) = self.encounters.get_mut(id) {
            if matches!(encounter.phase, PhaseState::Active) {
                encounter.phase = PhaseState::Ending {
                    reason,
                    entered_at: now,
                    cutoff,
                    closure_sources: encounter.sources.clone(),
                };
            }
        }
    }

    fn add_source_member(&mut self, encounter_id: &EncounterId, source: &LogSource) {
        let Some(encounter) = self.encounters.get_mut(encounter_id) else {
            return;
        };
        if encounter.sources.insert(source.id.clone()) {
            let source_state = self.sources.get(&source.id);
            let continuity = source_state.map_or(0, |state| state.continuity);
            encounter
                .source_continuity
                .insert(source.id.clone(), continuity);
            if source_state.is_none_or(|state| state.registered_at > encounter.start_receipt) {
                encounter.partial_sources.insert(source.id.clone());
            }
            encounter
                .managed
                .insert(ParticipantId::new(&source.server, &source.character));
        }
    }

    fn promote_prior_facts(
        &mut self,
        encounter_id: &EncounterId,
        source: &LogSourceId,
        target: &CanonicalTargetId,
        anchor: EqSecond,
    ) {
        let earliest = anchor.checked_add(-30).unwrap_or(EqSecond::new(i64::MIN));
        let candidates = self
            .ledger
            .iter()
            .enumerate()
            .filter(|(index, fact)| {
                !self.assignments.contains_key(index)
                    && &fact.source.id == source
                    && fact.eq_time >= earliest
                    && fact.eq_time <= anchor
                    && matches!(&fact.event, FactEvent::Damage(damage)
                        if CanonicalTargetId::new(&self.resolve_observed(&fact.source, &damage.defender).0) == *target
                            && possible_player_name(&self.resolve_observed(&fact.source, &damage.attacker).0))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in candidates {
            let fact = self.ledger[index].clone();
            if let Some(encounter) = self.encounters.get_mut(encounter_id) {
                encounter.facts.push(index);
                encounter.start_eq = encounter.start_eq.min(fact.eq_time);
                encounter.start_receipt = encounter.start_receipt.min(fact.receipt);
                encounter.end_eq = encounter.end_eq.max(fact.eq_time);
            }
            self.assignments.insert(index, encounter_id.clone());
        }
    }

    fn merge_encounters(&mut self, left: EncounterId, right: EncounterId) -> EncounterId {
        if left == right {
            return left;
        }
        let chosen_id = if left <= right {
            left.clone()
        } else {
            right.clone()
        };
        let other_id = if chosen_id == left { right } else { left };
        let Some(mut chosen) = self.encounters.remove(&chosen_id) else {
            return other_id;
        };
        let Some(other) = self.encounters.remove(&other_id) else {
            self.encounters.insert(chosen_id.clone(), chosen);
            return chosen_id;
        };
        chosen.sources.extend(other.sources);
        chosen.source_continuity.extend(other.source_continuity);
        chosen.partial_sources.extend(other.partial_sources);
        chosen.managed.extend(other.managed);
        for (id, target) in other.targets {
            chosen
                .targets
                .entry(id)
                .and_modify(|current| {
                    current.first_damage = current.first_damage.min(target.first_damage);
                    current.last_damage = current.last_damage.max(target.last_damage);
                    current.terminal &= target.terminal;
                    current.terminal_at = current.terminal_at.max(target.terminal_at);
                })
                .or_insert(target);
        }
        chosen.facts.extend(other.facts);
        chosen.facts.sort_unstable();
        chosen.facts.dedup();
        chosen.fingerprints.extend(other.fingerprints);
        chosen.start_eq = chosen.start_eq.min(other.start_eq);
        chosen.end_eq = chosen.end_eq.max(other.end_eq);
        chosen.last_sustain_eq = chosen.last_sustain_eq.max(other.last_sustain_eq);
        chosen.start_receipt = chosen.start_receipt.min(other.start_receipt);
        chosen.last_sustain_receipt = chosen.last_sustain_receipt.max(other.last_sustain_receipt);
        chosen.phase = PhaseState::Active;
        chosen.frozen = None;
        chosen.last_snapshot = None;
        for assigned in self.assignments.values_mut() {
            if *assigned == other_id {
                *assigned = chosen_id.clone();
            }
        }
        for active in self.source_active.values_mut() {
            if *active == other_id {
                *active = chosen_id.clone();
            }
        }
        self.encounters.insert(chosen_id.clone(), chosen);
        chosen_id
    }

    fn advance_time(&mut self, now: MonoTime, urgency: &mut PublishUrgency) {
        for source in self.sources.values_mut() {
            if source.eof_passes >= 2
                && source.eof_first_at.is_some_and(|first| {
                    now.saturating_duration_since(first) >= self.policy.stable_eof_ms
                })
            {
                if let Some(observed) = source.observed {
                    source.closed_through = Some(
                        source
                            .closed_through
                            .map_or(observed, |current| current.max(observed)),
                    );
                }
            }
        }

        let inactive = self
            .encounters
            .iter()
            .filter(|(_, encounter)| {
                matches!(encounter.phase, PhaseState::Active)
                    && now.saturating_duration_since(encounter.last_sustain_receipt)
                        >= self.policy.inactivity_ms
            })
            .map(|(id, encounter)| (id.clone(), encounter.last_sustain_eq))
            .collect::<Vec<_>>();
        for (id, cutoff) in inactive {
            self.enter_ending(&id, EndReason::Inactivity, now, cutoff);
            *urgency = PublishUrgency::Immediate;
        }

        let ending = self
            .encounters
            .iter()
            .filter_map(|(id, encounter)| match &encounter.phase {
                PhaseState::Ending {
                    reason,
                    entered_at,
                    cutoff,
                    closure_sources,
                } => {
                    let all_closed = closure_sources.iter().all(|source| {
                        self.sources
                            .get(source)
                            .and_then(|state| state.closed_through)
                            .is_some_and(|closed| closed >= *cutoff)
                    });
                    (all_closed
                        || now.saturating_duration_since(*entered_at)
                            >= self.policy.ending_grace_ms)
                        .then_some((
                            id.clone(),
                            *reason,
                            *cutoff,
                            closure_sources.clone(),
                            all_closed,
                        ))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (id, reason, cutoff, closure_sources, all_closed) in ending {
            if !all_closed {
                if let Some(encounter) = self.encounters.get_mut(&id) {
                    for source in closure_sources {
                        if self
                            .sources
                            .get(&source)
                            .and_then(|state| state.closed_through)
                            .is_none_or(|closed| closed < cutoff)
                        {
                            encounter.partial_sources.insert(source);
                        }
                    }
                }
            }
            self.finalize(&id, reason, now);
            *urgency = PublishUrgency::Immediate;
        }

        let expired = self
            .encounters
            .iter()
            .filter_map(|(id, encounter)| match encounter.phase {
                PhaseState::Held { until, .. } if now >= until => Some(id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let had_expired = !expired.is_empty();
        for id in expired {
            self.encounters.remove(&id);
            *urgency = PublishUrgency::Immediate;
        }
        if had_expired && self.encounters.is_empty() && !self.ledger.is_empty() {
            self.ledger.clear();
            self.assignments.clear();
        }
    }

    fn finalize(&mut self, id: &EncounterId, reason: EndReason, now: MonoTime) {
        let until = now.saturating_add(self.policy.held_ms);
        if let Some(encounter) = self.encounters.get_mut(id) {
            encounter.phase = PhaseState::Held { reason, until };
        }
        let encounter = self.encounters.get(id).cloned();
        if let Some(encounter) = encounter {
            if let Some(mut snapshot) = self.compute_snapshot(&encounter, 0) {
                snapshot.phase = EncounterPhase::Held;
                snapshot.end_reason = Some(reason);
                snapshot.held_until = Some(until);
                snapshot.revision = encounter_revision(&snapshot);
                let snapshot = Arc::new(snapshot);
                if let Some(state) = self.encounters.get_mut(id) {
                    state.revision = snapshot.revision;
                    state.last_snapshot = Some(snapshot.clone());
                    state.frozen = Some(snapshot);
                }
            }
        }
        self.source_active.retain(|_, encounter| encounter != id);
    }

    fn rebuild_book(&mut self) -> Option<Arc<EncounterBookSnapshot>> {
        let ids = self.encounters.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let Some(encounter) = self.encounters.get(&id).cloned() else {
                continue;
            };
            if matches!(encounter.phase, PhaseState::Held { .. }) {
                continue;
            }
            let Some(mut candidate) = self.compute_snapshot(&encounter, 0) else {
                continue;
            };
            let same = encounter.last_snapshot.as_ref().is_some_and(|previous| {
                let mut previous = (**previous).clone();
                previous.revision = 0;
                previous == candidate
            });
            if !same {
                let revision = encounter_revision(&candidate);
                candidate.revision = revision;
                let snapshot = Arc::new(candidate);
                if let Some(state) = self.encounters.get_mut(&id) {
                    state.revision = revision;
                    state.last_snapshot = Some(snapshot);
                }
            }
        }

        let mut snapshots = self
            .encounters
            .values()
            .filter_map(|encounter| {
                encounter
                    .frozen
                    .clone()
                    .or_else(|| encounter.last_snapshot.clone())
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.id.cmp(&right.id));
        if self.current_book.encounters.as_ref() == snapshots.as_slice() {
            return None;
        }
        let revision = book_revision(&snapshots);
        let book = Arc::new(EncounterBookSnapshot {
            revision,
            encounters: snapshots.into(),
        });
        self.current_book = book.clone();
        Some(book)
    }

    fn compute_snapshot(
        &self,
        encounter: &EncounterState,
        revision: u64,
    ) -> Option<EncounterSnapshot> {
        let elections = self.elect(encounter)?;
        if elections.is_empty() {
            return None;
        }
        let raid_damage = elections.values().try_fold(0u128, |total, election| {
            total.checked_add(election.winner.damage)
        })?;
        if raid_damage == 0 {
            return None;
        }
        let encounter_seconds = inclusive_seconds(encounter.start_eq, encounter.end_eq);
        let mut target_damage: BTreeMap<CanonicalTargetId, (u128, EqSecond)> = BTreeMap::new();
        for election in elections.values() {
            for (target, damage) in &election.winner.target_damage {
                let first = election
                    .winner
                    .target_first
                    .get(target)
                    .copied()
                    .unwrap_or(encounter.start_eq);
                let entry = target_damage.entry(target.clone()).or_insert((0, first));
                entry.0 = entry.0.checked_add(*damage)?;
                entry.1 = entry.1.min(first);
            }
        }
        let (primary, _) = target_damage
            .iter()
            .max_by(|(left_id, left), (right_id, right)| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| right.1.cmp(&left.1))
                    .then_with(|| right_id.cmp(left_id))
            })?;
        let additional = target_damage.len().saturating_sub(1);
        let primary_display = encounter
            .targets
            .get(primary)
            .map(|target| target.display.clone())
            .unwrap_or_else(|| primary.0.clone());
        let title: Arc<str> = if additional == 0 {
            primary_display
        } else {
            Arc::from(format!("{} +{} mobs", primary_display, additional))
        };

        let mut rows = elections
            .into_values()
            .filter(|election| election.winner.damage > 0)
            .map(|election| {
                let active_seconds = election.winner.active_seconds.max(1);
                let managed = encounter.managed.contains(&election.winner.participant)
                    || self.is_managed(&election.winner.participant);
                DpsRowSnapshot {
                    rank: 0,
                    participant: election.winner.participant,
                    display_name: election.winner.display,
                    managed,
                    has_pet_damage: election.winner.has_pet,
                    provisional_pet: election.winner.provisional_pet,
                    damage: election.winner.damage,
                    active_seconds,
                    dps: round_rate(election.winner.damage, active_seconds)?,
                    sdps: round_rate(election.winner.damage, encounter_seconds)?,
                    contribution_millionths: ratio_millionths(election.winner.damage, raid_damage),
                    source_quality: election.quality,
                    elected_source: election.winner.source,
                }
                .into()
            })
            .collect::<Option<Vec<DpsRowSnapshot>>>()?;
        rows.sort_by(|left, right| {
            right
                .damage
                .cmp(&left.damage)
                .then_with(|| right.dps.cmp(&left.dps))
                .then_with(|| left.participant.cmp(&right.participant))
        });
        for (index, row) in rows.iter_mut().enumerate() {
            row.rank = index + 1;
        }
        let (phase, end_reason, held_until) = match encounter.phase {
            PhaseState::Active => (EncounterPhase::Active, None, None),
            PhaseState::Ending { reason, .. } => (EncounterPhase::EndingGrace, Some(reason), None),
            PhaseState::Held { reason, until } => (EncounterPhase::Held, Some(reason), Some(until)),
        };
        Some(EncounterSnapshot {
            id: encounter.id.clone(),
            revision,
            phase,
            end_reason,
            title,
            primary_target: primary.clone(),
            additional_target_names: additional,
            encounter_seconds,
            raid_damage,
            rows: rows.into(),
            source_members: encounter.sources.iter().cloned().collect::<Vec<_>>().into(),
            last_sustained_at: encounter.last_sustain_eq,
            held_until,
        })
    }

    fn elect(&self, encounter: &EncounterState) -> Option<BTreeMap<ParticipantId, Election>> {
        let watermark = self.comparison_watermark(encounter);
        let mut candidates: BTreeMap<(ParticipantId, LogSourceId), Candidate> = BTreeMap::new();
        for fact_index in &encounter.facts {
            let fact = self.ledger.get(*fact_index)?;
            if watermark.is_some_and(|watermark| fact.eq_time > watermark) {
                continue;
            }
            let FactEvent::Damage(damage) = &fact.event else {
                continue;
            };
            if damage.amount == 0 || damage.outcome != DamageOutcome::Hit {
                continue;
            }
            let (attacker_display, attacker) =
                self.resolve_observed(&fact.source, &damage.attacker);
            let (defender_display, defender) =
                self.resolve_observed(&fact.source, &damage.defender);
            let target = CanonicalTargetId::new(&defender_display);
            if !encounter.targets.contains_key(&target) || self.verified_players.contains(&defender)
            {
                continue;
            }
            let explicit_owner = damage
                .explicit_owner
                .as_ref()
                .map(|owner| self.resolve_observed(&fact.source, owner));
            let owner = explicit_owner
                .as_ref()
                .map(|(_, owner)| owner.clone())
                .or_else(|| {
                    self.claimed_owner_in_encounter(&fact.source, &attacker_display, encounter)
                });
            let provisional_pet = owner.is_none() && damage.kind == DamageKind::Pet;
            let participant = owner.clone().unwrap_or(attacker.clone());
            if participant == defender {
                continue;
            }
            let actor_allowed = owner.is_some()
                || provisional_pet
                || self.verified_players.contains(&attacker)
                || damage.attacker.perspective != Perspective::Named
                || possible_player_name(&attacker_display);
            if !actor_allowed {
                continue;
            }
            let display = explicit_owner
                .as_ref()
                .map(|(display, _)| Arc::from(display.as_str()))
                .or_else(|| {
                    owner
                        .as_ref()
                        .and_then(|owner| self.display_for_participant(owner))
                })
                .unwrap_or_else(|| Arc::from(attacker_display.as_str()));
            let source_state = self.sources.get(&fact.source.id);
            let complete = source_state.is_some_and(|source| {
                encounter.source_continuity.get(&fact.source.id) == Some(&source.continuity)
                    && !encounter.partial_sources.contains(&fact.source.id)
                    && source.registered_at <= encounter.start_receipt
            });
            let personal =
                participant == ParticipantId::new(&fact.source.server, &fact.source.character);
            let key = (participant.clone(), fact.source.id.clone());
            let candidate = candidates.entry(key).or_insert_with(|| Candidate {
                participant,
                display,
                source: fact.source.id.clone(),
                complete,
                personal,
                provisional_pet,
                has_pet: false,
                direct_damage: 0,
                pet_damage: 0,
                damage: 0,
                events: 0,
                ranges: BTreeMap::new(),
                target_damage: BTreeMap::new(),
                target_first: BTreeMap::new(),
                active_seconds: 1,
            });
            let amount = u128::from(damage.amount);
            candidate.damage = candidate.damage.checked_add(amount)?;
            if owner.is_some() || provisional_pet {
                candidate.pet_damage = candidate.pet_damage.checked_add(amount)?;
                candidate.has_pet = owner.is_some();
            } else {
                candidate.direct_damage = candidate.direct_damage.checked_add(amount)?;
            }
            candidate.events = candidate.events.checked_add(1)?;
            let target_total = candidate.target_damage.entry(target.clone()).or_insert(0);
            *target_total = target_total.checked_add(amount)?;
            candidate
                .target_first
                .entry(target.clone())
                .and_modify(|first| *first = (*first).min(fact.eq_time))
                .or_insert(fact.eq_time);
            candidate
                .ranges
                .entry(target)
                .and_modify(|range| {
                    range.0 = range.0.min(fact.eq_time);
                    range.1 = range.1.max(fact.eq_time);
                })
                .or_insert((fact.eq_time, fact.eq_time));
        }

        for candidate in candidates.values_mut() {
            candidate.active_seconds = union_range_seconds(
                candidate.ranges.values().copied().collect(),
                self.policy.active_range_bridge_seconds,
            );
        }
        let mut grouped: BTreeMap<ParticipantId, Vec<Candidate>> = BTreeMap::new();
        for candidate in candidates
            .into_values()
            .filter(|candidate| candidate.damage > 0)
        {
            grouped
                .entry(candidate.participant.clone())
                .or_default()
                .push(candidate);
        }
        let mut elections = BTreeMap::new();
        for (participant, mut options) in grouped {
            options.sort_by(|left, right| left.source.cmp(&right.source));
            let authoritative = options
                .iter()
                .position(|candidate| candidate.personal && candidate.complete);
            let winner_index = authoritative.unwrap_or_else(|| {
                let mut best = 0usize;
                for index in 1..options.len() {
                    if candidate_cmp(&options[index], &options[best]) == Ordering::Greater {
                        best = index;
                    }
                }
                best
            });
            let winner = options.remove(winner_index);
            let quality = if winner.provisional_pet {
                SourceQuality::ProvisionalPet
            } else if winner.personal && winner.complete {
                SourceQuality::AuthoritativePersonal
            } else if winner.complete {
                SourceQuality::CompleteObserver
            } else if winner.personal {
                SourceQuality::IncompletePersonal
            } else {
                SourceQuality::BestPartialObserver
            };
            let losers = options
                .into_iter()
                .map(|candidate| (candidate.source, candidate.damage))
                .collect();
            elections.insert(
                participant,
                Election {
                    winner,
                    quality,
                    losers,
                },
            );
        }
        Some(elections)
    }

    fn comparison_watermark(&self, encounter: &EncounterState) -> Option<EqSecond> {
        if matches!(encounter.phase, PhaseState::Held { .. }) {
            return Some(encounter.end_eq);
        }
        let max_observed = encounter
            .sources
            .iter()
            .filter_map(|source| self.sources.get(source).and_then(|state| state.observed))
            .max();
        let complete = encounter
            .sources
            .iter()
            .filter_map(|source| {
                let state = self.sources.get(source)?;
                let continuous = encounter.source_continuity.get(source) == Some(&state.continuity)
                    && !encounter.partial_sources.contains(source)
                    && state.registered_at <= encounter.start_receipt;
                let lagging = max_observed
                    .zip(state.observed)
                    .is_some_and(|(leader, observed)| {
                        leader
                            .checked_sub(observed)
                            .is_some_and(|gap| gap >= self.policy.lag_eq_seconds)
                            && self.now.saturating_duration_since(state.last_progress_at)
                                >= self.policy.lag_receipt_ms
                    });
                (continuous && !lagging).then_some(state)
            })
            .collect::<Vec<_>>();
        let comparison = if complete.is_empty() {
            encounter
                .sources
                .iter()
                .filter_map(|source| self.sources.get(source))
                .collect::<Vec<_>>()
        } else {
            complete
        };
        match comparison.as_slice() {
            [] => None,
            [only] => only.closed_through.or(only.observed),
            many if many.iter().all(|source| source.closed_through.is_some()) => {
                many.iter().filter_map(|source| source.closed_through).min()
            }
            _ => Some(
                encounter
                    .start_eq
                    .checked_add(-1)
                    .unwrap_or(EqSecond::new(i64::MIN)),
            ),
        }
    }

    fn claimed_owner(
        &self,
        source: &LogSource,
        pet_display: &str,
        eq_time: EqSecond,
    ) -> Option<ParticipantId> {
        let server = source.server.to_ascii_lowercase();
        let pet = pet_display.to_ascii_lowercase();
        self.ownership
            .iter()
            .filter(|claim| {
                claim.server.as_ref() == server.as_str()
                    && claim.pet.as_ref() == pet.as_str()
                    && claim.valid_from <= eq_time
            })
            .max_by(|left, right| {
                left.valid_from
                    .cmp(&right.valid_from)
                    .then_with(|| right.source.cmp(&left.source))
            })
            .map(|claim| claim.owner.clone())
    }

    fn claimed_owner_in_encounter(
        &self,
        source: &LogSource,
        pet_display: &str,
        encounter: &EncounterState,
    ) -> Option<ParticipantId> {
        let server = source.server.to_ascii_lowercase();
        let pet = pet_display.to_ascii_lowercase();
        let evidence_deadline = encounter
            .end_eq
            .checked_add(30)
            .unwrap_or(EqSecond::new(i64::MAX));
        self.ownership
            .iter()
            .filter(|claim| {
                claim.server.as_ref() == server.as_str()
                    && claim.pet.as_ref() == pet.as_str()
                    && claim.valid_from <= evidence_deadline
            })
            .max_by(|left, right| {
                left.valid_from
                    .cmp(&right.valid_from)
                    .then_with(|| right.source.cmp(&left.source))
            })
            .map(|claim| claim.owner.clone())
    }

    fn resolve_observed(
        &self,
        source: &LogSource,
        observed: &ObservedCombatant,
    ) -> (String, ParticipantId) {
        let display = if observed.perspective == Perspective::Named {
            observed.name.to_string()
        } else {
            source.character.to_string()
        };
        let participant = participant_from_name(&source.server, &display);
        (display, participant)
    }

    fn display_for_participant(&self, participant: &ParticipantId) -> Option<Arc<str>> {
        self.sources
            .values()
            .find(|source| {
                ParticipantId::new(&source.source.server, &source.source.character) == *participant
            })
            .map(|source| source.source.character.clone())
            .or_else(|| {
                self.ownership
                    .iter()
                    .rev()
                    .find(|claim| claim.owner == *participant)
                    .map(|claim| claim.owner_display.clone())
            })
    }

    fn is_managed(&self, participant: &ParticipantId) -> bool {
        self.sources.values().any(|source| {
            ParticipantId::new(&source.source.server, &source.source.character) == *participant
        })
    }

    fn managed_identities(&self, server: &str) -> BTreeSet<ParticipantId> {
        self.sources
            .values()
            .filter(|source| source.present && source.source.server.eq_ignore_ascii_case(server))
            .map(|source| ParticipantId::new(&source.source.server, &source.source.character))
            .collect()
    }

    fn encounter_id(
        &self,
        fact: &LedgerFact,
        target: &CanonicalTargetId,
        participant: &ParticipantId,
    ) -> EncounterId {
        EncounterId(Arc::from(format!(
            "{}:{}:{}:{}",
            fact.source.server.to_ascii_lowercase(),
            fact.eq_time.value(),
            target.as_str(),
            participant.canonical_name
        )))
    }

    fn diagnose(&mut self, message: impl Into<Arc<str>>) {
        if self.diagnostics.len() >= self.policy.max_diagnostics {
            self.diagnostics.pop_front();
        }
        self.diagnostics.push_back(message.into());
    }
}

fn participant_from_name(default_server: &str, display: &str) -> ParticipantId {
    let trimmed = display.trim();
    if let Some((server, character)) = trimmed.split_once('.') {
        if !server.is_empty() && !character.is_empty() && !character.contains('.') {
            return ParticipantId::new(server, character);
        }
    }
    ParticipantId::new(default_server, trimmed)
}

fn possible_player_name(display: &str) -> bool {
    let name = display.rsplit_once('.').map_or(display, |(_, name)| name);
    !name.is_empty()
        && !name.contains(char::is_whitespace)
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '`' | '\'')
        })
}

fn encounter_revision(snapshot: &EncounterSnapshot) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    hash_bytes(&mut hash, snapshot.id.as_str().as_bytes());
    hash_bytes(&mut hash, snapshot.title.as_bytes());
    hash_u64(&mut hash, snapshot.phase as u64);
    hash_u64(
        &mut hash,
        snapshot.end_reason.map_or(u64::MAX, |reason| reason as u64),
    );
    hash_u64(&mut hash, snapshot.additional_target_names as u64);
    hash_u64(&mut hash, snapshot.encounter_seconds);
    hash_bytes(&mut hash, &snapshot.last_sustained_at.value().to_le_bytes());
    hash_bytes(&mut hash, &snapshot.raid_damage.to_le_bytes());
    hash_bytes(&mut hash, snapshot.primary_target.as_str().as_bytes());
    hash_u64(
        &mut hash,
        snapshot.held_until.map_or(u64::MAX, MonoTime::as_millis),
    );
    for source in snapshot.source_members.iter() {
        hash_bytes(&mut hash, source.as_str().as_bytes());
    }
    for row in snapshot.rows.iter() {
        hash_u64(&mut hash, row.rank as u64);
        hash_bytes(&mut hash, row.participant.server.as_bytes());
        hash_bytes(&mut hash, row.participant.canonical_name.as_bytes());
        hash_bytes(&mut hash, row.display_name.as_bytes());
        hash_u64(&mut hash, row.managed as u64);
        hash_u64(&mut hash, row.has_pet_damage as u64);
        hash_u64(&mut hash, row.provisional_pet as u64);
        hash_bytes(&mut hash, &row.damage.to_le_bytes());
        hash_u64(&mut hash, row.active_seconds);
        hash_bytes(&mut hash, &row.dps.to_le_bytes());
        hash_bytes(&mut hash, &row.sdps.to_le_bytes());
        hash_u64(&mut hash, u64::from(row.contribution_millionths));
        hash_u64(&mut hash, row.source_quality as u64);
        hash_bytes(&mut hash, row.elected_source.as_str().as_bytes());
    }
    hash.max(1)
}

fn book_revision(snapshots: &[Arc<EncounterSnapshot>]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for snapshot in snapshots {
        hash_bytes(&mut hash, snapshot.id.as_str().as_bytes());
        hash_u64(&mut hash, snapshot.revision);
    }
    hash.max(1)
}

fn hash_u64(hash: &mut u64, value: u64) {
    hash_bytes(hash, &value.to_le_bytes());
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
    *hash ^= 0xff;
    *hash = hash.wrapping_mul(0x100000001b3);
}

fn candidate_cmp(left: &Candidate, right: &Candidate) -> Ordering {
    left.complete
        .cmp(&right.complete)
        .then_with(|| left.damage.cmp(&right.damage))
        .then_with(|| left.events.cmp(&right.events))
        .then_with(|| left.target_damage.len().cmp(&right.target_damage.len()))
        .then_with(|| left.active_seconds.cmp(&right.active_seconds))
        .then_with(|| right.source.cmp(&left.source))
}
