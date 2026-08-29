//! GINA `.gtp` package import.
//!
//! A `.gtp` file is a ZIP whose first entry is a `ShareData.xml` document:
//! `<SharedData><TriggerGroups><TriggerGroup>…` with nested groups and
//! `<Trigger>` elements. The mapping follows EQLP's importer with one
//! documented-intent fix: GINA boolean elements only count as enabled when
//! they read `true` — EQLP's `bool.TryParse(…, out _)` treats a literal
//! `False` as enabling, which is an acknowledged defect we do not copy.

use std::io::Read;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::eqlp::CodecError;
use crate::model::*;
use crate::netregex::CompatRegex;
use crate::pattern;
use crate::report::{CompatReport, CompatSeverity};

/// Maximum accepted decompressed XML size.
pub const MAX_XML_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ELEMENTS: usize = 500_000;

#[derive(Clone, Debug, Default)]
pub struct GinaImport {
    pub folders: Vec<Folder>,
    pub triggers: Vec<Trigger>,
    pub report: CompatReport,
}

/// Minimal XML tree: element name, text, children.
#[derive(Debug, Default)]
struct Element {
    name: String,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn text_of(&self, name: &str) -> String {
        self.child(name)
            .map(|child| child.text.trim().to_owned())
            .unwrap_or_default()
    }

    /// True only for a literal `true` (the GINA-boolean fix).
    fn flag(&self, name: &str) -> bool {
        self.text_of(name).eq_ignore_ascii_case("true")
    }
}

fn parse_xml(xml: &str) -> Result<Element, CodecError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut count = 0usize;
    loop {
        match reader
            .read_event()
            .map_err(|error| CodecError(format!("invalid GINA XML: {error}")))?
        {
            Event::Start(start) => {
                count += 1;
                if count > MAX_ELEMENTS {
                    return Err(CodecError(
                        "the GINA file contains too many elements".to_owned(),
                    ));
                }
                stack.push(Element {
                    name: String::from_utf8_lossy(start.name().as_ref()).into_owned(),
                    ..Element::default()
                });
            }
            Event::Empty(start) => {
                count += 1;
                let element = Element {
                    name: String::from_utf8_lossy(start.name().as_ref()).into_owned(),
                    ..Element::default()
                };
                stack
                    .last_mut()
                    .expect("root always present")
                    .children
                    .push(element);
            }
            Event::Text(text) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text.decode().unwrap_or_default());
                }
            }
            Event::End(_) => {
                let done = stack.pop().expect("balanced by reader");
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => return Ok(done),
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.pop().unwrap_or_default();
    if root.children.len() == 1 {
        return Ok(root.children.remove(0));
    }
    Ok(root)
}

/// Extract the XML text from a `.gtp` (ZIP) payload or accept raw XML.
fn read_share_xml(bytes: &[u8]) -> Result<String, CodecError> {
    if bytes.starts_with(b"PK") {
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor)
            .map_err(|error| CodecError(format!("could not open the .gtp archive: {error}")))?;
        if zip.is_empty() {
            return Err(CodecError("the .gtp archive is empty".to_owned()));
        }
        let entry = zip
            .by_index(0)
            .map_err(|error| CodecError(format!("could not read the .gtp entry: {error}")))?;
        let mut text = String::new();
        entry
            .take(MAX_XML_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|error| CodecError(format!("could not read the .gtp entry: {error}")))?;
        if text.len() as u64 > MAX_XML_BYTES {
            return Err(CodecError(
                "the GINA share data exceeds the 32 MB import limit".to_owned(),
            ));
        }
        return Ok(text);
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| CodecError("the file is neither a .gtp archive nor XML".to_owned()))
}

/// Import a GINA `.gtp` package (or bare `ShareData.xml`).
pub fn import(bytes: &[u8]) -> Result<GinaImport, CodecError> {
    let xml = read_share_xml(bytes)?;
    let root = parse_xml(&xml)?;
    let shared = if root.name == "SharedData" {
        &root
    } else {
        root.child("SharedData")
            .ok_or_else(|| CodecError("the GINA file has no SharedData element".to_owned()))?
    };
    let mut import = GinaImport::default();
    for child in &shared.children {
        walk_groups(child, None, &mut import);
    }
    import.report.triggers_imported = import.triggers.len();
    import.report.folders_imported = import.folders.len();
    import.report.triggers_quarantined = import
        .triggers
        .iter()
        .filter(|trigger| trigger.quarantine.is_some())
        .count();
    Ok(import)
}

fn walk_groups(element: &Element, parent: Option<FolderId>, import: &mut GinaImport) {
    match element.name.as_str() {
        "TriggerGroups" => {
            for child in &element.children {
                walk_groups(child, parent, import);
            }
        }
        "TriggerGroup" => {
            let folder = Folder {
                id: FolderId::new(),
                name: element.text_of("Name"),
                parent,
                index: import.folders.len() as u32,
                expanded: false,
            };
            let folder_id = folder.id;
            import.folders.push(folder);
            for nested in element.children_named("TriggerGroups") {
                walk_groups(nested, Some(folder_id), import);
            }
            if let Some(triggers) = element.child("Triggers") {
                let mut converted: Vec<Trigger> = triggers
                    .children_named("Trigger")
                    .filter_map(|trigger| convert_trigger(trigger, folder_id, import))
                    .collect();
                converted.sort_by_key(|a| a.name.to_lowercase());
                for (index, mut trigger) in converted.into_iter().enumerate() {
                    trigger.index = index as u32;
                    import.triggers.push(trigger);
                }
            }
        }
        _ => {}
    }
}

fn convert_trigger(
    element: &Element,
    folder: FolderId,
    import: &mut GinaImport,
) -> Option<Trigger> {
    let name = element.text_of("Name");
    let mut good = false;
    let mut trigger = Trigger {
        name: name.clone(),
        folder: Some(folder),
        enabled: false,
        comments: element.text_of("Comments"),
        pattern: Pattern {
            text: element.text_of("TriggerText"),
            use_regex: element.flag("EnableRegex"),
        },
        ..Trigger::default()
    };

    if element.flag("PlayMediaFile") {
        import.report.push(
            CompatSeverity::Warning,
            &name,
            "missing-media",
            "GINA shares do not include media files; assign a managed sound after import",
        );
    }

    if element.flag("UseText") {
        good = true;
        trigger.display_text = non_empty(element.text_of("DisplayText"));
    }
    if element.flag("UseTextToVoice") {
        good = true;
        trigger.speak_text = non_empty(element.text_of("TextToVoiceText"));
    }
    if element.flag("CopyToClipboard") {
        // Clipboard sharing is excluded; keep the text for round-trip.
        good = true;
        if let Some(text) = non_empty(element.text_of("ClipboardText")) {
            trigger
                .passthrough
                .insert("TextToShare".to_owned(), serde_json::Value::String(text));
            import.report.push(
                CompatSeverity::Info,
                &name,
                "retained-field",
                "clipboard sharing is not executed by Stonemite; the text is kept for re-export",
            );
        }
    }
    if element.flag("InterruptSpeech") {
        trigger.priority = 1;
    }

    let timer_type = element.text_of("TimerType");
    if !timer_type.is_empty() && timer_type != "NoTimer" {
        good = true;
        let mut behavior = TimerBehavior {
            duration_seconds: 0.0,
            ..TimerBehavior::default()
        };
        let timer_name = element.text_of("TimerName");
        if !timer_name.is_empty() && timer_name != name {
            behavior.timer_name = timer_name;
        }
        if timer_type == "Stopwatch" {
            // Stopwatches have no duration in GINA; EQLP defaults a minute.
            behavior.duration_seconds = 60.0;
            behavior.kind = TimerKind::Progress;
        } else {
            if let Ok(duration) = element.text_of("TimerDuration").parse::<i64>() {
                if duration > 0 {
                    behavior.duration_seconds = duration as f64;
                }
            }
            if let Ok(millis) = element.text_of("TimerMillisecondDuration").parse::<i64>() {
                if millis > 0 {
                    behavior.duration_seconds = (millis as f64 / 1000.0).max(0.2);
                }
            }
            if timer_type == "RepeatingTimer" {
                behavior.kind = TimerKind::Looping;
                // Bound repeats so an import cannot loop forever.
                behavior.times_to_loop = 5;
            } else {
                behavior.kind = TimerKind::Countdown;
            }
        }

        if let Some(ending) = element.child("TimerEndingTrigger") {
            if ending.flag("UseText") {
                behavior.warning.display_text = non_empty(ending.text_of("DisplayText"));
            }
            if ending.flag("UseTextToVoice") {
                behavior.warning.speak_text = non_empty(ending.text_of("TextToVoiceText"));
            }
        }
        if let Ok(ending_seconds) = element.text_of("TimerEndingTime").parse::<i64>() {
            // GINA defaults this to 1 even without warning text.
            if behavior.warning.speak_text.is_some()
                || behavior.warning.display_text.is_some()
                || ending_seconds > 1
            {
                behavior.warning_seconds = ending_seconds.max(0) as u32;
            }
        }

        behavior.restart_mode = match element.text_of("TimerStartBehavior").as_str() {
            "StartNewTimer" => TimerRestartMode::StartNew,
            "RestartTimer" => {
                if element.flag("RestartBasedOnTimerName") {
                    TimerRestartMode::RestartSameName
                } else {
                    TimerRestartMode::RestartAll
                }
            }
            _ => TimerRestartMode::IgnoreIfSameNameRunning,
        };

        if let Some(ended) = element.child("TimerEndedTrigger") {
            if ended.flag("UseText") {
                behavior.end.display_text = non_empty(ended.text_of("DisplayText"));
            }
            if ended.flag("UseTextToVoice") {
                behavior.end.speak_text = non_empty(ended.text_of("TextToVoiceText"));
            }
        }

        if let Some(enders) = element.child("TimerEarlyEnders") {
            for ender in enders.children_named("EarlyEnder").take(3) {
                let text = ender.text_of("EarlyEndText");
                if !text.is_empty() {
                    behavior.end_early_patterns.push(Pattern {
                        text,
                        use_regex: ender.flag("EnableRegex"),
                    });
                }
            }
        }
        trigger.timer = Some(behavior);
    }

    if !good {
        return None;
    }

    // Validate regexes; quarantine unsupported constructs.
    let mut quarantine = None;
    let mut validate = |text: &str, use_regex: bool, which: &str| -> Option<Quarantine> {
        if !use_regex || text.trim().is_empty() {
            return None;
        }
        let expanded = pattern::expand(text);
        let probe = crate::substitute::replace_code(
            &expanded.pattern,
            crate::substitute::CHARACTER_CODE,
            "Probe",
        );
        match CompatRegex::compile(&probe) {
            Ok(_) => None,
            Err(error) => {
                import.report.push(
                    CompatSeverity::Error,
                    &name,
                    "unsupported-regex",
                    format!("{which} pattern was quarantined: {error}"),
                );
                Some(Quarantine {
                    reason: "unsupported-regex".to_owned(),
                    detail: format!("{which}: {error}"),
                })
            }
        }
    };
    quarantine = quarantine.or_else(|| {
        validate(
            &trigger.pattern.text.clone(),
            trigger.pattern.use_regex,
            "main",
        )
    });
    if let Some(timer) = &trigger.timer {
        for pattern in timer.end_early_patterns.clone() {
            quarantine =
                quarantine.or_else(|| validate(&pattern.text, pattern.use_regex, "end-early"));
        }
    }
    trigger.quarantine = quarantine;
    Some(trigger)
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SHARE_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<SharedData>
  <TriggerGroups>
    <TriggerGroup>
      <Name>Raid</Name>
      <Triggers>
        <Trigger>
          <Name>CH Landed</Name>
          <TriggerText>^{S1} completes their spell\.$</TriggerText>
          <Comments>from gina</Comments>
          <EnableRegex>True</EnableRegex>
          <UseText>True</UseText>
          <DisplayText>CH from {S1}</DisplayText>
          <UseTextToVoice>False</UseTextToVoice>
          <TextToVoiceText>should not import</TextToVoiceText>
          <InterruptSpeech>True</InterruptSpeech>
          <TimerType>Timer</TimerType>
          <TimerName>CH</TimerName>
          <TimerDuration>10</TimerDuration>
          <TimerMillisecondDuration>10500</TimerMillisecondDuration>
          <TimerStartBehavior>RestartTimer</TimerStartBehavior>
          <RestartBasedOnTimerName>True</RestartBasedOnTimerName>
          <TimerEndingTime>3</TimerEndingTime>
          <TimerEndingTrigger>
            <UseText>True</UseText>
            <DisplayText>CH soon</DisplayText>
          </TimerEndingTrigger>
          <TimerEndedTrigger>
            <UseTextToVoice>True</UseTextToVoice>
            <TextToVoiceText>CH now</TextToVoiceText>
          </TimerEndedTrigger>
          <TimerEarlyEnders>
            <EarlyEnder>
              <EarlyEndText>is interrupted</EarlyEndText>
              <EnableRegex>False</EnableRegex>
            </EarlyEnder>
          </TimerEarlyEnders>
        </Trigger>
        <Trigger>
          <Name>Stopwatch One</Name>
          <TriggerText>begin stopwatch</TriggerText>
          <TimerType>Stopwatch</TimerType>
        </Trigger>
      </Triggers>
      <TriggerGroups>
        <TriggerGroup>
          <Name>Nested</Name>
          <Triggers>
            <Trigger>
              <Name>Voice Only</Name>
              <TriggerText>you have been mesmerized</TriggerText>
              <UseTextToVoice>True</UseTextToVoice>
              <InterruptSpeech>False</InterruptSpeech>
              <TextToVoiceText>mezzed</TextToVoiceText>
            </Trigger>
          </Triggers>
        </TriggerGroup>
      </TriggerGroups>
    </TriggerGroup>
  </TriggerGroups>
</SharedData>"#;

    fn gtp_bytes(xml: &str) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .start_file::<_, ()>("ShareData.xml", zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    #[test]
    fn gtp_zip_imports_groups_and_triggers() {
        let import = import(&gtp_bytes(SHARE_XML)).unwrap();
        assert_eq!(import.folders.len(), 2);
        assert_eq!(import.triggers.len(), 3);
        assert_eq!(import.folders[0].name, "Raid");
        assert_eq!(import.folders[1].name, "Nested");
        assert_eq!(import.folders[1].parent, Some(import.folders[0].id));

        let ch = import
            .triggers
            .iter()
            .find(|t| t.name == "CH Landed")
            .unwrap();
        assert!(ch.pattern.use_regex);
        assert_eq!(ch.display_text.as_deref(), Some("CH from {S1}"));
        // UseTextToVoice was literal False: the true-only fix keeps it off.
        assert_eq!(ch.speak_text, None);
        assert_eq!(ch.priority, 1);
        let timer = ch.timer.as_ref().unwrap();
        assert_eq!(timer.kind, TimerKind::Countdown);
        assert_eq!(timer.timer_name, "CH");
        assert!((timer.duration_seconds - 10.5).abs() < 1e-9);
        assert_eq!(timer.restart_mode, TimerRestartMode::RestartSameName);
        assert_eq!(timer.warning_seconds, 3);
        assert_eq!(timer.warning.display_text.as_deref(), Some("CH soon"));
        assert_eq!(timer.end.speak_text.as_deref(), Some("CH now"));
        assert_eq!(timer.end_early_patterns.len(), 1);
        assert!(!timer.end_early_patterns[0].use_regex);

        let stopwatch = import
            .triggers
            .iter()
            .find(|t| t.name == "Stopwatch One")
            .unwrap();
        let timer = stopwatch.timer.as_ref().unwrap();
        assert_eq!(timer.kind, TimerKind::Progress);
        assert_eq!(timer.duration_seconds, 60.0);
        // Default start behavior: do nothing if the same timer name runs.
        assert_eq!(
            timer.restart_mode,
            TimerRestartMode::IgnoreIfSameNameRunning
        );

        let voice = import
            .triggers
            .iter()
            .find(|t| t.name == "Voice Only")
            .unwrap();
        assert_eq!(voice.speak_text.as_deref(), Some("mezzed"));
        assert_eq!(voice.priority, 3);
        assert_eq!(voice.folder, Some(import.folders[1].id));

        // Everything arrives disabled.
        assert!(import.triggers.iter().all(|t| !t.enabled));
    }

    #[test]
    fn raw_xml_is_accepted_too() {
        let import = import(SHARE_XML.as_bytes()).unwrap();
        assert_eq!(import.triggers.len(), 3);
    }

    #[test]
    fn unsupported_regex_is_quarantined_with_a_report() {
        let xml = SHARE_XML.replace(
            r"^{S1} completes their spell\.$",
            r"(?&lt;open-close&gt;bad)",
        );
        let import = import(xml.as_bytes()).unwrap();
        let ch = import
            .triggers
            .iter()
            .find(|t| t.name == "CH Landed")
            .unwrap();
        assert!(ch.quarantine.is_some());
        assert_eq!(import.report.triggers_quarantined, 1);
        assert!(import
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "unsupported-regex"));
    }

    #[test]
    fn garbage_input_is_rejected() {
        assert!(import(b"not xml at all {").is_err());
        assert!(import(b"PK\x03\x04garbage").is_err());
    }
}
