//! Platform-neutral parsing and lookup for passive EverQuest client spell data.
//!
//! Modern clients keep mechanics in `spells_us.txt` and localized cast strings
//! in `spells_us_str.txt`. This crate reads those files only; it never inspects
//! or communicates with a running EverQuest process.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const CLASS_COUNT: usize = 16;
const LEGACY_CLASS_OFFSET: usize = 36;
const CURRENT_CLASS_OFFSET: usize = 38;

/// Stable numeric identifier from the first field of `spells_us.txt`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpellId(u32);

impl SpellId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SpellId {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<SpellId> for u32 {
    fn from(value: SpellId) -> Self {
        value.get()
    }
}

impl fmt::Display for SpellId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Resolved spell metadata suitable for application-level behavior.
#[derive(Clone, Debug)]
pub struct SpellDefinition {
    pub id: SpellId,
    pub name: Arc<str>,
    pub base_cast_time: Duration,
    pub unique_class: Option<&'static str>,
    pub cast_on_you: Option<Arc<str>>,
    pub cast_on_other: Option<Arc<str>>,
}

impl SpellDefinition {
    /// Returns true when a log body contains one of this spell's localized
    /// landing messages.
    pub fn matches_landing_message(&self, body: &str) -> bool {
        self.cast_on_you
            .as_deref()
            .is_some_and(|message| contains_message(body, message))
            || self
                .cast_on_other
                .as_deref()
                .is_some_and(|message| contains_message(body, message))
    }
}

#[derive(Clone, Debug)]
struct SpellRecord {
    id: SpellId,
    name: Arc<str>,
    base_cast_time: Duration,
    class_levels: [u8; CLASS_COUNT],
    cast_on_you: Option<Arc<str>>,
    cast_on_other: Option<Arc<str>>,
}

impl SpellRecord {
    fn definition(&self) -> SpellDefinition {
        SpellDefinition {
            id: self.id,
            name: self.name.clone(),
            base_cast_time: self.base_cast_time,
            unique_class: self.unique_class(),
            cast_on_you: self.cast_on_you.clone(),
            cast_on_other: self.cast_on_other.clone(),
        }
    }

    fn is_player_spell(&self) -> bool {
        self.class_levels
            .iter()
            .any(|level| (1..=253).contains(level))
    }

    fn matches_class(&self, class: &str) -> bool {
        class_index(class).is_some_and(|index| (1..=253).contains(&self.class_levels[index]))
    }

    fn unique_class(&self) -> Option<&'static str> {
        let mut classes = self
            .class_levels
            .iter()
            .enumerate()
            .filter(|(_, level)| (1..=253).contains(*level))
            .map(|(index, _)| class_code(index));
        let class = classes.next()?;
        classes.next().is_none().then_some(class)
    }
}

/// In-memory, class-aware spell catalog indexed by normalized spell name.
#[derive(Default)]
pub struct SpellCatalog {
    by_name: HashMap<String, Vec<SpellRecord>>,
}

impl SpellCatalog {
    /// Loads `spells_us.txt` and, when present, `spells_us_str.txt` from an EQ
    /// client data directory.
    pub fn load(eq_dir: &Path) -> io::Result<Self> {
        let spell_text = fs::read_to_string(eq_dir.join("spells_us.txt"))?;
        let string_text = fs::read_to_string(eq_dir.join("spells_us_str.txt")).unwrap_or_default();
        Ok(Self::from_text(&spell_text, &string_text))
    }

    /// Parses spell mechanics and localized strings supplied by the caller.
    pub fn from_text(spell_text: &str, string_text: &str) -> Self {
        let strings = parse_spell_strings(string_text);
        let mut by_name: HashMap<String, Vec<SpellRecord>> = HashMap::new();
        for line in spell_text.lines() {
            let fields = line.trim_end_matches('\r').split('^').collect::<Vec<_>>();
            let Some(id) = fields
                .first()
                .map(|value| value.trim_start_matches('\u{feff}'))
                .and_then(|value| value.parse::<u32>().ok())
                .map(SpellId::new)
            else {
                continue;
            };
            let Some(name) = fields
                .get(1)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(cast_ms) = fields.get(8).and_then(|value| value.parse::<u64>().ok()) else {
                continue;
            };
            let class_offset = if fields.len() >= 168 {
                CURRENT_CLASS_OFFSET
            } else {
                LEGACY_CLASS_OFFSET
            };
            let mut class_levels = [255; CLASS_COUNT];
            for (index, output) in class_levels.iter_mut().enumerate() {
                if let Some(level) = fields
                    .get(class_offset + index)
                    .and_then(|value| value.parse::<u8>().ok())
                {
                    *output = level;
                }
            }
            let (cast_on_you, cast_on_other) = strings.get(&id).cloned().unwrap_or_default();
            by_name
                .entry(normalize(name))
                .or_default()
                .push(SpellRecord {
                    id,
                    name: Arc::from(name),
                    base_cast_time: Duration::from_millis(cast_ms),
                    class_levels,
                    cast_on_you,
                    cast_on_other,
                });
        }
        for records in by_name.values_mut() {
            records.sort_by_key(|record| record.id);
        }
        Self { by_name }
    }

    /// Resolves a spell name, preferring the candidate available to `class`.
    /// Ambiguous unclassified names fail closed unless their relevant metadata
    /// is identical.
    pub fn resolve(&self, spell_name: &str, class: Option<&str>) -> Option<SpellDefinition> {
        let candidates = self.by_name.get(&normalize(spell_name))?;
        if let Some(class) = class {
            if let Some(record) = candidates.iter().find(|record| record.matches_class(class)) {
                return Some(record.definition());
            }
        }
        if let Some(record) =
            unique_candidate(candidates.iter().filter(|record| record.is_player_spell()))
        {
            return Some(record.definition());
        }
        unique_candidate(candidates.iter()).map(SpellRecord::definition)
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

fn unique_candidate<'a>(
    mut candidates: impl Iterator<Item = &'a SpellRecord>,
) -> Option<&'a SpellRecord> {
    let first = candidates.next()?;
    if candidates.all(|candidate| {
        candidate.base_cast_time == first.base_cast_time
            && candidate.cast_on_you == first.cast_on_you
            && candidate.cast_on_other == first.cast_on_other
    }) {
        Some(first)
    } else {
        None
    }
}

fn parse_spell_strings(text: &str) -> HashMap<SpellId, (Option<Arc<str>>, Option<Arc<str>>)> {
    let mut strings = HashMap::new();
    for line in text.lines() {
        let fields = line.trim_end_matches('\r').split('^').collect::<Vec<_>>();
        let Some(id) = fields
            .first()
            .map(|value| value.trim_start_matches('\u{feff}'))
            .and_then(|value| value.parse::<u32>().ok())
            .map(SpellId::new)
        else {
            continue;
        };
        strings.insert(
            id,
            (
                nonempty(fields.get(3).copied()),
                nonempty(fields.get(4).copied()),
            ),
        );
    }
    strings
}

fn nonempty(value: Option<&str>) -> Option<Arc<str>> {
    let value = value?.trim();
    (!value.is_empty()).then(|| Arc::from(value))
}

fn contains_message(body: &str, message: &str) -> bool {
    if message.is_empty() {
        return false;
    }
    body == message || body.contains(message)
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn class_code(index: usize) -> &'static str {
    [
        "WAR", "CLR", "PAL", "RNG", "SHK", "DRU", "MNK", "BRD", "ROG", "SHM", "NEC", "WIZ", "MAG",
        "ENC", "BST", "BER",
    ][index]
}

fn class_index(class: &str) -> Option<usize> {
    match class.to_ascii_uppercase().as_str() {
        "WAR" => Some(0),
        "CLR" => Some(1),
        "PAL" => Some(2),
        "RNG" => Some(3),
        "SHK" | "SHD" => Some(4),
        "DRU" => Some(5),
        "MNK" => Some(6),
        "BRD" => Some(7),
        "ROG" => Some(8),
        "SHM" => Some(9),
        "NEC" => Some(10),
        "WIZ" => Some(11),
        "MAG" => Some(12),
        "ENC" => Some(13),
        "BST" => Some(14),
        "BER" => Some(15),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell_line(id: u32, name: &str, cast_ms: u64, class_offset: usize, class: usize) -> String {
        let field_count = if class_offset == CURRENT_CLASS_OFFSET {
            168
        } else {
            class_offset + CLASS_COUNT + 1
        };
        let mut fields = vec![String::new(); field_count];
        fields[0] = id.to_string();
        fields[1] = name.to_owned();
        fields[8] = cast_ms.to_string();
        fields[class_offset + class] = "20".to_owned();
        fields.join("^")
    }

    #[test]
    fn resolves_duplicate_names_for_the_current_persona_class() {
        let text = [
            spell_line(13, "Complete Heal", 10_000, LEGACY_CLASS_OFFSET, 1),
            spell_line(1292, "Complete Heal", 1_000, LEGACY_CLASS_OFFSET, 11),
        ]
        .join("\n");
        let strings = "13^^^You are completely healed.^'s wounds are completely healed.^\n";
        let catalog = SpellCatalog::from_text(&text, strings);

        let cleric = catalog.resolve("complete heal", Some("CLR")).unwrap();
        assert_eq!(cleric.id, SpellId::new(13));
        assert_eq!(cleric.base_cast_time, Duration::from_secs(10));
        assert_eq!(cleric.unique_class, Some("CLR"));
        assert!(cleric.matches_landing_message("You are completely healed."));
        assert!(cleric.matches_landing_message("Bilka's wounds are completely healed."));

        let wizard = catalog.resolve("Complete Heal", Some("WIZ")).unwrap();
        assert_eq!(wizard.id, SpellId::new(1292));
        assert_eq!(wizard.base_cast_time, Duration::from_secs(1));
    }

    #[test]
    fn recognizes_the_current_168_field_class_offset() {
        let text = spell_line(7, "Hymn of Restoration", 3_000, CURRENT_CLASS_OFFSET, 7);
        let catalog = SpellCatalog::from_text(&text, "");
        assert_eq!(
            catalog
                .resolve("Hymn of Restoration", Some("BRD"))
                .unwrap()
                .base_cast_time,
            Duration::from_secs(3)
        );
    }

    #[test]
    fn refuses_ambiguous_unclassified_spell_names() {
        let text = [
            spell_line(1, "Mystery", 1_000, LEGACY_CLASS_OFFSET, 1),
            spell_line(2, "Mystery", 2_000, LEGACY_CLASS_OFFSET, 11),
        ]
        .join("\n");
        let catalog = SpellCatalog::from_text(&text, "");
        assert!(catalog.resolve("Mystery", None).is_none());
    }

    #[test]
    fn ignores_malformed_records_and_accepts_a_bom() {
        let text = format!(
            "not-an-id^Bad^^^^^^^1000\n\u{feff}{}",
            spell_line(42, "Valid", 500, LEGACY_CLASS_OFFSET, 1)
        );
        let catalog = SpellCatalog::from_text(&text, "");
        assert_eq!(
            catalog.resolve("Valid", Some("CLR")).unwrap().id,
            SpellId::new(42)
        );
        assert!(catalog.resolve("Bad", Some("CLR")).is_none());
    }
}
