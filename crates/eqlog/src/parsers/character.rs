use std::sync::Arc;

use crate::{
    CharacterEvent, CombatEvent, DomainParser, LogEvent, ParserError, RawLogLine, ZoneObservation,
};

const SLAIN_PREFIX: &str = "You have been slain by ";
const ZONE_ENTERED_PREFIX: &str = "You have entered ";

pub(super) struct CharacterParser;

impl DomainParser for CharacterParser {
    fn name(&self) -> &'static str {
        "character"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();
        if body
            .strip_prefix(SLAIN_PREFIX)
            .and_then(|remainder| remainder.strip_suffix('!'))
            .is_some_and(|killer| !killer.trim().is_empty())
        {
            events.push(LogEvent::Character(CharacterEvent::Died));
            return Ok(());
        }
        if let Some(zone) = body
            .strip_prefix(ZONE_ENTERED_PREFIX)
            .and_then(|zone| zone.strip_suffix('.'))
            .filter(|zone| !zone.trim().is_empty())
        {
            events.push(LogEvent::Character(CharacterEvent::Revived));
            events.push(LogEvent::Combat(CombatEvent::ZoneChanged(
                ZoneObservation {
                    zone: Arc::from(zone.trim()),
                },
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogSource;

    fn parse(body: &str) -> Vec<LogEvent> {
        let source = LogSource::new("pid:42", "Bilka", "xegony");
        let line = RawLogLine::new(source, None, body);
        let mut events = Vec::new();
        CharacterParser.parse(&line, &mut events).unwrap();
        events
    }

    #[test]
    fn death_and_confirmed_zone_entry_update_persistent_character_state() {
        assert_eq!(
            parse("You have been slain by a War Swarm invader!"),
            vec![LogEvent::Character(CharacterEvent::Died)]
        );
        assert_eq!(
            parse("You have entered The Nexus."),
            vec![
                LogEvent::Character(CharacterEvent::Revived),
                LogEvent::Combat(CombatEvent::ZoneChanged(ZoneObservation {
                    zone: Arc::from("The Nexus"),
                })),
            ]
        );
    }

    #[test]
    fn resurrection_start_and_malformed_lines_do_not_claim_recovery() {
        for body in [
            "You are being resurrected...",
            "You have entered .",
            "You have been slain by !",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
        }
    }
}
