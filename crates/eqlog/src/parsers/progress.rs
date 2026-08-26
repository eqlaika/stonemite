use crate::{DomainParser, LogEvent, ParserError, ProgressEvent, RawLogLine};

const LEVEL_GAINED: &str = "You have gained a level!";
const LEVEL_RAISED: &str = "You raise a level!";
const LEVEL_WELCOME_PREFIX: &str = "Welcome to level ";
const AA_POINT_GAINED: &str = "You have gained an ability point!";
const AA_POINT_TOTAL_PREFIX: &str = "You now have ";

pub(super) struct ProgressParser;

impl DomainParser for ProgressParser {
    fn name(&self) -> &'static str {
        "progress"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();

        if let Some(level) = parse_level_gain(body) {
            events.push(LogEvent::Progress(ProgressEvent::LevelGained { level }));
            return Ok(());
        }

        if is_aa_point_gain(body) {
            events.push(LogEvent::Progress(
                ProgressEvent::AlternateAdvancementPointGained,
            ));
        }

        Ok(())
    }
}

fn parse_level_gain(body: &str) -> Option<u16> {
    let remainder = body
        .strip_prefix(LEVEL_GAINED)
        .or_else(|| body.strip_prefix(LEVEL_RAISED))?
        .trim_start();
    remainder
        .strip_prefix(LEVEL_WELCOME_PREFIX)?
        .strip_suffix('!')?
        .parse()
        .ok()
}

fn is_aa_point_gain(body: &str) -> bool {
    let Some(remainder) = body.strip_prefix(AA_POINT_GAINED) else {
        return false;
    };
    let remainder = remainder.trim_start();
    if remainder.is_empty() {
        return true;
    }

    let Some(total) = remainder.strip_prefix(AA_POINT_TOTAL_PREFIX) else {
        return false;
    };
    [" ability point.", " ability points."]
        .into_iter()
        .find_map(|suffix| total.strip_suffix(suffix))
        .is_some_and(|points| points.parse::<u32>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogSource;

    fn parse(body: &str) -> Vec<LogEvent> {
        let source = LogSource::new("pid:42", "Bilka", "xegony");
        let line = RawLogLine::new(source, None, body);
        let mut events = Vec::new();
        ProgressParser.parse(&line, &mut events).unwrap();
        events
    }

    #[test]
    fn parses_level_gains_and_the_new_level() {
        for body in [
            "You have gained a level! Welcome to level 125!",
            "You raise a level!  Welcome to level 60!",
        ] {
            let expected_level = if body.ends_with("125!") { 125 } else { 60 };
            assert_eq!(
                parse(body),
                vec![LogEvent::Progress(ProgressEvent::LevelGained {
                    level: expected_level,
                })]
            );
        }
    }

    #[test]
    fn parses_aa_point_gains_with_or_without_the_available_total() {
        for body in [
            AA_POINT_GAINED,
            "You have gained an ability point! You now have 1 ability point.",
            "You have gained an ability point!  You now have 250 ability points.",
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Progress(
                    ProgressEvent::AlternateAdvancementPointGained
                )],
                "failed to parse {body:?}",
            );
        }
    }

    #[test]
    fn rejects_quoted_or_malformed_progress_lines() {
        for body in [
            "Bilka says, 'You have gained a level! Welcome to level 125!'",
            "You have gained a level! Welcome to level max!",
            "You have gained a level! Welcome to level 125.",
            "You have gained an ability point! You now have many ability points.",
            "You have gained an ability point! unexpected",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
        }
    }
}
