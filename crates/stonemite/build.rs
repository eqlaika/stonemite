mod calver;

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repository_root = manifest_dir
        .parent()
        .and_then(|crates| crates.parent())
        .expect("Stonemite manifest is not under <repository>/crates/stonemite");
    let version_path = repository_root.join("VERSION");
    println!("cargo:rerun-if-changed={}", version_path.display());

    let public_version = std::fs::read_to_string(&version_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", version_path.display()));
    let public_version = public_version.trim();
    let parsed_version = calver::CalVer::parse(public_version)
        .unwrap_or_else(|error| panic!("invalid {}: {error}", version_path.display()));
    assert_eq!(
        parsed_version.to_string(),
        public_version,
        "{} must contain the canonical version without a tag prefix",
        version_path.display()
    );
    let internal_version = parsed_version.cargo_version();
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        internal_version,
        "crates/stonemite/Cargo.toml is out of sync with VERSION; run scripts/version.py set {public_version}"
    );

    let tauri_config_path = manifest_dir.join("tauri.conf.json");
    let tauri_config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&tauri_config_path).unwrap_or_else(
            |error| panic!("failed to read {}: {error}", tauri_config_path.display()),
        ))
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", tauri_config_path.display()));
    assert_eq!(
        tauri_config.get("version").and_then(serde_json::Value::as_str),
        Some(internal_version.as_str()),
        "crates/stonemite/tauri.conf.json is out of sync with VERSION; run scripts/version.py set {public_version}"
    );
    println!("cargo:rustc-env=STONEMITE_VERSION={public_version}");

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
