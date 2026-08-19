use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    WatcherStartup,
    WatcherError,
    FileRead,
    FileReset,
    Parser,
    Reconciliation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogDiagnostic {
    pub kind: DiagnosticKind,
    pub path: Option<PathBuf>,
    pub message: String,
}

impl LogDiagnostic {
    pub(crate) fn new(
        kind: DiagnosticKind,
        path: Option<PathBuf>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path,
            message: message.into(),
        }
    }
}

impl fmt::Display for LogDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                formatter,
                "{} ({}): {}",
                kind_name(self.kind),
                path.display(),
                self.message
            ),
            None => write!(formatter, "{}: {}", kind_name(self.kind), self.message),
        }
    }
}

fn kind_name(kind: DiagnosticKind) -> &'static str {
    match kind {
        DiagnosticKind::WatcherStartup => "log watcher startup",
        DiagnosticKind::WatcherError => "log watcher",
        DiagnosticKind::FileRead => "log read",
        DiagnosticKind::FileReset => "log reset",
        DiagnosticKind::Parser => "log parser",
        DiagnosticKind::Reconciliation => "log reconciliation",
    }
}
