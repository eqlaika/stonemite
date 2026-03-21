use std::path::Path;
use std::time::{Duration, SystemTime};

/// A recently-active EQ character detected from log file modification times.
#[derive(Debug, Clone)]
pub struct CharCandidate {
    pub character: String,
    pub server: String,
}

/// Find characters with recently-modified log files (within `max_age`).
/// Returns candidates sorted by most recently modified first.
pub fn find_active_characters(eq_dir: &Path, max_age: Duration) -> Vec<CharCandidate> {
    let logs_dir = eq_dir.join("Logs");
    let Ok(entries) = std::fs::read_dir(&logs_dir) else {
        return Vec::new();
    };

    let now = SystemTime::now();
    let mut candidates: Vec<(CharCandidate, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if !name.starts_with("eqlog_") || !name.ends_with(".txt") {
            continue;
        }

        // Check modification time.
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if now.duration_since(modified).unwrap_or(max_age) > max_age {
            continue;
        }

        let stem = &name["eqlog_".len()..name.len() - ".txt".len()];
        let Some((character, server)) = stem.rsplit_once('_') else {
            continue;
        };

        candidates.push((
            CharCandidate {
                character: character.to_string(),
                server: server.to_string(),
            },
            modified,
        ));
    }

    // Sort by most recently modified first.
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    candidates.into_iter().map(|(c, _)| c).collect()
}
