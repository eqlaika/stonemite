use crate::config::Config;

const TELEMETRY_URL: &str = "https://luclin.laikasoft.co/";

/// Send an anonymous app_start event in a background thread.
/// Does nothing if telemetry is disabled or there is no telemetry ID.
pub fn send_app_start(config: &Config) {
    if !config.telemetry {
        return;
    }
    let Some(id) = config.telemetry_id.clone() else {
        return;
    };
    let version = if cfg!(debug_assertions) {
        format!("{}-dev", env!("GIT_SHA"))
    } else {
        crate::updater::current_version().to_string()
    };
    let os_version = os_version_string();

    std::thread::spawn(move || {
        let payload = serde_json::json!({
            "id": id,
            "app": "stonemite",
            "event": "app_start",
            "app_version": version,
            "os_version": os_version,
        });
        let _ = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .and_then(|c| c.post(TELEMETRY_URL).json(&payload).send());
    });
}

fn os_version_string() -> String {
    let ver = windows::Win32::System::SystemInformation::GetVersionExW;
    unsafe {
        let mut info: windows::Win32::System::SystemInformation::OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of_val(&info) as u32;
        #[allow(deprecated)]
        let _ = ver(&mut info);
        format!(
            "{}.{}.{}",
            info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
        )
    }
}
