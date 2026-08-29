//! EQLP interchange: `.tgf.gz` trigger packages and `.ogf.gz` overlay
//! packages.
//!
//! Both formats are gzip-compressed JSON arrays of `ExportTriggerNode`
//! (PascalCase System.Text.Json). Unknown fields are preserved verbatim in
//! each record's [`Passthrough`] and merged back on export, so a round-trip
//! keeps fields Stonemite never executes (webhooks, chat text, …).

use std::io::Read;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value};

use crate::model::*;
use crate::netregex::CompatRegex;
use crate::pattern;
use crate::report::{CompatReport, CompatSeverity};

/// Decompressed JSON size limit (matches EQLP's 100 MB input file cap,
/// tightened for post-inflation size).
pub const MAX_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum nodes accepted from one package.
pub const MAX_NODES: usize = 50_000;
/// Maximum folder nesting accepted from one package.
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecError(pub String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}

/// Result of importing an EQLP package into native records.
#[derive(Clone, Debug, Default)]
pub struct EqlpImport {
    pub folders: Vec<Folder>,
    pub triggers: Vec<Trigger>,
    pub text_overlays: Vec<TextOverlayPreset>,
    pub timer_overlays: Vec<TimerOverlayPreset>,
    pub report: CompatReport,
}

/// EQLP trigger fields the importer maps onto native fields. Everything
/// else round-trips through passthrough.
const MAPPED_TRIGGER_FIELDS: &[&str] = &[
    "Comments",
    "Pattern",
    "UseRegex",
    "PreviousPattern",
    "PreviousUseRegex",
    "MatchVariableCondition",
    "LockoutTime",
    "RepeatedResetTime",
    "VariableActions",
    "TextToDisplay",
    "TextToSpeak",
    "SoundToPlay",
    "Priority",
    "VoiceRate",
    "Volume",
    "SelectedOverlays",
    "FontColor",
    "ActiveColor",
    "IdleColor",
    "ResetColor",
    "EnableTimer",
    "TimerType",
    "DurationSeconds",
    "ResetDurationSeconds",
    "TimesToLoop",
    "TriggerAgainOption",
    "WarningSeconds",
    "AltTimerName",
    "WarningTextToDisplay",
    "WarningTextToSpeak",
    "WarningSoundToPlay",
    "EndTextToDisplay",
    "EndTextToSpeak",
    "EndSoundToPlay",
    "EndEarlyTextToDisplay",
    "EndEarlyTextToSpeak",
    "EndEarlySoundToPlay",
    "EndEarlyPattern",
    "EndEarlyPattern2",
    "EndEarlyPattern3",
    "EndUseRegex",
    "EndUseRegex2",
    "EndUseRegex3",
    "EndEarlyRepeatedCount",
    "EndTimerClearVariables",
];

const MAPPED_OVERLAY_FIELDS: &[&str] = &[
    "Id",
    "IsTextOverlay",
    "IsTimerOverlay",
    "FontSize",
    "FontColor",
    "BackgroundColor",
    "FadeDelay",
    "TimerMode",
    "SortBy",
    "ActiveColor",
    "IdleColor",
    "ResetColor",
    "ShowMillis",
    "IsDefault",
];

pub fn decompress(bytes: &[u8]) -> Result<String, CodecError> {
    let mut decoder = GzDecoder::new(bytes).take(MAX_DECOMPRESSED_BYTES + 1);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .map_err(|error| CodecError(format!("could not decompress the package: {error}")))?;
    if text.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(CodecError(
            "the package expands beyond the 64 MB import limit".to_owned(),
        ));
    }
    Ok(text)
}

fn compress(text: &str) -> Result<Vec<u8>, CodecError> {
    use std::io::Write;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(text.as_bytes())
        .and_then(|_| encoder.finish())
        .map_err(|error| CodecError(format!("could not compress the package: {error}")))
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a `.tgf.gz` or `.ogf.gz` payload. `existing_overlays` supplies
/// already-known overlay presets so trigger references resolve; overlays
/// found inside this package are resolved first (import overlays before
/// trigger references).
pub fn import(
    bytes: &[u8],
    existing_overlays: &[(OverlayId, String)],
) -> Result<EqlpImport, CodecError> {
    let text = decompress(bytes)?;
    let nodes: Vec<Value> = serde_json::from_str(&text)
        .map_err(|error| CodecError(format!("the package is not valid EQLP JSON: {error}")))?;

    let mut import = EqlpImport::default();
    let mut node_count = 0usize;

    // Pass 1: overlays, so trigger references resolve against them.
    for node in &nodes {
        collect_overlays(node, &mut import, 0, &mut node_count)?;
    }

    // Known overlay ids: EQLP id string → native id.
    let mut overlay_ids: Vec<(String, OverlayId, bool)> = Vec::new();
    for preset in &import.text_overlays {
        if let Some(Value::String(id)) = preset.passthrough.get("Id") {
            overlay_ids.push((id.clone(), preset.id, true));
        }
    }
    for preset in &import.timer_overlays {
        if let Some(Value::String(id)) = preset.passthrough.get("Id") {
            overlay_ids.push((id.clone(), preset.id, false));
        }
    }
    for (id, name) in existing_overlays {
        overlay_ids.push((name.clone(), *id, false));
    }

    // Pass 2: folders and triggers.
    let mut node_count = 0usize;
    for node in &nodes {
        import_node(node, None, &mut import, &overlay_ids, 0, &mut node_count)?;
    }

    import.report.triggers_imported = import.triggers.len();
    import.report.folders_imported = import.folders.len();
    import.report.overlays_imported = import.text_overlays.len() + import.timer_overlays.len();
    import.report.triggers_quarantined = import
        .triggers
        .iter()
        .filter(|trigger| trigger.quarantine.is_some())
        .count();
    Ok(import)
}

fn bump(node_count: &mut usize) -> Result<(), CodecError> {
    *node_count += 1;
    if *node_count > MAX_NODES {
        return Err(CodecError(format!(
            "the package contains more than {MAX_NODES} entries"
        )));
    }
    Ok(())
}

fn collect_overlays(
    node: &Value,
    import: &mut EqlpImport,
    depth: usize,
    node_count: &mut usize,
) -> Result<(), CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError(
            "the package folder tree is nested too deeply".to_owned(),
        ));
    }
    bump(node_count)?;
    let Some(object) = node.as_object() else {
        return Ok(());
    };
    if let Some(overlay) = object.get("OverlayData").and_then(Value::as_object) {
        let name = string_field(object, "Name").unwrap_or_else(|| "Unnamed overlay".to_owned());
        let eqlp_id = string_field(object, "Id");
        import_overlay(overlay, name, eqlp_id, import);
    }
    if let Some(children) = object.get("Nodes").and_then(Value::as_array) {
        for child in children {
            collect_overlays(child, import, depth + 1, node_count)?;
        }
    }
    Ok(())
}

fn import_overlay(
    overlay: &Map<String, Value>,
    name: String,
    eqlp_id: Option<String>,
    import: &mut EqlpImport,
) {
    let mut passthrough: Passthrough = overlay
        .iter()
        .filter(|(key, _)| !MAPPED_OVERLAY_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(id) = eqlp_id {
        passthrough.insert("Id".to_owned(), Value::String(id));
    }
    let color =
        |key: &str, default: &str| string_field(overlay, key).unwrap_or_else(|| default.to_owned());
    if bool_field(overlay, "IsTimerOverlay") {
        import.timer_overlays.push(TimerOverlayPreset {
            id: OverlayId::new(),
            name,
            is_default: bool_field(overlay, "IsDefault"),
            mode: match i64_field(overlay, "TimerMode").unwrap_or(0) {
                1 => TimerOverlayMode::Cooldown,
                _ => TimerOverlayMode::Standard,
            },
            sort_by: i64_field(overlay, "SortBy").unwrap_or(0) as i32,
            font_color: color("FontColor", "#FFFFFFFF"),
            active_color: color("ActiveColor", "#FF1D397E"),
            idle_color: color("IdleColor", "#FF8F1515"),
            reset_color: color("ResetColor", "#FF8F1515"),
            background_color: color("BackgroundColor", "#5F000000"),
            show_millis: bool_field(overlay, "ShowMillis"),
            passthrough,
        });
    } else {
        // Text overlays are the default kind for EQLP text nodes.
        import.text_overlays.push(TextOverlayPreset {
            id: OverlayId::new(),
            name,
            is_default: bool_field(overlay, "IsDefault"),
            font_size: string_field(overlay, "FontSize").unwrap_or_else(|| "12pt".to_owned()),
            font_color: color("FontColor", "#FFFFFFFF"),
            background_color: color("BackgroundColor", "#5F000000"),
            fade_delay_seconds: i64_field(overlay, "FadeDelay").unwrap_or(10).max(0) as u32,
            passthrough,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn import_node(
    node: &Value,
    parent: Option<FolderId>,
    import: &mut EqlpImport,
    overlay_ids: &[(String, OverlayId, bool)],
    depth: usize,
    node_count: &mut usize,
) -> Result<(), CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError(
            "the package folder tree is nested too deeply".to_owned(),
        ));
    }
    bump(node_count)?;
    let Some(object) = node.as_object() else {
        return Ok(());
    };
    let name = string_field(object, "Name").unwrap_or_else(|| "Unnamed".to_owned());

    if let Some(trigger) = object.get("TriggerData").and_then(Value::as_object) {
        let native = import_trigger(trigger, &name, parent, overlay_ids, &mut import.report);
        import.triggers.push(native);
        return Ok(());
    }
    if object
        .get("OverlayData")
        .is_some_and(|value| !value.is_null())
    {
        // Already handled in the overlay pass.
        return Ok(());
    }

    let folder = Folder {
        id: FolderId::new(),
        name,
        parent,
        index: import.folders.len() as u32,
        expanded: bool_field(object, "IsExpanded"),
    };
    let folder_id = folder.id;
    import.folders.push(folder);
    if let Some(children) = object.get("Nodes").and_then(Value::as_array) {
        for child in children {
            import_node(
                child,
                Some(folder_id),
                import,
                overlay_ids,
                depth + 1,
                node_count,
            )?;
        }
    }
    Ok(())
}

fn import_trigger(
    data: &Map<String, Value>,
    name: &str,
    parent: Option<FolderId>,
    overlay_ids: &[(String, OverlayId, bool)],
    report: &mut CompatReport,
) -> Trigger {
    let mut trigger = Trigger {
        name: name.to_owned(),
        folder: parent,
        // Imports arrive disabled by default; users enable them from the
        // compatibility confirmation.
        enabled: false,
        comments: string_field(data, "Comments").unwrap_or_default(),
        pattern: Pattern {
            text: string_field(data, "Pattern").unwrap_or_default(),
            use_regex: bool_field(data, "UseRegex"),
        },
        condition: string_field(data, "MatchVariableCondition").unwrap_or_default(),
        lockout_seconds: f64_field(data, "LockoutTime").unwrap_or(0.0),
        repeated_reset_seconds: f64_field(data, "RepeatedResetTime").unwrap_or(0.75),
        display_text: non_empty(string_field(data, "TextToDisplay")),
        speak_text: non_empty(string_field(data, "TextToSpeak")),
        sound: non_empty(string_field(data, "SoundToPlay")),
        priority: i64_field(data, "Priority").unwrap_or(3),
        voice_rate: i64_field(data, "VoiceRate").unwrap_or(0) as i32,
        volume: i64_field(data, "Volume").unwrap_or(4) as i32,
        font_color: non_empty(string_field(data, "FontColor")),
        active_color: non_empty(string_field(data, "ActiveColor")),
        idle_color: non_empty(string_field(data, "IdleColor")),
        reset_color: non_empty(string_field(data, "ResetColor")),
        ..Trigger::default()
    };

    if let Some(previous) = non_empty(string_field(data, "PreviousPattern")) {
        trigger.previous_pattern = Some(Pattern {
            text: previous,
            use_regex: bool_field(data, "PreviousUseRegex"),
        });
    }

    // Variable actions.
    if let Some(actions) = data.get("VariableActions").and_then(Value::as_array) {
        for action in actions.iter().filter_map(Value::as_object) {
            let op = match (
                i64_field(action, "ActionType").unwrap_or(0),
                i64_field(action, "DataType").unwrap_or(0),
            ) {
                (1, _) => VariableOp::Clear,
                (_, 1) => VariableOp::SetCounter,
                _ => VariableOp::SetValue,
            };
            trigger.variable_actions.push(VariableAction {
                op,
                name: string_field(action, "VariableName").unwrap_or_default(),
                value: string_field(action, "Value").unwrap_or_default(),
                step: f64_field(action, "Step").unwrap_or(1.0),
                initial_value: f64_field(action, "InitialValue").unwrap_or(0.0),
                time_to_live_seconds: f64_field(action, "TimeToLiveSeconds").unwrap_or(0.0),
            });
        }
    }

    // Timer. Legacy exports use EnableTimer with TimerType 0 for countdown.
    let mut timer_type = i64_field(data, "TimerType").unwrap_or(0);
    if timer_type == 0 && bool_field(data, "EnableTimer") {
        timer_type = 1;
    }
    if let Some(kind) = TimerKind::from_eqlp_code(timer_type) {
        let stage = |display: &str, speak: &str, sound: &str| TimerStageActions {
            display_text: non_empty(string_field(data, display)),
            speak_text: non_empty(string_field(data, speak)),
            sound: non_empty(string_field(data, sound)),
        };
        let mut end_early_patterns = Vec::new();
        for (pattern_key, regex_key) in [
            ("EndEarlyPattern", "EndUseRegex"),
            ("EndEarlyPattern2", "EndUseRegex2"),
            ("EndEarlyPattern3", "EndUseRegex3"),
        ] {
            if let Some(text) = non_empty(string_field(data, pattern_key)) {
                end_early_patterns.push(Pattern {
                    text,
                    use_regex: bool_field(data, regex_key),
                });
            }
        }
        trigger.timer = Some(TimerBehavior {
            kind,
            timer_name: string_field(data, "AltTimerName").unwrap_or_default(),
            duration_seconds: f64_field(data, "DurationSeconds").unwrap_or(0.2),
            reset_duration_seconds: f64_field(data, "ResetDurationSeconds").unwrap_or(0.0),
            times_to_loop: i64_field(data, "TimesToLoop").unwrap_or(0).max(0) as u32,
            restart_mode: TimerRestartMode::from_eqlp_code(
                i64_field(data, "TriggerAgainOption").unwrap_or(0),
            )
            .unwrap_or_default(),
            warning_seconds: i64_field(data, "WarningSeconds").unwrap_or(0).max(0) as u32,
            warning: stage(
                "WarningTextToDisplay",
                "WarningTextToSpeak",
                "WarningSoundToPlay",
            ),
            end: stage("EndTextToDisplay", "EndTextToSpeak", "EndSoundToPlay"),
            early_end: stage(
                "EndEarlyTextToDisplay",
                "EndEarlyTextToSpeak",
                "EndEarlySoundToPlay",
            ),
            end_early_patterns,
            end_early_repeated_count: i64_field(data, "EndEarlyRepeatedCount").unwrap_or(0).max(0)
                as u32,
            end_clear_variables: split_variable_list(
                &string_field(data, "EndTimerClearVariables").unwrap_or_default(),
            ),
        });
    }

    // Overlay references.
    if let Some(selected) = data.get("SelectedOverlays").and_then(Value::as_array) {
        for id in selected.iter().filter_map(Value::as_str) {
            if id.is_empty() {
                continue;
            }
            match overlay_ids.iter().find(|(eqlp, _, _)| eqlp == id) {
                Some((_, native, is_text)) => {
                    if *is_text {
                        trigger.text_overlays.push(*native);
                    } else {
                        trigger.timer_overlays.push(*native);
                    }
                }
                None => report.push(
                    CompatSeverity::Warning,
                    name,
                    "dangling-overlay",
                    format!("references overlay '{id}' which is not part of this import"),
                ),
            }
        }
    }

    // Retained-but-unexecuted fields worth calling out.
    for (field, what) in [
        ("ChatWebhook", "chat webhooks"),
        ("TextToSendToChat", "chat sending"),
        ("TextToShare", "clipboard sharing"),
    ] {
        if non_empty(string_field(data, field)).is_some() {
            report.push(
                CompatSeverity::Info,
                name,
                "retained-field",
                format!("{what} are not executed by Stonemite; the field is kept for re-export"),
            );
        }
    }

    // Validate regexes; quarantine instead of activating a changed pattern.
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
                report.push(
                    CompatSeverity::Error,
                    name,
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
    let quarantine = validate(
        &trigger.pattern.text.clone(),
        trigger.pattern.use_regex,
        "main",
    )
    .or_else(|| {
        trigger
            .previous_pattern
            .clone()
            .and_then(|previous| validate(&previous.text, previous.use_regex, "previous-line"))
    })
    .or_else(|| {
        trigger.timer.clone().and_then(|timer| {
            timer
                .end_early_patterns
                .iter()
                .find_map(|pattern| validate(&pattern.text, pattern.use_regex, "end-early"))
        })
    });
    trigger.quarantine = quarantine;

    // Passthrough: everything not mapped.
    trigger.passthrough = data
        .iter()
        .filter(|(key, _)| !MAPPED_TRIGGER_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    trigger
}

fn split_variable_list(list: &str) -> Vec<String> {
    list.split([',', ' ', ';'])
        .map(|name| {
            name.trim()
                .trim_start_matches('$')
                .trim_start_matches('{')
                .trim_end_matches('}')
                .trim()
                .to_owned()
        })
        .filter(|name| !name.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// What to export from a library.
#[derive(Clone, Debug)]
pub struct ExportSelection {
    pub folders: Vec<FolderId>,
    pub triggers: Vec<TriggerId>,
}

/// Export selected triggers (and their folder structure) as `.tgf.gz`.
/// EQLP exports cannot embed media; `report` receives a portability warning
/// for each managed-asset sound reference.
pub fn export_triggers(
    library: &TriggerLibrary,
    selection: &ExportSelection,
    report: &mut CompatReport,
) -> Result<Vec<u8>, CodecError> {
    let mut selected_triggers: Vec<&Trigger> = Vec::new();
    let mut selected_folders: Vec<FolderId> = Vec::new();
    for folder in &selection.folders {
        for id in library.folder_subtree(*folder) {
            if !selected_folders.contains(&id) {
                selected_folders.push(id);
            }
        }
    }
    for trigger in &library.triggers {
        let in_folder = trigger
            .folder
            .is_some_and(|folder| selected_folders.contains(&folder));
        if selection.triggers.contains(&trigger.id) || in_folder {
            selected_triggers.push(trigger);
        }
    }
    // Folders needed to hold the selected triggers (ancestors included).
    for trigger in &selected_triggers {
        let mut cursor = trigger.folder;
        while let Some(folder) = cursor {
            if !selected_folders.contains(&folder) {
                selected_folders.push(folder);
            }
            cursor = library.folder(folder).and_then(|folder| folder.parent);
        }
    }

    for trigger in &selected_triggers {
        for sound in trigger_sound_refs(trigger) {
            if library
                .assets
                .iter()
                .any(|asset| asset.name.eq_ignore_ascii_case(&sound))
            {
                report.push(
                    CompatSeverity::Warning,
                    &trigger.name,
                    "media-not-embedded",
                    format!(
                        "EQLP packages cannot embed media; '{sound}' must be shared separately"
                    ),
                );
            }
        }
    }

    let roots = build_export_tree(library, &selected_folders, &selected_triggers, None, 0)?;
    let json = serde_json::to_string(&roots)
        .map_err(|error| CodecError(format!("could not serialize the export: {error}")))?;
    compress(&json)
}

/// Export overlay presets as `.ogf.gz`.
pub fn export_overlays(library: &TriggerLibrary) -> Result<Vec<u8>, CodecError> {
    let mut nodes = Vec::new();
    for preset in &library.text_overlays {
        nodes.push(overlay_node(
            &preset.name,
            preset.id,
            &preset.passthrough,
            json!({
                "IsTextOverlay": true,
                "IsTimerOverlay": false,
                "IsDefault": preset.is_default,
                "FontSize": preset.font_size,
                "FontColor": preset.font_color,
                "BackgroundColor": preset.background_color,
                "FadeDelay": preset.fade_delay_seconds,
            }),
        ));
    }
    for preset in &library.timer_overlays {
        nodes.push(overlay_node(
            &preset.name,
            preset.id,
            &preset.passthrough,
            json!({
                "IsTextOverlay": false,
                "IsTimerOverlay": true,
                "IsDefault": preset.is_default,
                "TimerMode": match preset.mode {
                    TimerOverlayMode::Standard => 0,
                    TimerOverlayMode::Cooldown => 1,
                },
                "SortBy": preset.sort_by,
                "FontColor": preset.font_color,
                "ActiveColor": preset.active_color,
                "IdleColor": preset.idle_color,
                "ResetColor": preset.reset_color,
                "BackgroundColor": preset.background_color,
                "ShowMillis": preset.show_millis,
            }),
        ));
    }
    let json = serde_json::to_string(&nodes)
        .map_err(|error| CodecError(format!("could not serialize the export: {error}")))?;
    compress(&json)
}

fn overlay_node(name: &str, id: OverlayId, passthrough: &Passthrough, mapped: Value) -> Value {
    let mut overlay: Map<String, Value> = passthrough
        .iter()
        .filter(|(key, _)| key.as_str() != "Id")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Value::Object(mapped) = mapped {
        overlay.extend(mapped);
    }
    // EQLP resolves triggers' SelectedOverlays against the node Id: reuse the
    // original EQLP id when this preset was imported, else our UUID.
    let eqlp_id = passthrough
        .get("Id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string());
    json!({
        "Id": eqlp_id,
        "Name": name,
        "IsExpanded": false,
        "OriginalId": null,
        "TriggerData": null,
        "OverlayData": Value::Object(overlay),
        "Index": 0,
        "Parent": null,
        "Nodes": [],
    })
}

fn build_export_tree(
    library: &TriggerLibrary,
    folders: &[FolderId],
    triggers: &[&Trigger],
    parent: Option<FolderId>,
    depth: usize,
) -> Result<Vec<Value>, CodecError> {
    if depth > MAX_DEPTH {
        return Err(CodecError(
            "the folder tree is nested too deeply to export".to_owned(),
        ));
    }
    let mut nodes = Vec::new();
    let mut child_folders: Vec<&Folder> = library
        .folders
        .iter()
        .filter(|folder| folder.parent == parent && folders.contains(&folder.id))
        .collect();
    child_folders.sort_by_key(|folder| folder.index);
    for folder in child_folders {
        let children = build_export_tree(library, folders, triggers, Some(folder.id), depth + 1)?;
        nodes.push(json!({
            "Id": null,
            "Name": folder.name,
            "IsExpanded": folder.expanded,
            "OriginalId": null,
            "TriggerData": null,
            "OverlayData": null,
            "Index": folder.index,
            "Parent": null,
            "Nodes": children,
        }));
    }
    let mut child_triggers: Vec<&&Trigger> = triggers
        .iter()
        .filter(|trigger| trigger.folder == parent)
        .collect();
    child_triggers.sort_by_key(|trigger| trigger.index);
    for trigger in child_triggers {
        nodes.push(json!({
            "Id": null,
            "Name": trigger.name,
            "IsExpanded": false,
            "OriginalId": null,
            "TriggerData": export_trigger_data(library, trigger),
            "OverlayData": null,
            "Index": trigger.index,
            "Parent": null,
            "Nodes": [],
        }));
    }
    Ok(nodes)
}

fn export_trigger_data(library: &TriggerLibrary, trigger: &Trigger) -> Value {
    // Start from passthrough so unexecuted EQLP fields survive, then write
    // every mapped field on top.
    let mut data: Map<String, Value> = trigger
        .passthrough
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    let set = |data: &mut Map<String, Value>, key: &str, value: Value| {
        data.insert(key.to_owned(), value);
    };
    let opt_string = |value: &Option<String>| match value {
        Some(text) => Value::String(text.clone()),
        None => Value::Null,
    };

    set(
        &mut data,
        "Comments",
        Value::String(trigger.comments.clone()),
    );
    set(
        &mut data,
        "Pattern",
        Value::String(trigger.pattern.text.clone()),
    );
    set(
        &mut data,
        "UseRegex",
        Value::Bool(trigger.pattern.use_regex),
    );
    set(
        &mut data,
        "PreviousPattern",
        opt_string(&trigger.previous_pattern.as_ref().map(|p| p.text.clone())),
    );
    set(
        &mut data,
        "PreviousUseRegex",
        Value::Bool(
            trigger
                .previous_pattern
                .as_ref()
                .is_some_and(|p| p.use_regex),
        ),
    );
    set(
        &mut data,
        "MatchVariableCondition",
        Value::String(trigger.condition.clone()),
    );
    set(&mut data, "LockoutTime", json!(trigger.lockout_seconds));
    set(
        &mut data,
        "RepeatedResetTime",
        json!(trigger.repeated_reset_seconds),
    );
    set(
        &mut data,
        "TextToDisplay",
        opt_string(&trigger.display_text),
    );
    set(&mut data, "TextToSpeak", opt_string(&trigger.speak_text));
    set(&mut data, "SoundToPlay", opt_string(&trigger.sound));
    set(&mut data, "Priority", json!(trigger.priority));
    set(&mut data, "VoiceRate", json!(trigger.voice_rate));
    set(&mut data, "Volume", json!(trigger.volume));
    set(&mut data, "FontColor", opt_string(&trigger.font_color));
    set(&mut data, "ActiveColor", opt_string(&trigger.active_color));
    set(&mut data, "IdleColor", opt_string(&trigger.idle_color));
    set(&mut data, "ResetColor", opt_string(&trigger.reset_color));

    let actions: Vec<Value> = trigger
        .variable_actions
        .iter()
        .map(|action| {
            json!({
                "ActionType": if action.op == VariableOp::Clear { 1 } else { 0 },
                "DataType": if action.op == VariableOp::SetCounter { 1 } else { 0 },
                "VariableName": action.name,
                "Value": action.value,
                "Step": action.step,
                "InitialValue": action.initial_value,
                "TimeToLiveSeconds": action.time_to_live_seconds,
            })
        })
        .collect();
    set(&mut data, "VariableActions", Value::Array(actions));

    // Overlay references by original EQLP id (or our UUID for native ones).
    let mut selected = Vec::new();
    for id in trigger.text_overlays.iter().chain(&trigger.timer_overlays) {
        let text = library
            .text_overlays
            .iter()
            .find(|preset| preset.id == *id)
            .map(|preset| (&preset.passthrough, preset.id));
        let timer = library
            .timer_overlays
            .iter()
            .find(|preset| preset.id == *id)
            .map(|preset| (&preset.passthrough, preset.id));
        if let Some((passthrough, native)) = text.or(timer) {
            let eqlp = passthrough
                .get("Id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| native.to_string());
            selected.push(Value::String(eqlp));
        }
    }
    set(&mut data, "SelectedOverlays", Value::Array(selected));

    match &trigger.timer {
        Some(timer) => {
            set(&mut data, "EnableTimer", Value::Bool(true));
            set(&mut data, "TimerType", json!(timer.kind.eqlp_code()));
            set(&mut data, "DurationSeconds", json!(timer.duration_seconds));
            set(
                &mut data,
                "ResetDurationSeconds",
                json!(timer.reset_duration_seconds),
            );
            set(&mut data, "TimesToLoop", json!(timer.times_to_loop));
            set(
                &mut data,
                "TriggerAgainOption",
                json!(timer.restart_mode.eqlp_code()),
            );
            set(&mut data, "WarningSeconds", json!(timer.warning_seconds));
            set(
                &mut data,
                "AltTimerName",
                Value::String(timer.timer_name.clone()),
            );
            set(
                &mut data,
                "WarningTextToDisplay",
                opt_string(&timer.warning.display_text),
            );
            set(
                &mut data,
                "WarningTextToSpeak",
                opt_string(&timer.warning.speak_text),
            );
            set(
                &mut data,
                "WarningSoundToPlay",
                opt_string(&timer.warning.sound),
            );
            set(
                &mut data,
                "EndTextToDisplay",
                opt_string(&timer.end.display_text),
            );
            set(
                &mut data,
                "EndTextToSpeak",
                opt_string(&timer.end.speak_text),
            );
            set(&mut data, "EndSoundToPlay", opt_string(&timer.end.sound));
            set(
                &mut data,
                "EndEarlyTextToDisplay",
                opt_string(&timer.early_end.display_text),
            );
            set(
                &mut data,
                "EndEarlyTextToSpeak",
                opt_string(&timer.early_end.speak_text),
            );
            set(
                &mut data,
                "EndEarlySoundToPlay",
                opt_string(&timer.early_end.sound),
            );
            for (index, (pattern_key, regex_key)) in [
                ("EndEarlyPattern", "EndUseRegex"),
                ("EndEarlyPattern2", "EndUseRegex2"),
                ("EndEarlyPattern3", "EndUseRegex3"),
            ]
            .iter()
            .enumerate()
            {
                match timer.end_early_patterns.get(index) {
                    Some(pattern) => {
                        set(&mut data, pattern_key, Value::String(pattern.text.clone()));
                        set(&mut data, regex_key, Value::Bool(pattern.use_regex));
                    }
                    None => {
                        set(&mut data, pattern_key, Value::Null);
                        set(&mut data, regex_key, Value::Bool(false));
                    }
                }
            }
            set(
                &mut data,
                "EndEarlyRepeatedCount",
                json!(timer.end_early_repeated_count),
            );
            set(
                &mut data,
                "EndTimerClearVariables",
                Value::String(timer.end_clear_variables.join(", ")),
            );
        }
        None => {
            set(&mut data, "EnableTimer", Value::Bool(false));
            set(&mut data, "TimerType", json!(0));
        }
    }

    Value::Object(data)
}

fn trigger_sound_refs(trigger: &Trigger) -> Vec<String> {
    let mut sounds = Vec::new();
    let mut push = |value: &Option<String>| {
        if let Some(sound) = value {
            let lower = sound.to_ascii_lowercase();
            if lower.ends_with(".wav") || lower.ends_with(".mp3") {
                sounds.push(sound.clone());
            }
        }
    };
    push(&trigger.sound);
    if let Some(timer) = &trigger.timer {
        push(&timer.warning.sound);
        push(&timer.end.sound);
        push(&timer.early_end.sound);
    }
    sounds
}

// ---------------------------------------------------------------------------
// Field helpers
// ---------------------------------------------------------------------------

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn bool_field(object: &Map<String, Value>, key: &str) -> bool {
    object.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn i64_field(object: &Map<String, Value>, key: &str) -> Option<i64> {
    let value = object.get(key)?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|value| value as i64))
}

fn f64_field(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64)
}
