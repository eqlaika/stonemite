use std::cell::UnsafeCell;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, GetDC, GetDeviceCaps, GetStockObject, SetBkMode, COLOR_WINDOW,
    DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, HBRUSH, HFONT, LOGPIXELSY, ReleaseDC,
    TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
};
use windows::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
    BROWSEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;

/// SS_ETCHEDHORZ — horizontal etched line (static control style).
const SS_ETCHEDHORZ: u32 = 0x0010;

// Control IDs.
const IDC_EQ_DIR_EDIT: i32 = 100;
const IDC_EQ_DIR_BROWSE: i32 = 101;
const IDC_HOTKEY_COMBO: i32 = 102;
const IDC_SAVE: i32 = 103;
const IDC_CANCEL: i32 = 104;
const IDC_PIP_EDGE_COMBO: i32 = 105;

const HOTKEY_OPTIONS: &[&str] = &[
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    "Pause", "ScrollLock", "Insert", "Delete", "Home", "End", "PageUp", "PageDown",
];

const PIP_EDGE_OPTIONS: &[&str] = &["Right", "Left", "Top", "Bottom"];

/// Custom message posted to the tray window after settings are saved.
pub const WM_SETTINGS_CHANGED: u32 = WM_USER + 100;

struct DialogCell(UnsafeCell<Option<HWND>>);
unsafe impl Sync for DialogCell {}
static DIALOG_HWND: DialogCell = DialogCell(UnsafeCell::new(None));

/// Show the settings dialog. If already open, brings it to the foreground.
pub fn show() {
    unsafe {
        let existing = &mut *DIALOG_HWND.0.get();
        if let Some(hwnd) = *existing {
            if IsWindow(hwnd).as_bool() {
                let _ = SetForegroundWindow(hwnd);
                return;
            }
        }

        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES,
        });

        let hwnd = create_dialog();
        *existing = Some(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

unsafe fn create_dialog() -> HWND {
    let class_name = w!("StonemiteSettingsClass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(dialog_proc),
        lpszClassName: class_name.into(),
        hbrBackground: HBRUSH((COLOR_WINDOW.0 as isize + 1) as *mut _),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&wc);

    let dpi = get_dpi_scale(HWND::default());
    let w = scale(480, dpi);
    let h = scale(400, dpi);

    let sx = GetSystemMetrics(SM_CXSCREEN);
    let sy = GetSystemMetrics(SM_CYSCREEN);

    CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class_name,
        w!("Stonemite Settings"),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        (sx - w) / 2,
        (sy - h) / 2,
        w,
        h,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create settings dialog")
}

unsafe fn get_dpi_scale(hwnd: HWND) -> f64 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    let dpi = GetDpiForWindow(hwnd);
    if dpi > 0 {
        return dpi as f64 / 96.0;
    }
    let dc = GetDC(HWND::default());
    let val = GetDeviceCaps(dc, LOGPIXELSY);
    let _ = ReleaseDC(HWND::default(), dc);
    val as f64 / 96.0
}

fn scale(val: i32, dpi: f64) -> i32 {
    (val as f64 * dpi).round() as i32
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn populate_controls(hwnd: HWND) {
    let dpi = get_dpi_scale(hwnd);

    let margin = scale(20, dpi);
    let label_h = scale(18, dpi);
    let ctrl_h = scale(26, dpi);
    let btn_w = scale(90, dpi);
    let btn_h = scale(32, dpi);
    let browse_w = scale(36, dpi);
    let row_gap = scale(6, dpi);
    let section_gap = scale(18, dpi);


    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let cw = rc.right - rc.left;
    let ch = rc.bottom - rc.top;

    let font = create_font(dpi, false);
    let bold_font = create_font(dpi, true);
    let mut y = margin;

    // --- Section: EQ Directory ---
    let lbl = create_child(
        hwnd, w!("STATIC"), w!("EverQuest directory"),
        WS_CHILD | WS_VISIBLE, margin, y, cw - 2 * margin, label_h,
    );
    set_font(lbl, bold_font);
    y += label_h + row_gap;

    let edit_w = cw - 2 * margin - browse_w - scale(8, dpi);
    let edit = create_child(
        hwnd, w!("EDIT"), w!(""),
        WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        margin, y, edit_w, ctrl_h,
    );
    SetWindowLongPtrW(edit, GWLP_ID, IDC_EQ_DIR_EDIT as isize);
    set_font(edit, font);

    let browse = create_child(
        hwnd, w!("BUTTON"), w!("..."),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        margin + edit_w + scale(8, dpi), y, browse_w, ctrl_h,
    );
    SetWindowLongPtrW(browse, GWLP_ID, IDC_EQ_DIR_BROWSE as isize);
    set_font(browse, font);

    y += ctrl_h + section_gap;

    // --- Separator ---
    let _ = create_child(
        hwnd, w!("STATIC"), w!(""),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_ETCHEDHORZ),
        margin, y, cw - 2 * margin, scale(2, dpi),
    );
    y += scale(2, dpi) + section_gap;

    // --- Section: Hotkey ---
    let lbl2 = create_child(
        hwnd, w!("STATIC"), w!("Hide overlay hotkey"),
        WS_CHILD | WS_VISIBLE, margin, y, cw - 2 * margin, label_h,
    );
    set_font(lbl2, bold_font);
    y += label_h + row_gap;

    let desc = create_child(
        hwnd, w!("STATIC"), w!("Press this key to toggle PiP overlay visibility while EQ is focused"),
        WS_CHILD | WS_VISIBLE, margin, y, cw - 2 * margin, label_h,
    );
    set_font(desc, font);
    y += label_h + row_gap;

    let combo = create_child(
        hwnd, w!("COMBOBOX"), w!(""),
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_TABSTOP
            | WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | CBS_HASSTRINGS as u32),
        margin, y, scale(160, dpi), ctrl_h * 10,
    );
    SetWindowLongPtrW(combo, GWLP_ID, IDC_HOTKEY_COMBO as isize);
    set_font(combo, font);

    for key in HOTKEY_OPTIONS {
        let wide = to_wide(key);
        SendMessageW(combo, CB_ADDSTRING, WPARAM(0), LPARAM(wide.as_ptr() as isize));
    }

    y += ctrl_h + section_gap;

    // --- Separator ---
    let _ = create_child(
        hwnd, w!("STATIC"), w!(""),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_ETCHEDHORZ),
        margin, y, cw - 2 * margin, scale(2, dpi),
    );
    y += scale(2, dpi) + section_gap;

    // --- Section: PiP Edge ---
    let lbl3 = create_child(
        hwnd, w!("STATIC"), w!("PiP edge"),
        WS_CHILD | WS_VISIBLE, margin, y, cw - 2 * margin, label_h,
    );
    set_font(lbl3, bold_font);
    y += label_h + row_gap;

    let desc2 = create_child(
        hwnd, w!("STATIC"), w!("Screen edge where PiP thumbnails are anchored"),
        WS_CHILD | WS_VISIBLE, margin, y, cw - 2 * margin, label_h,
    );
    set_font(desc2, font);
    y += label_h + row_gap;

    let edge_combo = create_child(
        hwnd, w!("COMBOBOX"), w!(""),
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_TABSTOP
            | WINDOW_STYLE(CBS_DROPDOWNLIST as u32 | CBS_HASSTRINGS as u32),
        margin, y, scale(160, dpi), ctrl_h * 10,
    );
    SetWindowLongPtrW(edge_combo, GWLP_ID, IDC_PIP_EDGE_COMBO as isize);
    set_font(edge_combo, font);

    for opt in PIP_EDGE_OPTIONS {
        let wide = to_wide(opt);
        SendMessageW(edge_combo, CB_ADDSTRING, WPARAM(0), LPARAM(wide.as_ptr() as isize));
    }

    // --- Bottom button bar with separator ---
    let btn_y = ch - margin - btn_h;
    let sep_y = btn_y - section_gap;
    let _ = create_child(
        hwnd, w!("STATIC"), w!(""),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SS_ETCHEDHORZ),
        margin, sep_y, cw - 2 * margin, scale(2, dpi),
    );

    let cancel = create_child(
        hwnd, w!("BUTTON"), w!("Cancel"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
        cw - margin - btn_w, btn_y, btn_w, btn_h,
    );
    SetWindowLongPtrW(cancel, GWLP_ID, IDC_CANCEL as isize);
    set_font(cancel, font);

    let save = create_child(
        hwnd, w!("BUTTON"), w!("Save"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
        cw - margin - btn_w - scale(10, dpi) - btn_w, btn_y, btn_w, btn_h,
    );
    SetWindowLongPtrW(save, GWLP_ID, IDC_SAVE as isize);
    set_font(save, font);

    // Load current config values into controls.
    let cfg = Config::load();
    let eq_dir_wide = to_wide(&cfg.eq_dir);
    let _ = SetWindowTextW(edit, windows::core::PCWSTR(eq_dir_wide.as_ptr()));

    let hotkey_upper = cfg.hide_hotkey.trim().to_uppercase();
    for (i, key) in HOTKEY_OPTIONS.iter().enumerate() {
        if key.to_uppercase() == hotkey_upper {
            SendMessageW(combo, CB_SETCURSEL, WPARAM(i), LPARAM(0));
            break;
        }
    }

    let edge_idx = match cfg.pip_edge {
        crate::config::PipEdge::Right => 0,
        crate::config::PipEdge::Left => 1,
        crate::config::PipEdge::Top => 2,
        crate::config::PipEdge::Bottom => 3,
    };
    SendMessageW(edge_combo, CB_SETCURSEL, WPARAM(edge_idx), LPARAM(0));
}

unsafe fn create_child(
    parent: HWND,
    class: windows::core::PCWSTR,
    text: windows::core::PCWSTR,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> HWND {
    CreateWindowExW(Default::default(), class, text, style, x, y, w, h, parent, None, None, None)
        .expect("Failed to create control")
}

unsafe fn create_font(dpi: f64, bold: bool) -> HFONT {
    CreateFontW(
        -scale(14, dpi),
        0, 0, 0,
        if bold { FW_BOLD.0 } else { FW_NORMAL.0 } as i32,
        0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        0, 0, 0, 0,
        w!("Segoe UI"),
    )
}

unsafe fn set_font(hwnd: HWND, font: HFONT) {
    SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

unsafe fn get_ctrl(hwnd: HWND, id: i32) -> HWND {
    GetDlgItem(hwnd, id).expect("Control not found")
}

unsafe fn save_settings(hwnd: HWND) {
    let eq_dir_edit = get_ctrl(hwnd, IDC_EQ_DIR_EDIT);
    let combo = get_ctrl(hwnd, IDC_HOTKEY_COMBO);
    let edge_combo = get_ctrl(hwnd, IDC_PIP_EDGE_COMBO);

    let mut buf = [0u16; 1024];
    let len = GetWindowTextW(eq_dir_edit, &mut buf) as usize;
    let eq_dir = String::from_utf16_lossy(&buf[..len]);

    let sel = SendMessageW(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
    let hide_hotkey = if sel < HOTKEY_OPTIONS.len() {
        HOTKEY_OPTIONS[sel].to_string()
    } else {
        "F9".to_string()
    };

    let edge_sel = SendMessageW(edge_combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
    let pip_edge = match edge_sel {
        1 => crate::config::PipEdge::Left,
        2 => crate::config::PipEdge::Top,
        3 => crate::config::PipEdge::Bottom,
        _ => crate::config::PipEdge::Right,
    };

    let existing = Config::load();
    let cfg = Config {
        eq_dir,
        hide_hotkey,
        pip_edge,
        pip_strip_width: existing.pip_strip_width,
        pip_positions: existing.pip_positions,
        snap_grid: existing.snap_grid,
        telemetry: existing.telemetry,
        telemetry_id: existing.telemetry_id,
    };
    if let Err(e) = cfg.save() {
        eprintln!("Failed to save config: {e}");
    }
}

unsafe fn browse_folder(hwnd: HWND) {
    let title = to_wide("Select EverQuest directory");
    let bi = BROWSEINFOW {
        hwndOwner: hwnd,
        lpszTitle: windows::core::PCWSTR(title.as_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let pidl = SHBrowseForFolderW(&bi);
    if pidl.is_null() {
        return;
    }
    let mut path = [0u16; 260];
    if SHGetPathFromIDListW(pidl, &mut path).as_bool() {
        let edit = get_ctrl(hwnd, IDC_EQ_DIR_EDIT);
        let _ = SetWindowTextW(edit, windows::core::PCWSTR(path.as_ptr()));
    }
    windows::Win32::System::Com::CoTaskMemFree(Some(pidl as *const _));
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            populate_controls(hwnd);
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
            SetBkMode(hdc, TRANSPARENT);
            return LRESULT(GetStockObject(WHITE_BRUSH).0 as isize);
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            match id {
                IDC_SAVE => {
                    save_settings(hwnd);
                    if let Ok(tray) =
                        FindWindowW(w!("StonemiteTrayClass"), w!("Stonemite"))
                    {
                        let _ = PostMessageW(tray, WM_SETTINGS_CHANGED, WPARAM(0), LPARAM(0));
                    }
                    let _ = DestroyWindow(hwnd);
                }
                IDC_CANCEL => {
                    let _ = DestroyWindow(hwnd);
                }
                IDC_EQ_DIR_BROWSE => {
                    browse_folder(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            *DIALOG_HWND.0.get() = None;
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
