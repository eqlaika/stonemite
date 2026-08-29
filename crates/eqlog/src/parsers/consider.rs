use std::sync::Arc;

use crate::{ConsiderDifficulty, ConsiderEvent, DomainParser, LogEvent, ParserError, RawLogLine};

const TARGET_STANDING_MARKERS: &[&str] = &[
    " regards you ",
    " scowls at you",
    " glares at you",
    " looks your way",
    " kindly considers you",
    " looks upon you",
];

pub(super) struct ConsiderParser;

impl DomainParser for ConsiderParser {
    fn name(&self) -> &'static str {
        "consider"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        // EQ chat echoes wrap player text after a comma and opening quote.
        // Reject them before matching so quoted Consider phrases cannot become telemetry.
        if line.body.contains(", '") {
            return Ok(());
        }
        if line.body.trim().eq_ignore_ascii_case("Consider whom?") {
            events.push(LogEvent::Consider(ConsiderEvent::NoTarget));
            return Ok(());
        }
        let Some((standing, assessment)) = line.body.split_once(" -- ") else {
            return Ok(());
        };
        let Some(target) = consider_target(standing) else {
            return Ok(());
        };
        let (assessment, level) = assessment_and_level(assessment);
        let Some(difficulty) = difficulty_for(assessment) else {
            return Ok(());
        };
        events.push(LogEvent::Consider(ConsiderEvent::Target {
            target: Arc::from(target),
            difficulty,
            level,
        }));
        Ok(())
    }
}

fn consider_target(standing: &str) -> Option<&str> {
    TARGET_STANDING_MARKERS
        .iter()
        .filter_map(|marker| standing.find(marker))
        .min()
        .map(|offset| standing[..offset].trim())
        .filter(|target| !target.is_empty())
}

fn assessment_and_level(value: &str) -> (&str, Option<u16>) {
    let value = value.trim();
    let Some((assessment, level)) = value.rsplit_once(" (Lvl: ") else {
        return (value, None);
    };
    let Some(level) = level.strip_suffix(')').and_then(|value| value.parse().ok()) else {
        return (value, None);
    };
    (assessment.trim_end(), Some(level))
}

fn difficulty_for(assessment: &str) -> Option<ConsiderDifficulty> {
    let normalized = assessment.trim().trim_end_matches('.').to_ascii_lowercase();
    let difficulty = if normalized.contains("reasonably safe opponent") {
        ConsiderDifficulty::Green
    } else if normalized.contains("would have the upper hand") {
        ConsiderDifficulty::LightBlue
    } else if normalized.contains("kind of risky")
        || normalized.contains("appears to be quite formidable")
    {
        ConsiderDifficulty::Blue
    } else if normalized.contains("quite a gamble") {
        ConsiderDifficulty::White
    } else if normalized.contains("wipe the floor with you") {
        ConsiderDifficulty::Yellow
    } else if normalized.contains("tombstone") {
        ConsiderDifficulty::Red
    } else if normalized.starts_with("looks ") || normalized.starts_with("appears ") {
        ConsiderDifficulty::Unknown
    } else {
        return None;
    };
    Some(difficulty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogSource, ParserRegistry};

    fn parse(body: &str) -> Vec<LogEvent> {
        let line = RawLogLine::new(LogSource::new("pid:42", "Bilka", "xegony"), None, body);
        ParserRegistry::default()
            .parse(&line)
            .events
            .into_iter()
            .map(|event| event.event)
            .collect()
    }

    #[test]
    fn parses_live_consider_lines_with_source_target_level_and_difficulty() {
        assert_eq!(
            parse(
                "A grikbar kobold regards you indifferently -- looks like quite a gamble. (Lvl: 3)"
            ),
            vec![LogEvent::Consider(ConsiderEvent::Target {
                target: Arc::from("A grikbar kobold"),
                difficulty: ConsiderDifficulty::White,
                level: Some(3),
            })]
        );
        assert_eq!(
            parse("An aqua goblin shaman scowls at you, ready to attack -- what would you like your tombstone to say? (Lvl: 10)"),
            vec![LogEvent::Consider(ConsiderEvent::Target {
                target: Arc::from("An aqua goblin shaman"),
                difficulty: ConsiderDifficulty::Red,
                level: Some(10),
            })]
        );
        assert_eq!(
            parse("Deceit regards you indifferently -- what would you like your tombstone to say?"),
            vec![LogEvent::Consider(ConsiderEvent::Target {
                target: Arc::from("Deceit"),
                difficulty: ConsiderDifficulty::Red,
                level: None,
            })]
        );
    }

    #[test]
    fn parses_no_target_feedback_without_accepting_quoted_chat() {
        assert_eq!(
            parse("Consider whom?"),
            vec![LogEvent::Consider(ConsiderEvent::NoTarget)]
        );
        assert!(parse("Bob says, 'Consider whom?'").is_empty());
    }

    #[test]
    fn classifies_known_assessments_and_ignores_unrelated_chat() {
        let cases = [
            (
                "looks like a reasonably safe opponent.",
                ConsiderDifficulty::Green,
            ),
            (
                "looks like you would have the upper hand.",
                ConsiderDifficulty::LightBlue,
            ),
            (
                "looks kind of risky... you might win.",
                ConsiderDifficulty::Blue,
            ),
            ("appears to be quite formidable.", ConsiderDifficulty::Blue),
            (
                "looks like it would wipe the floor with you!",
                ConsiderDifficulty::Yellow,
            ),
        ];
        for (assessment, expected) in cases {
            let body = format!("Guinness regards you indifferently -- {assessment}");
            assert!(matches!(
                parse(&body).as_slice(),
                [LogEvent::Consider(ConsiderEvent::Target { difficulty, .. })] if *difficulty == expected
            ));
        }
        for assessment in cases.map(|(assessment, _)| assessment) {
            let body = format!("Bob says, 'Guinness regards you indifferently -- {assessment}'");
            assert!(
                parse(&body).is_empty(),
                "accepted quoted Consider text: {body}"
            );
        }
    }
}
