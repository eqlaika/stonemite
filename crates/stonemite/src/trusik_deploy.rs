use std::io::{Error, ErrorKind};
use std::path::Path;

/// The trusik input-proxy DLL, embedded at build time from the `trusik` crate's
/// output (see build.rs). Embedding — rather than shipping a loose dinput8.dll
/// next to the exe — keeps the DLL and the app in lockstep: a self-update
/// replaces stonemite.exe and the DLL travels inside it, so the two can never
/// drift out of sync and silently break auto-login.
static TRUSIK_DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dinput8.dll"));

/// Write the embedded dinput8.dll into the EQ directory.
pub fn deploy(eq_dir: &Path) -> std::io::Result<()> {
    if TRUSIK_DLL.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            "embedded trusik DLL is empty; build -p trusik before -p stonemite",
        ));
    }

    let dst = eq_dir.join("dinput8.dll");
    std::fs::write(&dst, TRUSIK_DLL)?;
    eprintln!("trusik: deployed embedded dinput8.dll -> {}", dst.display());
    Ok(())
}

/// Remove dinput8.dll from the EQ directory if it exists.
pub fn remove(eq_dir: &Path) -> std::io::Result<()> {
    let dll = eq_dir.join("dinput8.dll");
    if dll.exists() {
        std::fs::remove_file(&dll)?;
        eprintln!("trusik: removed {}", dll.display());
    }
    let log = eq_dir.join("stonemite_proxy.log");
    if log.exists() {
        let _ = std::fs::remove_file(&log);
    }
    Ok(())
}
