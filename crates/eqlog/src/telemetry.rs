use std::collections::HashMap;
use std::sync::Arc;

use super::event::{CharacterEvent, IdentityEvent, LogEvent, ParsedLogEvent, PetEvent};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CharacterKey {
    pub server: Arc<str>,
    pub character: Arc<str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterTelemetry {
    pub auto_attack: bool,
    pub dead: bool,
    pub class_code: Option<Arc<str>>,
    pub pet: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelemetryChange {
    pub character: CharacterKey,
    pub telemetry: CharacterTelemetry,
}

struct Entry {
    display_key: CharacterKey,
    telemetry: CharacterTelemetry,
}

/// Converts instantaneous typed events into persistent per-character state.
/// Timers and presentation notifications intentionally live elsewhere.
pub struct TelemetryReducer {
    characters: HashMap<(String, String), Entry>,
}

impl Default for TelemetryReducer {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryReducer {
    pub fn new() -> Self {
        Self {
            characters: HashMap::new(),
        }
    }

    pub fn apply(&mut self, event: &ParsedLogEvent) -> Option<TelemetryChange> {
        match &event.event {
            LogEvent::Identity(IdentityEvent::WhoResult(result)) => {
                let class_code = result.class_abbreviation.as_ref()?;
                let entry = self.entry(event.source.server.clone(), result.character.clone());
                if entry.telemetry.class_code.as_deref() == Some(class_code.as_ref()) {
                    return None;
                }
                entry.telemetry.class_code = Some(class_code.clone());
                Some(change(entry))
            }
            LogEvent::Pet(PetEvent::OwnershipClaimed { pet, owner }) => {
                let entry = self.entry(event.source.server.clone(), owner.clone());
                if entry.telemetry.pet.as_deref() == Some(pet.as_ref()) {
                    return None;
                }
                entry.telemetry.pet = Some(pet.clone());
                Some(change(entry))
            }
            LogEvent::Character(CharacterEvent::AutoAttackChanged { enabled }) => {
                let entry = self.entry(event.source.server.clone(), event.source.character.clone());
                if entry.telemetry.auto_attack == *enabled {
                    return None;
                }
                entry.telemetry.auto_attack = *enabled;
                Some(change(entry))
            }
            LogEvent::Character(CharacterEvent::Died) => {
                let entry = self.entry(event.source.server.clone(), event.source.character.clone());
                if entry.telemetry.dead {
                    return None;
                }
                entry.telemetry.dead = true;
                Some(change(entry))
            }
            LogEvent::Character(CharacterEvent::Revived) => {
                let entry = self.entry(event.source.server.clone(), event.source.character.clone());
                if !entry.telemetry.dead {
                    return None;
                }
                entry.telemetry.dead = false;
                Some(change(entry))
            }
            LogEvent::Combat(_) | LogEvent::Chat(_) | LogEvent::Notification(_) => None,
        }
    }

    fn entry(&mut self, server: Arc<str>, character: Arc<str>) -> &mut Entry {
        let normalized = (server.to_ascii_lowercase(), character.to_ascii_lowercase());
        self.characters.entry(normalized).or_insert_with(|| Entry {
            display_key: CharacterKey { server, character },
            telemetry: CharacterTelemetry::default(),
        })
    }
}

fn change(entry: &Entry) -> TelemetryChange {
    TelemetryChange {
        character: entry.display_key.clone(),
        telemetry: entry.telemetry.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdentityEvent, LogSource, WhoResult};

    fn event(source: LogSource, event: LogEvent) -> ParsedLogEvent {
        ParsedLogEvent {
            source,
            timestamp: None,
            event,
        }
    }

    fn empty_who(character: &str, class: &str) -> WhoResult {
        WhoResult {
            character: Arc::from(character),
            level: None,
            title: None,
            class_name: None,
            class_abbreviation: Some(Arc::from(class)),
            race: None,
            guild: None,
            zone: None,
            zone_short: None,
            is_anonymous: false,
            is_afk: false,
            is_lfg: false,
        }
    }

    #[test]
    fn case_insensitive_class_observations_are_deduplicated() {
        let mut reducer = TelemetryReducer::new();
        let first = event(
            LogSource::new("observer-1", "Observer", "Teek"),
            LogEvent::Identity(IdentityEvent::WhoResult(empty_who("Bilka", "BRD"))),
        );
        let repeated = event(
            LogSource::new("observer-1", "Observer", "teek"),
            LogEvent::Identity(IdentityEvent::WhoResult(empty_who("bilka", "BRD"))),
        );

        let change = reducer
            .apply(&first)
            .expect("first observation changes state");
        assert_eq!(change.character.character.as_ref(), "Bilka");
        assert_eq!(change.telemetry.class_code.as_deref(), Some("BRD"));
        assert!(reducer.apply(&repeated).is_none());
    }

    #[test]
    fn pet_replacement_is_persistent_state_not_a_transient_event() {
        let mut reducer = TelemetryReducer::new();
        let source = LogSource::new("client-1", "Saabra", "teek");
        let first = event(
            source.clone(),
            LogEvent::Pet(PetEvent::OwnershipClaimed {
                pet: Arc::from("Fluffy"),
                owner: Arc::from("Saabra"),
            }),
        );
        let replacement = event(
            source,
            LogEvent::Pet(PetEvent::OwnershipClaimed {
                pet: Arc::from("Sparky"),
                owner: Arc::from("Saabra"),
            }),
        );

        assert_eq!(
            reducer.apply(&first).unwrap().telemetry.pet.as_deref(),
            Some("Fluffy")
        );
        assert_eq!(
            reducer
                .apply(&replacement)
                .unwrap()
                .telemetry
                .pet
                .as_deref(),
            Some("Sparky")
        );
    }

    #[test]
    fn character_events_update_only_persistent_character_fields() {
        let mut reducer = TelemetryReducer::new();
        let source = LogSource::new("client-9", "Orlov", "teek");
        let auto_attack = event(
            source.clone(),
            LogEvent::Character(CharacterEvent::AutoAttackChanged { enabled: true }),
        );
        let died = event(source, LogEvent::Character(CharacterEvent::Died));

        let attack_state = reducer.apply(&auto_attack).unwrap().telemetry;
        assert!(attack_state.auto_attack);
        assert!(!attack_state.dead);
        let dead_state = reducer.apply(&died).unwrap().telemetry;
        assert!(dead_state.auto_attack);
        assert!(dead_state.dead);
    }
}
