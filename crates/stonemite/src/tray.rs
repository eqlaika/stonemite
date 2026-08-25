use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreateMenu, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, FindWindowW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, IsWindow, KillTimer, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, TrackPopupMenu, CS_HREDRAW,
    CS_VREDRAW, LR_DEFAULTCOLOR, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_HOTKEY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSW, WS_EX_TOOLWINDOW,
};

use crate::broadcast;
use crate::build_info;
use crate::config;
use crate::control;
use crate::log_watcher;
use crate::overlay;
use crate::settings_dialog;
use crate::updater;

#[cfg(stonemite_dev_build)]
const TRAY_ICON_ICO: &[u8] = include_bytes!("../assets/tray-dev.ico");
#[cfg(not(stonemite_dev_build))]
const TRAY_ICON_ICO: &[u8] = include_bytes!("../assets/tray.ico");
const STONEMITE_LOGO_PNG: &[u8] = include_bytes!("../assets/app.png");

/// Return the image resource for a requested square size from an ICO file.
pub(crate) fn icon_resource(ico_data: &[u8], desired_size: u32) -> Option<&[u8]> {
    // ICO header: 2 reserved + 2 type + 2 count = 6 bytes.
    if ico_data.len() < 6 || ico_data[..4] != [0, 0, 1, 0] {
        return None;
    }
    let count = u16::from_le_bytes([ico_data[4], ico_data[5]]) as usize;

    for i in 0..count {
        let offset = 6 + i * 16;
        if offset + 16 > ico_data.len() {
            return None;
        }
        let width = match ico_data[offset] {
            0 => 256,
            value => value as u32,
        };
        let height = match ico_data[offset + 1] {
            0 => 256,
            value => value as u32,
        };

        if width == desired_size && height == desired_size {
            let data_size = u32::from_le_bytes([
                ico_data[offset + 8],
                ico_data[offset + 9],
                ico_data[offset + 10],
                ico_data[offset + 11],
            ]) as usize;
            let data_offset = u32::from_le_bytes([
                ico_data[offset + 12],
                ico_data[offset + 13],
                ico_data[offset + 14],
                ico_data[offset + 15],
            ]) as usize;
            let data_end = data_offset.checked_add(data_size)?;
            return ico_data.get(data_offset..data_end);
        }
    }
    None
}

/// Decode the transparent high-resolution logo to premultiplied BGRA for Direct2D.
pub(crate) fn stonemite_icon_bgra() -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(STONEMITE_LOGO_PNG).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for pixel in image.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let premultiply = |channel: u8| ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8;
        pixels.extend_from_slice(&[
            premultiply(blue),
            premultiply(green),
            premultiply(red),
            alpha,
        ]);
    }
    Some((width, height, pixels))
}

/// Load an icon of the given size from an in-memory ICO file.
/// Returns None if parsing fails or the size isn't found.
unsafe fn load_icon_from_ico(
    ico_data: &[u8],
    desired_size: u32,
) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    let resource = icon_resource(ico_data, desired_size)?;
    CreateIconFromResourceEx(
        resource,
        true,
        0x00030000, // version
        desired_size as i32,
        desired_size as i32,
        LR_DEFAULTCOLOR,
    )
    .ok()
}

fn tray_tooltip() -> String {
    if build_info::is_development() {
        format!("Stonemite development — v{}", build_info::version())
    } else {
        "Stonemite".to_owned()
    }
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
/// Two hotkey IDs per named box cycle: next, then previous.
const HOTKEY_CYCLE_BASE: i32 = 100;
const HOTKEYS_PER_CYCLE: usize = 2;

/// Timer ID for polling EQ windows.
const TIMER_POLL_EQ: usize = 1;
/// Timer ID for Mouse Clutch lifecycle/focus/drain checks.
const TIMER_MOUSE_CLUTCH: usize = 2;
const _: () = assert!(
    TIMER_POLL_EQ != TIMER_MOUSE_CLUTCH
        && TIMER_POLL_EQ != control::TIMER_CONTROL_INPUT
        && TIMER_MOUSE_CLUTCH != control::TIMER_CONTROL_INPUT,
    "tray-window timer IDs must be unique",
);
/// Poll interval in milliseconds (2 seconds).
const POLL_INTERVAL_MS: u32 = 2000;
/// Keep lifecycle checks below the measured 28–30 ms background mouse poll.
const MOUSE_CLUTCH_TICK_MS: u32 = 15;

/// Custom message posted when a background update check finds a new version.
const WM_UPDATE_AVAILABLE: u32 = WM_USER + 2;
/// Queue the shared tray menu from the in-game Stonemite button.
const WM_SHOW_STONEMITE_MENU: u32 = WM_USER + 4;
/// Custom message posted when Login All finds every configured account open.
const WM_LOGIN_ALL_ALREADY_OPEN: u32 = WM_USER + 3;

static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);
static LOGIN_ALL_RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) unsafe fn request_stonemite_menu(source_hwnd: HWND) -> bool {
    let Ok(tray_hwnd) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) else {
        return false;
    };
    !tray_hwnd.is_invalid()
        && PostMessageW(
            tray_hwnd,
            WM_SHOW_STONEMITE_MENU,
            WPARAM(source_hwnd.0 as usize),
            LPARAM(0),
        )
        .is_ok()
}

/// Ask an existing tray instance to exit through its normal message loop.
/// This lets the trushar runtime close and join before the process disappears.
pub fn quit_existing_instance() -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite")) else {
            return false;
        };
        if hwnd.is_invalid() {
            return false;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        if process_id == 0 {
            return false;
        }
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) else {
            return false;
        };
        let posted = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok();
        let stopped = posted && WaitForSingleObject(process, 10_000).0 == 0;
        let _ = CloseHandle(process);
        stopped
    }
}

/// Run the tray icon and message loop. Returns whether the application should relaunch.
pub fn run() -> bool {
    unsafe { run_inner() }
    RESTART_REQUESTED.swap(false, Ordering::SeqCst)
}

fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
    unsafe { PostQuitMessage(0) };
}

unsafe fn run_inner() {
    // Register window class for our hidden message window.
    let class_name = w!("StonemiteTrayClass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        lpszClassName: class_name,
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

    // Load the profile-specific tray icon from embedded ICO data.
    let icon =
        load_icon_from_ico(TRAY_ICON_ICO, 16).or_else(|| load_icon_from_ico(TRAY_ICON_ICO, 32));

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
    let tip = tray_tooltip();
    for (i, ch) in tip.encode_utf16().enumerate() {
        if i >= nid.szTip.len() - 1 {
            break;
        }
        nid.szTip[i] = ch;
    }
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);

    // The dispatcher is now ready. The dedicated runtime thread is joined
    // before this Win32 window and the overlay/broadcast owner state go away.
    let config = config::Config::load();
    let eq_dir = config.eq_directory();
    let trushar_server = control::start(hwnd, &config.trushar, eq_dir);
    if let Err(error) = log_watcher::start(hwnd) {
        crate::diagnostics::debug_log(&format!("eq_logs: {error}"));
        eprintln!("{error}");
    }
    overlay::sync_log_sources();

    // Message loop.
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
        let _ = windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
    }

    // Join the log worker while the hidden window can still receive its final
    // posted wake. This also stops the notify/ReadDirectoryChangesW backend.
    log_watcher::stop();
    drop(trushar_server);
    control::stop();

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
            // Start polling timers for EQ discovery and clutch lifecycle.
            let _ = SetTimer(hwnd, TIMER_POLL_EQ, POLL_INTERVAL_MS, None);
            let _ = SetTimer(hwnd, TIMER_MOUSE_CLUTCH, MOUSE_CLUTCH_TICK_MS, None);
            // Check for updates in the background if due.
            maybe_auto_update_check(hwnd);
            // Register global hotkey for hiding overlay.
            let cfg = config::Config::load();
            register_hotkeys(hwnd, &cfg);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_POLL_EQ {
                overlay::tick();
            } else if wparam.0 == TIMER_MOUSE_CLUTCH {
                if broadcast::tick() {
                    overlay::broadcast_state_changed();
                }
            } else if wparam.0 == control::TIMER_CONTROL_INPUT {
                control::advance_input();
            }
            LRESULT(0)
        }
        x if x == control::WM_CONTROL_COMMAND => {
            control::drain_commands();
            LRESULT(0)
        }
        x if x == log_watcher::WM_LOG_READY => {
            if !overlay::try_drain_log_events() {
                let _ = PostMessageW(hwnd, log_watcher::WM_LOG_READY, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == WM_LBUTTONUP {
                settings_dialog::show();
            } else if event == WM_RBUTTONUP {
                overlay::stonemite_menu_opened();
                show_context_menu(hwnd, None);
                overlay::stonemite_tray_menu_closed();
            }
            LRESULT(0)
        }
        x if x == WM_SHOW_STONEMITE_MENU => {
            let source_hwnd = HWND(wparam.0 as *mut _);
            show_context_menu(hwnd, Some(source_hwnd));
            overlay::stonemite_button_menu_closed(hwnd, source_hwnd);
            LRESULT(0)
        }
        WM_HOTKEY => {
            let id = wparam.0 as i32;
            if id == HOTKEY_HIDE_OVERLAY && overlay::is_app_foreground() {
                overlay::toggle_hidden();
            } else if id == HOTKEY_BROADCAST_TOGGLE {
                control::toggle_broadcast_on_ui(true);
            } else if id >= HOTKEY_SWAP_BASE && id < HOTKEY_SWAP_BASE + MAX_SWAP_HOTKEYS as i32 {
                let slot = (id - HOTKEY_SWAP_BASE) as usize + 1; // 1-based window number
                overlay::swap_to_number(slot);
            } else if id >= HOTKEY_CYCLE_BASE
                && id < HOTKEY_CYCLE_BASE + (config::MAX_BOX_CYCLES * HOTKEYS_PER_CYCLE) as i32
            {
                let offset = (id - HOTKEY_CYCLE_BASE) as usize;
                let cycle_index = offset / HOTKEYS_PER_CYCLE;
                if offset % HOTKEYS_PER_CYCLE == 0 {
                    overlay::cycle_box_next(cycle_index);
                } else {
                    overlay::cycle_box_previous(cycle_index);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as u16;
            match id {
                ID_SHOW_OVERLAY => overlay::toggle_hidden(),
                ID_EDIT_MODE => overlay::toggle_edit_mode(),
                ID_BROADCAST_TOGGLE => {
                    control::toggle_broadcast_on_ui(true);
                }
                ID_LAUNCH_EQ => launch_eq(None, None),
                ID_LOGIN_ALL => {
                    if LOGIN_ALL_RUNNING.swap(true, Ordering::SeqCst) {
                        return LRESULT(0);
                    }
                    let cfg = config::Config::load();
                    let accounts: Vec<(String, Option<String>)> = cfg
                        .accounts
                        .iter()
                        .map(|a| {
                            let pw = crate::crypt::decrypt(&a.password).ok();
                            (a.username.clone(), pw)
                        })
                        .collect();
                    let hwnd_raw = hwnd.0 as usize;
                    std::thread::spawn(move || {
                        let _guard = LoginAllGuard;
                        let configured = accounts.len();
                        let open_usernames = crate::eq_windows::find_eq_login_usernames();
                        let (accounts, skipped) = filter_open_accounts(accounts, &open_usernames);
                        crate::diagnostics::debug_log(&format!(
                            "login_all: configured={configured} open={} skipped={skipped} launching={}",
                            open_usernames.len(),
                            accounts.len()
                        ));
                        if configured > 0 && accounts.is_empty() {
                            let hwnd = HWND(hwnd_raw as *mut _);
                            unsafe {
                                let _ = PostMessageW(
                                    hwnd,
                                    WM_LOGIN_ALL_ALREADY_OPEN,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                        for (username, password) in &accounts {
                            launch_eq(Some(username), password.as_deref());
                        }
                    });
                }
                ID_CONFIGURE_ACCOUNTS => {
                    settings_dialog::show();
                }
                ID_SETTINGS => {
                    let _ = settings_dialog::show();
                }
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
        x if x == WM_LOGIN_ALL_ALREADY_OPEN => {
            overlay::show_toast("All configured accounts are already open");
            LRESULT(0)
        }
        x if x == settings_dialog::WM_SETTINGS_CHANGED => {
            // Begin bounded Mouse Clutch release before applying any new hook
            // binding, then re-register the independent global hotkeys.
            broadcast::on_settings_changed();
            unregister_hotkeys(hwnd);
            let cfg = config::Config::load();
            register_hotkeys(hwnd, &cfg);
            // Reload overlay config (pip_edge, etc.), update the watched Logs
            // directory, and rebuild layout.
            overlay::reload_config();
            LRESULT(0)
        }
        x if x == settings_dialog::WM_RESTART_REQUESTED => {
            request_restart();
            LRESULT(0)
        }
        x if x == settings_dialog::WM_BEGIN_PAIRING => {
            LRESULT(if control::begin_pairing(wparam.0 as u32) {
                1
            } else {
                0
            })
        }
        x if x == settings_dialog::WM_CANCEL_PAIRING => {
            control::cancel_pairing();
            LRESULT(0)
        }
        x if x == settings_dialog::WM_PAIRING_STATUS => {
            LRESULT(if control::pairing_is_open() { 1 } else { 0 })
        }
        WM_DESTROY => {
            unregister_hotkeys(hwnd);
            let _ = KillTimer(hwnd, TIMER_POLL_EQ);
            let _ = KillTimer(hwnd, TIMER_MOUSE_CLUTCH);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_context_menu(hwnd: HWND, expected_source: Option<HWND>) {
    let cfg = config::Config::load();
    let menu = CreatePopupMenu().expect("Failed to create popup menu");

    let overlay_label = format!("Show overlay\t{}\0", cfg.hide_hotkey);
    let overlay_wide: Vec<u16> = overlay_label.encode_utf16().collect();
    let check_flag = if overlay::is_visible() {
        MF_CHECKED
    } else {
        MF_UNCHECKED
    };
    let _ = AppendMenuW(
        menu,
        MF_STRING | check_flag,
        ID_SHOW_OVERLAY as usize,
        windows::core::PCWSTR(overlay_wide.as_ptr()),
    );

    let edit_label = if overlay::is_edit_mode() {
        "Lock layout\0"
    } else {
        "Edit layout\0"
    };
    let edit_wide: Vec<u16> = edit_label.encode_utf16().collect();
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        ID_EDIT_MODE as usize,
        windows::core::PCWSTR(edit_wide.as_ptr()),
    );

    // Broadcasting controls and status (only shown if trusik is enabled).
    if cfg.trusik {
        let bc_label = if broadcast::is_active() {
            format!("Broadcasting: on\t{}\0", cfg.broadcast_hotkey)
        } else {
            format!("Broadcasting: off\t{}\0", cfg.broadcast_hotkey)
        };
        let bc_wide: Vec<u16> = bc_label.encode_utf16().collect();
        let bc_flag = if broadcast::is_active() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING | bc_flag,
            ID_BROADCAST_TOGGLE as usize,
            windows::core::PCWSTR(bc_wide.as_ptr()),
        );

        let clutch_status = broadcast::mouse_clutch_status();
        let clutch_error = broadcast::mouse_clutch_error();
        if clutch_error.is_none()
            && (!cfg.mouse_clutch_key.is_empty()
                || clutch_status != broadcast::MouseClutchStatus::Inactive)
        {
            let label = match clutch_status {
                broadcast::MouseClutchStatus::Inactive => {
                    format!("Mouse Clutch: ready\t{}\0", cfg.mouse_clutch_key)
                }
                broadcast::MouseClutchStatus::Active => "Mouse Clutch: active\0".to_owned(),
                broadcast::MouseClutchStatus::Releasing => "Mouse Clutch: releasing\0".to_owned(),
            };
            let wide: Vec<u16> = label.encode_utf16().collect();
            let checked = if clutch_status == broadcast::MouseClutchStatus::Inactive {
                MF_UNCHECKED
            } else {
                MF_CHECKED
            };
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_GRAYED | checked,
                0,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
        if let Some(error) = clutch_error {
            let label = format!("Mouse Clutch unavailable: {error}\0");
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
    }

    if cfg.accounts.is_empty() {
        let _ = AppendMenuW(menu, MF_STRING, ID_LAUNCH_EQ as usize, w!("Launch EQ"));
    } else {
        let login_menu = CreateMenu().expect("Failed to create login submenu");
        let _ = AppendMenuW(
            login_menu,
            MF_STRING,
            ID_LOGIN_ALL as usize,
            w!("Login all accounts"),
        );
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
        let _ = AppendMenuW(
            menu,
            MF_STRING | MF_POPUP,
            login_menu.0 as usize,
            w!("Login"),
        );
    }
    let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, w!("Settings..."));
    let update_label = format!("Check for updates\tv{}\0", updater::current_version());
    let update_wide: Vec<u16> = update_label.encode_utf16().collect();
    let _ = AppendMenuW(
        menu,
        MF_STRING,
        ID_CHECK_UPDATE as usize,
        windows::core::PCWSTR(update_wide.as_ptr()),
    );
    let _ = AppendMenuW(menu, MF_STRING, ID_EXIT as usize, w!("Exit"));

    if expected_source.is_some_and(|source| {
        source.is_invalid() || !IsWindow(source).as_bool() || GetForegroundWindow() != source
    }) {
        let _ = DestroyMenu(menu);
        return;
    }

    let mut pt = Default::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
    let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
}

unsafe fn do_update_check(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
    };

    match updater::check_and_update() {
        updater::UpdateResult::UpToDate => {
            MessageBoxW(
                hwnd,
                w!("You are running the latest version."),
                w!("Stonemite Update"),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        updater::UpdateResult::Updated {
            version: ver,
            notes,
        } => {
            let msg = if notes.is_empty() {
                format!("Updated to v{}! Stonemite will now restart.\0", ver)
            } else {
                format!(
                    "Updated to v{}! Stonemite will now restart.\n\n{}\0",
                    ver, notes
                )
            };
            let msg_wide: Vec<u16> = msg.encode_utf16().collect();
            MessageBoxW(
                hwnd,
                windows::core::PCWSTR(msg_wide.as_ptr()),
                w!("Stonemite Update"),
                MB_OK | MB_ICONINFORMATION,
            );
            request_restart();
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
    let Ok(config_lock) = config::Config::lock() else {
        return;
    };
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
    drop(config_lock);

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
            eprintln!(
                "Failed to register hide overlay hotkey: {}",
                cfg.hide_hotkey
            );
        }
    }
    if cfg.trusik {
        if let Some((mods, vk)) = cfg.broadcast_hotkey_vk() {
            if RegisterHotKey(hwnd, HOTKEY_BROADCAST_TOGGLE, HOT_KEY_MODIFIERS(mods), vk).is_err() {
                eprintln!(
                    "Failed to register broadcast hotkey: {}",
                    cfg.broadcast_hotkey
                );
            }
        }
    }
    for i in 0..MAX_SWAP_HOTKEYS {
        if let Some((mods, vk)) = cfg.swap_hotkey_vk(i) {
            if RegisterHotKey(
                hwnd,
                HOTKEY_SWAP_BASE + i as i32,
                HOT_KEY_MODIFIERS(mods),
                vk,
            )
            .is_err()
            {
                eprintln!(
                    "Failed to register swap hotkey {}: {}",
                    i + 1,
                    cfg.swap_hotkeys.get(i).map(|s| s.as_str()).unwrap_or("?")
                );
            }
        }
    }
    for (cycle_index, cycle) in cfg
        .box_cycles
        .iter()
        .take(config::MAX_BOX_CYCLES)
        .enumerate()
    {
        let directions = [
            ("next", cycle.next_hotkey.as_str(), cycle.next_hotkey_vk()),
            (
                "previous",
                cycle.previous_hotkey.as_str(),
                cycle.previous_hotkey_vk(),
            ),
        ];
        for (direction_index, (direction, binding, parsed)) in directions.into_iter().enumerate() {
            let Some((mods, vk)) = parsed else {
                continue;
            };
            let id = HOTKEY_CYCLE_BASE + (cycle_index * HOTKEYS_PER_CYCLE + direction_index) as i32;
            if RegisterHotKey(hwnd, id, HOT_KEY_MODIFIERS(mods | MOD_NOREPEAT.0), vk).is_err() {
                eprintln!(
                    "Failed to register {} {direction} cycle hotkey: {binding}",
                    cycle.name
                );
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
    for i in 0..config::MAX_BOX_CYCLES * HOTKEYS_PER_CYCLE {
        let _ = UnregisterHotKey(hwnd, HOTKEY_CYCLE_BASE + i as i32);
    }
}

struct LoginAllGuard;

impl Drop for LoginAllGuard {
    fn drop(&mut self) {
        LOGIN_ALL_RUNNING.store(false, Ordering::SeqCst);
    }
}

fn filter_open_accounts(
    accounts: Vec<(String, Option<String>)>,
    open_usernames: &HashSet<String>,
) -> (Vec<(String, Option<String>)>, usize) {
    let configured = accounts.len();
    let accounts: Vec<_> = accounts
        .into_iter()
        .filter(|(username, _)| !open_usernames.contains(&username.trim().to_ascii_lowercase()))
        .collect();
    let skipped = configured - accounts.len();
    (accounts, skipped)
}

fn launch_eq(username: Option<&str>, password: Option<&str>) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let exe = eq_dir.join("eqgame.exe");
    if !exe.exists() {
        eprintln!("eqgame.exe not found in {}", eq_dir.display());
        return;
    }
    crate::diagnostics::debug_log(&format!(
        "launch_eq: user={:?} has_password={}",
        username,
        password.is_some()
    ));
    if cfg.trusik {
        if let Err(error) = crate::trusik_deploy::deploy(&eq_dir) {
            if crate::trusik_deploy::is_in_use_error(&error) {
                // A running EQ client has the installed proxy mapped. New clients
                // can keep using it; protocol handshakes fail safely if it is too old.
                crate::diagnostics::debug_log(&format!(
                    "launch_eq: input proxy is in use; launching with installed copy: {error}"
                ));
            } else {
                crate::diagnostics::debug_log(&format!(
                    "launch_eq: input proxy update failed before spawn: {error}"
                ));
                overlay::show_toast(
                    "Input proxy update failed. Close all EverQuest clients and try again.",
                );
                return;
            }
        }
    }
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("patchme").current_dir(&eq_dir);
    if let Some(user) = username {
        cmd.arg(format!("/login:{user}"));
    }
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            crate::diagnostics::debug_log(&format!("launch_eq: spawned pid={pid}"));
            if let Some(pw) = password {
                crate::auto_type::spawn(pid, pw.to_string());
            }
        }
        Err(e) => {
            crate::diagnostics::debug_log(&format!("launch_eq: spawn failed: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRODUCTION_APP_ICON: &[u8] = include_bytes!("../assets/app.ico");
    const DEVELOPMENT_APP_ICON: &[u8] = include_bytes!("../assets/app-dev.ico");
    const PRODUCTION_TRAY_ICON: &[u8] = include_bytes!("../assets/tray.ico");
    const DEVELOPMENT_TRAY_ICON: &[u8] = include_bytes!("../assets/tray-dev.ico");

    #[test]
    fn icon_assets_contain_required_sizes() {
        for icon in [
            PRODUCTION_APP_ICON,
            DEVELOPMENT_APP_ICON,
            PRODUCTION_TRAY_ICON,
            DEVELOPMENT_TRAY_ICON,
        ] {
            for size in [16, 32, 48, 256] {
                assert!(icon_resource(icon, size).is_some(), "missing {size}px icon");
            }
        }
    }

    #[test]
    fn overlay_logo_uses_high_resolution_premultiplied_pixels() {
        let (width, height, pixels) = stonemite_icon_bgra().expect("decode overlay logo");
        assert_eq!((width, height), (256, 256));
        assert_eq!(pixels.len(), 256 * 256 * 4);
        assert_eq!(pixels[3], 0, "the logo background must be transparent");
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 255));
        for pixel in pixels.chunks_exact(4) {
            let alpha = pixel[3];
            assert!(pixel[0] <= alpha && pixel[1] <= alpha && pixel[2] <= alpha);
        }
    }

    #[test]
    fn development_icons_differ_from_production() {
        assert_ne!(DEVELOPMENT_APP_ICON, PRODUCTION_APP_ICON);
        assert_ne!(DEVELOPMENT_TRAY_ICON, PRODUCTION_TRAY_ICON);
    }

    #[test]
    fn tooltip_identifies_the_selected_build_flavor() {
        let tooltip = tray_tooltip();
        assert!(tooltip.encode_utf16().count() < 128);
        if build_info::is_development() {
            assert!(tooltip.starts_with("Stonemite development — v"));
        } else {
            assert_eq!(tooltip, "Stonemite");
        }
    }

    #[test]
    fn login_all_skips_open_accounts_case_insensitively_and_preserves_order() {
        let accounts = vec![
            ("First".to_owned(), Some("one".to_owned())),
            ("SECOND".to_owned(), Some("two".to_owned())),
            ("Third".to_owned(), Some("three".to_owned())),
        ];
        let open_usernames = HashSet::from(["second".to_owned()]);

        let (accounts, skipped) = filter_open_accounts(accounts, &open_usernames);

        assert_eq!(skipped, 1);
        assert_eq!(
            accounts
                .into_iter()
                .map(|(username, _)| username)
                .collect::<Vec<_>>(),
            vec!["First", "Third"]
        );
    }
}
