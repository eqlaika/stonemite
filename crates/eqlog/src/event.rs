use std::sync::Arc;

use super::raw::{EqTimestamp, LogSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogEventDomain {
    Identity,
    Pets,
    Character,
    Casting,
    Combat,
    Chat,
    Notification,
    Progress,
    Consider,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedLogEvent {
    pub source: LogSource,
    pub timestamp: Option<EqTimestamp>,
    pub event: LogEvent,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogEvent {
    Identity(IdentityEvent),
    Pet(PetEvent),
    Character(CharacterEvent),
    Casting(CastingEvent),
    Combat(CombatEvent),
    Chat(ChatEvent),
    Notification(NotificationEvent),
    Progress(ProgressEvent),
    Consider(ConsiderEvent),
}

impl LogEvent {
    pub fn domain(&self) -> LogEventDomain {
        match self {
            Self::Identity(_) => LogEventDomain::Identity,
            Self::Pet(_) => LogEventDomain::Pets,
            Self::Character(_) => LogEventDomain::Character,
            Self::Casting(_) => LogEventDomain::Casting,
            Self::Combat(_) => LogEventDomain::Combat,
            Self::Chat(_) => LogEventDomain::Chat,
            Self::Notification(_) => LogEventDomain::Notification,
            Self::Progress(_) => LogEventDomain::Progress,
            Self::Consider(_) => LogEventDomain::Consider,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityEvent {
    WhoResult(WhoResult),
    PersonaLoaded(PersonaLoaded),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonaLoaded {
    pub class_name: Arc<str>,
    pub class_abbreviation: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhoResult {
    pub character: Arc<str>,
    pub level: Option<u16>,
    pub title: Option<Arc<str>>,
    pub class_name: Option<Arc<str>>,
    pub class_abbreviation: Option<Arc<str>>,
    pub race: Option<Arc<str>>,
    pub guild: Option<Arc<str>>,
    pub zone: Option<Arc<str>>,
    pub zone_short: Option<Arc<str>>,
    pub is_anonymous: bool,
    pub is_afk: bool,
    pub is_lfg: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PetEvent {
    OwnershipClaimed { pet: Arc<str>, owner: Arc<str> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    Spell,
    Song,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastingEvent {
    Started { spell: Arc<str>, kind: CastKind },
    Fizzled { spell: Option<Arc<str>> },
    Interrupted { spell: Option<Arc<str>> },
    Resisted { spell: Arc<str> },
    ConcentrationRecovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackProblem {
    OutOfRange,
    TooClose,
    LineOfSight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perspective {
    Named,
    You,
    Your,
    Yourself,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObservedCombatant {
    pub name: Arc<str>,
    pub perspective: Perspective,
}

impl ObservedCombatant {
    pub fn named(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            perspective: Perspective::Named,
        }
    }

    pub fn you() -> Self {
        Self {
            name: Arc::from("You"),
            perspective: Perspective::You,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DamageKind {
    Melee,
    DirectSpell,
    DamageOverTime,
    Proc,
    DamageShield,
    Bane,
    Pet,
    OtherIncluded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DamageOutcome {
    Hit,
    Miss,
    Dodge,
    Parry,
    Block,
    Riposte,
    Invulnerable,
    Absorbed,
    Rejected,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DamageModifiers {
    pub critical: bool,
    pub lucky: bool,
    pub strikethrough: bool,
    pub wild_rampage: bool,
    pub twincast: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParserProvenance {
    Melee,
    DirectSpell,
    PeriodicSpell,
    NonMelee,
    CombatAttempt,
    Slain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamageObservation {
    pub attacker: ObservedCombatant,
    pub explicit_owner: Option<ObservedCombatant>,
    pub defender: ObservedCombatant,
    pub amount: u64,
    pub kind: DamageKind,
    pub ability: Option<Arc<str>>,
    pub outcome: DamageOutcome,
    pub modifiers: DamageModifiers,
    pub provenance: ParserProvenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombatAttempt {
    pub attacker: ObservedCombatant,
    pub defender: ObservedCombatant,
    pub outcome: DamageOutcome,
    pub kind: DamageKind,
    pub ability: Option<Arc<str>>,
    pub provenance: ParserProvenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSlainObservation {
    pub target: ObservedCombatant,
    pub killer: Option<ObservedCombatant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PetOwnershipObservation {
    pub pet: ObservedCombatant,
    pub owner: ObservedCombatant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerEvidence {
    pub player: ObservedCombatant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneObservation {
    pub zone: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CombatEvent {
    Damage(DamageObservation),
    Attempt(CombatAttempt),
    TargetSlain(TargetSlainObservation),
    PetOwnership(PetOwnershipObservation),
    PlayerEvidence(PlayerEvidence),
    ZoneChanged(ZoneObservation),
    // Compatibility signals retained for the existing PiP combat-awareness
    // reducer. Rich observations above are the authoritative DPS input.
    WeaponDamageDealt,
    DamageTaken,
    AttackBlocked(AttackProblem),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    IncomingTell(IncomingTell),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingTell {
    pub sender: Arc<str>,
    pub message: Arc<str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressEvent {
    LevelGained { level: u16 },
    AlternateAdvancementPointGained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsiderDifficulty {
    Green,
    LightBlue,
    Blue,
    White,
    Yellow,
    Red,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsiderEvent {
    Target {
        target: Arc<str>,
        difficulty: ConsiderDifficulty,
        level: Option<u16>,
    },
    NoTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationEvent {
    GroupInvite { inviter: Arc<str> },
    GroupInviteAccepted,
    GroupInviteDeclined { inviter: Arc<str> },
    RaidInvite { inviter: Arc<str> },
    TradeProposed { trader: Arc<str> },
    TradeCancelled,
    ResurrectionOffered,
    CharacterSlain { killer: Arc<str> },
}

/// Persistent-character facts have their own domain rather than being mixed
/// into transient notifications or timers. Parsers for these events can be
/// added independently as EQ line coverage grows.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterEvent {
    AutoAttackChanged { enabled: bool },
    Died,
    Revived,
}
