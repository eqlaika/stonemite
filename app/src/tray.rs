use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreateMenu, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyWindow, GetCursorPos, GetMessageW, KillTimer,
    MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    LR_DEFAULTCOLOR, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetTimer, TrackPopupMenu, CS_HREDRAW, CS_VREDRAW,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_HOTKEY, WM_TIMER,
    WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW,
};

use crate::broadcast;
use crate::config;
use crate::overlay;
use crate::settings_dialog;
use crate::updater;

/// Load an icon of the given size from an in-memory ICO file.
/// Returns None if parsing fails or the size isn't found.
unsafe fn load_icon_from_ico(
    ico_data: &[u8],
    desired_size: u32,
) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    // ICO header: 2 reserved + 2 type + 2 count = 6 bytes
    if ico_data.len() < 6 {
        return None;
    }
    let count = u16::from_le_bytes([ico_data[4], ico_data[5]]) as usize;

    // Find the entry matching desired_size
    for i in 0..count {
        let offset = 6 + i * 16;
        if offset + 16 > ico_data.len() {
            return None;
        }
        let w = ico_data[offset] as u32;
        let w = if w == 0 { 256 } else { w };
        let h = ico_data[offset + 1] as u32;
        let h = if h == 0 { 256 } else { h };

        if w == desired_size && h == desired_size {
            let data_size =
                u32::from_le_bytes([ico_data[offset + 8], ico_data[offset + 9], ico_data[offset + 10], ico_data[offset + 11]]) as usize;
            let data_offset =
                u32::from_le_bytes([ico_data[offset + 12], ico_data[offset + 13], ico_data[offset + 14], ico_data[offset + 15]]) as usize;

            if data_offset + data_size > ico_data.len() {
                return None;
            }

            let icon = CreateIconFromResourceEx(
                &ico_data[data_offset..data_offset + data_size],
                true,
                0x00030000, // version
                desired_size as i32,
                desired_size as i32,
                LR_DEFAULTCOLOR,
            );
            return icon.ok();
        }
    }
    None
}

const WM_TRAY: u32 = WM_USER + 1;
const ID_LAUNCH_EQ: u16 = 1000;
const ID_EXIT: u16 = 1001;
const ID_SETTINGS: u16 = 1002;
const ID_SHOW_OVERLAY: u16 = 1003;
const ID_CHECK_UPDATE: u16 = 1004;
const ID_EDIT_MODE: u16 = 1005;
const ID_BROADCAST_TOGGLE: u16 = 1006;
const ID_CONFIGURE_ACCOUNTS: u16 = 1007;
const ID_LOGIN_ALL: u16 = 1008;
/// Account login IDs start here: 2000, 2001, 2002, ...
const ID_LOGIN_ACCOUNT_BASE: u16 = 2000;

/// Hotkey ID for hide-overlay toggle.
const HOTKEY_HIDE_OVERLAY: i32 = 1;
/// Hotkey ID for broadcast toggle.
const HOTKEY_BROADCAST_TOGGLE: i32 = 2;
/// Hotkey IDs for swap-to-window (slots 1–6). IDs 10–15.
const HOTKEY_SWAP_BASE: i32 = 10;
const MAX_SWAP_HOTKEYS: usize = 6;

/// Timer ID for polling EQ windows.
const TIMER_POLL_EQ: usize = 1;
/// Poll interval in milliseconds (2 seconds).
const POLL_INTERVAL_MS: u32 = 2000;

/// Custom message posted when a background update check finds a new version.
const WM_UPDATE_AVAILABLE: u32 = WM_USER + 2;

/// Run the tray icon and message loop. Blocks until exit.
pub fn run() {
    unsafe { run_inner() }
}

unsafe fn run_inner() {
    // Register window class for our hidden message window.
    let class_name = w!("StonemiteTrayClass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        lpszClassName: class_name.into(),
        style: CS_HREDRAW | CS_VREDRAW,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Create hidden message window.
    let hwnd = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        class_name,
        w!("Stonemite"),
        Default::default(),
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create message window");

    // Load tray icon from embedded ICO data.
    let icon = load_icon_from_ico(include_bytes!("../assets/tray.ico"), 16)
        .or_else(|| load_icon_from_ico(include_bytes!("../assets/tray.ico"), 32));

    // Add tray icon.
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        ..Default::default()
    };
    if let Some(icon) = icon {
        nid.hIcon = icon;
    }
    // Tooltip
    let tip = "Stonemite";
    for (i, ch) in tip.encode_utf16().enumerate() {
        if i >= nid.szTip.len() - 1 {
            break;
        }
        nid.szTip[i] = ch;
    }
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);

    // Message loop.
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
        let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
    }

    // Cleanup tray icon.
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    if let Some(icon) = icon {
        let _ = DestroyIcon(icon);
    }
    let _ = DestroyWindow(hwnd);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            // Start polling timer for EQ window detection.
            let _ = SetTimer(hwnd, TIMER_POLL_EQ, POLL_INTERVAL_MS, None);
            // Check for updates in the background if due.
            maybe_auto_update_check(hwnd);
            // Register global hotkey for hiding overlay.
            let cfg = config::Config::load();
            register_hotkeys(hwnd, &cfg);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_POLL_EQ {
                overlay::poll();
            }
            LRESULT(0)
        }
        WM_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            // WM_LBUTTONUP = 0x0202, WM_RBUTTONUP = 0x0205
            if event == 0x0202 || event == 0x0205 {
                show_context_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            if id == HOTKEY_HIDE_OVERLAY && overlay::is_eq_active() {
                overlay::toggle_hidden();
            } else if id == HOTKEY_BROADCAST_TOGGLE {
                broadcast::toggle();
                overlay::refresh_broadcast_label();
                overlay::show_toast(if broadcast::is_active() {
                    "Key broadcasting enabled"
                } else {
                    "Key broadcasting disabled"
                });
            } else if id >= HOTKEY_SWAP_BASE && id < HOTKEY_SWAP_BASE + MAX_SWAP_HOTKEYS as i32 {
                let slot = (id - HOTKEY_SWAP_BASE) as usize + 1; // 1-based window number
                overlay::swap_to_number(slot);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            match id {
                ID_SHOW_OVERLAY => overlay::toggle_hidden(),
                ID_EDIT_MODE => overlay::toggle_edit_mode(),
                ID_BROADCAST_TOGGLE => {
                    broadcast::toggle();
                    overlay::refresh_broadcast_label();
                    overlay::show_toast(if broadcast::is_active() {
                        "Key broadcasting enabled"
                    } else {
                        "Key broadcasting disabled"
                    });
                }
                ID_LAUNCH_EQ => launch_eq(None, None),
                ID_LOGIN_ALL => {
                    let cfg = config::Config::load();
                    let accounts: Vec<(String, Option<String>)> = cfg.accounts.iter().map(|a| {
                        let pw = crate::crypt::decrypt(&a.password).ok();
                        (a.username.clone(), pw)
                    }).collect();
                    std::thread::spawn(move || {
                        for (username, password) in &accounts {
                            launch_eq(Some(username), password.as_deref());
                        }
                    });
                }
                ID_CONFIGURE_ACCOUNTS => {
                    settings_dialog::show();
                }
                ID_SETTINGS => settings_dialog::show(),
                ID_CHECK_UPDATE => do_update_check(hwnd),
                ID_EXIT => PostQuitMessage(0),
                _ if id >= ID_LOGIN_ACCOUNT_BASE => {
                    let index = (id - ID_LOGIN_ACCOUNT_BASE) as usize;
                    let cfg = config::Config::load();
                    if let Some(account) = cfg.accounts.get(index) {
                        let pw = crate::crypt::decrypt(&account.password).ok();
                        launch_eq(Some(&account.username), pw.as_deref());
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        x if x == WM_UPDATE_AVAILABLE => {
            overlay::show_toast(&format!(
                "Stonemite v{} available — check for updates to install",
                update_version_from_wparam(wparam)
            ));
            LRESULT(0)
        }
        x if x == settings_dialog::WM_SETTINGS_CHANGED => {
            // Re-register hotkeys with new config.
            unregister_hotkeys(hwnd);
            let cfg = config::Config::load();
            register_hotkeys(hwnd, &cfg);
            broadcast::on_settings_changed();
            // Reload overlay config (pip_edge, etc.) and rebuild layout.
            overlay::force_rebuild();
            LRESULT(0)
        }
        WM_DESTROY => {
            unregister_hotkeys(hwnd);
            let _ = KillTimer(hwnd, TIMER_POLL_EQ);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_context_menu(hwnd: HWND) {
    let cfg = config::Config::load();
    let menu = CreatePopupMenu().expect("Failed to create popup menu");

    let overlay_label = format!("Show overlay\t{}\0", cfg.hide_hotkey);
    let overlay_wide: Vec<u16> = overlay_label.encode_utf16().collect();
    let check_flag = if overlay::is_visible() { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING | check_flag, ID_SHOW_OVERLAY as usize,
        windows::core::PCWSTR(overlay_wide.as_ptr()));

    let edit_label = if overlay::is_edit_mode() { "Lock layout\0" } else { "Edit layout\0" };
    let edit_wide: Vec<u16> = edit_label.encode_utf16().collect();
    let _ = AppendMenuW(menu, MF_STRING, ID_EDIT_MODE as usize,
        windows::core::PCWSTR(edit_wide.as_ptr()));

    // Broadcasting toggle (only shown if trusik is enabled).
    if cfg.trusik {
        let bc_label = if broadcast::is_active() {
            format!("Broadcasting: on\t{}\0", cfg.broadcast_hotkey)
        } else {
            format!("Broadcasting: off\t{}\0", cfg.broadcast_hotkey)
        };
        let bc_wide: Vec<u16> = bc_label.encode_utf16().collect();
        let bc_flag = if broadcast::is_active() { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | bc_flag, ID_BROADCAST_TOGGLE as usize,
            windows::core::PCWSTR(bc_wide.as_ptr()));
    }

    if cfg.accounts.is_empty() {
        let _ = AppendMenuW(menu, MF_STRING, ID_LAUNCH_EQ as usize, w!("Launch EQ"));
    } else {
        let login_menu = CreateMenu().expect("Failed to create login submenu");
        let _ = AppendMenuW(login_menu, MF_STRING, ID_LOGIN_ALL as usize, w!("Login all accounts"));
        let _ = AppendMenuW(login_menu, MF_SEPARATOR, 0, None);
        for (i, account) in cfg.accounts.iter().enumerate() {
            let label = format!("{}\0", account.username);
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(
                login_menu,
                MF_STRING,
                (ID_LOGIN_ACCOUNT_BASE + i as u16) as usize,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
        let _ = AppendMenuW(login_menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(
            login_menu,
            MF_STRING,
            ID_CONFIGURE_ACCOUNTS as usize,
            w!("Configure accounts..."),
        );
        let _ = AppendMenuW(menu, MF_STRING | MF_POPUP, login_menu.0 as usize, w!("Login"));
    }
    let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("Settings..."));
    let update_label = format!("Check for updates\tv{}\0", updater::current_version());
    let update_wide: Vec<u16> = update_label.encode_utf16().collect();
    let _ = AppendMenuW(menu, MF_STRING, ID_CHECK_UPDATE as usize,
        windows::core::PCWSTR(update_wide.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, ID_EXIT as usize, w!("Exit"));

    let mut pt = Default::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
}

unsafe fn do_update_check(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK, MB_ICONINFORMATION, MB_ICONERROR};

    match updater::check_and_update() {
        updater::UpdateResult::UpToDate => {
            MessageBoxW(
                hwnd,
                w!("You are running the latest version."),
                w!("Stonemite Update"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        updater::UpdateResult::Updated { version: ver, notes } => {
            let msg = if notes.is_empty() {
                format!("Updated to v{}! Stonemite will now restart.\0", ver)
            } else {
                format!("Updated to v{}! Stonemite will now restart.\n\n{}\0", ver, notes)
            };
            let msg_wide: Vec<u16> = msg.encode_utf16().collect();
            MessageBoxW(
                hwnd,
                windows::core::PCWSTR(msg_wide.as_ptr()),
                w!("Stonemite Update"),
                MB_OK | MB_ICONINFORMATION,
            );
            updater::restart();
        }
        updater::UpdateResult::Error(e) => {
            let msg = format!("Update check failed:\n{}\0", e);
            let msg_wide: Vec<u16> = msg.encode_utf16().collect();
            MessageBoxW(
                hwnd,
                windows::core::PCWSTR(msg_wide.as_ptr()),
                w!("Stonemite Update"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

/// Check if an automatic update check is due and spawn a background thread if so.
fn maybe_auto_update_check(hwnd: HWND) {
    let mut cfg = config::Config::load();
    if !cfg.auto_update_check {
        return;
    }

    // Check if enough days have elapsed since last check.
    if let Some(ref last) = cfg.last_update_check {
        if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(last) {
            let elapsed = chrono::Utc::now().signed_duration_since(last_time);
            if elapsed.num_days() < cfg.update_check_interval_days as i64 {
                return;
            }
        }
    }

    // Record that we're checking now.
    cfg.last_update_check = Some(chrono::Utc::now().to_rfc3339());
    let _ = cfg.save();

    // Spawn background check.
    let hwnd_raw = hwnd.0 as usize;
    std::thread::spawn(move || {
        if let updater::CheckResult::Available { version } = updater::check_for_update() {
            let ptr = Box::into_raw(Box::new(version));
            let hwnd = HWND(hwnd_raw as *mut _);
            unsafe {
                let _ = PostMessageW(hwnd, WM_UPDATE_AVAILABLE, WPARAM(ptr as usize), LPARAM(0));
            }
        }
    });
}

/// Extract the version string from WPARAM (pointer to a heap-allocated String).
fn update_version_from_wparam(wparam: WPARAM) -> String {
    unsafe {
        let ptr = wparam.0 as *mut String;
        if ptr.is_null() {
            return String::new();
        }
        *Box::from_raw(ptr)
    }
}

unsafe fn register_hotkeys(hwnd: HWND, cfg: &config::Config) {
    if let Some((mods, vk)) = cfg.hide_hotkey_vk() {
        if RegisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY, HOT_KEY_MODIFIERS(mods), vk).is_err() {
            eprintln!("Failed to register hide overlay hotkey: {}", cfg.hide_hotkey);
        }
    }
    if cfg.trusik {
        if let Some((mods, vk)) = cfg.broadcast_hotkey_vk() {
            if RegisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE, HOT_KEY_MODIFIERS(mods), vk).is_err() {
                eprintln!("Failed to register broadcast hotkey: {}", cfg.broadcast_hotkey);
            }
        }
    }
    for i in 0..MAX_SWAP_HOTKEYS {
        if let Some((mods, vk)) = cfg.swap_hotkey_vk(i) {
            if RegisterHotKey(hwnd, HOTKEY_SWAP_BASE + i as i32, HOT_KEY_MODIFIERS(mods), vk).is_err() {
                eprintln!("Failed to register swap hotkey {}: {}", i + 1,
                    cfg.swap_hotkeys.get(i).map(|s| s.as_str()).unwrap_or("?"));
            }
        }
    }
}

unsafe fn unregister_hotkeys(hwnd: HWND) {
    let _ = UnregisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY);
    let _ = UnregisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE);
    for i in 0..MAX_SWAP_HOTKEYS {
        let _ = UnregisterHotKey(hwnd, HOTKEY_SWAP_BASE + i as i32);
    }
}

fn launch_eq(username: Option<&str>, password: Option<&str>) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let exe = eq_dir.join("eqgame.exe");
    if !exe.exists() {
        eprintln!("eqgame.exe not found in {}", eq_dir.display());
        return;
    }
    overlay::debug_log(&format!(
        "launch_eq: user={:?} has_password={}",
        username,
        password.is_some()
    ));
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("patchme").current_dir(&eq_dir);
    if let Some(user) = username {
        cmd.arg(format!("/login:{user}"));
    }
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            overlay::debug_log(&format!("launch_eq: spawned pid={pid}"));
            if let Some(pw) = password {
                crate::auto_type::spawn(pid, pw.to_string());
            }
        }
        Err(e) => {
            overlay::debug_log(&format!("launch_eq: spawn failed: {e}"));
        }
    }
}
