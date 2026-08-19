use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static START_TIME: OnceLock<std::time::Instant> = OnceLock::new();

/// Initialize the log file path next to the DLL (i.e. in the EQ directory).
pub fn init() {
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("stonemite_proxy.log")))
        .unwrap_or_else(|| PathBuf::from("stonemite_proxy.log"));

    // Truncate any previous log on attach.
    let _ = std::fs::write(&path, "");
    let _ = LOG_PATH.set(path);
    let _ = START_TIME.set(std::time::Instant::now());
}

/// Append a timestamped line to the log file.
pub fn write(msg: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let pid = std::process::id();
    let elapsed = START_TIME.get().map_or(0.0, |t| t.elapsed().as_secs_f64());
    let _ = writeln!(file, "[{elapsed:>8.3}s pid:{pid}] {msg}");
}
