//! EQLP `.tgf.gz` / `.ogf.gz` codec tests, including golden-fixture import
//! and export/re-import semantic equivalence.

use std::io::Write;

use eqtrigger::eqlp::{self, ExportSelection};
use eqtrigger::*;

fn gz(json: &str) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(json.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

/// A representative EQLP 2.3.x export: one folder holding one full-featured
/// trigger, one quarantine candidate, plus a text overlay and timer overlay
/// referenced by id.
const FIXTURE_TGF: &str = r##"[
  {
    "Id": null,
    "IsExpanded": true,
    "Name": "Raid Triggers",
    "OriginalId": null,
    "TriggerData": null,
    "OverlayData": null,
    "Index": 0,
    "Parent": null,
    "Nodes": [
      {
        "Id": null,
        "IsExpanded": false,
        "Name": "Complete Heal",
        "OriginalId": "nag-123",
        "TriggerData": {
          "Private": false,
          "LastTriggered": 0,
          "AltTimerName": "CH {S1}",
          "Comments": "classic CH chain",
          "RepeatedResetTime": 0.75,
          "DurationSeconds": 10.5,
          "EnableTimer": true,
          "TimerType": 1,
          "EndEarlyPattern": "{S1} has been slain",
          "EndEarlyPattern2": null,
          "EndEarlyPattern3": null,
          "EndUseRegex": true,
          "EndUseRegex2": false,
          "EndUseRegex3": false,
          "EndEarlyRepeatedCount": 0,
          "WorstEvalTime": -1,
          "Pattern": "^{S1} begins to cast Complete Heal\\.$",
          "PreviousPattern": "shouts, 'CH -- {S2}'",
          "MatchVariableCondition": "{S1} != null",
          "Priority": 2,
          "TriggerAgainOption": 2,
          "UseRegex": true,
          "PreviousUseRegex": false,
          "ActiveColor": "#FF1D397E",
          "IdleColor": null,
          "ResetColor": null,
          "FontColor": "#FFFFFFFF",
          "IconSource": "keep-this-icon.png",
          "SelectedOverlays": ["overlay-text-1", "overlay-timer-1", "missing-overlay"],
          "ResetDurationSeconds": 2,
          "WarningSeconds": 3,
          "EndEarlyTextToDisplay": "CH broken!",
          "EndTextToDisplay": "CH landed",
          "TextToDisplay": "CH inc from {S1}",
          "WarningTextToDisplay": null,
          "EndEarlyTextToSpeak": null,
          "EndTextToSpeak": "heal landed",
          "TextToSpeak": "c h incoming",
          "WarningTextToSpeak": "c h soon",
          "SoundToPlay": null,
          "EndEarlySoundToPlay": null,
          "EndSoundToPlay": null,
          "WarningSoundToPlay": null,
          "EndTimerClearVariables": "chTarget, {chCaster}",
          "ChatWebhook": "https://discord.example/webhook",
          "TextToSendToChat": "CH inc",
          "TextToShare": null,
          "TimesToLoop": 0,
          "LockoutTime": 1.5,
          "VoiceRate": 2,
          "Volume": 4,
          "VariableActions": [
            {
              "ActionType": 0,
              "DataType": 0,
              "VariableName": "chCaster",
              "Value": "{S1}",
              "Step": 1,
              "InitialValue": 0,
              "TimeToLiveSeconds": 30
            },
            {
              "ActionType": 0,
              "DataType": 1,
              "VariableName": "chCount",
              "Value": "",
              "Step": 1,
              "InitialValue": 0,
              "TimeToLiveSeconds": 0
            }
          ]
        },
        "OverlayData": null,
        "Index": 0,
        "Parent": null,
        "Nodes": []
      },
      {
        "Id": null,
        "IsExpanded": false,
        "Name": "Balancing Group Trigger",
        "OriginalId": null,
        "TriggerData": {
          "Pattern": "(?<open-close>unsupported)",
          "UseRegex": true,
          "Priority": 3,
          "Volume": 4,
          "SelectedOverlays": [],
          "VariableActions": []
        },
        "OverlayData": null,
        "Index": 1,
        "Parent": null,
        "Nodes": []
      }
    ]
  },
  {
    "Id": "overlay-text-1",
    "IsExpanded": false,
    "Name": "Raid Text",
    "OriginalId": null,
    "TriggerData": null,
    "OverlayData": {
      "IsTextOverlay": true,
      "IsTimerOverlay": false,
      "IsDefault": true,
      "FontSize": "16pt",
      "FontColor": "#FFEEEEEE",
      "BackgroundColor": "#5F000000",
      "FadeDelay": 8,
      "Top": 120,
      "Left": 260,
      "Width": 500,
      "Height": 150
    },
    "Index": 0,
    "Parent": null,
    "Nodes": []
  },
  {
    "Id": "overlay-timer-1",
    "IsExpanded": false,
    "Name": "Raid Timers",
    "OriginalId": null,
    "TriggerData": null,
    "OverlayData": {
      "IsTextOverlay": false,
      "IsTimerOverlay": true,
      "TimerMode": 1,
      "SortBy": 1,
      "FontColor": "#FFFFFFFF",
      "ActiveColor": "#FF1D397E",
      "IdleColor": "#FF8F1515",
      "ResetColor": "#FF8F1515",
      "BackgroundColor": "#5F000000",
      "ShowMillis": true,
      "Top": 300,
      "Left": 40
    },
    "Index": 1,
    "Parent": null,
    "Nodes": []
  }
]"##;

fn import_fixture() -> eqlp::EqlpImport {
    eqlp::import(&gz(FIXTURE_TGF), &[]).unwrap()
}

#[test]
fn golden_fixture_imports_with_full_field_mapping() {
    let import = import_fixture();
    assert_eq!(import.folders.len(), 1);
    assert_eq!(import.triggers.len(), 2);
    assert_eq!(import.text_overlays.len(), 1);
    assert_eq!(import.timer_overlays.len(), 1);

    let ch = &import.triggers[0];
    assert_eq!(ch.name, "Complete Heal");
    assert!(!ch.enabled, "imports must arrive disabled");
    assert_eq!(ch.folder, Some(import.folders[0].id));
    assert_eq!(ch.pattern.text, r"^{S1} begins to cast Complete Heal\.$");
    assert!(ch.pattern.use_regex);
    let previous = ch.previous_pattern.as_ref().unwrap();
    assert_eq!(previous.text, "shouts, 'CH -- {S2}'");
    assert!(!previous.use_regex);
    assert_eq!(ch.condition, "{S1} != null");
    assert_eq!(ch.lockout_seconds, 1.5);
    assert_eq!(ch.priority, 2);
    assert_eq!(ch.voice_rate, 2);
    assert_eq!(ch.display_text.as_deref(), Some("CH inc from {S1}"));
    assert_eq!(ch.speak_text.as_deref(), Some("c h incoming"));

    let timer = ch.timer.as_ref().unwrap();
    assert_eq!(timer.kind, TimerKind::Countdown);
    assert_eq!(timer.timer_name, "CH {S1}");
    assert!((timer.duration_seconds - 10.5).abs() < 1e-9);
    assert_eq!(timer.reset_duration_seconds, 2.0);
    assert_eq!(timer.warning_seconds, 3);
    assert_eq!(timer.restart_mode, TimerRestartMode::RestartSameName);
    assert_eq!(timer.warning.speak_text.as_deref(), Some("c h soon"));
    assert_eq!(timer.end.display_text.as_deref(), Some("CH landed"));
    assert_eq!(timer.early_end.display_text.as_deref(), Some("CH broken!"));
    assert_eq!(timer.end_early_patterns.len(), 1);
    assert!(timer.end_early_patterns[0].use_regex);
    assert_eq!(
        timer.end_clear_variables,
        vec!["chTarget".to_owned(), "chCaster".to_owned()]
    );

    assert_eq!(ch.variable_actions.len(), 2);
    assert_eq!(ch.variable_actions[0].op, VariableOp::SetValue);
    assert_eq!(ch.variable_actions[0].name, "chCaster");
    assert_eq!(ch.variable_actions[0].time_to_live_seconds, 30.0);
    assert_eq!(ch.variable_actions[1].op, VariableOp::SetCounter);

    // Overlay references resolved by kind; the dangling one reported.
    assert_eq!(ch.text_overlays, vec![import.text_overlays[0].id]);
    assert_eq!(ch.timer_overlays, vec![import.timer_overlays[0].id]);
    assert!(import
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "dangling-overlay" && issue.detail.contains("missing-overlay")));

    // Unexecuted fields are retained and reported.
    assert_eq!(
        ch.passthrough.get("ChatWebhook").and_then(|v| v.as_str()),
        Some("https://discord.example/webhook")
    );
    assert_eq!(
        ch.passthrough.get("IconSource").and_then(|v| v.as_str()),
        Some("keep-this-icon.png")
    );
    assert!(import
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "retained-field"));

    // The balancing-group trigger is quarantined, not reinterpreted.
    let bad = &import.triggers[1];
    assert!(bad.quarantine.is_some());
    assert_eq!(import.report.triggers_quarantined, 1);

    // Overlay presets keep their EQLP ids and geometry passthrough.
    let text = &import.text_overlays[0];
    assert_eq!(text.name, "Raid Text");
    assert_eq!(text.font_size, "16pt");
    assert_eq!(text.fade_delay_seconds, 8);
    assert_eq!(
        text.passthrough.get("Top").and_then(|v| v.as_i64()),
        Some(120)
    );
    let timer_overlay = &import.timer_overlays[0];
    assert_eq!(timer_overlay.mode, TimerOverlayMode::Cooldown);
    assert!(timer_overlay.show_millis);
    assert_eq!(timer_overlay.sort_by, 1);
}

#[test]
fn export_reimport_is_semantically_equivalent() {
    let import = import_fixture();
    let mut library = TriggerLibrary::new();
    library.folders = import.folders.clone();
    library.triggers = import.triggers.clone();
    library.text_overlays = import.text_overlays.clone();
    library.timer_overlays = import.timer_overlays.clone();

    let mut report = CompatReport::default();
    let exported = eqlp::export_triggers(
        &library,
        &ExportSelection {
            folders: vec![library.folders[0].id],
            triggers: vec![],
        },
        &mut report,
    )
    .unwrap();

    let reimport = eqlp::import(&exported, &[]).unwrap();
    assert_eq!(reimport.folders.len(), 1);
    assert_eq!(reimport.triggers.len(), 2);

    let original = &library.triggers[0];
    let round_tripped = &reimport.triggers[0];
    // Ids regenerate on import; compare semantic content.
    assert_eq!(round_tripped.name, original.name);
    assert_eq!(round_tripped.pattern, original.pattern);
    assert_eq!(round_tripped.previous_pattern, original.previous_pattern);
    assert_eq!(round_tripped.condition, original.condition);
    assert_eq!(round_tripped.lockout_seconds, original.lockout_seconds);
    assert_eq!(round_tripped.display_text, original.display_text);
    assert_eq!(round_tripped.speak_text, original.speak_text);
    assert_eq!(round_tripped.priority, original.priority);
    assert_eq!(round_tripped.voice_rate, original.voice_rate);
    assert_eq!(round_tripped.volume, original.volume);
    assert_eq!(round_tripped.variable_actions, original.variable_actions);
    let original_timer = original.timer.as_ref().unwrap();
    let round_timer = round_tripped.timer.as_ref().unwrap();
    assert_eq!(round_timer, original_timer);

    // Unexecuted fields survived the round trip byte-for-byte.
    assert_eq!(
        round_tripped.passthrough.get("ChatWebhook"),
        original.passthrough.get("ChatWebhook")
    );
    assert_eq!(
        round_tripped.passthrough.get("IconSource"),
        original.passthrough.get("IconSource")
    );
    assert_eq!(
        round_tripped.passthrough.get("TextToSendToChat"),
        original.passthrough.get("TextToSendToChat")
    );
}

#[test]
fn overlay_export_round_trips_presets() {
    let import = import_fixture();
    let mut library = TriggerLibrary::new();
    library.text_overlays = import.text_overlays.clone();
    library.timer_overlays = import.timer_overlays.clone();

    let exported = eqlp::export_overlays(&library).unwrap();
    let reimport = eqlp::import(&exported, &[]).unwrap();
    assert_eq!(reimport.text_overlays.len(), 1);
    assert_eq!(reimport.timer_overlays.len(), 1);
    let text = &reimport.text_overlays[0];
    assert_eq!(text.name, "Raid Text");
    assert_eq!(text.font_size, "16pt");
    // The EQLP id is preserved so re-imported triggers still resolve.
    assert_eq!(
        text.passthrough.get("Id").and_then(|v| v.as_str()),
        Some("overlay-text-1")
    );
    assert_eq!(reimport.timer_overlays[0].mode, TimerOverlayMode::Cooldown);
}

#[test]
fn malformed_packages_are_rejected_without_panicking() {
    assert!(eqlp::import(b"not gzip", &[]).is_err());
    assert!(eqlp::import(&gz("not json"), &[]).is_err());
    assert!(eqlp::import(&gz("{\"an\": \"object not array\"}"), &[]).is_err());
    // A non-object node is tolerated (skipped), not fatal.
    assert!(eqlp::import(&gz("[1, 2, 3]"), &[]).is_ok());
}

#[test]
fn quarantined_triggers_still_reexport_their_original_pattern() {
    let import = import_fixture();
    let mut library = TriggerLibrary::new();
    library.folders = import.folders.clone();
    library.triggers = import.triggers.clone();

    let mut report = CompatReport::default();
    let exported = eqlp::export_triggers(
        &library,
        &ExportSelection {
            folders: vec![library.folders[0].id],
            triggers: vec![],
        },
        &mut report,
    )
    .unwrap();
    let text = eqlp::decompress(&exported).unwrap();
    assert!(text.contains("(?<open-close>unsupported)"));
}
