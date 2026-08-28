use std::sync::Arc;

use crate::{
    AttackProblem, CombatAttempt, CombatEvent, DamageKind, DamageModifiers, DamageObservation,
    DamageOutcome, DomainParser, LogEvent, ObservedCombatant, ParserError, ParserProvenance,
    Perspective, RawLogLine, TargetSlainObservation,
};

const MELEE_VERBS: &[&str] = &[
    "backstab",
    "backstabs",
    "bash",
    "bashes",
    "bite",
    "bites",
    "claw",
    "claws",
    "crush",
    "crushes",
    "frenzy",
    "frenzies",
    "gore",
    "gores",
    "hit",
    "hits",
    "kick",
    "kicks",
    "maul",
    "mauls",
    "pierce",
    "pierces",
    "punch",
    "punches",
    "shoot",
    "shoots",
    "slash",
    "slashes",
    "smash",
    "smashes",
    "strike",
    "strikes",
];

const OUT_OF_RANGE_MESSAGES: &[&str] = &[
    "Your target is out of range, get closer!",
    "Your target is too far away, get closer!",
    "Your target is too far away.",
    "You are too far away to attack that target.",
];

const TOO_CLOSE_MESSAGES: &[&str] = &[
    "Your target is too close to use a ranged weapon!",
    "You are too close to your target. Get farther away.",
];

const LINE_OF_SIGHT_MESSAGES: &[&str] = &[
    "You cannot see your target.",
    "You can't hit them from here.",
];

pub(super) struct CombatParser;

impl DomainParser for CombatParser {
    fn name(&self) -> &'static str {
        "combat"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();
        if looks_quoted(body) {
            return Ok(());
        }

        if let Some(damage) = parse_damage(body) {
            if damage.attacker.perspective == Perspective::You {
                events.push(LogEvent::Combat(CombatEvent::WeaponDamageDealt));
            }
            if damage.defender.perspective == Perspective::You {
                events.push(LogEvent::Combat(CombatEvent::DamageTaken));
            }
            events.push(LogEvent::Combat(CombatEvent::Damage(damage)));
            return Ok(());
        }

        if let Some(attempt) = parse_rejected_damage_attempt(body)
            .or_else(|| parse_attempt(body))
            .or_else(|| parse_taunt(body))
        {
            events.push(LogEvent::Combat(CombatEvent::Attempt(attempt)));
            return Ok(());
        }

        if let Some(slain) = parse_slain(body) {
            events.push(LogEvent::Combat(CombatEvent::TargetSlain(slain)));
            return Ok(());
        }

        let event = if OUT_OF_RANGE_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::OutOfRange))
        } else if TOO_CLOSE_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::TooClose))
        } else if LINE_OF_SIGHT_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::LineOfSight))
        } else {
            None
        };

        if let Some(event) = event {
            events.push(LogEvent::Combat(event));
        }
        Ok(())
    }
}

fn parse_damage(body: &str) -> Option<DamageObservation> {
    parse_melee_damage(body)
        .or_else(|| parse_direct_spell_damage(body))
        .or_else(|| parse_damage_shield(body))
        .or_else(|| parse_periodic_damage(body))
}

fn parse_melee_damage(body: &str) -> Option<DamageObservation> {
    let (action, damage_text) = body.rsplit_once(" for ")?;
    let amount = parse_points_of_damage(damage_text)?;
    let (attacker_text, verb, defender_text) = split_melee_action(action)?;
    let (attacker, explicit_owner) = observed_with_owner(attacker_text)?;
    let defender = observed(defender_text)?;
    let pet_damage = explicit_owner.is_some();
    Some(DamageObservation {
        attacker,
        explicit_owner,
        defender,
        amount,
        kind: if pet_damage {
            DamageKind::Pet
        } else {
            DamageKind::Melee
        },
        ability: Some(Arc::from(normalize_melee_verb(verb))),
        outcome: DamageOutcome::Hit,
        modifiers: parse_modifiers(body),
        provenance: ParserProvenance::Melee,
    })
}

fn split_melee_action(action: &str) -> Option<(&str, &str, &str)> {
    let words: Vec<_> = action.split_whitespace().collect();
    let verb_index = words.iter().position(|word| MELEE_VERBS.contains(word))?;
    if verb_index == 0 || verb_index + 1 >= words.len() {
        return None;
    }
    let verb_start = nth_word_start(action, verb_index)?;
    let verb_end = nth_word_end(action, verb_index + 1)?;
    let attacker = action[..verb_start].trim();
    let verb = action[verb_start..verb_end].trim();
    let mut defender = action[verb_end..].trim();
    if verb == "frenzy" || verb == "frenzies" {
        defender = defender.strip_prefix("on ").unwrap_or(defender);
    }
    (!attacker.is_empty() && !defender.is_empty()).then_some((attacker, verb, defender))
}

fn nth_word_start(value: &str, index: usize) -> Option<usize> {
    let mut in_word = false;
    let mut current = 0usize;
    for (offset, character) in value.char_indices() {
        if character.is_whitespace() {
            in_word = false;
        } else if !in_word {
            if current == index {
                return Some(offset);
            }
            current += 1;
            in_word = true;
        }
    }
    None
}

fn nth_word_end(value: &str, index_after: usize) -> Option<usize> {
    if index_after == 0 {
        return Some(0);
    }
    let start = nth_word_start(value, index_after - 1)?;
    Some(
        value[start..]
            .find(char::is_whitespace)
            .map_or(value.len(), |relative| start + relative),
    )
}

fn normalize_melee_verb(verb: &str) -> &'static str {
    match verb {
        "backstabs" => "backstab",
        "bashes" => "bash",
        "bites" => "bite",
        "claws" => "claw",
        "crushes" => "crush",
        "frenzies" => "frenzy",
        "gores" => "gore",
        "hits" => "hit",
        "kicks" => "kick",
        "mauls" => "maul",
        "pierces" => "pierce",
        "punches" => "punch",
        "shoots" => "shoot",
        "slashes" => "slash",
        "smashes" => "smash",
        "strikes" => "strike",
        "backstab" => "backstab",
        "bash" => "bash",
        "bite" => "bite",
        "claw" => "claw",
        "crush" => "crush",
        "frenzy" => "frenzy",
        "gore" => "gore",
        "hit" => "hit",
        "kick" => "kick",
        "maul" => "maul",
        "pierce" => "pierce",
        "punch" => "punch",
        "shoot" => "shoot",
        "slash" => "slash",
        "smash" => "smash",
        "strike" => "strike",
        _ => "melee",
    }
}

fn parse_direct_spell_damage(body: &str) -> Option<DamageObservation> {
    let (action, damage_text) = body.rsplit_once(" for ")?;
    let (amount_text, tail) = damage_text.split_once(" point")?;
    let amount = parse_amount(amount_text)?;
    let tail = tail
        .strip_prefix("s of ")
        .or_else(|| tail.strip_prefix(" of "))?;
    let (_, ability) = tail.split_once(" damage by ")?;
    let ability = trim_period_and_modifiers(ability);
    if ability.is_empty() {
        return None;
    }
    let (attacker_text, defender_text) = action
        .split_once(" hits ")
        .or_else(|| action.split_once(" hit "))?;
    let (attacker, explicit_owner) = observed_with_owner(attacker_text)?;
    let defender = observed(defender_text)?;
    let pet_damage = explicit_owner.is_some();
    Some(DamageObservation {
        attacker,
        explicit_owner,
        defender,
        amount,
        kind: if pet_damage {
            DamageKind::Pet
        } else {
            DamageKind::DirectSpell
        },
        ability: Some(Arc::from(ability)),
        outcome: DamageOutcome::Hit,
        modifiers: parse_modifiers(body),
        provenance: ParserProvenance::DirectSpell,
    })
}

fn parse_damage_shield(body: &str) -> Option<DamageObservation> {
    let (action, damage_text) = body.rsplit_once(" for ")?;
    let amount_text = trim_period_and_modifiers(damage_text);
    let (amount, suffix) = amount_text.split_once(' ')?;
    if suffix != "point of non-melee damage" && suffix != "points of non-melee damage" {
        return None;
    }
    let amount = parse_amount(amount)?;
    let (defender_text, source) = action.split_once(" is ")?;
    let (_, source) = source.split_once(" by ")?;
    let (attacker_text, ability) = if let Some(ability) = source.strip_prefix("YOUR ") {
        ("Your", ability)
    } else if let Some((attacker, ability)) = source
        .split_once("'s ")
        .or_else(|| source.split_once("`s "))
    {
        (attacker, ability)
    } else {
        return None;
    };
    let (attacker, explicit_owner) = observed_with_owner(attacker_text)?;
    Some(DamageObservation {
        attacker,
        explicit_owner,
        defender: observed(defender_text)?,
        amount,
        kind: DamageKind::DamageShield,
        ability: Some(Arc::from(ability)),
        outcome: DamageOutcome::Hit,
        modifiers: parse_modifiers(body),
        provenance: ParserProvenance::NonMelee,
    })
}

fn parse_periodic_damage(body: &str) -> Option<DamageObservation> {
    let (defender_text, remainder) = body
        .split_once(" has taken ")
        .or_else(|| body.split_once(" have taken "))?;
    let bane = remainder.starts_with("an extra ");
    let remainder = remainder.strip_prefix("an extra ").unwrap_or(remainder);
    let amount_end = remainder.find(char::is_whitespace)?;
    let amount = parse_amount(&remainder[..amount_end])?;
    let after_amount = remainder[amount_end..].trim_start();
    let source = after_amount
        .strip_prefix("damage from ")
        .or_else(|| strip_points_non_melee_prefix(after_amount))?;
    let source = trim_period_and_modifiers(source);

    let (ability, attacker_text) = if let Some(local) = source.strip_prefix("your ") {
        (trim_spell_suffix(local), "Your")
    } else if let Some((ability, attacker)) = source.rsplit_once(" by ") {
        (trim_spell_suffix(ability), trim_spell_suffix(attacker))
    } else {
        return None;
    };
    if ability.is_empty() || attacker_text.is_empty() {
        return None;
    }
    let (attacker, explicit_owner) = observed_with_owner(attacker_text)?;
    let defender = observed(defender_text)?;
    let pet_damage = explicit_owner.is_some();
    Some(DamageObservation {
        attacker,
        explicit_owner,
        defender,
        amount,
        kind: if pet_damage {
            DamageKind::Pet
        } else if bane {
            DamageKind::Bane
        } else {
            DamageKind::DamageOverTime
        },
        ability: Some(Arc::from(ability)),
        outcome: DamageOutcome::Hit,
        modifiers: parse_modifiers(body),
        provenance: ParserProvenance::PeriodicSpell,
    })
}

fn strip_points_non_melee_prefix(value: &str) -> Option<&str> {
    let tail = value
        .strip_prefix("points of ")
        .or_else(|| value.strip_prefix("point of "))?;
    tail.strip_prefix("non-melee damage from ")
        .or_else(|| tail.strip_prefix("damage from "))
}

fn parse_rejected_damage_attempt(body: &str) -> Option<CombatAttempt> {
    let (action, damage_text) = body.rsplit_once(" for ")?;
    let damage_text = trim_period_and_modifiers(damage_text);
    if !(damage_text.ends_with("point of damage") || damage_text.ends_with("points of damage"))
        || parse_points_of_damage(damage_text).is_some()
    {
        return None;
    }
    let (attacker_text, verb, defender_text) = split_melee_action(action)?;
    let (attacker, _) = observed_with_owner(attacker_text)?;
    Some(CombatAttempt {
        attacker,
        defender: observed(defender_text)?,
        outcome: DamageOutcome::Rejected,
        kind: DamageKind::Melee,
        ability: Some(Arc::from(normalize_melee_verb(verb))),
        provenance: ParserProvenance::CombatAttempt,
    })
}

fn parse_attempt(body: &str) -> Option<CombatAttempt> {
    let (attacker_text, remainder) = body
        .split_once(" tries to ")
        .or_else(|| body.split_once(" try to "))?;
    let (action, result) = remainder.split_once(", but ")?;
    let (verb, defender_text) = action.split_once(' ')?;
    if !MELEE_VERBS.contains(&verb) || defender_text.trim().is_empty() {
        return None;
    }
    let result_lower = result.to_ascii_lowercase();
    let outcome = if result_lower.contains("invulnerable") {
        DamageOutcome::Invulnerable
    } else if result_lower.contains("dodge") {
        DamageOutcome::Dodge
    } else if result_lower.contains("parr") {
        DamageOutcome::Parry
    } else if result_lower.contains("block") {
        DamageOutcome::Block
    } else if result_lower.contains("riposte") {
        DamageOutcome::Riposte
    } else if result_lower.contains("absorb") {
        DamageOutcome::Absorbed
    } else if result_lower.contains("miss") {
        DamageOutcome::Miss
    } else {
        return None;
    };
    let (attacker, _) = observed_with_owner(attacker_text)?;
    Some(CombatAttempt {
        attacker,
        defender: observed(defender_text)?,
        outcome,
        kind: DamageKind::Melee,
        ability: Some(Arc::from(normalize_melee_verb(verb))),
        provenance: ParserProvenance::CombatAttempt,
    })
}

fn parse_taunt(body: &str) -> Option<CombatAttempt> {
    if let Some(target) = body
        .strip_prefix("You capture ")
        .and_then(|value| value.strip_suffix("'s attention!"))
    {
        return Some(CombatAttempt {
            attacker: ObservedCombatant::you(),
            defender: observed(target)?,
            outcome: DamageOutcome::Rejected,
            kind: DamageKind::OtherIncluded,
            ability: Some(Arc::from("taunt")),
            provenance: ParserProvenance::CombatAttempt,
        });
    }
    let (target, participant) = body
        .strip_suffix(" due to an improved taunt.")?
        .split_once(" is focused on attacking ")?;
    Some(CombatAttempt {
        attacker: observed(target)?,
        defender: observed(participant)?,
        outcome: DamageOutcome::Rejected,
        kind: DamageKind::OtherIncluded,
        ability: Some(Arc::from("improved taunt")),
        provenance: ParserProvenance::CombatAttempt,
    })
}

fn parse_slain(body: &str) -> Option<TargetSlainObservation> {
    if let Some(target) = body
        .strip_prefix("You have slain ")
        .and_then(|value| value.strip_suffix('!'))
    {
        return Some(TargetSlainObservation {
            target: observed(target)?,
            killer: Some(ObservedCombatant::you()),
        });
    }
    if let Some((target, killer)) = body
        .strip_suffix('!')
        .and_then(|value| value.split_once(" was slain by "))
        .or_else(|| {
            body.strip_suffix('!')
                .and_then(|value| value.split_once(" has been slain by "))
        })
    {
        return Some(TargetSlainObservation {
            target: observed(target)?,
            killer: observed(killer),
        });
    }
    body.strip_suffix(" died.")
        .and_then(observed)
        .map(|target| TargetSlainObservation {
            target,
            killer: None,
        })
}

fn observed_with_owner(value: &str) -> Option<(ObservedCombatant, Option<ObservedCombatant>)> {
    let value = value.trim();
    if let Some((pet, owner)) = value
        .strip_suffix(')')
        .and_then(|value| value.rsplit_once(" (Owner: "))
    {
        return Some((observed(pet)?, observed(owner)));
    }
    for marker in ["`s pet", "'s pet"] {
        if let Some(owner) = value.strip_suffix(marker) {
            return Some((observed(value)?, observed(owner)));
        }
    }
    Some((observed(value)?, None))
}

fn observed(value: &str) -> Option<ObservedCombatant> {
    let value = value
        .trim()
        .trim_end_matches(|character| matches!(character, '.' | '!' | ','));
    if value.is_empty() || value.contains('\u{fffd}') {
        return None;
    }
    let perspective = if value.eq_ignore_ascii_case("you") {
        Perspective::You
    } else if value.eq_ignore_ascii_case("your") {
        Perspective::Your
    } else if value.eq_ignore_ascii_case("yourself") {
        Perspective::Yourself
    } else {
        Perspective::Named
    };
    Some(ObservedCombatant {
        name: Arc::from(value),
        perspective,
    })
}

fn parse_points_of_damage(value: &str) -> Option<u64> {
    let value = trim_period_and_modifiers(value);
    let (amount, suffix) = value.split_once(' ')?;
    if suffix != "point of damage" && suffix != "points of damage" {
        return None;
    }
    parse_amount(amount)
}

fn parse_amount(value: &str) -> Option<u64> {
    if value.is_empty() {
        return None;
    }
    if value.contains(',') {
        let mut groups = value.split(',');
        let first = groups.next()?;
        if first.is_empty() || first.len() > 3 || !first.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        if !groups.all(|group| group.len() == 3 && group.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
    } else if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.replace(',', "").parse().ok()
}

fn trim_period_and_modifiers(value: &str) -> &str {
    value
        .split_once(". (")
        .map_or(value, |(value, _)| value)
        .trim_end_matches('.')
        .trim()
}

fn trim_spell_suffix(value: &str) -> &str {
    trim_period_and_modifiers(value)
        .strip_suffix(" spell")
        .unwrap_or_else(|| trim_period_and_modifiers(value))
        .trim()
}

fn parse_modifiers(body: &str) -> DamageModifiers {
    DamageModifiers {
        critical: body.contains("Critical"),
        lucky: body.contains("Lucky"),
        strikethrough: body.contains("Strikethrough"),
        wild_rampage: body.contains("Wild Rampage"),
        twincast: body.contains("Twincast"),
    }
}

fn looks_quoted(body: &str) -> bool {
    body.contains(" says, '")
        || body.contains(" tells you, '")
        || body.contains(" told you, '")
        || body.contains(" shouts, '")
        || body.contains(" auctions, '")
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

    fn damage(body: &str) -> DamageObservation {
        parse(body)
            .into_iter()
            .find_map(|event| match event {
                LogEvent::Combat(CombatEvent::Damage(damage)) => Some(damage),
                _ => None,
            })
            .expect("damage observation")
    }

    #[test]
    fn parses_first_second_and_third_person_melee_facts() {
        let local = damage("You slash a wan ghoul knight for 28 points of damage.");
        assert_eq!(local.attacker.perspective, Perspective::You);
        assert_eq!(local.defender.name.as_ref(), "a wan ghoul knight");
        assert_eq!(local.amount, 28);
        assert_eq!(local.kind, DamageKind::Melee);
        assert!(parse("You slash a rat for 28 points of damage.")
            .iter()
            .any(|event| matches!(event, LogEvent::Combat(CombatEvent::WeaponDamageDealt))));

        let third = damage(
            "Astralx crushes Sontalak for 126,225 points of damage. (Strikethrough Critical)",
        );
        assert_eq!(third.attacker.name.as_ref(), "Astralx");
        assert_eq!(third.defender.name.as_ref(), "Sontalak");
        assert_eq!(third.amount, 126_225);
        assert!(third.modifiers.critical && third.modifiers.strikethrough);

        let incoming = damage("A wan ghoul knight slashes YOU for 31 points of damage.");
        assert_eq!(incoming.defender.perspective, Perspective::You);
        assert!(
            parse("A wan ghoul knight slashes YOU for 31 points of damage.")
                .iter()
                .any(|event| matches!(event, LogEvent::Combat(CombatEvent::DamageTaken)))
        );
    }

    #[test]
    fn parses_direct_and_periodic_spell_damage() {
        let direct = damage("You hit a gnoll for 123 points of fire damage by Burst of Flames.");
        assert_eq!(direct.kind, DamageKind::DirectSpell);
        assert_eq!(direct.ability.as_deref(), Some("Burst of Flames"));

        let dot = damage("A gnoll has taken 108,790 damage from your Mind Coil Rk. II.");
        assert_eq!(dot.attacker.perspective, Perspective::Your);
        assert_eq!(dot.kind, DamageKind::DamageOverTime);
        assert_eq!(dot.ability.as_deref(), Some("Mind Coil Rk. II"));

        let observed = damage(
            "Dovhesi has taken 173674 damage from Curse of the Shrine by Grendish the Crusader.",
        );
        assert_eq!(observed.attacker.name.as_ref(), "Grendish the Crusader");
        assert_eq!(observed.defender.name.as_ref(), "Dovhesi");

        let bane = damage("a wave sentinel has taken an extra 6250000 points of non-melee damage from your Divergent Lightning spell.");
        assert_eq!(bane.kind, DamageKind::Bane);
    }

    #[test]
    fn parses_damage_shield_owner_and_ability() {
        let named =
            damage("Tantor is pierced by Tolzol's thorns for 6718 points of non-melee damage.");
        assert_eq!(named.attacker.name.as_ref(), "Tolzol");
        assert_eq!(named.defender.name.as_ref(), "Tantor");
        assert_eq!(named.kind, DamageKind::DamageShield);
        assert_eq!(named.ability.as_deref(), Some("thorns"));

        let local = damage(
            "A failed reclaimer is pierced by YOUR thorns for 193 points of non-melee damage.",
        );
        assert_eq!(local.attacker.perspective, Perspective::Your);
    }

    #[test]
    fn parses_pet_owner_and_merges_no_identity_policy_in_parser() {
        let pet = damage("Gaber (Owner: Claus) hits a worry wraith for 116 points of damage.");
        assert_eq!(pet.attacker.name.as_ref(), "Gaber");
        assert_eq!(pet.explicit_owner.as_ref().unwrap().name.as_ref(), "Claus");
        assert_eq!(pet.kind, DamageKind::Pet);
    }

    #[test]
    fn parses_attempt_outcomes_without_damage() {
        for (body, expected) in [
            (
                "You try to crush a desert madman, but miss!",
                DamageOutcome::Miss,
            ),
            (
                "A bat tries to bite YOU, but YOU dodge!",
                DamageOutcome::Dodge,
            ),
            (
                "Tolzol tries to crush Dendritic Golem, but Dendritic Golem is INVULNERABLE!",
                DamageOutcome::Invulnerable,
            ),
            (
                "Romance tries to bash Vulak`Aerr, but Vulak`Aerr parries!",
                DamageOutcome::Parry,
            ),
        ] {
            assert!(
                parse(body).iter().any(|event| matches!(
                    event,
                    LogEvent::Combat(CombatEvent::Attempt(attempt)) if attempt.outcome == expected
                )),
                "failed to parse {body:?}"
            );
        }
    }

    #[test]
    fn parses_general_slain_messages() {
        for (body, target) in [
            ("You have slain a failed bodyguard!", "a failed bodyguard"),
            ("Kizante`s pet was slain by a rockborn!", "Kizante`s pet"),
            ("Strangle`s pet has been slain by Kzerk!", "Strangle`s pet"),
            ("Terris Thule died.", "Terris Thule"),
        ] {
            assert!(parse(body).iter().any(|event| matches!(
                event,
                LogEvent::Combat(CombatEvent::TargetSlain(slain)) if slain.target.name.as_ref() == target
            )), "failed to parse {body:?}");
        }
    }

    #[test]
    fn validates_amounts_and_fails_closed_on_unknown_forms() {
        for body in [
            "You slash a rat for 1,23 points of damage.",
            "You slash a rat for many points of damage.",
            "You slash a rat for 0x10 points of damage.",
        ] {
            let events = parse(body);
            assert!(events
                .iter()
                .all(|event| !matches!(event, LogEvent::Combat(CombatEvent::Damage(_)))));
            assert!(events.iter().any(|event| matches!(
                event,
                LogEvent::Combat(CombatEvent::Attempt(attempt))
                    if attempt.outcome == DamageOutcome::Rejected
            )));
        }
        for body in [
            "You were hit by non-melee for 16 damage.",
            "A spell has taken 20 damage from an unknown source.",
        ] {
            assert!(parse(body)
                .iter()
                .all(|event| !matches!(event, LogEvent::Combat(CombatEvent::Damage(_)))));
        }
    }

    #[test]
    fn parses_positioning_and_line_of_sight_failures() {
        for (body, problem) in [
            (
                "Your target is out of range, get closer!",
                AttackProblem::OutOfRange,
            ),
            (
                "Your target is too far away, get closer!",
                AttackProblem::OutOfRange,
            ),
            (
                "Your target is too close to use a ranged weapon!",
                AttackProblem::TooClose,
            ),
            ("You cannot see your target.", AttackProblem::LineOfSight),
            ("You can't hit them from here.", AttackProblem::LineOfSight),
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Combat(CombatEvent::AttackBlocked(problem))]
            );
        }
    }

    #[test]
    fn quoted_combat_text_does_not_emit_combat_events() {
        for body in [
            "Bob tells you, 'Your target is out of range, get closer!'",
            "Bob says, 'You slash a rat for 28 points of damage.'",
        ] {
            assert!(parse(body)
                .iter()
                .all(|event| !matches!(event, LogEvent::Combat(_))));
        }
    }
}
