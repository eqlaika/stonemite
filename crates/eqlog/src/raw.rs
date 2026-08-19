use std::sync::Arc;

/// Opaque, stable identity for one EQ log source.
///
/// Applications choose the identifier. It may be a process ID, account slot,
/// file identity, UUID, or any other value that is stable for the lifetime of
/// the source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogSourceId(Arc<str>);

impl LogSourceId {
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<T> From<T> for LogSourceId
where
    T: Into<Arc<str>>,
{
    fn from(id: T) -> Self {
        Self::new(id)
    }
}

/// Stable attribution supplied by the application that owns the log source.
/// Character and server names are carried downstream so parsers and telemetry
/// consumers never need to rediscover identity from line contents.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LogSource {
    pub id: LogSourceId,
    pub character: Arc<str>,
    pub server: Arc<str>,
}

impl LogSource {
    pub fn new(
        id: impl Into<LogSourceId>,
        character: impl Into<Arc<str>>,
        server: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            id: id.into(),
            character: character.into(),
            server: server.into(),
        }
    }
}

/// EQ's display timestamp. It is deliberately not a timer clock because a log
/// timestamp can be absent, malformed, or adjusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EqTimestamp(Arc<str>);

impl EqTimestamp {
    pub fn new(timestamp: impl Into<Arc<str>>) -> Self {
        Self(timestamp.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A complete newline-terminated record supplied to the parser pipeline.
///
/// `eqlog` intentionally does not read or watch files. Applications are
/// responsible for delivering only complete records in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLogLine {
    pub source: LogSource,
    pub timestamp: Option<EqTimestamp>,
    pub body: Arc<str>,
}

/// Result of decoding one complete EQ log record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRawLogLine {
    pub line: RawLogLine,
    pub had_invalid_utf8: bool,
}

impl RawLogLine {
    pub fn new(
        source: LogSource,
        timestamp: Option<EqTimestamp>,
        body: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            source,
            timestamp,
            body: body.into(),
        }
    }

    /// Decode one complete record. Invalid UTF-8 is replaced rather than
    /// splitting or dropping the record and is reported to the caller.
    pub fn decode(source: LogSource, bytes: &[u8]) -> DecodedRawLogLine {
        let decoded = String::from_utf8_lossy(bytes);
        let had_invalid_utf8 = matches!(decoded, std::borrow::Cow::Owned(_));
        let text = decoded.as_ref();

        // EverQuest normally starts every record with a bracketed timestamp.
        // Keep the historical permissive behavior and find the first bracket
        // pair so consumers do not lose unusual but otherwise valid records.
        let envelope = text.find('[').and_then(|open| {
            text[open..]
                .find(']')
                .map(|relative_close| (open, open + relative_close))
        });
        let (timestamp, body) = match envelope {
            Some((open, close)) if close > open => (
                Some(EqTimestamp::new(Arc::from(&text[open + 1..close]))),
                Arc::from(text[close + 1..].trim_start_matches(' ')),
            ),
            _ => (None, Arc::from(text)),
        };

        DecodedRawLogLine {
            line: Self {
                source,
                timestamp,
                body,
            },
            had_invalid_utf8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> LogSource {
        LogSource::new("client-7", "Bilka", "bristlebane")
    }

    #[test]
    fn parses_timestamped_and_unknown_records() {
        let decoded = RawLogLine::decode(
            source(),
            b"[Wed Mar 25 11:15:35 2026] Players in EverQuest:",
        );
        assert!(!decoded.had_invalid_utf8);
        assert_eq!(
            decoded.line.timestamp.as_ref().map(EqTimestamp::as_str),
            Some("Wed Mar 25 11:15:35 2026")
        );
        assert_eq!(&*decoded.line.body, "Players in EverQuest:");

        let unknown = RawLogLine::decode(source(), b"unrecognized line");
        assert!(!unknown.had_invalid_utf8);
        assert!(unknown.line.timestamp.is_none());
        assert_eq!(&*unknown.line.body, "unrecognized line");
    }

    #[test]
    fn invalid_utf8_is_preserved_as_one_lossy_record() {
        let decoded = RawLogLine::decode(source(), b"bad \xff line");
        assert!(decoded.had_invalid_utf8);
        assert_eq!(&*decoded.line.body, "bad \u{fffd} line");
    }

    #[test]
    fn source_identifier_is_application_defined() {
        let source = LogSource::new("account-slot:main", "Bilka", "teek");
        assert_eq!(source.id.as_str(), "account-slot:main");
    }
}
