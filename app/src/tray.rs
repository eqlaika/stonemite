use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyWindow, GetCursorPos, GetMessageW, KillTimer, MF_CHECKED, MF_STRING, MF_UNCHECKED,
    LR_DEFAULTCOLOR, MSG, PostQuitMessage, RegisterClassW,
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

/// Hotkey ID for hide-overlay toggle.
const HOTKEY_HIDE_OVERLAY: i32 = 1;
/// Hotkey ID for broadcast toggle.
const HOTKEY_BROADCAST_TOGGLE: i32 = 2;

/// Timer ID for polling EQ windows.
const TIMER_POLL_EQ: usize = 1;
/// Poll interval in milliseconds (2 seconds).
const POLL_INTERVAL_MS: u32 = 2000;

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
            // Register global hotkey for hiding overlay.
            let cfg = config::Config::load();
            if let Some((mods, vk)) = cfg.hide_hotkey_vk() {
                let _ = RegisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY, HOT_KEY_MODIFIERS(mods), vk);
            }
            if cfg.trusik {
                if let Some((mods, vk)) = cfg.broadcast_hotkey_vk() {
                    let _ = RegisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE, HOT_KEY_MODIFIERS(mods), vk);
                }
            }
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
            if wparam.0 as i32 == HOTKEY_HIDE_OVERLAY && overlay::is_eq_active() {
                overlay::toggle_hidden();
            } else if wparam.0 as i32 == HOTKEY_BROADCAST_TOGGLE {
                broadcast::toggle();
                overlay::refresh_broadcast_label();
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
                }
                ID_LAUNCH_EQ => launch_eq(),
                ID_SETTINGS => settings_dialog::show(),
                ID_CHECK_UPDATE => do_update_check(hwnd),
                ID_EXIT => PostQuitMessage(0),
                _ => {}
            }
            LRESULT(0)
        }
        x if x == settings_dialog::WM_SETTINGS_CHANGED => {
            // Re-register hotkeys with new config.
            let _ = UnregisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY);
            let _ = UnregisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE);
            let cfg = config::Config::load();
            if let Some((mods, vk)) = cfg.hide_hotkey_vk() {
                let _ = RegisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY, HOT_KEY_MODIFIERS(mods), vk);
            }
            if cfg.trusik {
                if let Some((mods, vk)) = cfg.broadcast_hotkey_vk() {
                    let _ = RegisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE, HOT_KEY_MODIFIERS(mods), vk);
                }
            }
            broadcast::on_settings_changed();
            // Reload overlay config (pip_edge, etc.) and rebuild layout.
            overlay::force_rebuild();
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = UnregisterHotKey(hwnd, HOTKEY_HIDE_OVERLAY);
            let _ = UnregisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE);
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

    let _ = AppendMenuW(menu, MF_STRING, ID_LAUNCH_EQ as usize, w!("Launch EQ"));
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

fn launch_eq() {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let exe = eq_dir.join("eqgame.exe");
    if !exe.exists() {
        eprintln!("eqgame.exe not found in {}", eq_dir.display());
        return;
    }
    if let Err(e) = std::process::Command::new(&exe)
        .arg("patchme")
        .current_dir(&eq_dir)
        .spawn()
    {
        eprintln!("Failed to launch EQ: {e}");
    }
}
