//! Trigger library persistence.
//!
//! Layout (under the host-supplied root, e.g. `%APPDATA%\Stonemite\triggers`):
//!
//! ```text
//! triggers/
//! ├── library.json      versioned native schema
//! ├── assets/           managed WAV/MP3 media (content-addressed names)
//! ├── backups/          rotated copies written before each save
//! └── quarantine/       individual records that failed validation
//! ```
//!
//! Loading is salvage-oriented: a malformed record is moved to
//! `quarantine/` and the rest of the library loads; only an unreadable
//! file with no usable backup yields an empty library (and the damaged
//! file is preserved alongside).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::*;
use crate::report::{CompatReport, CompatSeverity};

/// Backups kept in rotation.
const BACKUP_KEEP: usize = 10;
/// Managed asset size cap (one file).
pub const MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;
/// Library file size cap.
pub const MAX_LIBRARY_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug)]
pub struct StoreError(pub String);

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        StoreError(error.to_string())
    }
}

pub struct TriggerStore {
    root: PathBuf,
}

/// Outcome of loading, including salvage notes.
#[derive(Debug, Default)]
pub struct LoadOutcome {
    pub library: TriggerLibrary,
    pub report: CompatReport,
}

impl TriggerStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn library_path(&self) -> PathBuf {
        self.root.join("library.json")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    fn quarantine_dir(&self) -> PathBuf {
        self.root.join("quarantine")
    }

    /// Load the library, salvaging around malformed records.
    pub fn load(&self) -> LoadOutcome {
        let mut outcome = LoadOutcome {
            library: TriggerLibrary::new(),
            ..LoadOutcome::default()
        };
        let path = self.library_path();
        let text = match read_limited(&path, MAX_LIBRARY_BYTES) {
            Ok(Some(text)) => text,
            Ok(None) => return outcome,
            Err(error) => {
                outcome.report.push(
                    CompatSeverity::Error,
                    "library.json",
                    "unreadable-library",
                    error.to_string(),
                );
                return self.load_backup_fallback(outcome);
            }
        };

        // Fast path: the whole document deserializes.
        match serde_json::from_str::<TriggerLibrary>(&text) {
            Ok(library) => {
                outcome.library = migrate(library, &mut outcome.report);
                outcome
            }
            Err(_) => self.salvage(&text, outcome),
        }
    }

    /// Per-record salvage: parse as loose JSON, validate each record
    /// individually, quarantine the failures.
    fn salvage(&self, text: &str, mut outcome: LoadOutcome) -> LoadOutcome {
        let Ok(Value::Object(document)) = serde_json::from_str::<Value>(text) else {
            outcome.report.push(
                CompatSeverity::Error,
                "library.json",
                "corrupt-library",
                "the library file is not valid JSON; falling back to the newest backup",
            );
            self.preserve_damaged(text);
            return self.load_backup_fallback(outcome);
        };

        let mut library = TriggerLibrary::new();
        if let Some(version) = document.get("schemaVersion").and_then(Value::as_u64) {
            library.schema_version = version as u32;
        }

        macro_rules! salvage_list {
            ($field:literal, $target:expr, $kind:ty) => {
                if let Some(Value::Array(records)) = document.get($field) {
                    for (position, record) in records.iter().enumerate() {
                        match serde_json::from_value::<$kind>(record.clone()) {
                            Ok(value) => $target.push(value),
                            Err(error) => {
                                let name = record
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unnamed");
                                self.quarantine_record($field, position, name, record);
                                outcome.report.push(
                                    CompatSeverity::Error,
                                    format!("{}[{position}] ({name})", $field),
                                    "quarantined-record",
                                    format!("moved to quarantine/: {error}"),
                                );
                            }
                        }
                    }
                }
            };
        }

        salvage_list!("folders", library.folders, Folder);
        salvage_list!("triggers", library.triggers, Trigger);
        salvage_list!("profiles", library.profiles, Profile);
        salvage_list!("textOverlays", library.text_overlays, TextOverlayPreset);
        salvage_list!("timerOverlays", library.timer_overlays, TimerOverlayPreset);
        salvage_list!("assets", library.assets, AssetRecord);

        outcome.library = migrate(library, &mut outcome.report);
        outcome
    }

    fn preserve_damaged(&self, text: &str) {
        let path = self
            .quarantine_dir()
            .join(format!("library-damaged-{}.json", timestamp()));
        let _ = fs::create_dir_all(self.quarantine_dir());
        let _ = fs::write(path, text);
    }

    fn quarantine_record(&self, field: &str, position: usize, name: &str, record: &Value) {
        let _ = fs::create_dir_all(self.quarantine_dir());
        let safe: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(40)
            .collect();
        let path = self
            .quarantine_dir()
            .join(format!("{field}-{position}-{safe}-{}.json", timestamp()));
        if let Ok(text) = serde_json::to_string_pretty(record) {
            let _ = fs::write(path, text);
        }
    }

    fn load_backup_fallback(&self, mut outcome: LoadOutcome) -> LoadOutcome {
        for backup in self.list_backups() {
            if let Ok(Some(text)) = read_limited(&backup, MAX_LIBRARY_BYTES) {
                if let Ok(library) = serde_json::from_str::<TriggerLibrary>(&text) {
                    outcome.report.push(
                        CompatSeverity::Warning,
                        backup.display().to_string(),
                        "backup-restored",
                        "the library was restored from this backup",
                    );
                    outcome.library = migrate(library, &mut outcome.report);
                    return outcome;
                }
            }
        }
        outcome
    }

    /// Newest first.
    fn list_backups(&self) -> Vec<PathBuf> {
        let mut backups: Vec<PathBuf> = fs::read_dir(self.backups_dir())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|extension| extension == "json")
                    })
                    .collect()
            })
            .unwrap_or_default();
        backups.sort();
        backups.reverse();
        backups
    }

    /// Save with backup rotation and atomic replacement.
    pub fn save(&self, library: &TriggerLibrary) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(self.assets_dir())?;
        fs::create_dir_all(self.backups_dir())?;

        let path = self.library_path();
        // Rotate a backup of the current file first.
        if path.exists() {
            let backup = self
                .backups_dir()
                .join(format!("library-{}.json", timestamp()));
            let _ = fs::copy(&path, backup);
            let backups = self.list_backups();
            for old in backups.iter().skip(BACKUP_KEEP) {
                let _ = fs::remove_file(old);
            }
        }

        let mut on_disk = library.clone();
        on_disk.schema_version = SCHEMA_VERSION;
        let text = serde_json::to_string_pretty(&on_disk)
            .map_err(|error| StoreError(format!("could not serialize the library: {error}")))?;
        let temp = self
            .root
            .join(format!(".library-{}.tmp", std::process::id()));
        fs::write(&temp, &text)?;
        // std::fs::rename replaces the destination atomically on both
        // Windows (MOVEFILE_REPLACE_EXISTING) and POSIX.
        fs::rename(&temp, &path).map_err(|error| {
            let _ = fs::remove_file(&temp);
            StoreError(format!("could not replace library.json: {error}"))
        })?;
        Ok(())
    }

    /// Copy a WAV/MP3 into the managed asset store; returns the record.
    /// Existing identical content is reused.
    pub fn add_asset(
        &self,
        library: &mut TriggerLibrary,
        source: &Path,
    ) -> Result<AssetRecord, StoreError> {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| StoreError("the file has no usable name".to_owned()))?
            .to_owned();
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".wav") && !lower.ends_with(".mp3") {
            return Err(StoreError(
                "only .wav and .mp3 files can be managed".to_owned(),
            ));
        }
        let metadata = fs::metadata(source)?;
        if metadata.len() > MAX_ASSET_BYTES {
            return Err(StoreError(
                "the file exceeds the 32 MB media limit".to_owned(),
            ));
        }
        let bytes = fs::read(source)?;
        let sha256 = hex(&Sha256::digest(&bytes));
        if let Some(existing) = library
            .assets
            .iter()
            .find(|asset| asset.sha256 == sha256 && asset.name.eq_ignore_ascii_case(&name))
        {
            return Ok(existing.clone());
        }
        let file_name = format!("{}-{}", &sha256[..8], sanitize_file_name(&name));
        fs::create_dir_all(self.assets_dir())?;
        fs::write(self.assets_dir().join(&file_name), &bytes)?;
        let record = AssetRecord {
            name,
            file_name,
            sha256,
            size: metadata.len(),
        };
        library.assets.push(record.clone());
        Ok(record)
    }

    /// Resolve a trigger's sound reference to a playable path. Only managed
    /// assets and bare file names resolve; path traversal never escapes the
    /// asset directory.
    pub fn resolve_sound(&self, library: &TriggerLibrary, reference: &str) -> Option<PathBuf> {
        if reference.contains(['/', '\\']) || reference.contains("..") {
            return None;
        }
        let record = library
            .assets
            .iter()
            .find(|asset| asset.name.eq_ignore_ascii_case(reference))?;
        let path = self.assets_dir().join(&record.file_name);
        path.exists().then_some(path)
    }

    /// Remove asset files no longer referenced by any asset record.
    pub fn sweep_assets(&self, library: &TriggerLibrary) -> Result<usize, StoreError> {
        let mut removed = 0;
        let entries = match fs::read_dir(self.assets_dir()) {
            Ok(entries) => entries,
            Err(_) => return Ok(0),
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !library.assets.iter().any(|asset| asset.file_name == name)
                && fs::remove_file(entry.path()).is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// Schema migrations: each step upgrades one version. Version 1 is current.
fn migrate(mut library: TriggerLibrary, report: &mut CompatReport) -> TriggerLibrary {
    if library.schema_version == 0 {
        // Pre-release libraries carried no version; nothing else changes.
        library.schema_version = 1;
    }
    if library.schema_version > SCHEMA_VERSION {
        report.push(
            CompatSeverity::Warning,
            "library.json",
            "newer-schema",
            format!(
                "the library was written by a newer Stonemite (schema {} > {}); unknown fields are preserved but invisible",
                library.schema_version, SCHEMA_VERSION
            ),
        );
        library.schema_version = SCHEMA_VERSION;
    }
    library
}

fn read_limited(path: &Path, limit: u64) -> Result<Option<String>, StoreError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(StoreError(format!(
            "{} exceeds the size limit",
            path.display()
        )));
    }
    Ok(Some(text))
}

fn sanitize_file_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:017}", now.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("eqtrigger-store-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_library() -> TriggerLibrary {
        let mut library = TriggerLibrary::new();
        library.triggers.push(Trigger {
            name: "One".to_owned(),
            pattern: Pattern::literal("hello"),
            ..Trigger::default()
        });
        library.profiles.push(Profile {
            name: "Default".to_owned(),
            ..Profile::default()
        });
        library
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let library = sample_library();
        store.save(&library).unwrap();
        let loaded = store.load();
        assert!(loaded.report.issues.is_empty());
        assert_eq!(loaded.library, library);
    }

    #[test]
    fn missing_library_loads_empty_without_errors() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let loaded = store.load();
        assert!(loaded.report.issues.is_empty());
        assert!(loaded.library.triggers.is_empty());
        assert_eq!(loaded.library.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn malformed_record_is_quarantined_not_fatal() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        store.save(&sample_library()).unwrap();

        // Corrupt one trigger record (wrong type for `pattern`).
        let path = store.library_path();
        let mut document: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        document["triggers"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({ "name": "broken", "pattern": 42 }));
        fs::write(&path, serde_json::to_string(&document).unwrap()).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.library.triggers.len(), 1);
        assert_eq!(loaded.library.triggers[0].name, "One");
        assert!(loaded
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "quarantined-record"));
        // The broken record was preserved on disk.
        let quarantined: Vec<_> = fs::read_dir(dir.0.join("triggers/quarantine"))
            .unwrap()
            .collect();
        assert_eq!(quarantined.len(), 1);
    }

    #[test]
    fn corrupt_file_falls_back_to_newest_backup() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let library = sample_library();
        store.save(&library).unwrap();
        // A second save rotates a backup of the good file.
        store.save(&library).unwrap();
        fs::write(store.library_path(), "{{{{ total garbage").unwrap();

        let loaded = store.load();
        assert_eq!(loaded.library.triggers.len(), 1);
        assert!(loaded
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "backup-restored"));
    }

    #[test]
    fn backups_rotate_and_are_capped() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        for _ in 0..15 {
            store.save(&sample_library()).unwrap();
            // Millisecond timestamps must differ.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(store.list_backups().len() <= BACKUP_KEEP + 1);
    }

    #[test]
    fn assets_are_content_addressed_and_traversal_safe() {
        let dir = TempDir::new();
        let store = TriggerStore::new(dir.0.join("triggers"));
        let mut library = sample_library();
        let wav = dir.0.join("beep.wav");
        fs::write(&wav, b"RIFF0000WAVEfake").unwrap();

        let record = store.add_asset(&mut library, &wav).unwrap();
        assert_eq!(record.name, "beep.wav");
        assert!(store.assets_dir().join(&record.file_name).exists());
        // Same content re-adds as the same record.
        let again = store.add_asset(&mut library, &wav).unwrap();
        assert_eq!(again, record);
        assert_eq!(library.assets.len(), 1);

        assert!(store.resolve_sound(&library, "beep.wav").is_some());
        assert!(store.resolve_sound(&library, "BEEP.WAV").is_some());
        assert!(store.resolve_sound(&library, "../beep.wav").is_none());
        assert!(store
            .resolve_sound(&library, "c:\\windows\\x.wav")
            .is_none());
        assert!(store.resolve_sound(&library, "other.wav").is_none());

        // Unreferenced files are swept.
        fs::write(store.assets_dir().join("orphan.wav"), b"x").unwrap();
        assert_eq!(store.sweep_assets(&library).unwrap(), 1);
        assert!(store.assets_dir().join(&record.file_name).exists());

        let text = fs::read_to_string(dir.0.join("beep.wav")).unwrap();
        assert_eq!(text, "RIFF0000WAVEfake");

        // Non-media is refused.
        let exe = dir.0.join("evil.exe");
        fs::write(&exe, b"MZ").unwrap();
        assert!(store.add_asset(&mut library, &exe).is_err());
    }
}
