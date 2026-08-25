use crate::{AttackProblem, CombatEvent, DomainParser, LogEvent, ParserError, RawLogLine};

const OUTGOING_WEAPON_VERBS: &[&str] = &[
    "backstab", "bash", "bite", "claw", "crush", "frenzy", "gore", "hit", "kick", "maul", "pierce",
    "punch", "shoot", "slash", "smash", "strike",
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

const SONG_INTERRUPTED_MESSAGES: &[&str] = &[
    "Your song ends abruptly.",
    "You miss a note, bringing your song to a close!",
];

pub(super) struct CombatParser;

impl DomainParser for CombatParser {
    fn name(&self) -> &'static str {
        "combat"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();

        let event = if is_outgoing_weapon_damage(body) {
            Some(CombatEvent::WeaponDamageDealt)
        } else if is_incoming_melee_damage(body) {
            Some(CombatEvent::DamageTaken)
        } else if OUT_OF_RANGE_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::OutOfRange))
        } else if TOO_CLOSE_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::TooClose))
        } else if LINE_OF_SIGHT_MESSAGES.contains(&body) {
            Some(CombatEvent::AttackBlocked(AttackProblem::LineOfSight))
        } else if SONG_INTERRUPTED_MESSAGES.contains(&body) {
            Some(CombatEvent::SongInterrupted)
        } else if is_song_start(body) {
            Some(CombatEvent::SongStarted)
        } else {
            None
        };

        if let Some(event) = event {
            events.push(LogEvent::Combat(event));
        }
        Ok(())
    }
}

fn is_outgoing_weapon_damage(body: &str) -> bool {
    let Some(after_you) = body.strip_prefix("You ") else {
        return false;
    };
    let Some(verb) = after_you.split_whitespace().next() else {
        return false;
    };
    OUTGOING_WEAPON_VERBS.contains(&verb) && has_points_of_damage_suffix(body)
}

fn is_incoming_melee_damage(body: &str) -> bool {
    body.contains(" YOU for ") && has_points_of_damage_suffix(body)
}

fn is_song_start(body: &str) -> bool {
    body.strip_prefix("You begin singing ")
        .and_then(|remainder| remainder.strip_suffix('.'))
        .is_some_and(|name| !name.trim().is_empty())
}

fn has_points_of_damage_suffix(body: &str) -> bool {
    let Some((_, damage)) = body.rsplit_once(" for ") else {
        return false;
    };
    let mut words = damage.split_whitespace();
    starts_with_amount(damage)
        && matches!(words.next(), Some(_))
        && matches!(words.next(), Some("point" | "points"))
        && words.next() == Some("of")
        && words.next() == Some("damage.")
        && words.next().is_none()
}

fn starts_with_amount(value: &str) -> bool {
    value.split_whitespace().next().is_some_and(|amount| {
        !amount.is_empty() && amount.bytes().all(|b| b.is_ascii_digit() || b == b',')
    })
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

    #[test]
    fn parses_local_weapon_damage_without_spell_or_chat_false_positives() {
        for body in [
            "You slash a wan ghoul knight for 28 points of damage.",
            "You hit a rat for 1 point of damage.",
            "You shoot a decaying skeleton for 1,234 points of damage.",
            "You frenzy on a wan ghoul knight for 12 points of damage.",
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Combat(CombatEvent::WeaponDamageDealt)],
                "failed to parse {body:?}"
            );
        }
        for body in [
            "You hit a gnoll for 123 points of fire damage by Burst of Flames.",
            "You say, 'You slash a rat for 28 points of damage.'",
            "Laika slashes a rat for 28 points of damage.",
            "You try to slash a rat, but miss!",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
        }
    }

    #[test]
    fn parses_only_melee_damage_received_by_the_local_character() {
        for body in [
            "A wan ghoul knight slashes YOU for 31 points of damage.",
            "a rat hits YOU for 1 point of damage.",
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Combat(CombatEvent::DamageTaken)],
                "failed to parse {body:?}"
            );
        }
        for body in [
            "You were hit by non-melee for 16 damage.",
            "You have taken 450 points of damage.",
            "You have taken 1,250 damage from Venomous Cloud by poison.",
            "A wizard hits YOU for 900 points of fire damage by Sunstrike.",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
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
    fn parses_song_heartbeats_and_interruptions_without_generic_casts() {
        assert_eq!(
            parse("You begin singing Psalm of Veeshan."),
            vec![LogEvent::Combat(CombatEvent::SongStarted)]
        );
        assert!(parse("You begin casting War March of Jocelyn.").is_empty());
        for body in [
            "Your song ends abruptly.",
            "You miss a note, bringing your song to a close!",
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Combat(CombatEvent::SongInterrupted)]
            );
        }
        assert!(parse("You begin singing .").is_empty());
    }

    #[test]
    fn quoted_combat_text_does_not_emit_combat_events() {
        let events = parse("Bob tells you, 'Your target is out of range, get closer!'");
        assert!(events
            .iter()
            .all(|event| !matches!(event, LogEvent::Combat(_))));
    }
}
