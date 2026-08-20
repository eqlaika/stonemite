use std::sync::Arc;

use crate::{ChatEvent, DomainParser, IncomingTell, LogEvent, ParserError, RawLogLine};

use super::valid_player_name;

const INCOMING_TELL_MARKER: &str = " tells you, '";

pub(super) struct ChatParser;

impl DomainParser for ChatParser {
    fn name(&self) -> &'static str {
        "chat"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        if let Some(tell) = parse_incoming_tell(&line.body) {
            events.push(LogEvent::Chat(ChatEvent::IncomingTell(tell)));
        }
        Ok(())
    }
}

fn parse_incoming_tell(body: &str) -> Option<IncomingTell> {
    let (sender, quoted_message) = body.split_once(INCOMING_TELL_MARKER)?;
    let sender = sender.trim();
    let message = quoted_message.strip_suffix('\'')?;
    if !valid_player_name(sender) || message.is_empty() {
        return None;
    }
    Some(IncomingTell {
        sender: Arc::from(sender),
        message: Arc::from(message),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LogSource, ParserRegistry};

    fn parse(body: &str) -> Vec<LogEvent> {
        let source = LogSource::new("pid:42", "Bilka", "xegony");
        let line = RawLogLine::new(source, None, body);
        ParserRegistry::default()
            .parse(&line)
            .events
            .into_iter()
            .map(|event| event.event)
            .collect()
    }

    #[test]
    fn parses_incoming_tell_sender_and_message() {
        assert_eq!(
            parse("Laika tells you, 'what in the world are you doing?'").as_slice(),
            [LogEvent::Chat(ChatEvent::IncomingTell(IncomingTell {
                sender: Arc::from("Laika"),
                message: Arc::from("what in the world are you doing?"),
            }))]
        );
    }

    #[test]
    fn preserves_apostrophes_and_cross_server_senders() {
        assert_eq!(
            parse("Xegony.Laika tells you, 'I'm busy; James' group isn't.'").as_slice(),
            [LogEvent::Chat(ChatEvent::IncomingTell(IncomingTell {
                sender: Arc::from("Xegony.Laika"),
                message: Arc::from("I'm busy; James' group isn't."),
            }))]
        );
    }

    #[test]
    fn rejects_outgoing_empty_and_unterminated_tells() {
        for body in [
            "You told Laika, 'hello'",
            " tells you, 'hello'",
            "Laika tells you, ''",
            "Laika tells you, 'unterminated",
            "Bob says, 'Alice tells you, 'hello''",
            "Alice..Xegony tells you, 'hello'",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
        }
    }
}
