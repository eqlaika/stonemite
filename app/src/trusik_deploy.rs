use std::path::Path;

/// Copy dinput8.dll from the stonemite install dir to the EQ directory.
pub fn deploy(eq_dir: &Path) -> std::io::Result<()> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no exe parent dir"))?
        .to_path_buf();

    let src = exe_dir.join("dinput8.dll");
    if !src.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("trusik DLL not found at {}", src.display()),
        ));
    }

    let dst = eq_dir.join("dinput8.dll");
    std::fs::copy(&src, &dst)?;
    eprintln!("trusik: deployed {} -> {}", src.display(), dst.display());
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
