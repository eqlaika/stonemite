use std::fmt;
use std::sync::Arc;

pub use eqlog::{EqSecond, LogEvent, LogSource, LogSourceId, SourceRecordId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonoTime(u64);

impl MonoTime {
    pub const ZERO: Self = Self(0);

    pub const fn from_millis(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub const fn saturating_add(self, millis: u64) -> Self {
        Self(self.0.saturating_add(millis))
    }

    pub const fn saturating_duration_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombatPolicy {
    pub version: u16,
    pub inactivity_ms: u64,
    pub ending_grace_ms: u64,
    pub held_ms: u64,
    pub stable_eof_ms: u64,
    pub lag_eq_seconds: i64,
    pub lag_receipt_ms: u64,
    pub active_range_bridge_seconds: i64,
    pub max_encounters: usize,
    pub max_ledger_facts: usize,
    pub max_record_ids: usize,
    pub max_diagnostics: usize,
}

impl CombatPolicy {
    pub fn mvp_v1() -> Self {
        Self {
            version: 1,
            inactivity_ms: 30_000,
            ending_grace_ms: 2_000,
            held_ms: 8_000,
            stable_eof_ms: 2_000,
            lag_eq_seconds: 2,
            lag_receipt_ms: 2_000,
            active_range_bridge_seconds: 6,
            max_encounters: 16,
            max_ledger_facts: 100_000,
            max_record_ids: 200_000,
            max_diagnostics: 256,
        }
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.version == 0
            || self.inactivity_ms == 0
            || self.ending_grace_ms == 0
            || self.held_ms == 0
            || self.stable_eof_ms == 0
            || self.lag_eq_seconds <= 0
            || self.active_range_bridge_seconds < 0
            || self.max_encounters == 0
            || self.max_ledger_facts == 0
            || self.max_record_ids == 0
            || self.max_diagnostics == 0
        {
            Err(PolicyError)
        } else {
            Ok(())
        }
    }
}

impl Default for CombatPolicy {
    fn default() -> Self {
        Self::mvp_v1()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyError;

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("combat policy contains an invalid zero or negative bound")
    }
}

impl std::error::Error for PolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapReason {
    FileRecreated,
    FileTruncated,
    BoundaryChanged,
    OversizedRecord,
    DiscardUntilNewline,
    SourceReassociated,
    CalendarRegression,
    OutOfOrderClosedSecond,
    GenerationChanged,
}

#[derive(Clone, Debug)]
pub struct CombatRecord {
    pub id: SourceRecordId,
    pub source: LogSource,
    pub eq_time: Option<EqSecond>,
    pub events: Arc<[LogEvent]>,
}

impl CombatRecord {
    pub fn new(
        id: SourceRecordId,
        source: LogSource,
        eq_time: Option<EqSecond>,
        events: impl Into<Arc<[LogEvent]>>,
    ) -> Self {
        Self {
            id,
            source,
            eq_time,
            events: events.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum EngineInput {
    SourceRegistered {
        source: LogSource,
        generation: u64,
    },
    SourceRemoved {
        source: LogSourceId,
    },
    SourceGap {
        source: LogSourceId,
        generation: u64,
        reason: GapReason,
    },
    SourceStableEof {
        source: LogSourceId,
        generation: u64,
    },
    Record(CombatRecord),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PublishUrgency {
    #[default]
    None,
    Coalescible,
    Immediate,
}

#[derive(Clone, Debug)]
pub struct EngineUpdate {
    pub snapshot: Option<Arc<EncounterBookSnapshot>>,
    pub urgency: PublishUrgency,
    pub next_deadline: Option<MonoTime>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EncounterId(pub(crate) Arc<str>);

impl EncounterId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParticipantId {
    pub server: Arc<str>,
    pub canonical_name: Arc<str>,
}

impl ParticipantId {
    pub fn new(server: &str, name: &str) -> Self {
        Self {
            server: Arc::from(server.to_ascii_lowercase()),
            canonical_name: Arc::from(name.to_ascii_lowercase()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalTargetId(pub Arc<str>);

impl CanonicalTargetId {
    pub fn new(name: &str) -> Self {
        Self(Arc::from(name.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncounterPhase {
    Active,
    EndingGrace,
    Held,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    AllTargetsTerminal,
    Inactivity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceQuality {
    AuthoritativePersonal,
    CompleteObserver,
    BestPartialObserver,
    IncompletePersonal,
    ProvisionalPet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpsRowSnapshot {
    pub rank: usize,
    pub participant: ParticipantId,
    pub display_name: Arc<str>,
    pub managed: bool,
    pub has_pet_damage: bool,
    pub provisional_pet: bool,
    pub damage: u128,
    pub active_seconds: u64,
    pub dps: u128,
    pub sdps: u128,
    pub contribution_millionths: u32,
    pub source_quality: SourceQuality,
    pub elected_source: LogSourceId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncounterSnapshot {
    pub id: EncounterId,
    pub revision: u64,
    pub phase: EncounterPhase,
    pub end_reason: Option<EndReason>,
    pub title: Arc<str>,
    pub primary_target: CanonicalTargetId,
    pub additional_target_names: usize,
    pub encounter_seconds: u64,
    pub raid_damage: u128,
    /// All globally ranked positive-damage participants. Presentation applies
    /// top-N plus managed-extra projection without changing engine state.
    pub rows: Arc<[DpsRowSnapshot]>,
    pub source_members: Arc<[LogSourceId]>,
    pub last_sustained_at: EqSecond,
    pub held_until: Option<MonoTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncounterBookSnapshot {
    pub revision: u64,
    pub encounters: Arc<[Arc<EncounterSnapshot>]>,
}

impl Default for EncounterBookSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            encounters: Arc::from([]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateExplanation {
    pub participant: ParticipantId,
    pub elected_source: LogSourceId,
    pub quality: SourceQuality,
    pub damage: u128,
    pub direct_damage: u128,
    pub pet_damage: u128,
    pub losing_sources: Arc<[(LogSourceId, u128)]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncounterExplanation {
    pub encounter: EncounterId,
    pub candidates: Arc<[CandidateExplanation]>,
    pub diagnostics: Arc<[Arc<str>]>,
}

/// Project the visible global top N and append participating managed rows that
/// the cutoff omitted. Appended rows retain their true global rank.
pub fn project_visible_rows(rows: &[DpsRowSnapshot], top_rows: usize) -> Vec<DpsRowSnapshot> {
    let cutoff = top_rows.min(rows.len());
    let mut visible = rows[..cutoff].to_vec();
    visible.extend(rows[cutoff..].iter().filter(|row| row.managed).cloned());
    visible
}

/// Pure presentation selection. Stable encounter identity is the final tie.
pub fn select_presented<'a>(
    book: &'a EncounterBookSnapshot,
    active_source: Option<&LogSourceId>,
) -> Option<&'a EncounterSnapshot> {
    let mut active: Vec<_> = book
        .encounters
        .iter()
        .filter(|encounter| encounter.phase != EncounterPhase::Held)
        .map(Arc::as_ref)
        .collect();
    active.sort_by(|left, right| {
        let left_current = active_source.is_some_and(|source| left.source_members.contains(source));
        let right_current =
            active_source.is_some_and(|source| right.source_members.contains(source));
        right_current
            .cmp(&left_current)
            .then_with(|| right.source_members.len().cmp(&left.source_members.len()))
            .then_with(|| right.last_sustained_at.cmp(&left.last_sustained_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    if let Some(encounter) = active.first() {
        return Some(*encounter);
    }

    book.encounters
        .iter()
        .filter(|encounter| encounter.phase == EncounterPhase::Held)
        .map(Arc::as_ref)
        .max_by(|left, right| {
            left.held_until
                .cmp(&right.held_until)
                .then_with(|| right.id.cmp(&left.id))
        })
}
