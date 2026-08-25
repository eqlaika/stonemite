mod character;
mod chat;
mod combat;
mod identity;
mod notifications;
mod pets;

use std::error::Error;
use std::fmt;

use crate::{LogEvent, LogSource, ParsedLogEvent, RawLogLine};

fn valid_player_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

/// Ordered collection of coherent EQ parsing domains.
///
/// Every registered parser sees every raw line. Domains should use cheap text
/// discriminators before expensive matching and compile reusable patterns when
/// the parser is constructed rather than once per line.
pub struct ParserRegistry {
    parsers: Vec<Box<dyn DomainParser>>,
}

impl Default for ParserRegistry {
    fn default() -> Self {
        let mut registry = Self::empty();
        registry.register(character::CharacterParser);
        registry.register(chat::ChatParser);
        registry.register(combat::CombatParser);
        registry.register(identity::IdentityParser::default());
        registry.register(notifications::NotificationParser);
        registry.register(pets::PetParser);
        registry
    }
}

impl ParserRegistry {
    /// Create a registry without built-in parsers.
    pub fn empty() -> Self {
        Self {
            parsers: Vec::new(),
        }
    }

    pub fn parse(&mut self, line: &RawLogLine) -> ParseOutcome {
        let mut outcome = ParseOutcome::default();
        for parser in &mut self.parsers {
            let mut events = Vec::new();
            match parser.parse(line, &mut events) {
                Ok(()) => outcome
                    .events
                    .extend(events.into_iter().map(|event| ParsedLogEvent {
                        source: line.source.clone(),
                        timestamp: line.timestamp.clone(),
                        event,
                    })),
                Err(error) => outcome.errors.push(ParserFailure {
                    parser: parser.name(),
                    message: error.to_string(),
                }),
            }
        }
        outcome
    }

    /// Reset transient parser state for a truncated, recreated, or removed log
    /// source. Persistent telemetry state is intentionally unaffected.
    pub fn reset_source(&mut self, source: &LogSource) {
        for parser in &mut self.parsers {
            parser.reset_source(source);
        }
    }

    /// Add a parsing domain after all currently registered domains.
    pub fn register(&mut self, parser: impl DomainParser + 'static) {
        self.parsers.push(Box::new(parser));
    }
}

#[derive(Default)]
pub struct ParseOutcome {
    pub events: Vec<ParsedLogEvent>,
    pub errors: Vec<ParserFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParserFailure {
    pub parser: &'static str,
    pub message: String,
}

/// One coherent EQ log parsing domain.
pub trait DomainParser: Send {
    fn name(&self) -> &'static str;

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError>;

    fn reset_source(&mut self, _source: &LogSource) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParserError(String);

impl ParserError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ParserError {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    struct CountingParser(Arc<AtomicUsize>);

    impl DomainParser for CountingParser {
        fn name(&self) -> &'static str {
            "counting"
        }

        fn parse(
            &mut self,
            _line: &RawLogLine,
            _events: &mut Vec<LogEvent>,
        ) -> Result<(), ParserError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn pet_parsing_remains_independent_while_a_who_block_is_open() {
        let mut registry = ParserRegistry::default();
        let source = LogSource::new("client-1", "Saabra", "teek");
        let header = RawLogLine::decode(source.clone(), b"[now] Players in EverQuest:").line;
        assert!(registry.parse(&header).events.is_empty());

        let claim = RawLogLine::decode(source, b"[now] Fluffy says, 'My leader is Saabra.'").line;
        let events = registry.parse(&claim).events;
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, LogEvent::Pet(_)));
    }

    #[test]
    fn registry_dispatches_each_raw_line_to_registered_domain_parsers() {
        let count = Arc::new(AtomicUsize::new(0));
        let mut registry = ParserRegistry::empty();
        registry.register(CountingParser(count.clone()));
        registry.register(CountingParser(count.clone()));
        let line = RawLogLine::decode(
            LogSource::new("client-1", "Bilka", "teek"),
            b"[now] unknown",
        )
        .line;

        let outcome = registry.parse(&line);
        assert!(outcome.errors.is_empty());
        assert!(outcome.events.is_empty());
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
