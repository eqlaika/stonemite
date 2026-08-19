use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use super::diagnostic::{DiagnosticKind, LogDiagnostic};
use super::WorkerWake;

/// Thin adapter around Windows ReadDirectoryChangesW via `notify`.
/// Notifications are wake-up hints only; no event payload is treated as log
/// content or as proof that a write occurred exactly once.
pub(crate) struct DirectoryWatcher {
    watcher: Option<RecommendedWatcher>,
    watched_dir: Option<PathBuf>,
    last_failure: Option<String>,
    wake: Arc<WorkerWake>,
}

impl DirectoryWatcher {
    pub fn new(wake: Arc<WorkerWake>) -> Self {
        Self {
            watcher: None,
            watched_dir: None,
            last_failure: None,
            wake,
        }
    }

    pub fn ensure_directory(&mut self, logs_dir: &Path) -> Vec<LogDiagnostic> {
        let mut diagnostics = Vec::new();
        if self.watched_dir.as_deref() == Some(logs_dir) && self.watcher.is_some() {
            return diagnostics;
        }

        if let (Some(watcher), Some(old_dir)) = (&mut self.watcher, self.watched_dir.take()) {
            let _ = watcher.unwatch(&old_dir);
        }

        if self.watcher.is_none() {
            let wake = self.wake.clone();
            match notify::recommended_watcher(move |result| handle_event(&wake, result)) {
                Ok(watcher) => self.watcher = Some(watcher),
                Err(error) => {
                    self.report_failure(
                        logs_dir,
                        format!("could not create filesystem watcher: {error}"),
                        &mut diagnostics,
                    );
                    return diagnostics;
                }
            }
        }

        let Some(watcher) = self.watcher.as_mut() else {
            return diagnostics;
        };
        match watcher.watch(logs_dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                self.watched_dir = Some(logs_dir.to_path_buf());
                if self.last_failure.take().is_some() {
                    diagnostics.push(LogDiagnostic::new(
                        DiagnosticKind::Reconciliation,
                        Some(logs_dir.to_path_buf()),
                        "filesystem watcher recovered",
                    ));
                }
            }
            Err(error) => self.report_failure(
                logs_dir,
                format!("could not watch Logs directory: {error}"),
                &mut diagnostics,
            ),
        }
        diagnostics
    }

    fn report_failure(
        &mut self,
        logs_dir: &Path,
        failure: String,
        diagnostics: &mut Vec<LogDiagnostic>,
    ) {
        if self.last_failure.as_deref() != Some(&failure) {
            diagnostics.push(LogDiagnostic::new(
                DiagnosticKind::WatcherStartup,
                Some(logs_dir.to_path_buf()),
                failure.clone(),
            ));
        }
        self.last_failure = Some(failure);
        self.watched_dir = None;
    }
}

fn handle_event(wake: &WorkerWake, result: notify::Result<Event>) {
    match result {
        Ok(event) if is_change_event(&event.kind) => {
            let had_paths = !event.paths.is_empty();
            let relevant: Vec<_> = event
                .paths
                .into_iter()
                .filter(|path| is_eq_log_path(path))
                .collect();
            if !relevant.is_empty() {
                wake.record_paths(relevant);
            } else if !had_paths {
                // Empty/ambiguous notifications (including backend overflow
                // signals) still wake the authoritative offset reconciliation.
                wake.request_reconciliation();
            }
        }
        Ok(_) => {}
        Err(error) => wake.record_watcher_error(error.to_string()),
    }
}

fn is_change_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Any
            | EventKind::Create(_)
            | EventKind::Modify(_)
            | EventKind::Remove(_)
            | EventKind::Other
    )
}

fn is_eq_log_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lowercase = name.to_ascii_lowercase();
            lowercase.starts_with("eqlog_") && lowercase.ends_with(".txt")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_only_eq_log_files() {
        assert!(is_eq_log_path(Path::new("C:/EQ/Logs/eqlog_Bilka_teek.txt")));
        assert!(is_eq_log_path(Path::new("EQLOG_BILKA_TEEK.TXT")));
        assert!(!is_eq_log_path(Path::new("eqlog_Bilka_teek.log")));
        assert!(!is_eq_log_path(Path::new("dbg.txt")));
    }
}
