use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Log line parsing — generic EQ log format
// ---------------------------------------------------------------------------

/// A parsed EQ log line: timestamp + message body.
#[allow(dead_code)]
pub struct LogLine<'a> {
    pub timestamp: &'a str,
    pub body: &'a str,
}

impl<'a> LogLine<'a> {
    /// Parse a raw log line into timestamp and body.
    /// Format: `[Wed Mar 25 11:15:35 2026] message body here`
    pub fn parse(line: &'a str) -> Option<Self> {
        let open = line.find('[')?;
        let close = line[open..].find(']')? + open;
        Some(Self {
            timestamp: &line[open + 1..close],
            body: line[close + 1..].trim_start_matches(' '),
        })
    }
}

// ---------------------------------------------------------------------------
// /who line parsing
// ---------------------------------------------------------------------------

/// A fully parsed /who result entry.
#[allow(dead_code)]
pub struct WhoEntry<'a> {
    pub character: &'a str,
    pub level: Option<u16>,
    pub title: Option<&'a str>,
    pub class_name: Option<&'a str>,
    pub race: Option<&'a str>,
    pub guild: Option<&'a str>,
    pub zone: Option<&'a str>,
    pub zone_short: Option<&'a str>,
    pub is_anonymous: bool,
    pub is_afk: bool,
    pub is_lfg: bool,
}

impl<'a> WhoEntry<'a> {
    /// Parse a /who result line body (timestamp already stripped).
    ///
    /// Example inputs:
    ///   ` AFK [130 Lyricist (Bard)] Bilka (Wood Elf) <Realm of Insanity> ZONE: The Dreadlands (dreadlands)   LFG`
    ///   `[1 Magician] Saabra (Dark Elf)  ZONE: North Desert of Ro (northro)`
    ///   `[ANONYMOUS] Someone`
    pub fn parse(body: &'a str) -> Option<Self> {
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

        // Character name = first word after "] ".
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

        // Parse bracket: "130 Lyricist (Bard)" or "1 Magician" or "2 Shadow Knight"
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

    pub fn class_abbreviation(&self) -> Option<&'static str> {
        self.class_name.and_then(class_abbreviation)
    }
}

/// Parse bracket content like "130 Lyricist (Bard)" or "1 Magician".
/// Returns (level, title, class_name).
fn parse_bracket(bracket: &str) -> (Option<u16>, Option<&str>, Option<&str>) {
    // Split off the level number.
    let (level_str, rest) = match bracket.find(' ') {
        Some(i) => (&bracket[..i], bracket[i + 1..].trim_start()),
        None => return (None, None, None),
    };
    let level = level_str.parse::<u16>().ok();

    // Titled: "Lyricist (Bard)" — class is in parens.
    if let (Some(popen), Some(pclose)) = (rest.rfind('('), rest.rfind(')')) {
        if pclose > popen + 1 {
            let class_name = &rest[popen + 1..pclose];
            let title = rest[..popen].trim();
            let title = if title.is_empty() { None } else { Some(title) };
            return (level, title, Some(class_name));
        }
    }

    // Untitled: "Magician" or "Shadow Knight" — everything after level is the class.
    if rest.is_empty() {
        (level, None, None)
    } else {
        (level, None, Some(rest))
    }
}

/// Extract the first parenthesized value after `skip` bytes (e.g. race after character name).
fn parse_parens(s: &str, skip: usize) -> Option<&str> {
    let rest = s.get(skip..)?;
    let open = rest.find('(')?;
    let close = rest.find(')')?;
    if close > open + 1 {
        Some(&rest[open + 1..close])
    } else {
        None
    }
}

/// Extract guild name from `<Guild Name>`.
fn parse_guild(s: &str) -> Option<&str> {
    let open = s.find('<')?;
    let close = s.find('>')?;
    if close > open + 1 {
        Some(&s[open + 1..close])
    } else {
        None
    }
}

/// Extract zone name from `ZONE: The Dreadlands (dreadlands)`.
fn parse_zone_name(s: &str) -> Option<&str> {
    let idx = s.find("ZONE: ")?;
    let rest = &s[idx + 6..];
    // Zone name ends at the short name in parens, or end of string.
    let end = rest.rfind('(').unwrap_or(rest.len());
    let name = rest[..end].trim();
    if name.is_empty() { None } else { Some(name) }
}

/// Extract zone short name from `ZONE: ... (shortname)`.
fn parse_zone_short(s: &str) -> Option<&str> {
    let idx = s.find("ZONE: ")?;
    let rest = &s[idx..];
    let open = rest.rfind('(')?;
    let close = rest.rfind(')')?;
    if close > open + 1 {
        Some(&rest[open + 1..close])
    } else {
        None
    }
}

fn class_abbreviation(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
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

// ---------------------------------------------------------------------------
// Log tailer — file tracking and /who state machine
// ---------------------------------------------------------------------------

pub struct ClassUpdate {
    pub character: String,
    pub server: String,
    pub class_abbrev: &'static str,
}

#[derive(Default)]
enum WhoParseState {
    #[default]
    Idle,
    InBlock,
}

struct FileState {
    offset: u64,
    who_state: WhoParseState,
}

pub struct LogTailer {
    files: HashMap<PathBuf, FileState>,
}

impl LogTailer {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    pub fn poll(
        &mut self,
        eq_dir: &Path,
        active_chars: &[(String, String)],
    ) -> Vec<ClassUpdate> {
        let logs_dir = eq_dir.join("Logs");
        let mut updates = Vec::new();

        // Build set of expected log paths.
        let mut expected: HashMap<PathBuf, &str> = HashMap::new();
        for (name, server) in active_chars {
            let filename = format!("eqlog_{name}_{server}.txt");
            let path = logs_dir.join(filename);
            expected.insert(path, server.as_str());
        }

        // Remove entries for characters no longer active.
        self.files.retain(|path, _| expected.contains_key(path));

        for (path, server) in &expected {
            let Ok(mut file) = File::open(path) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            let file_len = metadata.len();

            let file_state = self.files.entry(path.clone()).or_insert_with(|| {
                // New file: seek to end, skip history.
                FileState {
                    offset: file_len,
                    who_state: WhoParseState::default(),
                }
            });

            // Handle truncation.
            if file_len < file_state.offset {
                file_state.offset = 0;
                file_state.who_state = WhoParseState::Idle;
            }

            if file_state.offset >= file_len {
                continue;
            }

            if file.seek(SeekFrom::Start(file_state.offset)).is_err() {
                continue;
            }

            let mut buf = Vec::new();
            let bytes_to_read = (file_len - file_state.offset).min(1024 * 1024) as usize;
            buf.resize(bytes_to_read, 0);
            let Ok(n) = file.read(&mut buf) else {
                continue;
            };
            buf.truncate(n);
            file_state.offset += n as u64;

            let text = String::from_utf8_lossy(&buf);
            for line in text.lines() {
                let Some(log_line) = LogLine::parse(line) else { continue };
                process_who_line(log_line.body, &mut file_state.who_state, server, &mut updates);
            }
        }

        updates
    }
}

fn process_who_line(
    body: &str,
    state: &mut WhoParseState,
    server: &str,
    updates: &mut Vec<ClassUpdate>,
) {
    if body.starts_with("OFFLINE MODE") {
        return;
    }

    match state {
        WhoParseState::Idle => {
            if body.contains("Players in EverQuest:") {
                *state = WhoParseState::InBlock;
            }
        }
        WhoParseState::InBlock => {
            if (body.contains("There is") || body.contains("There are"))
                && body.contains("player")
            {
                *state = WhoParseState::Idle;
                return;
            }
            if body.contains("who request was cut short") {
                *state = WhoParseState::Idle;
                return;
            }
            if let Some(entry) = WhoEntry::parse(body) {
                if let Some(abbrev) = entry.class_abbreviation() {
                    updates.push(ClassUpdate {
                        character: entry.character.to_string(),
                        server: server.to_string(),
                        class_abbrev: abbrev,
                    });
                }
            }
        }
    }
}
