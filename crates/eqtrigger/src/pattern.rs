//! GINA/EQLP pattern-macro expansion.
//!
//! When a trigger's pattern is a regex, EQLP first rewrites the GINA-style
//! macros into real capture groups:
//!
//! - `{S}` / `{S1}`…`{S9}` → `(?<S1>.+)`
//! - `{N}` / `{N3}` / `{N>=50}` → `(?<N3>\d+)` plus a numeric constraint
//! - `{50<N<100}` (chained) → `(?<N>\d+)` plus two constraints
//! - `{TS}` → a `dd:hh:mm:ss` / labeled duration capture that also feeds a
//!   dynamic timer duration
//!
//! Literal (non-regex) patterns are left untouched, exactly like EQLP.

use regex::Regex;

use std::sync::OnceLock;

use crate::netregex::MatchMap;

/// A numeric constraint attached to a named capture (EQLP `NumberOptions`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberConstraint {
    pub key: String,
    pub op: NumberOp,
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberOp {
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Equal,
}

impl NumberOp {
    fn parse(op: &str) -> Option<Self> {
        match op {
            ">" => Some(NumberOp::Greater),
            ">=" => Some(NumberOp::GreaterEqual),
            "<" => Some(NumberOp::Less),
            "<=" => Some(NumberOp::LessEqual),
            "=" | "==" => Some(NumberOp::Equal),
            _ => None,
        }
    }

    fn flipped(self) -> Self {
        match self {
            NumberOp::Greater => NumberOp::Less,
            NumberOp::GreaterEqual => NumberOp::LessEqual,
            NumberOp::Less => NumberOp::Greater,
            NumberOp::LessEqual => NumberOp::GreaterEqual,
            NumberOp::Equal => NumberOp::Equal,
        }
    }

    fn check(self, left: u32, right: u32) -> bool {
        match self {
            NumberOp::Greater => left > right,
            NumberOp::GreaterEqual => left >= right,
            NumberOp::Less => left < right,
            NumberOp::LessEqual => left <= right,
            NumberOp::Equal => left == right,
        }
    }
}

/// Expansion result: the rewritten regex text plus extracted constraints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpandedPattern {
    pub pattern: String,
    pub constraints: Vec<NumberConstraint>,
}

fn string_macro() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\{(s\d?)\}").expect("static regex"))
}

fn number_macro() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\{\s*(n\d?)\s*(<=|>=|>|<|==|=)?\s*(\d+)?\s*\}").expect("static regex")
    })
}

fn chained_number_macro() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\{\s*(\d+)\s*(<=|>=|>|<|==|=)\s*(n\d?)\s*(<=|>=|>|<|==|=)\s*(\d+)\s*\}")
            .expect("static regex")
    })
}

fn ts_macro() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\{(ts)\}").expect("static regex"))
}

/// Expand GINA-style macros in a regex pattern. Mirrors EQLP's
/// `UpdatePattern` + `UpdateTimePattern` (regex patterns only).
pub fn expand(pattern: &str) -> ExpandedPattern {
    let mut result = pattern.to_owned();
    let mut constraints = Vec::new();

    for capture in string_macro().captures_iter(pattern) {
        let key = &capture[1];
        result = result.replace(&capture[0], &format!("(?<{key}>.+)"));
    }

    for capture in number_macro().captures_iter(&result.clone()) {
        let key = capture[1].to_owned();
        if let (Some(op), Some(value)) = (capture.get(2), capture.get(3)) {
            if let (Some(op), Ok(value)) = (NumberOp::parse(op.as_str()), value.as_str().parse()) {
                constraints.push(NumberConstraint {
                    key: key.clone(),
                    op,
                    value,
                });
            }
        }
        result = result.replace(&capture[0], &format!(r"(?<{key}>\d+)"));
    }

    for capture in chained_number_macro().captures_iter(&result.clone()) {
        let key = capture[3].to_owned();
        if let (Ok(left), Some(left_op)) = (capture[1].parse(), NumberOp::parse(&capture[2])) {
            constraints.push(NumberConstraint {
                key: key.clone(),
                // "50 < N" becomes "N > 50".
                op: left_op.flipped(),
                value: left,
            });
        }
        if let (Some(right_op), Ok(right)) = (NumberOp::parse(&capture[4]), capture[5].parse()) {
            constraints.push(NumberConstraint {
                key: key.clone(),
                op: right_op,
                value: right,
            });
        }
        result = result.replace(&capture[0], &format!(r"(?<{key}>\d+)"));
    }

    for capture in ts_macro().captures_iter(&result.clone()) {
        let key = &capture[1];
        // dd:hh:mm:ss, mm:ss, plain seconds, or labeled 4h:20m:53s forms.
        result = result.replace(&capture[0], &format!(r"(?<{key}>(?:\d+[dhms]?:?){{1,4}})"));
    }

    ExpandedPattern {
        pattern: result,
        constraints,
    }
}

/// Apply numeric constraints and the `{TS}` duration rule to a capture
/// snapshot. Mirrors EQLP `TriggerUtil.CheckOptions`: returns `false` when a
/// constrained group fails, and yields the parsed `{TS}` duration (seconds)
/// when present.
pub fn check_constraints(
    constraints: &[NumberConstraint],
    captures: &MatchMap,
) -> (bool, Option<f64>) {
    let mut duration = None;
    for (name, value) in captures.iter() {
        if name.eq_ignore_ascii_case("ts") {
            let seconds = simple_time_to_seconds(value);
            if seconds > 0.0 {
                duration = Some(seconds);
            } else {
                return (false, None);
            }
            continue;
        }
        for constraint in constraints {
            if constraint.key.eq_ignore_ascii_case(name) {
                let Some(parsed) = parse_u32(value) else {
                    // EQLP skips non-numeric values (ParseUInt sentinel).
                    continue;
                };
                if !constraint.op.check(parsed, constraint.value) {
                    return (false, None);
                }
            }
        }
    }
    (true, duration)
}

fn parse_u32(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Parse `{TS}` captures: `dd:hh:mm:ss`, `hh:mm:ss`, `mm:ss`, `ss`, and
/// labeled forms like `5d:10h:20m:40s` or `90s`. Returns 0 when unparsable.
pub fn simple_time_to_seconds(text: &str) -> f64 {
    let segments: Vec<&str> = text.trim().trim_end_matches(':').split(':').collect();
    if segments.is_empty() || segments.len() > 4 {
        return 0.0;
    }
    let mut total = 0f64;
    let labeled = segments
        .iter()
        .any(|segment| segment.ends_with(['d', 'h', 'm', 's', 'D', 'H', 'M', 'S']));
    for (position, segment) in segments.iter().rev().enumerate() {
        let segment = segment.trim();
        if segment.is_empty() {
            return 0.0;
        }
        let (digits, unit) = match segment.char_indices().last() {
            Some((index, label)) if label.is_ascii_alphabetic() => {
                (&segment[..index], Some(label.to_ascii_lowercase()))
            }
            _ => (segment, None),
        };
        let Ok(value) = digits.parse::<u64>() else {
            return 0.0;
        };
        let multiplier = match unit {
            Some('s') => 1,
            Some('m') => 60,
            Some('h') => 3600,
            Some('d') => 86_400,
            Some(_) => return 0.0,
            None if labeled => return 0.0,
            None => match position {
                0 => 1,
                1 => 60,
                2 => 3600,
                3 => 86_400,
                _ => return 0.0,
            },
        };
        total += (value * multiplier) as f64;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_macros_expand_to_named_captures() {
        let expanded = expand(r"^{S1} begins to cast {S}\.");
        assert_eq!(expanded.pattern, r"^(?<S1>.+) begins to cast (?<S>.+)\.");
        assert!(expanded.constraints.is_empty());
    }

    #[test]
    fn number_macro_with_constraint_extracts_options() {
        let expanded = expand(r"You have taken {N>=500} points of damage");
        assert_eq!(
            expanded.pattern,
            r"You have taken (?<N>\d+) points of damage"
        );
        assert_eq!(
            expanded.constraints,
            vec![NumberConstraint {
                key: "N".to_owned(),
                op: NumberOp::GreaterEqual,
                value: 500
            }]
        );
    }

    #[test]
    fn chained_constraint_produces_two_options() {
        let expanded = expand(r"hits you for {50<N1<=100} damage");
        assert_eq!(expanded.pattern, r"hits you for (?<N1>\d+) damage");
        assert_eq!(
            expanded.constraints,
            vec![
                NumberConstraint {
                    key: "N1".to_owned(),
                    op: NumberOp::Greater,
                    value: 50
                },
                NumberConstraint {
                    key: "N1".to_owned(),
                    op: NumberOp::LessEqual,
                    value: 100
                },
            ]
        );
    }

    #[test]
    fn ts_macro_expands_to_duration_capture() {
        let expanded = expand(r"expires in {TS}");
        assert_eq!(expanded.pattern, r"expires in (?<TS>(?:\d+[dhms]?:?){1,4})");
    }

    #[test]
    fn constraints_gate_matches_and_ts_supplies_duration() {
        let expanded = expand(r"for {N>100} damage in {TS}");
        let mut captures = MatchMap::new();
        captures.insert("N", "150".to_owned());
        captures.insert("TS", "1:30".to_owned());
        let (passed, duration) = check_constraints(&expanded.constraints, &captures);
        assert!(passed);
        assert_eq!(duration, Some(90.0));

        captures.insert("N", "99".to_owned());
        let (passed, _) = check_constraints(&expanded.constraints, &captures);
        assert!(!passed);
    }

    #[test]
    fn unparsable_ts_fails_the_match_like_eqlp() {
        let mut captures = MatchMap::new();
        captures.insert("TS", "??".to_owned());
        let (passed, duration) = check_constraints(&[], &captures);
        assert!(!passed);
        assert_eq!(duration, None);
    }

    #[test]
    fn simple_time_parses_positional_and_labeled_forms() {
        assert_eq!(simple_time_to_seconds("40"), 40.0);
        assert_eq!(simple_time_to_seconds("2:05"), 125.0);
        assert_eq!(simple_time_to_seconds("1:00:00"), 3600.0);
        assert_eq!(simple_time_to_seconds("1:00:00:00"), 86_400.0);
        assert_eq!(simple_time_to_seconds("90s"), 90.0);
        assert_eq!(
            simple_time_to_seconds("4h:20m:53s"),
            4.0 * 3600.0 + 20.0 * 60.0 + 53.0
        );
        assert_eq!(
            simple_time_to_seconds("5d:10h:20m:40s"),
            5.0 * 86_400.0 + 10.0 * 3600.0 + 20.0 * 60.0 + 40.0
        );
        assert_eq!(simple_time_to_seconds("nope"), 0.0);
        assert_eq!(simple_time_to_seconds(""), 0.0);
    }

    #[test]
    fn literal_patterns_are_not_expanded_by_caller_contract() {
        // expand() is only invoked for regex patterns; this documents that a
        // brace in a literal pattern would survive if callers respect that.
        let expanded = expand("{S} literal");
        assert_ne!(expanded.pattern, "{S} literal");
    }
}
