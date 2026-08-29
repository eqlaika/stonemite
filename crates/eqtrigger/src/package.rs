//! Native `.stonemite-triggers` packages.
//!
//! A ZIP archive carrying selected triggers, the presets they depend on,
//! and their managed media, so a share round-trips losslessly (which EQLP
//! packages cannot do — they never embed media):
//!
//! ```text
//! manifest.json          format marker + version + native records
//! assets/<file_name>     content-addressed media referenced by triggers
//! ```
//!
//! Import defends against hostile archives: entry-count, per-file and
//! total-size limits, and strict name validation (no traversal, no
//! absolute paths, no drive letters).

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::eqlp::CodecError;
use crate::model::*;
use crate::report::{CompatReport, CompatSeverity};
use crate::store::TriggerStore;

pub const FORMAT: &str = "stonemite-triggers";
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 4_096;
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    version: u32,
    folders: Vec<Folder>,
    triggers: Vec<Trigger>,
    text_overlays: Vec<TextOverlayPreset>,
    timer_overlays: Vec<TimerOverlayPreset>,
    assets: Vec<AssetRecord>,
}

#[derive(Debug, Default)]
pub struct PackageImport {
    pub folders: Vec<Folder>,
    pub triggers: Vec<Trigger>,
    pub text_overlays: Vec<TextOverlayPreset>,
    pub timer_overlays: Vec<TimerOverlayPreset>,
    /// Asset records plus their raw bytes, verified against their digests.
    pub assets: Vec<(AssetRecord, Vec<u8>)>,
    pub report: CompatReport,
}

/// Export the selection plus its dependencies (referenced presets and
/// managed media) into a `.stonemite-triggers` archive.
pub fn export(
    library: &TriggerLibrary,
    store: Option<&TriggerStore>,
    folder_ids: &[FolderId],
    trigger_ids: &[TriggerId],
) -> Result<Vec<u8>, CodecError> {
    let mut folders: Vec<FolderId> = Vec::new();
    for folder in folder_ids {
        for id in library.folder_subtree(*folder) {
            if !folders.contains(&id) {
                folders.push(id);
            }
        }
    }
    let triggers: Vec<Trigger> = library
        .triggers
        .iter()
        .filter(|trigger| {
            trigger_ids.contains(&trigger.id)
                || trigger
                    .folder
                    .is_some_and(|folder| folders.contains(&folder))
        })
        .cloned()
        .collect();
    // Ancestor folders needed to rebuild the tree.
    for trigger in &triggers {
        let mut cursor = trigger.folder;
        while let Some(folder) = cursor {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
            cursor = library.folder(folder).and_then(|f| f.parent);
        }
    }

    // Dependent presets.
    let text_overlays: Vec<TextOverlayPreset> = library
        .text_overlays
        .iter()
        .filter(|preset| {
            triggers
                .iter()
                .any(|t| t.text_overlays.contains(&preset.id))
        })
        .cloned()
        .collect();
    let timer_overlays: Vec<TimerOverlayPreset> = library
        .timer_overlays
        .iter()
        .filter(|preset| {
            triggers
                .iter()
                .any(|t| t.timer_overlays.contains(&preset.id))
        })
        .cloned()
        .collect();

    // Dependent media.
    let mut assets: Vec<AssetRecord> = Vec::new();
    for trigger in &triggers {
        for sound in sound_refs(trigger) {
            if let Some(record) = library
                .assets
                .iter()
                .find(|asset| asset.name.eq_ignore_ascii_case(&sound))
            {
                if !assets
                    .iter()
                    .any(|existing| existing.file_name == record.file_name)
                {
                    assets.push(record.clone());
                }
            }
        }
    }

    let manifest = Manifest {
        format: FORMAT.to_owned(),
        version: FORMAT_VERSION,
        folders: library
            .folders
            .iter()
            .filter(|folder| folders.contains(&folder.id))
            .cloned()
            .collect(),
        triggers,
        text_overlays,
        timer_overlays,
        assets: assets.clone(),
    };

    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::FileOptions::<()>::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("manifest.json", options)
            .and_then(|_| {
                writer.write_all(
                    serde_json::to_string_pretty(&manifest)
                        .map_err(|error| zip::result::ZipError::Io(std::io::Error::other(error)))?
                        .as_bytes(),
                )?;
                Ok(())
            })
            .map_err(|error| CodecError(format!("could not write the package: {error}")))?;
        for record in &assets {
            let Some(store) = store else { continue };
            let path = store.assets_dir().join(&record.file_name);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            writer
                .start_file(format!("assets/{}", record.file_name), options)
                .and_then(|_| {
                    writer.write_all(&bytes)?;
                    Ok(())
                })
                .map_err(|error| CodecError(format!("could not write the package: {error}")))?;
        }
        writer
            .finish()
            .map_err(|error| CodecError(format!("could not finish the package: {error}")))?;
    }
    Ok(buffer.into_inner())
}

/// Read and validate a `.stonemite-triggers` archive.
pub fn import(bytes: &[u8]) -> Result<PackageImport, CodecError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|error| CodecError(format!("could not open the package: {error}")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(CodecError(format!(
            "the package has more than {MAX_ENTRIES} entries"
        )));
    }

    let mut manifest: Option<Manifest> = None;
    let mut asset_bytes: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total: u64 = 0;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| CodecError(format!("could not read the package: {error}")))?;
        let name = entry.name().to_owned();
        validate_entry_name(&name)?;
        if entry.size() > MAX_ENTRY_BYTES {
            return Err(CodecError(format!(
                "'{name}' exceeds the per-file size limit"
            )));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err(CodecError(
                "the package exceeds the total size limit".to_owned(),
            ));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| CodecError(format!("could not read '{name}': {error}")))?;
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            return Err(CodecError(format!(
                "'{name}' inflates beyond the size limit"
            )));
        }
        if name == "manifest.json" {
            let parsed: Manifest = serde_json::from_slice(&bytes)
                .map_err(|error| CodecError(format!("invalid manifest: {error}")))?;
            if parsed.format != FORMAT {
                return Err(CodecError(
                    "this is not a Stonemite trigger package".to_owned(),
                ));
            }
            manifest = Some(parsed);
        } else if let Some(asset_name) = name.strip_prefix("assets/") {
            asset_bytes.push((asset_name.to_owned(), bytes));
        }
        // Unknown top-level entries are ignored for forward compatibility.
    }

    let manifest =
        manifest.ok_or_else(|| CodecError("the package has no manifest.json".to_owned()))?;
    let mut import = PackageImport {
        folders: manifest.folders,
        triggers: manifest.triggers,
        text_overlays: manifest.text_overlays,
        timer_overlays: manifest.timer_overlays,
        ..PackageImport::default()
    };
    if manifest.version > FORMAT_VERSION {
        import.report.push(
            CompatSeverity::Warning,
            "manifest.json",
            "newer-package",
            format!(
                "the package was written by a newer Stonemite (version {} > {FORMAT_VERSION})",
                manifest.version
            ),
        );
    }

    // Imports arrive disabled, matching the EQLP/GINA paths.
    for trigger in &mut import.triggers {
        trigger.enabled = false;
    }

    for record in manifest.assets {
        match asset_bytes
            .iter()
            .find(|(name, _)| *name == record.file_name)
        {
            Some((_, bytes)) => {
                let digest = hex(&Sha256::digest(bytes));
                if digest == record.sha256 {
                    import.assets.push((record, bytes.clone()));
                } else {
                    import.report.push(
                        CompatSeverity::Error,
                        &record.name,
                        "corrupt-media",
                        "the embedded media does not match its digest and was skipped",
                    );
                }
            }
            None => import.report.push(
                CompatSeverity::Warning,
                &record.name,
                "missing-media",
                "the manifest references media that is not in the package",
            ),
        }
    }

    import.report.triggers_imported = import.triggers.len();
    import.report.folders_imported = import.folders.len();
    import.report.overlays_imported = import.text_overlays.len() + import.timer_overlays.len();
    Ok(import)
}

/// Write imported media into the store's asset directory (used after the
/// user confirms an import).
pub fn install_assets(
    store: &TriggerStore,
    library: &mut TriggerLibrary,
    assets: &[(AssetRecord, Vec<u8>)],
) -> Result<(), CodecError> {
    std::fs::create_dir_all(store.assets_dir()).map_err(|error| CodecError(error.to_string()))?;
    for (record, bytes) in assets {
        validate_entry_name(&record.file_name)?;
        let path = store.assets_dir().join(&record.file_name);
        if !path.exists() {
            std::fs::write(&path, bytes).map_err(|error| CodecError(error.to_string()))?;
        }
        if !library
            .assets
            .iter()
            .any(|existing| existing.file_name == record.file_name)
        {
            library.assets.push(record.clone());
        }
    }
    Ok(())
}

fn validate_entry_name(name: &str) -> Result<(), CodecError> {
    let suspicious = name.is_empty()
        || name.len() > 512
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('/')
        || name.contains(':')
        || name.chars().any(|c| c.is_control());
    if suspicious {
        return Err(CodecError(format!(
            "the package contains an unsafe entry name: '{name}'"
        )));
    }
    // Belt and braces: every component must be a normal component.
    for component in Path::new(name).components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(CodecError(format!(
                "the package contains an unsafe entry path: '{name}'"
            )));
        }
    }
    Ok(())
}

fn sound_refs(trigger: &Trigger) -> Vec<String> {
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("eqtrigger-package-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn library_with_media(store: &TriggerStore, dir: &TempDir) -> TriggerLibrary {
        let mut library = TriggerLibrary::new();
        let folder = Folder {
            name: "Pack".to_owned(),
            ..Folder::default()
        };
        let preset = TextOverlayPreset {
            name: "Preset".to_owned(),
            ..TextOverlayPreset::default()
        };
        let wav = dir.0.join("ding.wav");
        fs::write(&wav, b"RIFFxxxxWAVEdata").unwrap();
        store.add_asset(&mut library, &wav).unwrap();
        library.triggers.push(Trigger {
            name: "Ding".to_owned(),
            folder: Some(folder.id),
            enabled: true,
            pattern: Pattern::literal("ding"),
            sound: Some("ding.wav".to_owned()),
            text_overlays: vec![preset.id],
            ..Trigger::default()
        });
        library.folders.push(folder);
        library.text_overlays.push(preset);
        library
    }

    #[test]
    fn native_package_round_trips_triggers_presets_and_media() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let library = library_with_media(&store, &dir);

        let bytes = export(&library, Some(&store), &[library.folders[0].id], &[]).unwrap();
        let imported = import(&bytes).unwrap();

        assert_eq!(imported.folders.len(), 1);
        assert_eq!(imported.triggers.len(), 1);
        assert_eq!(imported.text_overlays.len(), 1);
        assert_eq!(imported.assets.len(), 1);
        assert_eq!(imported.assets[0].0.name, "ding.wav");
        assert_eq!(imported.assets[0].1, b"RIFFxxxxWAVEdata");
        // Native ids are preserved (unlike EQLP round-trips).
        assert_eq!(imported.triggers[0].id, library.triggers[0].id);
        assert_eq!(
            imported.triggers[0].text_overlays,
            vec![library.text_overlays[0].id]
        );
        // But imports still arrive disabled.
        assert!(!imported.triggers[0].enabled);
        assert!(!imported.report.has_errors());

        // Install media into a second store.
        let second = TriggerStore::new(dir.0.join("second"));
        let mut target = TriggerLibrary::new();
        install_assets(&second, &mut target, &imported.assets).unwrap();
        assert_eq!(target.assets.len(), 1);
        assert!(second.resolve_sound(&target, "ding.wav").is_some());
    }

    #[test]
    fn traversal_and_hostile_entries_are_rejected() {
        for name in [
            "../evil.wav",
            "assets/../../evil.wav",
            "/absolute.wav",
            "c:pwn.wav",
            "bad\\slash.wav",
        ] {
            assert!(
                validate_entry_name(name).is_err(),
                "{name} must be rejected"
            );
        }

        // A real archive with a traversal entry is rejected wholesale.
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::FileOptions::<()>::default();
            writer.start_file("manifest.json", options).unwrap();
            writer
                .write_all(br#"{"format":"stonemite-triggers","version":1,"folders":[],"triggers":[],"textOverlays":[],"timerOverlays":[],"assets":[]}"#)
                .unwrap();
            writer
                .start_file("assets/../../escape.wav", options)
                .unwrap();
            writer.write_all(b"pwn").unwrap();
            writer.finish().unwrap();
        }
        assert!(import(&buffer.into_inner()).is_err());
    }

    #[test]
    fn corrupted_media_is_skipped_with_a_report() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let library = library_with_media(&store, &dir);
        let bytes = export(&library, Some(&store), &[library.folders[0].id], &[]).unwrap();

        // Tamper: rebuild the zip with modified asset content.
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::FileOptions::<()>::default();
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index).unwrap();
                let name = entry.name().to_owned();
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                if name.starts_with("assets/") {
                    content = b"tampered".to_vec();
                }
                writer.start_file(name, options).unwrap();
                writer.write_all(&content).unwrap();
            }
            writer.finish().unwrap();
        }

        let imported = import(&buffer.into_inner()).unwrap();
        assert!(imported.assets.is_empty());
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "corrupt-media"));
    }

    #[test]
    fn non_package_zip_is_refused() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .start_file::<_, ()>("random.txt", zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap();
        }
        let error = import(&buffer.into_inner()).unwrap_err();
        assert!(error.0.contains("manifest"));
    }
}
