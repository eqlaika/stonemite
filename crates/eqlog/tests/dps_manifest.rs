use eqlog::{CombatEvent, LogEvent, LogSource, ParserRegistry, RawLogLine};

#[test]
fn approved_dps_manifest_has_exact_parser_dispositions() {
    let manifest: toml::Value = include_str!("fixtures/dps/manifest.toml")
        .parse()
        .expect("valid DPS fixture manifest");
    assert_eq!(manifest["version"].as_integer(), Some(1));
    let cases = manifest["case"].as_array().expect("case array");
    assert!(cases.len() >= 15, "the finite v1 gate must stay explicit");

    let mut parser = ParserRegistry::default();
    for case in cases {
        let id = case["id"].as_str().expect("case id");
        assert!(!case["provenance"].as_str().unwrap_or_default().is_empty());
        assert!(case["included"].as_bool().is_some());
        let body = case["body"].as_str().expect("case body");
        let line = RawLogLine::new(LogSource::new("fixture", "Bilka", "xegony"), None, body);
        let outcome = parser.parse(&line);
        assert!(outcome.errors.is_empty(), "parser failure for {id}");
        let disposition = outcome.events.iter().find_map(|event| match &event.event {
            LogEvent::Combat(CombatEvent::Damage(_)) => Some("damage"),
            LogEvent::Combat(CombatEvent::Attempt(_)) => Some("attempt"),
            LogEvent::Combat(CombatEvent::TargetSlain(_)) => Some("slain"),
            _ => None,
        });
        let expected = case["expect"].as_str().expect("expected disposition");
        assert_eq!(disposition.unwrap_or("none"), expected, "fixture {id}");
    }
}
