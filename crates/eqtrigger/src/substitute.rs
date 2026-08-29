//! Match-variable text substitution.
//!
//! EQLP resolves `{name}` / `${name}` tokens (with optional `.modifier:arg`)
//! against capture groups, the raw line (`{l}`), and stored variables — in
//! that order — leaving unresolved tokens verbatim. Built-in codes
//! (`{counter}`, `{repeated}`, `{logtime}`) are substituted by the engine
//! before stored variables so they take precedence.

use regex::Regex;
use std::sync::OnceLock;

use crate::netregex::MatchMap;

pub const CHARACTER_CODE: &str = "{c}";
pub const COUNTER_CODE: &str = "{counter}";
pub const REPEATED_CODE: &str = "{repeated}";
pub const LOGTIME_CODE: &str = "{logtime}";
pub const NULL_CODE: &str = "{null}";
pub const LINE_CODE: &str = "{l}";
pub const TIMER_WARN_TIME_CODE: &str = "{timer-warn-time-value}";

fn token_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\$?\{(?<name>[a-zA-Z0-9_]+)(?:\.(?<modifier>[a-zA-Z0-9_]+)(?::(?<arg>[^}]*))?)?\}",
        )
        .expect("static regex")
    })
}

/// Replace `{name}` tokens using `resolve`; unresolved tokens stay verbatim.
pub fn replace_tokens(text: &str, mut resolve: impl FnMut(&str) -> Option<String>) -> String {
    if !text.contains('{') {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for capture in token_regex().captures_iter(text) {
        let whole = capture.get(0).expect("group 0");
        result.push_str(&text[last..whole.start()]);
        last = whole.end();
        let name = &capture["name"];
        match resolve(name) {
            Some(value) => {
                let modifier = capture.name("modifier").map(|m| m.as_str());
                let arg = capture.name("arg").map(|m| m.as_str());
                result.push_str(&apply_modifier(&value, modifier, arg));
            }
            None => result.push_str(whole.as_str()),
        }
    }
    result.push_str(&text[last..]);
    result
}

/// One pass against a capture map (EQLP `ProcessMatchesText`).
pub fn replace_from_matches(text: &str, matches: Option<&MatchMap>) -> String {
    match matches {
        Some(matches) if !text.is_empty() => {
            replace_tokens(text, |name| matches.get(name).map(str::to_owned))
        }
        _ => text.to_owned(),
    }
}

/// Replace the `{l}` raw-line code (case-insensitive).
pub fn replace_line_code(text: &str, line: &str) -> String {
    replace_code(text, LINE_CODE, line)
}

/// Case-insensitive whole-code replacement (for `{c}`, `{counter}`, …).
pub fn replace_code(text: &str, code: &str, value: &str) -> String {
    if text.len() < code.len() {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        match lower.find(&code.to_ascii_lowercase()) {
            Some(index) => {
                result.push_str(&rest[..index]);
                result.push_str(value);
                rest = &rest[index + code.len()..];
            }
            None => {
                result.push_str(rest);
                return result;
            }
        }
    }
}

pub fn contains_code(text: &str, code: &str) -> bool {
    crate::netregex::contains_ignore_case(text, code)
}

/// Full display/TTS resolution order (EQLP `ProcessDisplayText`):
/// original matches → current matches → previous matches → `{l}` → variables.
pub fn resolve_template(
    template: &str,
    line: &str,
    matches: Option<&MatchMap>,
    original_matches: Option<&MatchMap>,
    previous_matches: Option<&MatchMap>,
    variables: &MatchMap,
) -> Option<String> {
    if template.is_empty() || template.eq_ignore_ascii_case(NULL_CODE) {
        return None;
    }
    let mut text = replace_from_matches(template, original_matches);
    text = replace_from_matches(&text, matches);
    text = replace_from_matches(&text, previous_matches);
    text = replace_line_code(&text, line);
    text = replace_tokens(&text, |name| variables.get(name).map(str::to_owned));
    Some(text)
}

fn apply_modifier(value: &str, modifier: Option<&str>, arg: Option<&str>) -> String {
    let Some(modifier) = modifier else {
        return value.to_owned();
    };
    match modifier.to_ascii_lowercase().as_str() {
        "capitalize" => {
            let mut chars = value.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
        "upper" => value.to_uppercase(),
        "lower" => value.to_lowercase(),
        "number" => match value.parse::<f64>() {
            Ok(number) => group_thousands(number),
            Err(_) => value.to_owned(),
        },
        "padleft" => pad(value, arg, Pad::Left),
        "padright" => pad(value, arg, Pad::Right),
        "center" => pad(value, arg, Pad::Center),
        _ => value.to_owned(),
    }
}

enum Pad {
    Left,
    Right,
    Center,
}

fn pad(value: &str, arg: Option<&str>, side: Pad) -> String {
    let width: usize = arg.and_then(|arg| arg.trim().parse().ok()).unwrap_or(0);
    let len = value.chars().count();
    if width <= len {
        return value.to_owned();
    }
    let missing = width - len;
    match side {
        Pad::Left => format!("{}{}", " ".repeat(missing), value),
        Pad::Right => format!("{}{}", value, " ".repeat(missing)),
        Pad::Center => {
            let left = missing / 2;
            format!(
                "{}{}{}",
                " ".repeat(left),
                value,
                " ".repeat(missing - left)
            )
        }
    }
}

fn group_thousands(number: f64) -> String {
    let rounded = number.round() as i64;
    let digits = rounded.abs().to_string();
    let mut grouped = String::new();
    for (count, digit) in digits.chars().rev().enumerate() {
        if count > 0 && count % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let body: String = grouped.chars().rev().collect();
    if rounded < 0 {
        format!("-{body}")
    } else {
        body
    }
}

/// Characters EQLP strips before sending text to TTS.
pub fn sanitize_tts(text: &str) -> String {
    text.chars()
        .filter(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    ' ' | '.' | ',' | '!' | '?' | ';' | ':' | '\'' | '"' | '-' | '(' | ')'
                )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_resolve_in_both_brace_styles() {
        let matches = MatchMap::from([("S1", "Bristlebane")]);
        assert_eq!(
            replace_from_matches("Hail {S1} and ${S1}!", Some(&matches)),
            "Hail Bristlebane and Bristlebane!"
        );
    }

    #[test]
    fn unresolved_tokens_stay_verbatim() {
        let matches = MatchMap::new();
        assert_eq!(
            replace_from_matches("keep {unknown} intact", Some(&matches)),
            "keep {unknown} intact"
        );
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let matches = MatchMap::from([("KILLER", "a gnoll")]);
        assert_eq!(
            replace_from_matches("slain by {killer}", Some(&matches)),
            "slain by a gnoll"
        );
    }

    #[test]
    fn modifiers_apply_after_resolution() {
        let matches = MatchMap::from([("s1", "kafka"), ("n", "1234567")]);
        assert_eq!(
            replace_from_matches("{s1.upper} {s1.capitalize} {n.number}", Some(&matches)),
            "KAFKA Kafka 1,234,567"
        );
        assert_eq!(
            replace_from_matches("[{s1.padleft:7}]", Some(&matches)),
            "[  kafka]"
        );
        assert_eq!(
            replace_from_matches("[{s1.center:9}]", Some(&matches)),
            "[  kafka  ]"
        );
    }

    #[test]
    fn resolution_order_matches_eqlp() {
        let matches = MatchMap::from([("x", "current")]);
        let previous = MatchMap::from([("x", "previous"), ("p", "prev-only")]);
        let variables = MatchMap::from([("x", "variable"), ("v", "var-only")]);
        let resolved = resolve_template(
            "{x} {p} {v} {l}",
            "the line",
            Some(&matches),
            None,
            Some(&previous),
            &variables,
        )
        .unwrap();
        assert_eq!(resolved, "current prev-only var-only the line");
    }

    #[test]
    fn null_code_suppresses_output() {
        assert_eq!(
            resolve_template("{NULL}", "x", None, None, None, &MatchMap::new()),
            None
        );
        assert_eq!(
            resolve_template("", "x", None, None, None, &MatchMap::new()),
            None
        );
    }

    #[test]
    fn code_replacement_is_case_insensitive() {
        assert_eq!(
            replace_code("count {COUNTER} times", COUNTER_CODE, "4"),
            "count 4 times"
        );
        assert_eq!(
            replace_code("{C} rules", CHARACTER_CODE, "Bilka"),
            "Bilka rules"
        );
    }

    #[test]
    fn tts_sanitizer_strips_disallowed_characters() {
        assert_eq!(
            sanitize_tts("Slain by a gnoll! <#@[]> now."),
            "Slain by a gnoll!  now."
        );
    }
}
