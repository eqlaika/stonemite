use std::collections::HashMap;
use std::sync::Arc;

use super::{DomainParser, ParserError};
use crate::{IdentityEvent, LogEvent, LogSource, RawLogLine, WhoResult};

#[derive(Default)]
pub(super) struct IdentityParser {
    who_states: HashMap<LogSource, WhoParseState>,
}

#[derive(Clone, Copy, Default)]
enum WhoParseState {
    #[default]
    Idle,
    InBlock,
}

impl DomainParser for IdentityParser {
    fn name(&self) -> &'static str {
        "identity"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();
        if body.starts_with("OFFLINE MODE") {
            return Ok(());
        }

        let state = self.who_states.entry(line.source.clone()).or_default();
        match state {
            WhoParseState::Idle => {
                if body.contains("Players in EverQuest:") {
                    *state = WhoParseState::InBlock;
                }
            }
            WhoParseState::InBlock => {
                if ((body.contains("There is") || body.contains("There are"))
                    && body.contains("player"))
                    || body.contains("who request was cut short")
                {
                    *state = WhoParseState::Idle;
                } else if let Some(entry) = WhoEntry::parse(body) {
                    events.push(LogEvent::Identity(IdentityEvent::WhoResult(
                        entry.into_owned(),
                    )));
                }
            }
        }
        Ok(())
    }

    fn reset_source(&mut self, source: &LogSource) {
        self.who_states.remove(source);
    }
}

struct WhoEntry<'a> {
    character: &'a str,
    level: Option<u16>,
    title: Option<&'a str>,
    class_name: Option<&'a str>,
    race: Option<&'a str>,
    guild: Option<&'a str>,
    zone: Option<&'a str>,
    zone_short: Option<&'a str>,
    is_anonymous: bool,
    is_afk: bool,
    is_lfg: bool,
}

impl<'a> WhoEntry<'a> {
    fn parse(body: &'a str) -> Option<Self> {
        let open = body.find('[')?;
        let close = body.find(']')?;
        if close <= open {
            return None;
        }

        let before_bracket = &body[..open];
        let bracket = &body[open + 1..close];
        let after_bracket = &body[close + 1..];
        let is_afk = before_bracket.contains("AFK");
        let is_lfg = after_bracket.trim_end().ends_with("LFG");
        let after_trimmed = after_bracket.trim_start();
        let character = after_trimmed.split_whitespace().next()?;
        if character.is_empty() {
            return None;
        }

        if bracket == "ANONYMOUS" {
            return Some(Self {
                character,
                level: None,
                title: None,
                class_name: None,
                race: parse_parens(after_trimmed, character.len()),
                guild: parse_guild(after_trimmed),
                zone: parse_zone_name(after_trimmed),
                zone_short: parse_zone_short(after_trimmed),
                is_anonymous: true,
                is_afk,
                is_lfg,
            });
        }

        let (level, title, class_name) = parse_bracket(bracket);
        Some(Self {
            character,
            level,
            title,
            class_name,
            race: parse_parens(after_trimmed, character.len()),
            guild: parse_guild(after_trimmed),
            zone: parse_zone_name(after_trimmed),
            zone_short: parse_zone_short(after_trimmed),
            is_anonymous: false,
            is_afk,
            is_lfg,
        })
    }

    fn into_owned(self) -> WhoResult {
        WhoResult {
            character: Arc::from(self.character),
            level: self.level,
            title: self.title.map(Arc::from),
            class_name: self.class_name.map(Arc::from),
            class_abbreviation: self.class_name.and_then(class_abbreviation).map(Arc::from),
            race: self.race.map(Arc::from),
            guild: self.guild.map(Arc::from),
            zone: self.zone.map(Arc::from),
            zone_short: self.zone_short.map(Arc::from),
            is_anonymous: self.is_anonymous,
            is_afk: self.is_afk,
            is_lfg: self.is_lfg,
        }
    }
}

fn parse_bracket(bracket: &str) -> (Option<u16>, Option<&str>, Option<&str>) {
    let (level_str, rest) = match bracket.find(' ') {
        Some(index) => (&bracket[..index], bracket[index + 1..].trim_start()),
        None => return (None, None, None),
    };
    let level = level_str.parse::<u16>().ok();

    if let (Some(open), Some(close)) = (rest.rfind('('), rest.rfind(')')) {
        if close > open + 1 {
            let class_name = &rest[open + 1..close];
            let title = rest[..open].trim();
            return (
                level,
                (!title.is_empty()).then_some(title),
                Some(class_name),
            );
        }
    }

    if rest.is_empty() {
        (level, None, None)
    } else {
        (level, None, Some(rest))
    }
}

fn parse_parens(value: &str, skip: usize) -> Option<&str> {
    let rest = value.get(skip..)?;
    let open = rest.find('(')?;
    let close = rest.find(')')?;
    (close > open + 1).then_some(&rest[open + 1..close])
}

fn parse_guild(value: &str) -> Option<&str> {
    let open = value.find('<')?;
    let close = value.find('>')?;
    (close > open + 1).then_some(&value[open + 1..close])
}

fn parse_zone_name(value: &str) -> Option<&str> {
    let index = value.find("ZONE: ")?;
    let rest = &value[index + 6..];
    let end = rest.rfind('(').unwrap_or(rest.len());
    let name = rest[..end].trim();
    (!name.is_empty()).then_some(name)
}

fn parse_zone_short(value: &str) -> Option<&str> {
    let index = value.find("ZONE: ")?;
    let rest = &value[index..];
    let open = rest.rfind('(')?;
    let close = rest.rfind(')')?;
    (close > open + 1).then_some(&rest[open + 1..close])
}

fn class_abbreviation(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "bard" => Some("BRD"),
        "beastlord" => Some("BST"),
        "berserker" => Some("BER"),
        "cleric" => Some("CLR"),
        "druid" => Some("DRU"),
        "enchanter" => Some("ENC"),
        "magician" => Some("MAG"),
        "monk" => Some("MNK"),
        "necromancer" => Some("NEC"),
        "paladin" => Some("PAL"),
        "ranger" => Some("RNG"),
        "rogue" => Some("ROG"),
        "shadow knight" => Some("SHK"),
        "shaman" => Some("SHM"),
        "warrior" => Some("WAR"),
        "wizard" => Some("WIZ"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> LogSource {
        LogSource::new("client-1", "Bilka", "teek")
    }

    fn line(body: &str) -> RawLogLine {
        RawLogLine {
            source: source(),
            timestamp: None,
            body: Arc::from(body),
        }
    }

    fn parse_who(lines: &[&str]) -> Vec<WhoResult> {
        let mut parser = IdentityParser::default();
        let mut events = Vec::new();
        for body in lines {
            parser.parse(&line(body), &mut events).unwrap();
        }
        events
            .into_iter()
            .map(|event| match event {
                LogEvent::Identity(IdentityEvent::WhoResult(result)) => result,
                _ => panic!("unexpected event"),
            })
            .collect()
    }

    #[test]
    fn preserves_titled_who_fields_and_class_abbreviation() {
        let results = parse_who(&[
            "Players in EverQuest:",
            " AFK [130 Lyricist (Bard)] Bilka (Wood Elf) <Realm of Insanity> ZONE: The Dreadlands (dreadlands)   LFG",
            "There is 1 player in EverQuest.",
        ]);
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(&*result.character, "Bilka");
        assert_eq!(result.level, Some(130));
        assert_eq!(result.title.as_deref(), Some("Lyricist"));
        assert_eq!(result.class_name.as_deref(), Some("Bard"));
        assert_eq!(result.class_abbreviation.as_deref(), Some("BRD"));
        assert_eq!(result.race.as_deref(), Some("Wood Elf"));
        assert_eq!(result.guild.as_deref(), Some("Realm of Insanity"));
        assert_eq!(result.zone.as_deref(), Some("The Dreadlands"));
        assert_eq!(result.zone_short.as_deref(), Some("dreadlands"));
        assert!(result.is_afk);
        assert!(result.is_lfg);
    }

    #[test]
    fn preserves_untitled_anonymous_and_shadow_knight_behavior() {
        let results = parse_who(&[
            "Players in EverQuest:",
            "[1 Magician] Saabra (Dark Elf)  ZONE: North Desert of Ro (northro)",
            "[2 Shadow Knight] Orlov (Ogre)",
            "[ANONYMOUS] Someone",
            "There are 3 players in EverQuest.",
        ]);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].class_abbreviation.as_deref(), Some("MAG"));
        assert_eq!(results[1].class_abbreviation.as_deref(), Some("SHK"));
        assert!(results[2].is_anonymous);
        assert!(results[2].class_name.is_none());
    }

    #[test]
    fn who_state_ends_on_cut_short_and_resets_on_file_generation() {
        let mut parser = IdentityParser::default();
        let mut events = Vec::new();
        parser
            .parse(&line("Players in EverQuest:"), &mut events)
            .unwrap();
        parser
            .parse(&line("Your who request was cut short."), &mut events)
            .unwrap();
        parser
            .parse(&line("[1 Magician] NotInBlock"), &mut events)
            .unwrap();
        assert!(events.is_empty());

        parser
            .parse(&line("Players in EverQuest:"), &mut events)
            .unwrap();
        parser.reset_source(&source());
        parser
            .parse(&line("[1 Magician] AlsoNotInBlock"), &mut events)
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn offline_mode_does_not_advance_or_end_who_state() {
        let results = parse_who(&[
            "Players in EverQuest:",
            "OFFLINE MODE enabled",
            "[1 Magician] Saabra",
            "There is 1 player in EverQuest.",
        ]);
        assert_eq!(results.len(), 1);
        assert_eq!(&*results[0].character, "Saabra");
    }

    #[test]
    fn all_existing_class_mappings_are_stable() {
        let expected = [
            ("Bard", "BRD"),
            ("Beastlord", "BST"),
            ("Berserker", "BER"),
            ("Cleric", "CLR"),
            ("Druid", "DRU"),
            ("Enchanter", "ENC"),
            ("Magician", "MAG"),
            ("Monk", "MNK"),
            ("Necromancer", "NEC"),
            ("Paladin", "PAL"),
            ("Ranger", "RNG"),
            ("Rogue", "ROG"),
            ("Shadow Knight", "SHK"),
            ("Shaman", "SHM"),
            ("Warrior", "WAR"),
            ("Wizard", "WIZ"),
        ];
        for (class, abbreviation) in expected {
            assert_eq!(class_abbreviation(class), Some(abbreviation));
        }
        assert_eq!(class_abbreviation("Unknown"), None);
    }
}
