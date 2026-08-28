use std::sync::Arc;

use super::{DomainParser, ParserError};
use crate::{CastKind, CastingEvent, LogEvent, RawLogLine};

const CONCENTRATION_RECOVERED: &str = "You regain your concentration and continue your casting.";
const SONG_INTERRUPTED_MESSAGES: &[&str] = &[
    "Your song ends abruptly.",
    "You miss a note, bringing your song to a close!",
];

pub(super) struct CastingParser;

impl DomainParser for CastingParser {
    fn name(&self) -> &'static str {
        "casting"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();
        if looks_quoted(body) {
            return Ok(());
        }

        let event = if let Some(spell) = named_between(body, "You begin casting ", ".") {
            Some(CastingEvent::Started {
                spell: Arc::from(spell),
                kind: CastKind::Spell,
            })
        } else if let Some(spell) = named_between(body, "You begin singing ", ".") {
            Some(CastingEvent::Started {
                spell: Arc::from(spell),
                kind: CastKind::Song,
            })
        } else if body == "Your spell fizzles!" {
            Some(CastingEvent::Fizzled { spell: None })
        } else if let Some(spell) = named_between(body, "Your ", " spell fizzles!") {
            Some(CastingEvent::Fizzled {
                spell: Some(Arc::from(spell)),
            })
        } else if body == "Your spell is interrupted." || SONG_INTERRUPTED_MESSAGES.contains(&body)
        {
            Some(CastingEvent::Interrupted { spell: None })
        } else if let Some(spell) = named_between(body, "Your ", " spell is interrupted.") {
            Some(CastingEvent::Interrupted {
                spell: Some(Arc::from(spell)),
            })
        } else if body == CONCENTRATION_RECOVERED {
            Some(CastingEvent::ConcentrationRecovered)
        } else {
            parse_resist(body).map(|spell| CastingEvent::Resisted {
                spell: Arc::from(spell),
            })
        };

        if let Some(event) = event {
            events.push(LogEvent::Casting(event));
        }
        Ok(())
    }
}

fn named_between<'a>(body: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let value = body.strip_prefix(prefix)?.strip_suffix(suffix)?.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_resist(body: &str) -> Option<&str> {
    if let Some(spell) = named_between(body, "Your target resisted the ", " spell.") {
        return Some(spell);
    }
    let (_, spell) = body.split_once(" resisted your ")?;
    let spell = spell.strip_suffix('!')?.trim();
    (!spell.is_empty()).then_some(spell)
}

fn looks_quoted(body: &str) -> bool {
    body.contains(" says, '")
        || body.contains(" tells you, '")
        || body.contains(" told you, '")
        || body.contains(" shouts, '")
        || body.contains(" auctions, '")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogSource, ParserRegistry, RawLogLine};

    fn parse(body: &str) -> Vec<LogEvent> {
        let line = RawLogLine::new(LogSource::new("pid:42", "Bilka", "teek"), None, body);
        ParserRegistry::default()
            .parse(&line)
            .events
            .into_iter()
            .filter_map(|event| matches!(event.event, LogEvent::Casting(_)).then_some(event.event))
            .collect()
    }

    #[test]
    fn parses_spell_and_song_starts_with_names() {
        assert_eq!(
            parse("You begin casting Complete Heal."),
            vec![LogEvent::Casting(CastingEvent::Started {
                spell: Arc::from("Complete Heal"),
                kind: CastKind::Spell,
            })]
        );
        assert_eq!(
            parse("You begin singing Psalm of Veeshan."),
            vec![LogEvent::Casting(CastingEvent::Started {
                spell: Arc::from("Psalm of Veeshan"),
                kind: CastKind::Song,
            })]
        );
        assert!(parse("You begin casting .").is_empty());
    }

    #[test]
    fn parses_terminal_cast_outcomes() {
        assert_eq!(
            parse("Your spell fizzles!"),
            vec![LogEvent::Casting(CastingEvent::Fizzled { spell: None })]
        );
        assert_eq!(
            parse("Your Complete Heal spell fizzles!"),
            vec![LogEvent::Casting(CastingEvent::Fizzled {
                spell: Some(Arc::from("Complete Heal")),
            })]
        );
        assert_eq!(
            parse("Your Complete Heal spell is interrupted."),
            vec![LogEvent::Casting(CastingEvent::Interrupted {
                spell: Some(Arc::from("Complete Heal")),
            })]
        );
        assert_eq!(
            parse("Your song ends abruptly."),
            vec![LogEvent::Casting(CastingEvent::Interrupted { spell: None })]
        );
        assert_eq!(
            parse("a ghoul resisted your Root!"),
            vec![LogEvent::Casting(CastingEvent::Resisted {
                spell: Arc::from("Root"),
            })]
        );
        assert_eq!(
            parse("Your target resisted the Root spell."),
            vec![LogEvent::Casting(CastingEvent::Resisted {
                spell: Arc::from("Root"),
            })]
        );
    }

    #[test]
    fn concentration_recovery_is_nonterminal_and_quoted_text_is_ignored() {
        assert_eq!(
            parse(CONCENTRATION_RECOVERED),
            vec![LogEvent::Casting(CastingEvent::ConcentrationRecovered)]
        );
        assert!(parse("Bob says, 'Your spell fizzles!'").is_empty());
    }
}
