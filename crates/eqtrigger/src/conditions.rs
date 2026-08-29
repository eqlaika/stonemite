//! Match-variable condition expressions.
//!
//! Port of EQLP's `ConditionParser`/`ConditionEvaluator`. Grammar:
//!
//! ```text
//! expression -> and (OR and)*
//! and        -> unary (AND unary)*
//! unary      -> NOT unary | comparison
//! comparison -> primary (OP primary)?
//! primary    -> {variable} | literal | '(' expression ')'
//! ```
//!
//! Operators accept symbolic and word forms (`>=`, `gte`, `contains`, …).
//! A `None` parse result means the expression is invalid; the engine then
//! blocks the trigger (matching EQLP: non-empty unparsable condition = never
//! fires).

const MAX_NESTING_DEPTH: usize = 10;

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Or(Box<Condition>, Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
    Compare {
        left: Operand,
        op: CompareOp,
        right: Operand,
    },
    Truthy(Operand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Variable(String),
    Str(String),
    Number(f64),
    Bool(bool),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Variable(String),
    Str(String),
    Number(f64),
    Bool(bool),
    Null,
    Op(CompareOp),
    And,
    Or,
    Not,
    LeftParen,
    RightParen,
    End,
}

pub fn parse(expression: &str) -> Option<Condition> {
    if expression.trim().is_empty() {
        return None;
    }
    let tokens = tokenize(expression)?;
    let mut parser = Parser {
        tokens,
        index: 0,
        depth: 0,
    };
    let node = parser.parse_or()?;
    (parser.current() == &Token::End).then_some(node)
}

fn word_token(word: &str) -> Option<Token> {
    let lower = word.to_ascii_lowercase();
    Some(match lower.as_str() {
        "eq" => Token::Op(CompareOp::Eq),
        "neq" => Token::Op(CompareOp::Ne),
        "gt" => Token::Op(CompareOp::Gt),
        "ge" | "gte" => Token::Op(CompareOp::Ge),
        "lt" => Token::Op(CompareOp::Lt),
        "le" | "lte" => Token::Op(CompareOp::Le),
        "contains" => Token::Op(CompareOp::Contains),
        "and" => Token::And,
        "or" => Token::Or,
        "not" => Token::Not,
        "true" => Token::Bool(true),
        "false" => Token::Bool(false),
        "null" => Token::Null,
        _ => return None,
    })
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Variable: {name} or ${name}
        if c == '{' || (c == '$' && chars.get(i + 1) == Some(&'{')) {
            let brace = if c == '$' { i + 1 } else { i };
            let close = chars[brace + 1..].iter().position(|c| *c == '}')? + brace + 1;
            let name: String = chars[brace + 1..close].iter().collect();
            tokens.push(Token::Variable(name));
            i = close + 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let close = chars[i + 1..].iter().position(|other| *other == c)? + i + 1;
            tokens.push(Token::Str(chars[i + 1..close].iter().collect()));
            i = close + 1;
            continue;
        }
        if c == '(' {
            tokens.push(Token::LeftParen);
            i += 1;
            continue;
        }
        if c == ')' {
            tokens.push(Token::RightParen);
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()))
        {
            let start = i;
            if c == '-' {
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(text.parse().ok()?));
            continue;
        }
        if "=!<>&|".contains(c) {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let token = match two.as_str() {
                "==" => Some((Token::Op(CompareOp::Eq), 2)),
                "!=" | "<>" => Some((Token::Op(CompareOp::Ne), 2)),
                ">=" => Some((Token::Op(CompareOp::Ge), 2)),
                "<=" => Some((Token::Op(CompareOp::Le), 2)),
                "&&" => Some((Token::And, 2)),
                "||" => Some((Token::Or, 2)),
                _ => None,
            }
            .or(match c {
                '=' => Some((Token::Op(CompareOp::Eq), 1)),
                '>' => Some((Token::Op(CompareOp::Gt), 1)),
                '<' => Some((Token::Op(CompareOp::Lt), 1)),
                '!' => Some((Token::Not, 1)),
                _ => None,
            })?;
            tokens.push(token.0);
            i += token.1;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            tokens.push(word_token(&word).unwrap_or(Token::Str(word)));
            continue;
        }
        return None;
    }
    tokens.push(Token::End);
    Some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    depth: usize,
}

impl Parser {
    fn current(&self) -> &Token {
        self.tokens.get(self.index).unwrap_or(&Token::End)
    }

    fn advance(&mut self) {
        if self.index < self.tokens.len() - 1 {
            self.index += 1;
        }
    }

    fn parse_or(&mut self) -> Option<Condition> {
        let mut node = self.parse_and()?;
        while self.current() == &Token::Or {
            self.advance();
            let right = self.parse_and()?;
            node = Condition::Or(Box::new(node), Box::new(right));
        }
        Some(node)
    }

    fn parse_and(&mut self) -> Option<Condition> {
        let mut node = self.parse_unary()?;
        while self.current() == &Token::And {
            self.advance();
            let right = self.parse_unary()?;
            node = Condition::And(Box::new(node), Box::new(right));
        }
        Some(node)
    }

    fn parse_unary(&mut self) -> Option<Condition> {
        if self.current() == &Token::Not {
            self.depth += 1;
            if self.depth > MAX_NESTING_DEPTH {
                return None;
            }
            self.advance();
            let operand = self.parse_unary()?;
            self.depth -= 1;
            return Some(Condition::Not(Box::new(operand)));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Option<Condition> {
        let left = self.parse_primary()?;
        if let Token::Op(op) = self.current() {
            let op = *op;
            self.advance();
            let right = self.parse_primary_operand()?;
            let Primary::Operand(left) = left else {
                // A parenthesized sub-expression as a comparison operand is
                // not valid, mirroring EQLP's evaluator returning null.
                return None;
            };
            return Some(Condition::Compare { left, op, right });
        }
        Some(match left {
            Primary::Operand(operand) => Condition::Truthy(operand),
            Primary::Expression(condition) => condition,
        })
    }

    fn parse_primary(&mut self) -> Option<Primary> {
        if self.current() == &Token::LeftParen {
            self.depth += 1;
            if self.depth > MAX_NESTING_DEPTH {
                return None;
            }
            self.advance();
            let node = self.parse_or()?;
            if self.current() != &Token::RightParen {
                return None;
            }
            self.advance();
            self.depth -= 1;
            return Some(Primary::Expression(node));
        }
        self.parse_primary_operand().map(Primary::Operand)
    }

    fn parse_primary_operand(&mut self) -> Option<Operand> {
        let operand = match self.current() {
            Token::Variable(name) => Operand::Variable(name.clone()),
            Token::Str(text) => Operand::Str(text.clone()),
            Token::Number(value) => Operand::Number(*value),
            Token::Bool(value) => Operand::Bool(*value),
            Token::Null => Operand::Null,
            _ => return None,
        };
        self.advance();
        Some(operand)
    }
}

enum Primary {
    Operand(Operand),
    Expression(Condition),
}

/// Evaluate against a variable resolver returning `None` for unset names.
pub fn evaluate(condition: &Condition, resolve: &dyn Fn(&str) -> Option<String>) -> bool {
    match condition {
        Condition::Or(left, right) => evaluate(left, resolve) || evaluate(right, resolve),
        Condition::And(left, right) => evaluate(left, resolve) && evaluate(right, resolve),
        Condition::Not(inner) => !evaluate(inner, resolve),
        Condition::Truthy(operand) => match operand {
            Operand::Variable(name) => resolve(name).is_some_and(|value| !value.is_empty()),
            Operand::Bool(value) => *value,
            Operand::Null => false,
            _ => true,
        },
        Condition::Compare { left, op, right } => {
            let left_value = operand_value(left, resolve);
            let right_value = operand_value(right, resolve);
            compare(left_value, left, *op, right_value, right)
        }
    }
}

fn operand_value(operand: &Operand, resolve: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    match operand {
        Operand::Variable(name) => resolve(name),
        Operand::Str(text) => Some(text.clone()),
        Operand::Number(value) => Some(format_number(*value)),
        Operand::Bool(value) => Some(if *value { "true" } else { "false" }.to_owned()),
        Operand::Null => None,
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn compare(
    left: Option<String>,
    left_operand: &Operand,
    op: CompareOp,
    right: Option<String>,
    right_operand: &Operand,
) -> bool {
    match op {
        CompareOp::Eq => equals(left, left_operand, right, right_operand),
        CompareOp::Ne => !equals(left, left_operand, right, right_operand),
        CompareOp::Contains => match (&left, &right) {
            (Some(left), Some(right)) => crate::netregex::contains_ignore_case(left, right),
            _ => false,
        },
        CompareOp::Gt | CompareOp::Ge | CompareOp::Lt | CompareOp::Le => {
            // Unset variables compare as 0; non-numeric strings fail.
            let left = left.map_or(Some(0.0), |value| parse_number(&value));
            let right = right.map_or(Some(0.0), |value| parse_number(&value));
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            match op {
                CompareOp::Gt => left > right,
                CompareOp::Ge => left >= right,
                CompareOp::Lt => left < right,
                CompareOp::Le => left <= right,
                _ => unreachable!(),
            }
        }
    }
}

fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn equals(
    left: Option<String>,
    left_operand: &Operand,
    right: Option<String>,
    right_operand: &Operand,
) -> bool {
    match (&left, &right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => {
            // Explicit null literal = null check; two unset vars are not equal.
            let explicit_null =
                matches!(left_operand, Operand::Null) || matches!(right_operand, Operand::Null);
            explicit_null && left.is_none() && right.is_none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn eval(expression: &str, vars: &[(&str, &str)]) -> Option<bool> {
        let condition = parse(expression)?;
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.to_string()))
            .collect();
        Some(evaluate(&condition, &|name| {
            map.get(&name.to_lowercase()).cloned()
        }))
    }

    #[test]
    fn numeric_comparisons_treat_unset_as_zero() {
        assert_eq!(eval("{hp} < 50", &[]), Some(true));
        assert_eq!(eval("{hp} < 50", &[("hp", "80")]), Some(false));
        assert_eq!(eval("{hp} >= 50", &[("hp", "50")]), Some(true));
        assert_eq!(eval("{hp} > 10", &[("hp", "banana")]), Some(false));
    }

    #[test]
    fn string_equality_is_case_insensitive() {
        assert_eq!(eval("{who} = 'Kafka'", &[("who", "kafka")]), Some(true));
        assert_eq!(eval("{who} != Kafka", &[("who", "kafka")]), Some(false));
        assert_eq!(
            eval("{who} eq bare_word", &[("who", "Bare_Word")]),
            Some(true)
        );
    }

    #[test]
    fn contains_and_boolean_logic() {
        assert_eq!(
            eval(
                "{msg} contains 'flame' and ({n} > 3 or {always})",
                &[("msg", "a Flame Burst hits"), ("n", "2"), ("always", "yes")]
            ),
            Some(true)
        );
        assert_eq!(eval("not {missing}", &[]), Some(true),);
        assert_eq!(eval("!{present}", &[("present", "x")]), Some(false));
    }

    #[test]
    fn null_semantics_match_eqlp() {
        assert_eq!(eval("{a} = null", &[]), Some(true));
        assert_eq!(eval("{a} = null", &[("a", "x")]), Some(false));
        // Two unset variables are not equal.
        assert_eq!(eval("{a} = {b}", &[]), Some(false));
    }

    #[test]
    fn word_operators_parse() {
        assert_eq!(eval("{n} gte 5", &[("n", "5")]), Some(true));
        assert_eq!(eval("{n} lt 5", &[("n", "5")]), Some(false));
    }

    #[test]
    fn invalid_expressions_return_none() {
        assert!(parse("").is_none());
        assert!(parse("{unclosed").is_none());
        assert!(parse("{a} > ").is_none());
        assert!(parse("((((((((((({a})))))))))))").is_none()); // too deep
        assert!(parse("{a} = 1 extra").is_none());
    }

    #[test]
    fn parenthesized_groups_evaluate() {
        assert_eq!(
            eval(
                "({a} = 1 or {b} = 2) and {c} = 3",
                &[("b", "2"), ("c", "3")]
            ),
            Some(true)
        );
    }
}
