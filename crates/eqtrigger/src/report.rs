//! Import/export compatibility reports.
//!
//! Every codec (EQLP, GINA, native package) produces a [`CompatReport`]
//! describing what could not be represented faithfully — quarantined
//! regexes, dangling overlay references, missing media, retained-but-
//! unexecuted fields — instead of silently dropping anything.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompatSeverity {
    /// Informational: imported cleanly with a note.
    Info,
    /// Imported but degraded (retained field, approximation).
    Warning,
    /// The item was quarantined or skipped.
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatIssue {
    pub severity: CompatSeverity,
    /// Which trigger/overlay/folder the issue concerns (path-like name).
    pub subject: String,
    /// Stable machine-readable class (e.g. `unsupported-regex`,
    /// `dangling-overlay`, `missing-media`, `retained-field`).
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompatReport {
    pub issues: Vec<CompatIssue>,
    pub triggers_imported: usize,
    pub folders_imported: usize,
    pub overlays_imported: usize,
    pub triggers_quarantined: usize,
}

impl CompatReport {
    pub fn push(
        &mut self,
        severity: CompatSeverity,
        subject: impl Into<String>,
        code: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.issues.push(CompatIssue {
            severity,
            subject: subject.into(),
            code: code.into(),
            detail: detail.into(),
        });
    }

    pub fn merge(&mut self, other: CompatReport) {
        self.issues.extend(other.issues);
        self.triggers_imported += other.triggers_imported;
        self.folders_imported += other.folders_imported;
        self.overlays_imported += other.overlays_imported;
        self.triggers_quarantined += other.triggers_quarantined;
    }

    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == CompatSeverity::Error)
    }
}
