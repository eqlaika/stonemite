//! Tauri commands backing the Trigger Manager workbench.
//!
//! These run in the settings subprocess. Every mutation goes through the
//! on-disk trigger store; the running tray process hot-reloads after the
//! standard `WM_SETTINGS_CHANGED` notification. The test bench runs a
//! scratch engine on a virtual clock — nothing here can reach input
//! broadcasting or the live overlay.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use eqtrigger::store::TriggerStore;
use eqtrigger::{
    eqlp, gina, package, ActionEvent, CharacterContext, CompatReport, CompiledLibrary, FolderId,
    LineTrace, TimerSnapshot, Trigger, TriggerEngine, TriggerId, TriggerLibrary,
};

fn store() -> Result<TriggerStore, String> {
    crate::log_watcher::store_root()
        .map(TriggerStore::new)
        .ok_or_else(|| "Stonemite could not locate its configuration directory".to_owned())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerLibraryPayload {
    pub library: TriggerLibrary,
    pub report: CompatReport,
    pub builtin_sounds: Vec<BuiltinSoundOption>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinSoundOption {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub fn load_trigger_library() -> Result<TriggerLibraryPayload, String> {
    let outcome = store()?.load();
    Ok(TriggerLibraryPayload {
        library: outcome.library,
        report: outcome.report,
        builtin_sounds: crate::sound::BUILTIN_SOUNDS
            .iter()
            .map(|sound| BuiltinSoundOption {
                id: sound.id.to_owned(),
                label: sound.label.to_owned(),
            })
            .collect(),
    })
}

/// Hard caps keeping a hostile or runaway save from exhausting the host.
const MAX_TRIGGERS: usize = 50_000;
const MAX_FOLDERS: usize = 10_000;

#[tauri::command]
pub fn save_trigger_library(library: TriggerLibrary) -> Result<(), String> {
    if library.triggers.len() > MAX_TRIGGERS {
        return Err(format!("the library exceeds {MAX_TRIGGERS} triggers"));
    }
    if library.folders.len() > MAX_FOLDERS {
        return Err(format!("the library exceeds {MAX_FOLDERS} folders"));
    }
    store()?
        .save(&library)
        .map_err(|error| format!("Stonemite could not save the trigger library: {error}"))?;
    super::settings_dialog::notify_tray();
    Ok(())
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportKind {
    EqlpTriggers,
    EqlpOverlays,
    Gina,
    Stonemite,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub kind: ImportKind,
    pub file_name: String,
    pub folder_count: usize,
    pub trigger_count: usize,
    pub overlay_count: usize,
    pub asset_count: usize,
    pub quarantined: usize,
    pub trigger_names: Vec<String>,
    pub report: CompatReport,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    /// Wrap the import in a new root folder with this name; `None` merges at
    /// the library root.
    pub new_folder_name: Option<String>,
    /// Replace same-named triggers in the same folder instead of keeping both.
    pub replace_same_names: bool,
    /// Enable the imported triggers immediately (default: disabled).
    pub enable: bool,
}

struct ParsedImport {
    kind: ImportKind,
    folders: Vec<eqtrigger::Folder>,
    triggers: Vec<Trigger>,
    text_overlays: Vec<eqtrigger::TextOverlayPreset>,
    timer_overlays: Vec<eqtrigger::TimerOverlayPreset>,
    assets: Vec<(eqtrigger::AssetRecord, Vec<u8>)>,
    report: CompatReport,
}

const MAX_IMPORT_FILE_BYTES: u64 = 256 * 1024 * 1024;

fn parse_import(path: &str, library: &TriggerLibrary) -> Result<ParsedImport, String> {
    let path = PathBuf::from(path);
    let size = std::fs::metadata(&path)
        .map_err(|error| format!("Stonemite could not read the file: {error}"))?
        .len();
    if size > MAX_IMPORT_FILE_BYTES {
        return Err("the file exceeds the 256 MB import limit".to_owned());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Stonemite could not read the file: {error}"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if name.ends_with(".gtp") {
        let import = gina::import(&bytes).map_err(|error| error.to_string())?;
        return Ok(ParsedImport {
            kind: ImportKind::Gina,
            folders: import.folders,
            triggers: import.triggers,
            text_overlays: Vec::new(),
            timer_overlays: Vec::new(),
            assets: Vec::new(),
            report: import.report,
        });
    }
    if name.ends_with(".stonemite-triggers") {
        let import = package::import(&bytes).map_err(|error| error.to_string())?;
        return Ok(ParsedImport {
            kind: ImportKind::Stonemite,
            folders: import.folders,
            triggers: import.triggers,
            text_overlays: import.text_overlays,
            timer_overlays: import.timer_overlays,
            assets: import.assets,
            report: import.report,
        });
    }
    if name.ends_with(".tgf.gz") || name.ends_with(".ogf.gz") {
        // Existing overlays participate in reference resolution.
        let known: Vec<(eqtrigger::OverlayId, String)> = library
            .text_overlays
            .iter()
            .map(|preset| (preset.id, eqlp_overlay_key(&preset.passthrough, preset.id)))
            .chain(
                library
                    .timer_overlays
                    .iter()
                    .map(|preset| (preset.id, eqlp_overlay_key(&preset.passthrough, preset.id))),
            )
            .collect();
        let import = eqlp::import(&bytes, &known).map_err(|error| error.to_string())?;
        let kind = if name.ends_with(".ogf.gz") {
            ImportKind::EqlpOverlays
        } else {
            ImportKind::EqlpTriggers
        };
        return Ok(ParsedImport {
            kind,
            folders: import.folders,
            triggers: import.triggers,
            text_overlays: import.text_overlays,
            timer_overlays: import.timer_overlays,
            assets: Vec::new(),
            report: import.report,
        });
    }
    Err("Stonemite imports .tgf.gz, .ogf.gz, .gtp, and .stonemite-triggers files".to_owned())
}

fn eqlp_overlay_key(passthrough: &eqtrigger::Passthrough, id: eqtrigger::OverlayId) -> String {
    passthrough
        .get("Id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string())
}

#[tauri::command]
pub fn choose_trigger_import_file() -> Option<String> {
    rfd::FileDialog::new()
        .add_filter("Trigger packages", &["gz", "gtp", "stonemite-triggers"])
        .pick_file()
        .map(|path| path.display().to_string())
}

#[tauri::command]
pub fn preview_trigger_import(path: String) -> Result<ImportPreview, String> {
    let library = store()?.load().library;
    let parsed = parse_import(&path, &library)?;
    let mut trigger_names: Vec<String> = parsed
        .triggers
        .iter()
        .map(|trigger| trigger.name.clone())
        .collect();
    trigger_names.truncate(200);
    Ok(ImportPreview {
        kind: parsed.kind,
        file_name: PathBuf::from(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(path),
        folder_count: parsed.folders.len(),
        trigger_count: parsed.triggers.len(),
        overlay_count: parsed.text_overlays.len() + parsed.timer_overlays.len(),
        asset_count: parsed.assets.len(),
        quarantined: parsed
            .triggers
            .iter()
            .filter(|trigger| trigger.quarantine.is_some())
            .count(),
        trigger_names,
        report: parsed.report,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub triggers_added: usize,
    pub triggers_replaced: usize,
    pub folders_added: usize,
    pub overlays_added: usize,
    pub assets_added: usize,
}

#[tauri::command]
pub fn commit_trigger_import(
    path: String,
    options: ImportOptions,
) -> Result<ImportSummary, String> {
    let store = store()?;
    let mut library = store.load().library;
    let mut parsed = parse_import(&path, &library)?;

    // Optional wrapper folder.
    let mut root_parent = None;
    if let Some(name) = options
        .new_folder_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let wrapper = eqtrigger::Folder {
            name: name.to_owned(),
            index: library.folders.len() as u32,
            ..eqtrigger::Folder::default()
        };
        root_parent = Some(wrapper.id);
        library.folders.push(wrapper);
    }
    for folder in &mut parsed.folders {
        if folder.parent.is_none() {
            folder.parent = root_parent;
        }
    }

    if options.enable {
        for trigger in &mut parsed.triggers {
            if trigger.quarantine.is_none() {
                trigger.enabled = true;
            }
        }
    }

    let mut replaced = 0;
    if options.replace_same_names {
        // Same-named triggers directly in the target root are replaced.
        let names: Vec<String> = parsed
            .triggers
            .iter()
            .map(|trigger| trigger.name.to_lowercase())
            .collect();
        let before = library.triggers.len();
        library.triggers.retain(|existing| {
            !(existing.folder == root_parent && names.contains(&existing.name.to_lowercase()))
        });
        replaced = before - library.triggers.len();
    }

    // Native packages can collide on preserved ids; regenerate on conflict.
    for trigger in &mut parsed.triggers {
        if library.trigger(trigger.id).is_some() {
            trigger.id = TriggerId::new();
        }
    }

    let folders_added = parsed.folders.len();
    let triggers_added = parsed.triggers.len();
    let overlays_added = parsed.text_overlays.len() + parsed.timer_overlays.len();
    let assets_added = parsed.assets.len();

    library.folders.extend(parsed.folders);
    library.triggers.extend(parsed.triggers);
    // Skip overlay presets that already exist (same EQLP id).
    for preset in parsed.text_overlays {
        let key = eqlp_overlay_key(&preset.passthrough, preset.id);
        if !library
            .text_overlays
            .iter()
            .any(|existing| eqlp_overlay_key(&existing.passthrough, existing.id) == key)
        {
            library.text_overlays.push(preset);
        }
    }
    for preset in parsed.timer_overlays {
        let key = eqlp_overlay_key(&preset.passthrough, preset.id);
        if !library
            .timer_overlays
            .iter()
            .any(|existing| eqlp_overlay_key(&existing.passthrough, existing.id) == key)
        {
            library.timer_overlays.push(preset);
        }
    }
    package::install_assets(&store, &mut library, &parsed.assets)
        .map_err(|error| error.to_string())?;

    store
        .save(&library)
        .map_err(|error| format!("Stonemite could not save the trigger library: {error}"))?;
    super::settings_dialog::notify_tray();
    Ok(ImportSummary {
        triggers_added,
        triggers_replaced: replaced,
        folders_added,
        overlays_added,
        assets_added,
    })
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    EqlpTriggers,
    EqlpOverlays,
    Stonemite,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportScope {
    pub format: ExportFormat,
    pub full_library: bool,
    pub folder_ids: Vec<FolderId>,
    pub trigger_ids: Vec<TriggerId>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    /// `None` when the user cancelled the save dialog.
    pub path: Option<String>,
    pub report: CompatReport,
}

#[tauri::command]
pub fn export_trigger_selection(scope: ExportScope) -> Result<ExportOutcome, String> {
    let store = store()?;
    let library = store.load().library;

    let (folders, triggers) = if scope.full_library {
        (
            library
                .folders
                .iter()
                .filter(|folder| folder.parent.is_none())
                .map(|folder| folder.id)
                .collect(),
            library
                .triggers
                .iter()
                .filter(|trigger| trigger.folder.is_none())
                .map(|trigger| trigger.id)
                .collect(),
        )
    } else {
        (scope.folder_ids.clone(), scope.trigger_ids.clone())
    };

    let mut report = CompatReport::default();
    let (bytes, extension, filter_name) = match scope.format {
        ExportFormat::EqlpTriggers => (
            eqlp::export_triggers(
                &library,
                &eqlp::ExportSelection { folders, triggers },
                &mut report,
            )
            .map_err(|error| error.to_string())?,
            "tgf.gz",
            "EQLP Triggers File",
        ),
        ExportFormat::EqlpOverlays => (
            eqlp::export_overlays(&library).map_err(|error| error.to_string())?,
            "ogf.gz",
            "EQLP Overlays File",
        ),
        ExportFormat::Stonemite => (
            package::export(&library, Some(&store), &folders, &triggers)
                .map_err(|error| error.to_string())?,
            "stonemite-triggers",
            "Stonemite Trigger Package",
        ),
    };

    let Some(path) = rfd::FileDialog::new()
        .add_filter(filter_name, &[extension])
        .set_file_name(format!("stonemite-triggers.{extension}"))
        .save_file()
    else {
        return Ok(ExportOutcome { path: None, report });
    };
    std::fs::write(&path, bytes)
        .map_err(|error| format!("Stonemite could not write the export: {error}"))?;
    Ok(ExportOutcome {
        path: Some(path.display().to_string()),
        report,
    })
}

// ---------------------------------------------------------------------------
// Managed media
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn add_trigger_media() -> Result<Option<eqtrigger::AssetRecord>, String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Sounds", &["wav", "mp3"])
        .pick_file()
    else {
        return Ok(None);
    };
    let store = store()?;
    let mut library = store.load().library;
    let record = store
        .add_asset(&mut library, &path)
        .map_err(|error| error.to_string())?;
    store
        .save(&library)
        .map_err(|error| format!("Stonemite could not save the trigger library: {error}"))?;
    super::settings_dialog::notify_tray();
    Ok(Some(record))
}

#[tauri::command]
pub fn preview_trigger_sound(reference: String) -> Result<(), String> {
    let store = store()?;
    let library = store.load().library;
    if let Some(path) = store.resolve_sound(&library, &reference) {
        crate::audio::preview_file(&path);
        return Ok(());
    }
    if crate::sound::find_id(&reference).is_some() {
        crate::sound::play(&reference);
        return Ok(());
    }
    Err(format!("'{reference}' is not a managed sound"))
}

// ---------------------------------------------------------------------------
// Test bench
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestBenchRequest {
    pub lines: Vec<String>,
    pub character: String,
    pub server: String,
    /// Evaluate every non-quarantined trigger, not just enabled ones.
    pub include_disabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestBenchLine {
    pub line: String,
    /// Milliseconds on the bench's virtual clock (from log timestamps when
    /// present; one second per line otherwise).
    pub at_ms: u64,
    pub trace: LineTrace,
    pub events: Vec<ActionEvent>,
    pub timers_after: Vec<TimerSnapshot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestBenchResult {
    pub lines: Vec<TestBenchLine>,
    pub compile_errors: Vec<String>,
    pub active_triggers: usize,
}

const MAX_BENCH_LINES: usize = 2_000;

#[tauri::command]
pub fn run_trigger_test(request: TestBenchRequest) -> Result<TestBenchResult, String> {
    if request.lines.len() > MAX_BENCH_LINES {
        return Err(format!(
            "the test bench accepts up to {MAX_BENCH_LINES} lines"
        ));
    }
    let mut library = store()?.load().library;
    if request.include_disabled {
        for trigger in &mut library.triggers {
            if trigger.quarantine.is_none() {
                trigger.enabled = true;
            }
        }
    }
    let compiled = std::sync::Arc::new(CompiledLibrary::compile(&library));
    let compile_errors = compiled
        .compile_errors
        .iter()
        .map(|(id, error)| {
            let name = library
                .trigger(*id)
                .map(|trigger| trigger.name.as_str())
                .unwrap_or("unknown");
            format!("{name}: {error}")
        })
        .collect();
    let active_triggers = compiled.active_trigger_count();
    let mut engine = TriggerEngine::new(compiled);
    let context = CharacterContext {
        key: "test-bench".to_owned(),
        character: if request.character.trim().is_empty() {
            "Tester".to_owned()
        } else {
            request.character.trim().to_owned()
        },
        server: request.server.trim().to_owned(),
    };

    let source = eqlog::LogSource::new(
        "test-bench",
        context.character.as_str(),
        context.server.as_str(),
    );
    let mut result_lines = Vec::new();
    let mut first_second: Option<i64> = None;
    for (index, raw_line) in request.lines.iter().enumerate() {
        let decoded = eqlog::RawLogLine::decode(source.clone(), raw_line.as_bytes());
        let line = decoded.line;
        let at_ms = line
            .timestamp
            .as_ref()
            .and_then(|timestamp| timestamp.second())
            .map(|second| {
                let base = *first_second.get_or_insert(second.value());
                (second.value() - base).max(0) as u64 * 1_000
            })
            .unwrap_or(index as u64 * 1_000);

        // Fire timers that came due between lines, so end/warning actions
        // appear in order.
        let advance = engine.advance(at_ms);
        let mut events = advance.events;

        let mut trace = LineTrace::default();
        let log_time = line
            .timestamp
            .as_ref()
            .map(|timestamp| timestamp.as_str().to_owned());
        let batch = engine.process_line(
            &context,
            &line.body,
            log_time.as_deref(),
            at_ms,
            Some(&mut trace),
        );
        events.extend(batch.events);
        result_lines.push(TestBenchLine {
            line: raw_line.clone(),
            at_ms,
            trace,
            events,
            timers_after: engine.timer_snapshots(),
        });
    }

    Ok(TestBenchResult {
        lines: result_lines,
        compile_errors,
        active_triggers,
    })
}
