use std::sync::Arc;

use chrono::NaiveDateTime;

/// Opaque, stable identity for one EQ log source.
///
/// Applications choose the identifier. It may be a process ID, account slot,
/// file identity, UUID, or any other value that is stable for the lifetime of
/// the source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// A stable source-local identity assigned by the complete-record framer.
/// Sequence zero is the first accepted record in a generation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceRecordId {
    pub source: LogSourceId,
    pub generation: u64,
    pub sequence: u64,
}

impl SourceRecordId {
    pub fn new(source: LogSourceId, generation: u64, sequence: u64) -> Self {
        Self {
            source,
            generation,
            sequence,
        }
    }
}

/// A comparable local civil second from an EverQuest log timestamp.
///
/// The numeric representation deliberately performs no timezone conversion.
/// It is only an ordering/duration coordinate for logs produced on one host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EqSecond(i64);

impl EqSecond {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, seconds: i64) -> Option<Self> {
        self.0.checked_add(seconds).map(Self)
    }

    pub fn checked_sub(self, other: Self) -> Option<i64> {
        self.0.checked_sub(other.0)
    }
}

/// EQ's display timestamp, retaining the exact source text and a validated,
/// comparable local civil second when the standard envelope parses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EqTimestamp {
    original: Arc<str>,
    second: Option<EqSecond>,
}

impl EqTimestamp {
    pub fn new(timestamp: impl Into<Arc<str>>) -> Self {
        let original = timestamp.into();
        let second = parse_eq_second(&original);
        Self { original, second }
    }

    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub const fn second(&self) -> Option<EqSecond> {
        self.second
    }
}

fn parse_eq_second(value: &str) -> Option<EqSecond> {
    // EQ uses an English weekday/month timestamp. `%e` accepts a space-padded
    // day while `%d` covers zero-padded clients and captured fixtures.
    ["%a %b %e %H:%M:%S %Y", "%a %b %d %H:%M:%S %Y"]
        .into_iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
        .map(|timestamp| EqSecond::new(timestamp.and_utc().timestamp()))
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
        assert!(decoded
            .line
            .timestamp
            .as_ref()
            .and_then(EqTimestamp::second)
            .is_some());

        let unknown = RawLogLine::decode(source(), b"unrecognized line");
        assert!(!unknown.had_invalid_utf8);
        assert!(unknown.line.timestamp.is_none());
        assert_eq!(&*unknown.line.body, "unrecognized line");
    }

    #[test]
    fn retains_malformed_timestamp_text_without_fabricating_event_time() {
        let decoded = RawLogLine::decode(source(), b"[now] unparseable time");
        let timestamp = decoded.line.timestamp.expect("timestamp envelope");
        assert_eq!(timestamp.as_str(), "now");
        assert_eq!(timestamp.second(), None);
    }

    #[test]
    fn comparable_seconds_preserve_civil_duration_and_order() {
        let first = EqTimestamp::new("Wed Mar 25 11:15:35 2026")
            .second()
            .unwrap();
        let second = EqTimestamp::new("Wed Mar 25 11:15:41 2026")
            .second()
            .unwrap();
        assert_eq!(second.checked_sub(first), Some(6));
        assert!(second > first);
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
