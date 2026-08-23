use std::io::Write;
use std::sync::OnceLock;

/// Append one timestamped entry to Stonemite's diagnostic log.
pub(crate) fn debug_log(message: &str) {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let elapsed = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let log_path = std::path::Path::new(&appdata)
            .join("Stonemite")
            .join("debug.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            let _ = writeln!(file, "[{elapsed:>8.3}s] {message}");
        }
    }
}
