//! Bounded .NET-regex compatibility adapter.
//!
//! EQLP compiles user patterns with .NET `Regex` (IgnoreCase +
//! CultureInvariant + a 50 ms match timeout). Stonemite reproduces the
//! common construct set through `fancy-regex` (lookaround, backreferences,
//! atomic groups) with an explicit backtrack limit standing in for the
//! timeout. Constructs that cannot be translated faithfully are rejected
//! with a stable reason so the importer can quarantine the trigger instead
//! of silently activating a changed expression.

use std::collections::HashMap;

/// Hard cap on accepted pattern length; longer patterns are quarantined.
pub const MAX_PATTERN_LEN: usize = 4096;
/// Backtracking budget per match attempt. This bounds pathological patterns
/// the way EQLP's 50 ms regex timeout does.
pub const BACKTRACK_LIMIT: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexCompatError {
    /// Stable machine-readable class: `unsupported-construct`,
    /// `pattern-too-long`, or `invalid-regex`.
    pub reason: &'static str,
    pub detail: String,
}

impl std::fmt::Display for RegexCompatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason, self.detail)
    }
}

/// A compiled, case-insensitive pattern with .NET-style match snapshots.
#[derive(Clone, Debug)]
pub struct CompatRegex {
    regex: fancy_regex::Regex,
    /// Names for each capture-group slot (index 0 = whole match).
    group_names: Vec<Option<String>>,
}

/// Case-insensitive capture map mirroring EQLP's OrdinalIgnoreCase
/// dictionary: keys are stored as written, lookups fold ASCII case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchMap {
    entries: Vec<(String, String)>,
}

impl MatchMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: &str, value: String) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
        {
            entry.1 = value;
        } else {
            self.entries.push((key.to_owned(), value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(existing, _)| existing.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub fn extend_from(&mut self, other: &MatchMap) {
        for (key, value) in other.iter() {
            self.insert(key, value.to_owned());
        }
    }
}

impl<const N: usize> From<[(&str, &str); N]> for MatchMap {
    fn from(pairs: [(&str, &str); N]) -> Self {
        let mut map = MatchMap::new();
        for (key, value) in pairs {
            map.insert(key, value.to_owned());
        }
        map
    }
}

/// Result of a successful match: named/numbered captures plus the span of
/// the first overall match (for trace highlighting).
#[derive(Clone, Debug, Default)]
pub struct MatchOutcome {
    pub captures: MatchMap,
    pub spans: Vec<(usize, usize)>,
}

impl CompatRegex {
    /// Compile a .NET-flavored pattern case-insensitively.
    pub fn compile(pattern: &str) -> Result<Self, RegexCompatError> {
        if pattern.len() > MAX_PATTERN_LEN {
            return Err(RegexCompatError {
                reason: "pattern-too-long",
                detail: format!(
                    "pattern is {} characters; the limit is {MAX_PATTERN_LEN}",
                    pattern.len()
                ),
            });
        }
        detect_unsupported(pattern)?;
        let translated = translate(pattern);
        let regex = fancy_regex::RegexBuilder::new(&format!("(?i){translated}"))
            .backtrack_limit(BACKTRACK_LIMIT)
            .build()
            .map_err(|error| RegexCompatError {
                reason: "invalid-regex",
                detail: error.to_string(),
            })?;
        let group_names = regex
            .capture_names()
            .map(|name| name.map(str::to_owned))
            .collect();
        Ok(Self { regex, group_names })
    }

    /// .NET `Regex.Matches` + EQLP `SnapshotMatches`: every match's groups
    /// flattened into one case-insensitive map (later matches overwrite).
    /// Numbered groups appear under their index ("1", "2", …).
    ///
    /// Returns `None` when the pattern does not match, and treats a
    /// backtrack-limit overrun as a non-match (EQLP disables the trigger on
    /// timeout; the engine layers that policy on top of `had_error`).
    pub fn snapshot_matches(&self, text: &str) -> Result<Option<MatchOutcome>, RegexCompatError> {
        let mut outcome = MatchOutcome::default();
        let mut found = false;
        let mut start = 0;
        // Bounded number of scans: EQLP takes all matches, but a pathological
        // zero-width loop must terminate.
        for _ in 0..64 {
            if start > text.len() {
                break;
            }
            let captures =
                self.regex
                    .captures_from_pos(text, start)
                    .map_err(|error| RegexCompatError {
                        reason: "match-budget-exceeded",
                        detail: error.to_string(),
                    })?;
            let Some(captures) = captures else { break };
            let whole = captures.get(0).expect("group 0 always present");
            found = true;
            outcome.spans.push((whole.start(), whole.end()));
            for (slot, name) in self.group_names.iter().enumerate().skip(1) {
                if let Some(group) = captures.get(slot) {
                    match name {
                        Some(name) => outcome.captures.insert(name, group.as_str().to_owned()),
                        None => outcome
                            .captures
                            .insert(&slot.to_string(), group.as_str().to_owned()),
                    }
                }
            }
            start = if whole.end() > whole.start() {
                whole.end()
            } else {
                whole.end() + 1
            };
        }
        Ok(found.then_some(outcome))
    }
}

/// Reject .NET constructs that have no faithful translation.
fn detect_unsupported(pattern: &str) -> Result<(), RegexCompatError> {
    let bytes = pattern.as_bytes();
    let mut i = 0;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' if !in_class => in_class = true,
            b']' if in_class => in_class = false,
            b'-' if in_class && bytes.get(i + 1) == Some(&b'[') => {
                return Err(unsupported("character-class subtraction ([a-[b]])"));
            }
            b'(' if !in_class && bytes.get(i + 1) == Some(&b'?') => {
                let rest = &pattern[i + 2..];
                if rest.starts_with('(') {
                    return Err(unsupported("conditional expression ((?(…)…))"));
                }
                if let Some(inner) = rest.strip_prefix('<').or(rest.strip_prefix("P<")) {
                    // (?<name>…) is fine; (?<name-other>…) balances groups.
                    if let Some(end) = inner.find('>') {
                        if inner[..end].contains('-') {
                            return Err(unsupported("balancing group ((?<a-b>…))"));
                        }
                    }
                }
                // Inline options group: (?flags) or (?flags:…) where flags
                // may include a - separator. .NET's `n` (explicit capture)
                // changes group numbering we don't reproduce.
                let flag_run: String = rest
                    .chars()
                    .take_while(|c| matches!(c, 'i' | 'm' | 's' | 'x' | 'n' | '-'))
                    .collect();
                if !flag_run.is_empty()
                    && rest[flag_run.len()..].starts_with([':', ')'])
                    && flag_run.contains('n')
                {
                    return Err(unsupported("inline (?n) explicit-capture option"));
                }
            }
            b'*' | b'+' | b'?' | b'}' if !in_class && bytes.get(i + 1) == Some(&b'+') => {
                return Err(unsupported("possessive quantifier (e.g. *+)"));
            }
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

fn unsupported(what: &str) -> RegexCompatError {
    RegexCompatError {
        reason: "unsupported-construct",
        detail: format!("this pattern uses a .NET-only regex feature: {what}"),
    }
}

/// Translate .NET spellings onto fancy-regex equivalents.
fn translate(pattern: &str) -> String {
    // \Z (end, before a final newline) ≈ \z for single log lines.
    let mut result = String::with_capacity(pattern.len());
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('Z') => {
                    chars.next();
                    result.push_str("\\z");
                }
                Some(next) => {
                    let next = *next;
                    chars.next();
                    result.push('\\');
                    result.push(next);
                }
                None => result.push('\\'),
            }
            continue;
        }
        result.push(c);
    }
    result
}

/// EQLP pre-filter: the longest run of letters/digits/spaces from the start
/// of the pattern, used as a cheap contains/starts-with check before running
/// the regex.
pub fn searchable_prefix(pattern: &str, start: usize) -> String {
    pattern
        .chars()
        .skip(start)
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .collect()
}

/// Case-insensitive contains for literal (non-regex) patterns.
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Case-insensitive starts-with.
pub fn starts_with_ignore_case(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len()
        && haystack.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Pre-filter derived from a compiled pattern, mirroring EQLP's
/// StartText/ContainsText fast path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Prefilter {
    #[default]
    None,
    StartsWith(String),
    Contains(String),
}

impl Prefilter {
    pub fn for_pattern(pattern: &str) -> Self {
        if pattern.len() <= 3 {
            return Prefilter::None;
        }
        if let Some(rest) = pattern.strip_prefix('^') {
            let text = searchable_prefix(rest, 0);
            if !text.is_empty() {
                return Prefilter::StartsWith(text);
            }
        } else {
            let text = searchable_prefix(pattern, 0);
            if text.len() > 2 {
                return Prefilter::Contains(text);
            }
        }
        Prefilter::None
    }

    pub fn admits(&self, line: &str) -> bool {
        match self {
            Prefilter::None => true,
            Prefilter::StartsWith(prefix) => starts_with_ignore_case(line, prefix),
            Prefilter::Contains(needle) => contains_ignore_case(line, needle),
        }
    }
}

/// Convenience: `HashMap` view for tests.
impl From<&MatchMap> for HashMap<String, String> {
    fn from(map: &MatchMap) -> Self {
        map.iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_is_case_insensitive_like_eqlp() {
        let regex = CompatRegex::compile(r"^You have been slain by (?<killer>.+)!$").unwrap();
        let outcome = regex
            .snapshot_matches("YOU HAVE BEEN SLAIN BY a gnoll!")
            .unwrap()
            .unwrap();
        assert_eq!(outcome.captures.get("KILLER"), Some("a gnoll"));
        assert_eq!(outcome.spans, vec![(0, 31)]);
    }

    #[test]
    fn numbered_groups_and_backreferences_work() {
        let regex = CompatRegex::compile(r"(\w+) tells you, '\1'").unwrap();
        let outcome = regex
            .snapshot_matches("Kafka tells you, 'Kafka'")
            .unwrap()
            .unwrap();
        assert_eq!(outcome.captures.get("1"), Some("Kafka"));
        assert!(regex
            .snapshot_matches("Kafka tells you, 'other'")
            .unwrap()
            .is_none());
    }

    #[test]
    fn lookahead_and_atomic_groups_are_supported() {
        assert!(CompatRegex::compile(r"(?=begins to cast)begins to cast (?<spell>.+)").is_ok());
        assert!(CompatRegex::compile(r"(?>a+)b").is_ok());
    }

    #[test]
    fn dotnet_only_constructs_are_rejected_not_reinterpreted() {
        for (pattern, what) in [
            (r"(?<open-close>x)", "balancing"),
            (r"(?(name)yes|no)", "conditional"),
            (r"[a-z-[aeiou]]+", "class subtraction"),
            (r"a*+b", "possessive"),
            (r"(?n)(x)", "explicit capture"),
            (r"(?in:x)(y)", "explicit capture scoped"),
        ] {
            let error = CompatRegex::compile(pattern).unwrap_err();
            assert_eq!(error.reason, "unsupported-construct", "{what}: {error}");
        }
    }

    #[test]
    fn escaped_metatext_is_not_misdetected() {
        // Literal braces/parens behind escapes must not trip detection.
        assert!(CompatRegex::compile(r"\(\?\(not a conditional\)\)").is_ok());
        assert!(CompatRegex::compile(r"literal \[a-\[b\]\]").is_ok());
    }

    #[test]
    fn dollar_z_upper_is_translated() {
        let regex = CompatRegex::compile(r"end here\Z").unwrap();
        assert!(regex
            .snapshot_matches("it must end here")
            .unwrap()
            .is_some());
        assert!(regex.snapshot_matches("end here not").unwrap().is_none());
    }

    #[test]
    fn snapshot_flattens_all_matches_like_eqlp() {
        let regex = CompatRegex::compile(r"(?<num>\d+)").unwrap();
        let outcome = regex.snapshot_matches("10 then 25").unwrap().unwrap();
        // Later matches overwrite earlier ones, matching SnapshotMatches.
        assert_eq!(outcome.captures.get("num"), Some("25"));
        assert_eq!(outcome.spans.len(), 2);
    }

    #[test]
    fn catastrophic_backtracking_hits_the_budget_instead_of_hanging() {
        // Plain patterns are delegated to the linear-time engine and simply
        // fail to match; a lookahead forces the backtracking engine, which
        // must stop at the budget instead of hanging.
        let plain = CompatRegex::compile(r"(a+)+$").unwrap();
        let evil = "a".repeat(64) + "b";
        assert!(plain.snapshot_matches(&evil).unwrap().is_none());

        let fancy = CompatRegex::compile(r"(a+)+\1$").unwrap();
        let error = fancy.snapshot_matches(&evil).unwrap_err();
        assert_eq!(error.reason, "match-budget-exceeded");
    }

    #[test]
    fn prefilter_matches_eqlp_start_and_contains_semantics() {
        assert_eq!(
            Prefilter::for_pattern("^You have entered (?<zone>.+)"),
            Prefilter::StartsWith("You have entered ".to_owned())
        );
        assert_eq!(
            Prefilter::for_pattern("begins to cast (?<spell>.+)"),
            Prefilter::Contains("begins to cast ".to_owned())
        );
        assert_eq!(Prefilter::for_pattern(r"^\d+"), Prefilter::None);
        assert!(Prefilter::StartsWith("You".to_owned()).admits("you have entered"));
        assert!(!Prefilter::Contains("cast".to_owned()).admits("no match"));
    }
}
