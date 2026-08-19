use std::sync::Arc;

use super::{DomainParser, ParserError};
use crate::{LogEvent, PetEvent, RawLogLine};

pub(super) struct PetParser;

impl DomainParser for PetParser {
    fn name(&self) -> &'static str {
        "pets"
    }

    fn parse(&mut self, line: &RawLogLine, events: &mut Vec<LogEvent>) -> Result<(), ParserError> {
        if let Some(claim) = PetClaim::parse(&line.body) {
            events.push(LogEvent::Pet(PetEvent::OwnershipClaimed {
                pet: Arc::from(claim.pet),
                owner: Arc::from(claim.owner),
            }));
        }
        Ok(())
    }
}

struct PetClaim<'a> {
    pet: &'a str,
    owner: &'a str,
}

impl<'a> PetClaim<'a> {
    fn parse(body: &'a str) -> Option<Self> {
        let marker = " says, 'My leader is ";
        let middle = body.find(marker)?;
        let pet = &body[..middle];
        if pet.is_empty() || pet.contains(' ') {
            return None;
        }
        let owner = body[middle + marker.len()..].strip_suffix(".'")?.trim();
        if owner.is_empty() || owner.contains(' ') {
            return None;
        }
        Some(Self { pet, owner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogSource;

    fn parse(body: &str) -> Vec<LogEvent> {
        let mut parser = PetParser;
        let mut events = Vec::new();
        parser
            .parse(
                &RawLogLine {
                    source: LogSource::new("client-1", "Saabra", "teek"),
                    timestamp: None,
                    body: Arc::from(body),
                },
                &mut events,
            )
            .unwrap();
        events
    }

    #[test]
    fn parses_existing_pet_claim_format() {
        let events = parse("Fluffy says, 'My leader is Saabra.'");
        assert_eq!(
            events,
            vec![LogEvent::Pet(PetEvent::OwnershipClaimed {
                pet: Arc::from("Fluffy"),
                owner: Arc::from("Saabra"),
            })]
        );
    }

    #[test]
    fn rejects_empty_and_multiword_pet_or_owner_names() {
        assert!(parse(" says, 'My leader is Saabra.'").is_empty());
        assert!(parse("Fluffy Pet says, 'My leader is Saabra.'").is_empty());
        assert!(parse("Fluffy says, 'My leader is .'").is_empty());
        assert!(parse("Fluffy says, 'My leader is Dark Owner.'").is_empty());
        assert!(parse("Fluffy says, 'My leader is Saabra'").is_empty());
    }
}
