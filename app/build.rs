use std::path::PathBuf;

fn main() {
    // Re-run build script when icon assets change.
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/tray.ico");

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
    if dll_src.exists() {
        std::fs::copy(&dll_src, &dll_dst).expect("failed to copy trusik dinput8.dll into OUT_DIR");
    } else {
        // Allow building the app alone (rust-analyzer, `cargo check`) without
        // the trusik DLL present. deploy() treats an empty payload as
        // "not embedded" and errors clearly at runtime.
        std::fs::write(&dll_dst, b"").expect("failed to write placeholder dinput8.dll");
        println!(
            "cargo:warning=trusik dinput8.dll not found at {}; embedding empty placeholder (build -p trusik first)",
            dll_src.display()
        );
    }
    println!("cargo:rerun-if-changed={}", dll_src.display());

    // Embed app icon as Windows resource (shows in taskbar, alt-tab, explorer)
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/app.ico");
    // Enable ComCtl32 v6 for modern themed controls in dialogs.
    res.set_manifest(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
</assembly>"#);
    res.compile().expect("Failed to compile Windows resources");
}
