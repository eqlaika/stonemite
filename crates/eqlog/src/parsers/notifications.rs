use std::sync::Arc;

use crate::{DomainParser, LogEvent, NotificationEvent, ParserError, RawLogLine};

use super::valid_player_name;

const GROUP_INVITE_SUFFIX: &str = " invites you to join a group.";
const GROUP_JOINED: &str = "You have joined the group.";
const GROUP_DECLINE_PREFIX: &str = "You cancel the invitation to join ";
const GROUP_DECLINE_SUFFIX: &str = "'s group.";
const RAID_INVITE_MARKER: &str = " invites you to join a raid.";
const TRADE_PROPOSAL_SUFFIX: &str = " is interested in making a trade.";
const TRADE_CANCELLED_SUFFIX: &str = " has cancelled the trade.";
const TRADE_CANCELLED: &str = "The trade has been cancelled.";
const TRADE_CANCELLED_BY_YOU: &str = "You have cancelled the trade.";
const RESURRECTION_OFFER: &str = "You have been offered a resurrection.";
const SLAIN_PREFIX: &str = "You have been slain by ";

pub(super) struct NotificationParser;

impl DomainParser for NotificationParser {
    fn name(&self) -> &'static str {
        "notifications"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        let body = line.body.as_ref();

        if let Some(inviter) = body.strip_suffix(GROUP_INVITE_SUFFIX) {
            if valid_player_name(inviter) {
                events.push(LogEvent::Notification(NotificationEvent::GroupInvite {
                    inviter: Arc::from(inviter),
                }));
            }
            return Ok(());
        }

        if body == GROUP_JOINED {
            events.push(LogEvent::Notification(
                NotificationEvent::GroupInviteAccepted,
            ));
            return Ok(());
        }

        if let Some(inviter) = body
            .strip_prefix(GROUP_DECLINE_PREFIX)
            .and_then(|remainder| remainder.strip_suffix(GROUP_DECLINE_SUFFIX))
        {
            if valid_player_name(inviter) {
                events.push(LogEvent::Notification(
                    NotificationEvent::GroupInviteDeclined {
                        inviter: Arc::from(inviter),
                    },
                ));
            }
            return Ok(());
        }

        if let Some((inviter, detail)) = body.split_once(RAID_INVITE_MARKER) {
            if valid_player_name(inviter) && (detail.is_empty() || detail.starts_with("  ")) {
                events.push(LogEvent::Notification(NotificationEvent::RaidInvite {
                    inviter: Arc::from(inviter),
                }));
            }
            return Ok(());
        }

        if let Some(trader) = body.strip_suffix(TRADE_PROPOSAL_SUFFIX) {
            if valid_player_name(trader) {
                events.push(LogEvent::Notification(NotificationEvent::TradeProposed {
                    trader: Arc::from(trader),
                }));
            }
            return Ok(());
        }

        let named_trade_cancelled = body
            .strip_suffix(TRADE_CANCELLED_SUFFIX)
            .is_some_and(valid_player_name);
        if named_trade_cancelled || matches!(body, TRADE_CANCELLED | TRADE_CANCELLED_BY_YOU) {
            events.push(LogEvent::Notification(NotificationEvent::TradeCancelled));
            return Ok(());
        }

        if body == RESURRECTION_OFFER {
            events.push(LogEvent::Notification(
                NotificationEvent::ResurrectionOffered,
            ));
            return Ok(());
        }

        if let Some(killer) = body
            .strip_prefix(SLAIN_PREFIX)
            .and_then(|remainder| remainder.strip_suffix('!'))
            .filter(|killer| !killer.trim().is_empty())
        {
            events.push(LogEvent::Notification(NotificationEvent::CharacterSlain {
                killer: Arc::from(killer),
            }));
        }

        Ok(())
    }
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
    fn parses_group_and_raid_invites_with_inviters() {
        assert_eq!(
            parse("Laika invites you to join a group."),
            vec![LogEvent::Notification(NotificationEvent::GroupInvite {
                inviter: Arc::from("Laika"),
            })]
        );
        assert_eq!(
            parse("Xegony.Laika invites you to join a raid.  Would you like to join?  If you have a mercenary, it will automatically be suspended."),
            vec![LogEvent::Notification(NotificationEvent::RaidInvite {
                inviter: Arc::from("Xegony.Laika"),
            })]
        );
    }

    #[test]
    fn parses_group_invite_acceptance_and_decline() {
        assert_eq!(
            parse(GROUP_JOINED),
            vec![LogEvent::Notification(
                NotificationEvent::GroupInviteAccepted
            )]
        );
        assert_eq!(
            parse("You cancel the invitation to join Laika's group."),
            vec![LogEvent::Notification(
                NotificationEvent::GroupInviteDeclined {
                    inviter: Arc::from("Laika"),
                }
            )]
        );
        assert!(parse("You cancel the invitation to join quoted text's group.").is_empty());
    }

    #[test]
    fn parses_trade_proposals_and_cancellations() {
        assert_eq!(
            parse("Laika is interested in making a trade."),
            vec![LogEvent::Notification(NotificationEvent::TradeProposed {
                trader: Arc::from("Laika"),
            })]
        );
        for body in [
            "Laika has cancelled the trade.",
            TRADE_CANCELLED,
            TRADE_CANCELLED_BY_YOU,
        ] {
            assert_eq!(
                parse(body),
                vec![LogEvent::Notification(NotificationEvent::TradeCancelled)],
                "failed to parse {body:?}"
            );
        }
    }

    #[test]
    fn parses_resurrection_offer() {
        assert_eq!(
            parse(RESURRECTION_OFFER),
            vec![LogEvent::Notification(
                NotificationEvent::ResurrectionOffered
            )]
        );
    }

    #[test]
    fn death_is_both_persistent_state_and_a_detailed_notification() {
        assert_eq!(
            parse("You have been slain by a War Swarm invader!"),
            vec![
                LogEvent::Character(crate::CharacterEvent::Died),
                LogEvent::Notification(NotificationEvent::CharacterSlain {
                    killer: Arc::from("a War Swarm invader"),
                }),
            ]
        );
    }

    #[test]
    fn rejects_outgoing_and_quoted_invites_or_malformed_deaths() {
        for body in [
            "You invite Laika to join your group.",
            "Bob says, 'Laika invites you to join a group.'",
            " invites you to join a group.",
            " is interested in making a trade.",
            "Bob says, 'Laika is interested in making a trade.'",
            "quoted text has cancelled the trade.",
            "You have been slain by !",
        ] {
            assert!(parse(body).is_empty(), "unexpected event for {body:?}");
        }
    }
}
