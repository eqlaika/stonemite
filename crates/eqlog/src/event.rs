use std::sync::Arc;

use super::raw::{EqTimestamp, LogSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogEventDomain {
    Identity,
    Pets,
    Character,
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
}

impl LogEvent {
    pub fn domain(&self) -> LogEventDomain {
        match self {
            Self::Identity(_) => LogEventDomain::Identity,
            Self::Pet(_) => LogEventDomain::Pets,
            Self::Character(_) => LogEventDomain::Character,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityEvent {
    WhoResult(WhoResult),
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
