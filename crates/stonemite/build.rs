use std::path::PathBuf;

fn main() {
    // Re-run build script when icon assets change.
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app-dev.ico");
    println!("cargo:rerun-if-changed=assets/tray.ico");
    println!("cargo:rerun-if-changed=assets/tray-dev.ico");
    println!("cargo:rerun-if-env-changed=STONEMITE_BUILD_LABEL");

    // Keep the executable/window and runtime tray icon on one profile-derived
    // build flavor. Cargo reports `debug` for the dev profile family and
    // `release` for the release profile family.
    let development_build = std::env::var("PROFILE").as_deref() == Ok("debug");
    println!("cargo:rustc-check-cfg=cfg(stonemite_dev_build)");
    if development_build {
        println!("cargo:rustc-cfg=stonemite_dev_build");
    }

    // Embed the trusik input-proxy DLL (dinput8.dll) into the exe so the app
    // and its DLL can never drift out of sync. The DLL is produced by the
    // `trusik` crate into target/<profile>/dinput8.dll and MUST be built
    // before this crate (the justfile builds -p trusik first). We copy it into
    // OUT_DIR, where trusik_deploy.rs picks it up via include_bytes!.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR = <target>/<profile>/build/<pkg>-<hash>/out
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR has an unexpected layout");
    let dll_src = profile_dir.join("dinput8.dll");
    let dll_dst = out_dir.join("dinput8.dll");
    let release_build = std::env::var("DEBUG").as_deref() == Ok("false");
    match std::fs::read(&dll_src) {
        Ok(dll) if dll.len() >= 2 && dll.starts_with(b"MZ") => {
            std::fs::write(&dll_dst, dll).expect("failed to copy trusik dinput8.dll into OUT_DIR");
        }
        Ok(_) => panic!(
            "trusik dinput8.dll at {} is empty or not a PE image; build -p trusik first",
            dll_src.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !release_build => {
            // Keep rust-analyzer and app-only debug checks usable. Runtime
            // deployment rejects this placeholder, while optimized/release
            // builds fail closed below.
            std::fs::write(&dll_dst, b"").expect("failed to write placeholder dinput8.dll");
            println!(
                "cargo:warning=trusik dinput8.dll not found at {}; embedding empty debug placeholder (build -p trusik first)",
                dll_src.display()
            );
        }
        Err(error) => panic!(
            "required trusik dinput8.dll is unavailable at {}: {error}; build -p trusik first",
            dll_src.display()
        ),
    }
    println!("cargo:rerun-if-changed={}", dll_src.display());

    // Let tauri-build generate the single Windows resource object so its ACL
    // metadata, icon, version information, and our existing DPI manifest cannot
    // conflict at link time.
    let app_icon = if development_build {
        "assets/app-dev.ico"
    } else {
        "assets/app.ico"
    };
    let windows = tauri_build::WindowsAttributes::new()
        .window_icon_path(app_icon)
        .app_manifest(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
    </windowsSettings>
  </application>
</assembly>"#,
        );
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attributes).expect("failed to prepare Tauri settings resources");
}
