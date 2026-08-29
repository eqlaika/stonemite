use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{LogicalPosition, Manager, Position, RunEvent};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, PostMessageW, SendMessageTimeoutW, SetForegroundWindow,
    SMTO_ABORTIFHUNG, WM_USER,
};

use crate::config::Config;
use crate::settings_model::{RunningCharacter, SaveOutcome, SettingsDraft, SettingsPayload};

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

static SETTINGS_PROCESS_ID: AtomicU32 = AtomicU32::new(0);

fn settings_identity_matches(
    foreground: HWND,
    titled_window: Option<HWND>,
    foreground_pid: u32,
    tracked_pid: u32,
) -> bool {
    !foreground.is_invalid()
        && titled_window == Some(foreground)
        && foreground_pid != 0
        && foreground_pid == tracked_pid
}

/// Return whether `hwnd` is the exact titled top-level window owned by the
/// settings child process spawned by this tray instance.
pub unsafe fn foreground_window_is_settings(hwnd: HWND) -> bool {
    let titled_window = FindWindowW(None, w!("Stonemite Settings")).ok();
    let mut foreground_pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut foreground_pid));
    settings_identity_matches(
        hwnd,
        titled_window,
        foreground_pid,
        SETTINGS_PROCESS_ID.load(Ordering::SeqCst),
    )
}

/// Show the Tauri settings window in a same-executable subprocess. If it is
/// already open, bring the existing window to the foreground.
pub fn show() -> bool {
    if SETTINGS_PROCESS_ID.load(Ordering::SeqCst) != 0 {
        unsafe {
            if let Ok(window) = FindWindowW(None, w!("Stonemite Settings")) {
                let _ = SetForegroundWindow(window);
            }
        }
        return true;
    }

    let Ok(executable) = std::env::current_exe() else {
        eprintln!("Failed to locate the Stonemite executable");
        return false;
    };
    match std::process::Command::new(executable)
        .arg("--settings")
        .spawn()
    {
        Ok(mut child) => {
            let child_pid = child.id();
            SETTINGS_PROCESS_ID.store(child_pid, Ordering::SeqCst);
            std::thread::spawn(move || {
                let _ = child.wait();
                let _ = SETTINGS_PROCESS_ID.compare_exchange(
                    child_pid,
                    0,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                );
            });
            true
        }
        Err(error) => {
            eprintln!("Failed to open settings: {error}");
            false
        }
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
            load_running_characters,
            save_settings,
            reset_dps_overlay_placement,
            choose_eq_directory,
            preview_notification_sound,
            begin_pairing,
            pairing_is_open,
            cancel_pairing,
            request_restart,
            open_external,
            crate::trigger_manager::load_trigger_library,
            crate::trigger_manager::save_trigger_library,
            crate::trigger_manager::choose_trigger_import_file,
            crate::trigger_manager::preview_trigger_import,
            crate::trigger_manager::commit_trigger_import,
            crate::trigger_manager::export_trigger_selection,
            crate::trigger_manager::add_trigger_media,
            crate::trigger_manager::preview_trigger_sound,
            crate::trigger_manager::run_trigger_test,
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
async fn load_running_characters() -> Vec<RunningCharacter> {
    running_characters_from_control()
        .await
        .unwrap_or_else(discover_running_characters)
}

async fn running_characters_from_control() -> Option<Vec<RunningCharacter>> {
    let config = Config::load();
    if !config.trushar.enabled {
        return None;
    }
    let port = config
        .trushar
        .bind
        .parse::<std::net::SocketAddr>()
        .ok()?
        .port();
    let mut request = format!("ws://127.0.0.1:{port}/trushar/v1")
        .into_client_request()
        .ok()?;
    if let Some(token) = config
        .trushar
        .auth_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).ok()?,
        );
    }

    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(1),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .ok()?
    .ok()?;
    let next = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .ok()?;
    let frame = next?.ok()?;
    let Message::Text(text) = frame else {
        return None;
    };
    let trushar::protocol::ServerMessage::State { state, .. } =
        serde_json::from_str(text.as_str()).ok()?
    else {
        return None;
    };
    Some(running_characters_from_clients(state.clients))
}

fn running_characters_from_clients(
    clients: Vec<trushar::protocol::WireClient>,
) -> Vec<RunningCharacter> {
    let mut characters: Vec<_> = clients
        .into_iter()
        .filter_map(|client| {
            Some(RunningCharacter {
                server: client.server?,
                character: client.character?,
                window_number: Some(client.window_number),
            })
        })
        .collect();
    sort_and_deduplicate_running(&mut characters);
    characters
}

fn discover_running_characters() -> Vec<RunningCharacter> {
    let mut characters: Vec<_> = crate::eq_windows::find_eq_windows()
        .into_iter()
        .filter_map(|window| {
            let (character, server) = crate::trusik_shm::read_character(window.pid)?;
            Some(RunningCharacter {
                server,
                character,
                window_number: None,
            })
        })
        .collect();
    sort_and_deduplicate_running(&mut characters);
    characters
}

fn sort_and_deduplicate_running(characters: &mut Vec<RunningCharacter>) {
    characters.sort_by(|left, right| {
        left.window_number
            .unwrap_or(usize::MAX)
            .cmp(&right.window_number.unwrap_or(usize::MAX))
            .then_with(|| {
                left.character
                    .to_ascii_lowercase()
                    .cmp(&right.character.to_ascii_lowercase())
            })
            .then_with(|| {
                left.server
                    .to_ascii_lowercase()
                    .cmp(&right.server.to_ascii_lowercase())
            })
    });
    let mut seen = HashSet::with_capacity(characters.len());
    characters.retain(|identity| {
        seen.insert((
            identity.server.to_ascii_lowercase(),
            identity.character.to_ascii_lowercase(),
        ))
    });
}

#[tauri::command]
fn save_settings(draft: SettingsDraft) -> Result<SaveOutcome, String> {
    let outcome = draft.save()?;
    notify_tray();
    Ok(outcome)
}

#[tauri::command]
fn reset_dps_overlay_placement() -> Result<(), String> {
    Config::update(|config| config.dps_overlay_placement = None)
        .map_err(|error| format!("Stonemite could not reset DPS overlay placement: {error}"))?;
    notify_tray();
    Ok(())
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
    let position = [
        position.x as f32 / scale_factor as f32,
        position.y as f32 / scale_factor as f32,
    ];
    let _ = Config::update(|config| {
        config.settings_position = Some(position);
    });
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

pub(crate) fn notify_tray() {
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
    fn settings_identity_requires_exact_window_and_tracked_process() {
        let foreground = HWND(41usize as *mut _);
        let other = HWND(42usize as *mut _);
        assert!(settings_identity_matches(
            foreground,
            Some(foreground),
            9001,
            9001,
        ));
        assert!(!settings_identity_matches(
            foreground,
            Some(other),
            9001,
            9001,
        ));
        assert!(!settings_identity_matches(
            foreground,
            Some(foreground),
            9001,
            9002,
        ));
        assert!(!settings_identity_matches(
            foreground,
            Some(foreground),
            9001,
            0,
        ));
        assert!(!settings_identity_matches(
            HWND::default(),
            Some(HWND::default()),
            9001,
            9001,
        ));
    }

    #[test]
    fn pairing_codes_keep_all_leading_zeroes() {
        assert_eq!(format_pairing_code(4_271), "004 271");
        assert_eq!(format_pairing_code(999_999), "999 999");
    }

    #[test]
    fn running_characters_use_window_order_and_require_complete_identities() {
        let client = |window_number, character: Option<&str>, server: Option<&str>| {
            trushar::protocol::WireClient {
                id: format!("client-{window_number}"),
                character: character.map(str::to_owned),
                server: server.map(str::to_owned),
                class_code: None,
                window_number,
                active: false,
                activatable: true,
                input_ready: false,
            }
        };
        let characters = running_characters_from_clients(vec![
            client(3, Some("Kafka"), Some("Xegony")),
            client(1, Some("Laika"), Some("Xegony")),
            client(2, None, Some("Xegony")),
        ]);

        assert_eq!(
            characters,
            vec![
                RunningCharacter {
                    server: "Xegony".into(),
                    character: "Laika".into(),
                    window_number: Some(1),
                },
                RunningCharacter {
                    server: "Xegony".into(),
                    character: "Kafka".into(),
                    window_number: Some(3),
                },
            ]
        );
    }
}
