use std::io::{Error, ErrorKind};
use std::path::Path;
use windows::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

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
    deploy_bytes(&dst, TRUSIK_DLL)
}

/// Return whether Windows refused replacement because a running client has the proxy mapped.
pub(crate) fn is_in_use_error(error: &Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code)
            if code == ERROR_SHARING_VIOLATION.0 as i32
                || code == ERROR_LOCK_VIOLATION.0 as i32
    )
}

fn deploy_bytes(dst: &Path, dll: &[u8]) -> std::io::Result<()> {
    // Loaded DLLs are locked against replacement on Windows. Avoid touching an
    // already-current file so launching additional clients remains possible,
    // while still retrying a previously blocked upgrade before the next launch.
    if std::fs::read(dst).is_ok_and(|current| current == dll) {
        return Ok(());
    }
    std::fs::write(dst, dll)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // This binary is Windows-only.
    fn identical_locked_proxy_does_not_need_replacement() {
        let directory = std::env::temp_dir().join(format!(
            "stonemite-trusik-deploy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("dinput8.dll");
        std::fs::write(&path, b"current proxy").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();

        assert!(deploy_bytes(&path, b"current proxy").is_ok());
        assert!(deploy_bytes(&path, b"different proxy").is_err());

        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn distinguishes_in_use_proxy_errors_from_access_denied() {
        assert!(is_in_use_error(&Error::from_raw_os_error(
            ERROR_SHARING_VIOLATION.0 as i32
        )));
        assert!(is_in_use_error(&Error::from_raw_os_error(
            ERROR_LOCK_VIOLATION.0 as i32
        )));
        assert!(!is_in_use_error(&Error::from_raw_os_error(5)));
    }
}
