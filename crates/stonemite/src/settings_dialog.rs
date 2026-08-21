use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{LogicalPosition, Manager, Position, RunEvent};
use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SendMessageTimeoutW, SetForegroundWindow, SMTO_ABORTIFHUNG, WM_USER,
};

use crate::config::Config;
use crate::settings_model::{SaveOutcome, SettingsDraft, SettingsPayload};

/// Custom message posted to the tray window after settings are saved.
pub const WM_SETTINGS_CHANGED: u32 = WM_USER + 100;
/// Custom message asking the tray application to shut down and relaunch.
pub const WM_RESTART_REQUESTED: u32 = WM_USER + 101;
/// Synchronous request to open a five-minute pairing window for a six-digit code.
pub const WM_BEGIN_PAIRING: u32 = WM_USER + 102;
/// Request to close any active pairing window.
pub const WM_CANCEL_PAIRING: u32 = WM_USER + 103;
/// Synchronous query for whether the current pairing window is still open.
pub const WM_PAIRING_STATUS: u32 = WM_USER + 104;

static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);

/// Show the Tauri settings window in a same-executable subprocess. If it is
/// already open, bring the existing window to the foreground.
pub fn show() {
    if SETTINGS_OPEN.load(Ordering::SeqCst) {
        unsafe {
            if let Ok(window) = FindWindowW(None, w!("Stonemite Settings")) {
                let _ = SetForegroundWindow(window);
            }
        }
        return;
    }

    let executable = std::env::current_exe().expect("failed to locate Stonemite executable");
    match std::process::Command::new(executable)
        .arg("--settings")
        .spawn()
    {
        Ok(mut child) => {
            SETTINGS_OPEN.store(true, Ordering::SeqCst);
            std::thread::spawn(move || {
                let _ = child.wait();
                SETTINGS_OPEN.store(false, Ordering::SeqCst);
            });
        }
        Err(error) => eprintln!("Failed to open settings: {error}"),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingSession {
    code: String,
    address: String,
    expires_in_seconds: u64,
}

pub fn run_standalone() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let config = Config::load();
            if let (Some(position), Some(window)) =
                (config.settings_position, app.get_webview_window("main"))
            {
                window.set_position(Position::Logical(LogicalPosition::new(
                    position[0] as f64,
                    position[1] as f64,
                )))?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            choose_eq_directory,
            preview_notification_sound,
            begin_pairing,
            pairing_is_open,
            cancel_pairing,
            request_restart,
            open_external,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(error) => {
            let download = rfd::MessageDialog::new()
                .set_title("Stonemite Settings could not open")
                .set_description(format!(
                    "Stonemite could not start its settings window:\n\n{error}\n\nMicrosoft Edge WebView2 may be missing. Download it now?"
                ))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if download == rfd::MessageDialogResult::Yes {
                let _ = webbrowser::open("https://go.microsoft.com/fwlink/p/?LinkId=2124703");
            }
            return;
        }
    };

    app.run(|app_handle, event| {
        if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
            save_window_position(app_handle);
            cancel_pairing_window();
        }
    });
}

#[tauri::command]
fn load_settings() -> Result<SettingsPayload, String> {
    SettingsPayload::load()
}

#[tauri::command]
fn save_settings(draft: SettingsDraft) -> Result<SaveOutcome, String> {
    let outcome = draft.save()?;
    notify_tray();
    Ok(outcome)
}

#[tauri::command(rename_all = "camelCase")]
fn choose_eq_directory(current_directory: String) -> Option<String> {
    rfd::FileDialog::new()
        .set_directory(current_directory)
        .pick_folder()
        .map(|path| path.display().to_string())
}

#[tauri::command]
fn preview_notification_sound(sound: String) -> Result<(), String> {
    crate::sound::play(&sound)
        .then_some(())
        .ok_or_else(|| "Stonemite could not play that notification sound".to_owned())
}

#[tauri::command]
fn begin_pairing() -> Result<PairingSession, String> {
    let code = generate_pairing_code();
    if !request_pairing_window(code) {
        return Err(
            "Save and restart Stonemite with local-network access before pairing.".to_owned(),
        );
    }
    let config = Config::load();
    Ok(PairingSession {
        code: format_pairing_code(code),
        address: crate::settings_model::integration_address(&config.trushar.bind),
        expires_in_seconds: trushar::server::PAIRING_CODE_TTL.as_secs(),
    })
}

#[tauri::command]
fn pairing_is_open() -> bool {
    send_tray_request(WM_PAIRING_STATUS, WPARAM(0)) == Some(1)
}

#[tauri::command]
fn cancel_pairing() {
    cancel_pairing_window();
}

#[tauri::command]
fn request_restart() {
    request_tray_restart();
}

#[tauri::command]
fn open_external(target: String) -> Result<(), String> {
    const PROJECT_URL: &str = "https://github.com/eqlaika/stonemite";
    const EMAIL_URL: &str = "mailto:laika@laikasoft.co";
    if target != PROJECT_URL && target != EMAIL_URL {
        return Err("That external destination is not allowed".to_owned());
    }
    webbrowser::open(&target)
        .map(|_| ())
        .map_err(|error| format!("Windows could not open the link: {error}"))
}

fn save_window_position(app_handle: &tauri::AppHandle) {
    let Some(window) = app_handle.get_webview_window("main") else {
        return;
    };
    let (Ok(position), Ok(scale_factor)) = (window.outer_position(), window.scale_factor()) else {
        return;
    };
    let mut config = Config::load();
    config.settings_position = Some([
        position.x as f32 / scale_factor as f32,
        position.y as f32 / scale_factor as f32,
    ]);
    let _ = config.save();
}

fn generate_pairing_code() -> u32 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u32::from_le_bytes(bytes[..4].try_into().expect("UUID has four bytes")) % 1_000_000
}

fn format_pairing_code(code: u32) -> String {
    format!("{:03} {:03}", code / 1_000, code % 1_000)
}

fn send_tray_request(message: u32, wparam: WPARAM) -> Option<usize> {
    unsafe {
        let tray = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")).ok()?;
        let mut result = 0usize;
        let sent = SendMessageTimeoutW(
            tray,
            message,
            wparam,
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            Duration::from_secs(1).as_millis() as u32,
            Some(&mut result),
        );
        (sent.0 != 0).then_some(result)
    }
}

fn request_pairing_window(code: u32) -> bool {
    send_tray_request(WM_BEGIN_PAIRING, WPARAM(code as usize)) == Some(1)
}

fn cancel_pairing_window() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) {
            let _ = PostMessageW(tray, WM_CANCEL_PAIRING, WPARAM(0), LPARAM(0));
        }
    }
}

fn notify_tray() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) {
            let _ = PostMessageW(tray, WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
}

fn request_tray_restart() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) {
            let _ = PostMessageW(tray, WM_RESTART_REQUESTED, WPARAM(0), LPARAM(0));
            return;
        }
    }

    if let Ok(executable) = std::env::current_exe() {
        let _ = std::process::Command::new(executable).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_codes_keep_all_leading_zeroes() {
        assert_eq!(format_pairing_code(4_271), "004 271");
        assert_eq!(format_pairing_code(999_999), "999 999");
    }
}
