use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::time::Duration;

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DrawTextW, EndPaint, FillRect, ScreenToClient,
    FrameRect, GetStockObject, InvalidateRect, SelectObject, SetBkMode, SetTextColor,
    BACKGROUND_MODE, DT_LEFT, DT_SINGLELINE, DT_TOP, FW_BOLD, HBRUSH, BLACK_BRUSH,
    PAINTSTRUCT,
};
use windows::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{config, eq_characters, eq_windows};
use crate::eq_windows::EqWindow;

/// Gap between thumbnails in pixels.
const THUMB_GAP: i32 = 4;

/// Maximum number of PiP thumbnails displayed.
const MAX_PIPS: usize = 5;

/// Maximum strip width as a fraction of monitor dimension.
const MAX_STRIP_WIDTH_FRACTION: f64 = 0.25;

/// Minimum strip width as a fraction of monitor dimension.
const MIN_STRIP_WIDTH_FRACTION: f64 = 0.05;

/// Width of the resize grab zone along the interior edge (logical pixels).
const RESIZE_HANDLE_WIDTH: i32 = 6;

/// Thumbnail opacity (0-255). ~80% = 204.
const THUMB_OPACITY_NORMAL: u8 = 204;
const THUMB_OPACITY_HOVER: u8 = 255;

/// Border thickness for hover highlight.
const BORDER_WIDTH: i32 = 3;

/// Height of the character name label overlay.
const LABEL_HEIGHT: i32 = 48;

/// Base ID for character-assign context menu items.
const IDM_CHAR_BASE: u32 = 5000;
/// Base ID for number-reassign context menu items.
const IDM_NUMBER_BASE: u32 = 6000;
/// Base ID for pip-edge context menu items.
const IDM_EDGE_BASE: u32 = 7000;
/// Menu ID for hide overlay action.
const IDM_HIDE_OVERLAY: u32 = 7100;

/// Distinct background colors for per-number labels (COLORREF = 0x00BBGGRR).
const LABEL_COLORS: &[u32] = &[
    0x00993300, // dark blue
    0x00006600, // dark green
    0x00000099, // dark red
    0x00996600, // dark teal
    0x00660099, // dark purple
    0x00009999, // dark yellow
];

/// Return the lowest positive integer not already used by any tracked window.
fn next_available_number(eq_windows: &[EqWindow]) -> usize {
    let mut n = 1;
    while eq_windows.iter().any(|w| w.number == n) {
        n += 1;
    }
    n
}

unsafe fn get_dpi_scale() -> f64 {
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSY};
    let dc = GetDC(HWND::default());
    let dpi = GetDeviceCaps(dc, LOGPIXELSY);
    let _ = ReleaseDC(HWND::default(), dc);
    dpi as f64 / 96.0
}

fn dpi(val: i32, scale: f64) -> i32 {
    (val as f64 * scale).round() as i32
}

fn color_for_number(number: usize) -> u32 {
    if number == 0 { return LABEL_COLORS[0]; }
    LABEL_COLORS[(number - 1) % LABEL_COLORS.len()]
}

pub fn debug_log(msg: &str) {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let elapsed = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let log_path = std::path::Path::new(&appdata).join("Stonemite").join("debug.log");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path) {
            let _ = writeln!(f, "[{elapsed:>8.3}s] {msg}");
        }
    }
}

struct ThumbnailEntry {
    eq_hwnd: HWND,
    pid: u32,
    thumb: isize,
    cell_rect: RECT,
    label: String,
    number: usize,
}

struct OverlayState {
    overlay_hwnd: HWND,
    thumbnails: Vec<ThumbnailEntry>,
    /// All tracked EQ windows with stable numbers and character assignments.
    eq_windows: Vec<EqWindow>,
    /// PIDs in PiP strip order. Positions are fixed; on swap, two PIDs exchange.
    pip_order: Vec<u32>,
    /// PID of the currently active (foreground) window.
    active_pid: Option<u32>,
    hovered_index: Option<usize>,
    monitor_rect: RECT,
    strip_width: i32,
    event_hook: HWINEVENTHOOK,
    context_menu_target_pid: Option<u32>,
    context_menu_candidates: Vec<eq_characters::CharCandidate>,
    /// Floating label window for the active (foreground) EQ window.
    active_label_hwnd: HWND,
    active_label_text: String,
    active_label_color: u32,
    /// Drag state: index of thumbnail being dragged, and start point.
    drag: Option<DragState>,
    /// Visual indicator: index where the dragged item would be dropped.
    drop_target: Option<usize>,
    /// User has toggled overlay hidden via hotkey.
    hidden_by_user: bool,
    /// DPI scale factor (1.0 = 96 DPI).
    dpi_scale: f64,
    /// Which screen edge the PiP strip is anchored to.
    pip_edge: config::PipEdge,
    /// Resize drag state.
    resize_drag: Option<ResizeDragState>,
    /// User-configured strip width override (pixels). None = auto.
    custom_strip_width: Option<i32>,
    /// Current strip height (needed for horizontal resize).
    strip_height: i32,
}

/// Minimum pixel distance before a click becomes a drag.
const DRAG_THRESHOLD: i32 = 8;

struct DragState {
    from_index: usize,
    start_pt: POINT,
    dragging: bool,
}

struct ResizeDragState {
    start_pt: POINT,
    start_size: i32,
}

struct OverlayCell(UnsafeCell<Option<OverlayState>>);
unsafe impl Sync for OverlayCell {}

static OVERLAY: OverlayCell = OverlayCell(UnsafeCell::new(None));

fn state() -> &'static mut Option<OverlayState> {
    unsafe { &mut *OVERLAY.0.get() }
}

fn is_eq_or_overlay(hwnd: HWND, eq_windows: &[EqWindow], overlay_hwnd: HWND, label_hwnd: HWND) -> bool {
    hwnd == overlay_hwnd || hwnd == label_hwnd || eq_windows.iter().any(|w| w.hwnd == hwnd)
}

unsafe fn update_visibility(s: &mut OverlayState) {
    if s.hidden_by_user {
        s.hovered_index = None;
        let _ = ShowWindow(s.overlay_hwnd, SW_HIDE);
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }
    let has_pip = !s.pip_order.is_empty();
    let fg = GetForegroundWindow();
    if has_pip && is_eq_or_overlay(fg, &s.eq_windows, s.overlay_hwnd, s.active_label_hwnd) {
        let _ = ShowWindow(s.overlay_hwnd, SW_SHOWNOACTIVATE);
        if !s.active_label_text.is_empty() {
            let _ = ShowWindow(s.active_label_hwnd, SW_SHOWNOACTIVATE);
        }
    } else {
        s.hovered_index = None;
        let _ = ShowWindow(s.overlay_hwnd, SW_HIDE);
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
    }
}

unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK, _event: u32, _hwnd: HWND,
    _id_object: i32, _id_child: i32, _id_event_thread: u32, _dw_ms_event_time: u32,
) {
    let Some(s) = state().as_mut() else { return };

    // If a different EQ window came to the foreground, swap it to be active.
    let fg = GetForegroundWindow();
    if let Some(fg_eq) = s.eq_windows.iter().find(|w| w.hwnd == fg) {
        let fg_pid = fg_eq.pid;
        if s.active_pid != Some(fg_pid) {
            if let Some(pos) = s.pip_order.iter().position(|p| *p == fg_pid) {
                if let Some(old_active) = s.active_pid {
                    s.pip_order[pos] = old_active;
                } else {
                    s.pip_order.remove(pos);
                }
                s.active_pid = Some(fg_pid);
                rebuild_thumbnails(s);
            }
        }
    }

    update_visibility(s);
}

pub fn init() -> HWND {
    unsafe { init_inner() }
}

unsafe fn init_inner() -> HWND {
    let class_name = w!("StonemiteOverlayClass");
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(overlay_wnd_proc),
        lpszClassName: class_name.into(),
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        style: CS_DBLCLKS,
        ..Default::default()
    };
    RegisterClassW(&wc);

    let label_class = w!("StonemiteLabelClass");
    let label_wc = WNDCLASSW {
        lpfnWndProc: Some(label_wnd_proc),
        lpszClassName: label_class.into(),
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        ..Default::default()
    };
    RegisterClassW(&label_wc);

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name, w!("StonemiteOverlay"), WS_POPUP,
        0, 0, 0, 0, None, None, None, None,
    ).expect("Failed to create overlay window");

    let label_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        label_class, w!("StonemiteLabel"), WS_POPUP,
        0, 0, 0, 0, None, None, None, None,
    ).expect("Failed to create label window");

    let _ = SetLayeredWindowAttributes(label_hwnd, None, 255, LWA_ALPHA);

    let hook = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
        None, Some(foreground_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
    );

    let cfg = config::Config::load();
    *state() = Some(OverlayState {
        overlay_hwnd: hwnd,
        thumbnails: Vec::new(),
        eq_windows: Vec::new(),
        pip_order: Vec::new(),
        active_pid: None,
        hovered_index: None,
        monitor_rect: RECT::default(),
        strip_width: 0,
        event_hook: hook,
        context_menu_target_pid: None,
        context_menu_candidates: Vec::new(),
        active_label_hwnd: label_hwnd,
        active_label_text: String::new(),
        active_label_color: LABEL_COLORS[0],
        drag: None,
        drop_target: None,
        hidden_by_user: false,
        dpi_scale: get_dpi_scale(),
        pip_edge: cfg.pip_edge,
        resize_drag: None,
        custom_strip_width: cfg.pip_strip_width.map(|v| v as i32),
        strip_height: 0,
    });

    hwnd
}

pub fn poll() {
    unsafe { poll_inner() }
}

unsafe fn poll_inner() {
    let Some(s) = state().as_mut() else { return };

    let new_windows = eq_windows::find_eq_windows();
    let old_pids: HashSet<u32> = s.eq_windows.iter().map(|w| w.pid).collect();
    let new_pids: HashSet<u32> = new_windows.iter().map(|w| w.pid).collect();

    if old_pids == new_pids {
        // Same set of PIDs — just update HWNDs (they can change if window is recreated).
        for nw in &new_windows {
            if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
                ow.hwnd = nw.hwnd;
            }
        }
        return;
    }

    // PIDs changed — handle additions and removals.
    let added: Vec<u32> = new_pids.difference(&old_pids).copied().collect();
    let removed: Vec<u32> = old_pids.difference(&new_pids).copied().collect();

    // Remove gone PIDs.
    for pid in &removed {
        s.eq_windows.retain(|w| w.pid != *pid);
        s.pip_order.retain(|p| *p != *pid);
        if s.active_pid == Some(*pid) {
            // Active window gone — promote first PiP if available.
            s.active_pid = s.pip_order.first().copied();
            if let Some(promoted) = s.active_pid {
                s.pip_order.retain(|p| *p != promoted);
            }
        }
    }

    // Determine which EQ window is actually in the foreground.
    let fg_hwnd = GetForegroundWindow();
    let fg_pid = new_windows.iter().find(|w| w.hwnd == fg_hwnd).map(|w| w.pid);

    // Add new PIDs.
    for pid in &added {
        let nw = new_windows.iter().find(|w| w.pid == *pid).unwrap();
        let number = next_available_number(&s.eq_windows);
        s.eq_windows.push(EqWindow {
            hwnd: nw.hwnd,
            pid: nw.pid,
            number,
            character: None,
            server: None,
        });
        if s.active_pid.is_none() {
            // Prefer the actual foreground window as active.
            if fg_pid == Some(nw.pid) || fg_pid.is_none() {
                s.active_pid = Some(nw.pid);
            } else {
                s.pip_order.push(nw.pid);
            }
        } else {
            s.pip_order.push(nw.pid);
        }
    }

    // If active_pid is still None (foreground window wasn't in the added set),
    // pick the first pip.
    if s.active_pid.is_none() {
        if let Some(first) = s.pip_order.first().copied() {
            s.active_pid = Some(first);
            s.pip_order.retain(|p| *p != first);
        }
    }

    // If the foreground EQ window ended up in pip_order, swap it to be active.
    if let Some(fg) = fg_pid {
        if s.active_pid != Some(fg) {
            if let Some(pos) = s.pip_order.iter().position(|p| *p == fg) {
                if let Some(old_active) = s.active_pid {
                    s.pip_order[pos] = old_active;
                } else {
                    s.pip_order.remove(pos);
                }
                s.active_pid = Some(fg);
            }
        }
    }

    // Cap to MAX_PIPS.
    s.pip_order.truncate(MAX_PIPS);

    // Update HWNDs for existing windows.
    for nw in &new_windows {
        if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
            ow.hwnd = nw.hwnd;
        }
    }

    rebuild_thumbnails(s);
    update_visibility(s);
}

fn format_label(w: &EqWindow) -> String {
    let name = w.character.as_deref().unwrap_or("(right-click)");
    format!("{}: {}", w.number, name)
}

unsafe fn show_char_menu(s: &mut OverlayState, target_pid: u32, screen_pt: POINT) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let candidates = eq_characters::find_active_characters(&eq_dir, Duration::from_secs(86400));

    let hmenu = CreatePopupMenu().unwrap();

    // Character assignment submenu, grouped by server.
    let char_menu = CreatePopupMenu().unwrap();

    // Collect unique servers in order of first appearance.
    let mut servers: Vec<String> = Vec::new();
    for c in &candidates {
        if !servers.contains(&c.server) {
            servers.push(c.server.clone());
        }
    }

    if servers.len() == 1 {
        // Single server — flat list, no sub-submenu needed.
        for (i, c) in candidates.iter().enumerate() {
            let label = format!("{}\0", c.character);
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(
                char_menu, MF_STRING,
                (IDM_CHAR_BASE + i as u32) as usize,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
    } else {
        // Multiple servers — one submenu per server.
        for server in &servers {
            let server_menu = CreatePopupMenu().unwrap();
            for (i, c) in candidates.iter().enumerate() {
                if c.server != *server { continue; }
                let label = format!("{}\0", c.character);
                let wide: Vec<u16> = label.encode_utf16().collect();
                let _ = AppendMenuW(
                    server_menu, MF_STRING,
                    (IDM_CHAR_BASE + i as u32) as usize,
                    windows::core::PCWSTR(wide.as_ptr()),
                );
            }
            let server_label = format!("{server}\0");
            let wide: Vec<u16> = server_label.encode_utf16().collect();
            let _ = AppendMenuW(char_menu, MF_POPUP, server_menu.0 as usize,
                windows::core::PCWSTR(wide.as_ptr()));
        }
    }

    if !candidates.is_empty() {
        let assign_label: Vec<u16> = "Assign character\0".encode_utf16().collect();
        let _ = AppendMenuW(hmenu, MF_POPUP, char_menu.0 as usize,
            windows::core::PCWSTR(assign_label.as_ptr()));
    }

    // Number reassignment submenu.
    let num_menu = CreatePopupMenu().unwrap();
    for n in 1..=s.eq_windows.len() {
        let label = format!("#{n}\0");
        let wide: Vec<u16> = label.encode_utf16().collect();
        let _ = AppendMenuW(
            num_menu, MF_STRING,
            (IDM_NUMBER_BASE + n as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
    let num_label: Vec<u16> = "Assign number\0".encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_POPUP, num_menu.0 as usize,
        windows::core::PCWSTR(num_label.as_ptr()));

    // PiP Edge submenu.
    let edge_menu = CreatePopupMenu().unwrap();
    let edge_options = [
        (config::PipEdge::Right, "Right"),
        (config::PipEdge::Left, "Left"),
        (config::PipEdge::Top, "Top"),
        (config::PipEdge::Bottom, "Bottom"),
    ];
    for (i, (edge, label)) in edge_options.iter().enumerate() {
        let text = format!("{label}\0");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let flags = if *edge == s.pip_edge { MF_STRING | MF_CHECKED } else { MF_STRING };
        let _ = AppendMenuW(
            edge_menu, flags,
            (IDM_EDGE_BASE + i as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
    let edge_label: Vec<u16> = "PiP edge\0".encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_POPUP, edge_menu.0 as usize,
        windows::core::PCWSTR(edge_label.as_ptr()));

    // Hide overlay item with hotkey hint.
    let hide_label = format!("Hide overlay\t{}\0", cfg.hide_hotkey);
    let hide_wide: Vec<u16> = hide_label.encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_STRING, IDM_HIDE_OVERLAY as usize,
        windows::core::PCWSTR(hide_wide.as_ptr()));

    s.context_menu_target_pid = Some(target_pid);
    s.context_menu_candidates = candidates;

    let _ = SetForegroundWindow(s.overlay_hwnd);
    let _ = TrackPopupMenu(hmenu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        screen_pt.x, screen_pt.y, 0, s.overlay_hwnd, None);
    let _ = DestroyMenu(hmenu);
    let _ = PostMessageW(s.overlay_hwnd, WM_NULL, WPARAM(0), LPARAM(0));
}

unsafe fn handle_char_assign(s: &mut OverlayState, cmd_id: u32) {
    let char_idx = (cmd_id - IDM_CHAR_BASE) as usize;
    let Some(target_pid) = s.context_menu_target_pid.take() else { return };
    let candidates = std::mem::take(&mut s.context_menu_candidates);

    let Some(candidate) = candidates.get(char_idx) else { return };

    if let Some(w) = s.eq_windows.iter_mut().find(|w| w.pid == target_pid) {
        w.character = Some(candidate.character.clone());
        w.server = Some(candidate.server.clone());
    }

    rebuild_thumbnails(s);
}

unsafe fn handle_number_assign(s: &mut OverlayState, new_number: usize) {
    let Some(target_pid) = s.context_menu_target_pid.take() else { return };
    let _ = std::mem::take(&mut s.context_menu_candidates);

    // If another window already has this number, swap numbers.
    let old_number = s.eq_windows.iter().find(|w| w.pid == target_pid).map(|w| w.number).unwrap_or(0);
    if let Some(other) = s.eq_windows.iter_mut().find(|w| w.number == new_number && w.pid != target_pid) {
        other.number = old_number;
    }
    if let Some(w) = s.eq_windows.iter_mut().find(|w| w.pid == target_pid) {
        w.number = new_number;
    }

    rebuild_thumbnails(s);
}

unsafe fn handle_edge_assign(s: &mut OverlayState, cmd_id: u32) {
    let edge = match cmd_id - IDM_EDGE_BASE {
        0 => config::PipEdge::Right,
        1 => config::PipEdge::Left,
        2 => config::PipEdge::Top,
        3 => config::PipEdge::Bottom,
        _ => return,
    };
    // If switching orientation, reset custom size.
    let old_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
    let new_vertical = matches!(edge, config::PipEdge::Right | config::PipEdge::Left);
    if old_vertical != new_vertical {
        s.custom_strip_width = None;
    }
    s.pip_edge = edge;
    // Persist to config.
    let mut cfg = config::Config::load();
    cfg.pip_edge = edge;
    if old_vertical != new_vertical {
        cfg.pip_strip_width = None;
    }
    let _ = cfg.save();
    rebuild_thumbnails(s);
    update_visibility(s);
}

unsafe fn swap_to(pip_index: usize) {
    let Some(s) = state().as_mut() else { return };

    if pip_index >= s.pip_order.len() {
        return;
    }

    let Some(old_active_pid) = s.active_pid else { return };
    let new_active_pid = s.pip_order[pip_index];

    // Swap: new active goes to foreground, old active takes the PiP slot.
    s.pip_order[pip_index] = old_active_pid;
    s.active_pid = Some(new_active_pid);

    // Bring new active window to foreground.
    if let Some(w) = s.eq_windows.iter().find(|w| w.pid == new_active_pid) {
        let _ = SetWindowPos(w.hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(w.hwnd);
    }

    // Rebuild thumbnails (pip_order positions unchanged, just content of one slot changed).
    rebuild_thumbnails(s);
    let _ = ShowWindow(s.overlay_hwnd, SW_SHOWNOACTIVATE);
}

unsafe fn rebuild_thumbnails(s: &mut OverlayState) {
    for entry in s.thumbnails.drain(..) {
        let _ = DwmUnregisterThumbnail(entry.thumb);
    }
    s.hovered_index = None;

    if s.pip_order.is_empty() {
        let _ = ShowWindow(s.overlay_hwnd, SW_HIDE);
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }

    let reference = s.eq_windows.first().map(|w| w.hwnd);
    s.monitor_rect = eq_windows::get_monitor_work_area(reference);

    let d = s.dpi_scale;
    let gap = dpi(THUMB_GAP, d);
    let border = dpi(BORDER_WIDTH, d);
    let label_h = dpi(LABEL_HEIGHT, d);

    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
    let n = s.pip_order.len() as i32;

    let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);

    let (strip_w, strip_h, strip_x, strip_y, cell_w, cell_h, thumb_w, thumb_h);

    if is_vertical {
        // Vertical stacking (Right/Left).
        let max_strip_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

        // Auto-size: fit all PiPs vertically, capped at max.
        let auto_max_thumb_w = max_strip_w - 2 * border;
        let auto_max_thumb_h = (auto_max_thumb_w as f64 * 9.0 / 16.0).round() as i32;
        let auto_max_cell_h = (mon_h - (n - 1).max(0) * gap) / n;
        let auto_thumb_h = (auto_max_cell_h - label_h - 2 * border).clamp(dpi(40, d), auto_max_thumb_h);
        let auto_thumb_w = (auto_thumb_h as f64 * 16.0 / 9.0).round() as i32;
        let auto_strip_w = auto_thumb_w + 2 * border;

        let effective_strip_w = if let Some(custom_w) = s.custom_strip_width {
            custom_w.clamp(min_strip_w, max_strip_w)
        } else {
            auto_strip_w
        };

        thumb_w = effective_strip_w - 2 * border;
        thumb_h = (thumb_w as f64 * 9.0 / 16.0).round() as i32;
        cell_w = effective_strip_w;
        cell_h = thumb_h + label_h + 2 * border;
        strip_w = cell_w;
        strip_h = n * cell_h + (n - 1).max(0) * gap;
        strip_x = match s.pip_edge {
            config::PipEdge::Left => s.monitor_rect.left,
            _ => s.monitor_rect.right - strip_w,
        };
        strip_y = s.monitor_rect.top;
    } else {
        // Horizontal stacking (Top/Bottom).
        let max_strip_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

        // Auto-size.
        let auto_max_thumb_h = max_strip_h - label_h - 2 * border;
        let auto_max_thumb_w = (auto_max_thumb_h as f64 * 16.0 / 9.0).round() as i32;
        let auto_max_cell_w = (mon_w - (n - 1).max(0) * gap) / n;
        let auto_thumb_w = (auto_max_cell_w - 2 * border).clamp(dpi(60, d), auto_max_thumb_w);
        let auto_thumb_h = (auto_thumb_w as f64 * 9.0 / 16.0).round() as i32;
        let auto_cell_h = auto_thumb_h + label_h + 2 * border;

        let effective_strip_h = if let Some(custom_h) = s.custom_strip_width {
            custom_h.clamp(min_strip_h, max_strip_h)
        } else {
            auto_cell_h
        };

        thumb_h = effective_strip_h - label_h - 2 * border;
        thumb_w = (thumb_h as f64 * 16.0 / 9.0).round() as i32;
        cell_w = thumb_w + 2 * border;
        cell_h = effective_strip_h;
        strip_w = n * cell_w + (n - 1).max(0) * gap;
        strip_h = cell_h;
        strip_x = s.monitor_rect.right - strip_w;
        strip_y = match s.pip_edge {
            config::PipEdge::Top => s.monitor_rect.top,
            _ => s.monitor_rect.bottom - strip_h,
        };
    }

    s.strip_width = strip_w;
    s.strip_height = strip_h;

    let _ = SetWindowPos(
        s.overlay_hwnd, HWND_TOP,
        strip_x, strip_y, strip_w, strip_h,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );

    // Render PiP thumbnails in pip_order (stable positions).
    for (i, &pid) in s.pip_order.iter().enumerate() {
        let Some(eq_win) = s.eq_windows.iter().find(|w| w.pid == pid) else { continue };

        let (x_offset, y_offset) = if is_vertical {
            (0, i as i32 * (cell_h + gap))
        } else {
            (i as i32 * (cell_w + gap), 0)
        };

        let cell_rect = RECT {
            left: x_offset, top: y_offset,
            right: x_offset + cell_w, bottom: y_offset + cell_h,
        };

        let thumb_rect = RECT {
            left: cell_rect.left + border,
            top: cell_rect.top + border + label_h,
            right: cell_rect.right - border,
            bottom: cell_rect.bottom - border,
        };

        let Ok(thumb) = DwmRegisterThumbnail(s.overlay_hwnd, eq_win.hwnd) else { continue };

        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: thumb_rect,
            fVisible: true.into(),
            opacity: THUMB_OPACITY_NORMAL,
            fSourceClientAreaOnly: true.into(),
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);

        s.thumbnails.push(ThumbnailEntry {
            eq_hwnd: eq_win.hwnd,
            pid,
            thumb,
            cell_rect,
            label: format_label(eq_win),
            number: eq_win.number,
        });
    }

    update_active_label(s);
    let _ = InvalidateRect(s.overlay_hwnd, None, true);
}

unsafe fn update_active_label(s: &mut OverlayState) {
    let active = s.active_pid.and_then(|pid| s.eq_windows.iter().find(|w| w.pid == pid));
    s.active_label_text = active.map(|w| format_label(w)).unwrap_or_default();
    s.active_label_color = active.map(|w| color_for_number(w.number)).unwrap_or(LABEL_COLORS[0]);

    if s.active_label_text.is_empty() {
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }

    let active_hwnd = active.unwrap().hwnd;
    let mut rect = RECT::default();
    let _ = GetClientRect(active_hwnd, &mut rect);
    let mut top_left = POINT { x: rect.left, y: rect.top };
    let _ = ClientToScreen(active_hwnd, &mut top_left);

    let label_h = dpi(LABEL_HEIGHT, s.dpi_scale);
    let text_width = (s.active_label_text.len() as i32 * (label_h - dpi(8, s.dpi_scale)) * 3) / 5 + label_h;
    let _ = SetWindowPos(
        s.active_label_hwnd, HWND_TOPMOST,
        top_left.x, top_left.y, text_width, label_h,
        SWP_NOACTIVATE,
    );

    let _ = InvalidateRect(s.active_label_hwnd, None, true);
}

/// Check if a client-coordinate point is in the resize grab zone (interior edge of the strip).
unsafe fn resize_hit_test(s: &OverlayState, pt: POINT) -> bool {
    if s.thumbnails.is_empty() { return false; }
    let handle_w = dpi(RESIZE_HANDLE_WIDTH, s.dpi_scale);
    match s.pip_edge {
        config::PipEdge::Right => pt.x < handle_w,
        config::PipEdge::Left => pt.x >= s.strip_width - handle_w,
        config::PipEdge::Top => pt.y >= s.strip_height - handle_w,
        config::PipEdge::Bottom => pt.y < handle_w,
    }
}

unsafe fn hit_test(s: &OverlayState, pt: POINT) -> Option<usize> {
    for (i, entry) in s.thumbnails.iter().enumerate() {
        let r = &entry.cell_rect;
        if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
            return Some(i);
        }
    }
    None
}

unsafe fn update_hover(s: &mut OverlayState, new_index: Option<usize>) {
    if s.hovered_index == new_index { return; }

    // Dim old thumbnail.
    if let Some(old) = s.hovered_index {
        if let Some(entry) = s.thumbnails.get(old) {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_NORMAL, ..Default::default()
            };
            let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
        }
    }

    // Brighten new thumbnail.
    if let Some(new) = new_index {
        if let Some(entry) = s.thumbnails.get(new) {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_HOVER, ..Default::default()
            };
            let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
        }
    }

    s.hovered_index = new_index;
    let _ = InvalidateRect(s.overlay_hwnd, None, true);
}

unsafe fn paint_label(hwnd: HWND, text: &str, bg_color: u32) {
    let d = state().as_ref().map_or(1.0, |s| s.dpi_scale);
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let bg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(bg_color));
    let _ = FillRect(hdc, &ps.rcPaint, bg_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg_brush);

    let font = CreateFontW(
        dpi(LABEL_HEIGHT - 8, d), 0, 0, 0, FW_BOLD.0 as i32,
        0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    rc.left += dpi(8, d);
    rc.top += dpi(4, d);
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut rc, DT_LEFT | DT_SINGLELINE | DT_TOP);

    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn paint_overlay(hwnd: HWND) {
    let Some(s) = state().as_ref() else { return };
    let d = s.dpi_scale;

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let black_brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);
    let _ = FillRect(hdc, &ps.rcPaint, black_brush);

    let font = CreateFontW(
        dpi(LABEL_HEIGHT - 8, d), 0, 0, 0, FW_BOLD.0 as i32,
        0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    // Determine drag state for visual feedback.
    let is_dragging = s.drag.as_ref().map_or(false, |d| d.dragging);
    let drag_from = s.drag.as_ref().map(|d| d.from_index);
    let drag_to = s.drop_target;

    for (i, entry) in s.thumbnails.iter().enumerate() {
        let is_drag_source = is_dragging && drag_from == Some(i);
        let is_drop_target = is_dragging && drag_to == Some(i) && drag_from != Some(i);

        // Dim the source thumbnail being dragged.
        if is_drag_source {
            let dim_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00333333));
            let _ = FillRect(hdc, &entry.cell_rect, dim_brush);
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(dim_brush);
        }

        let border = dpi(BORDER_WIDTH, d);
        let label_h = dpi(LABEL_HEIGHT, d);

        // Bright yellow border on drop target to indicate swap.
        if is_drop_target {
            let swap_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x0000CCFF)); // yellow
            let _ = FrameRect(hdc, &entry.cell_rect, swap_brush);
            for inset in 1..border + 1 {
                let r = RECT {
                    left: entry.cell_rect.left + inset, top: entry.cell_rect.top + inset,
                    right: entry.cell_rect.right - inset, bottom: entry.cell_rect.bottom - inset,
                };
                let _ = FrameRect(hdc, &r, swap_brush);
            }
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(swap_brush);
        } else if s.hovered_index == Some(i) && !is_dragging {
            // Normal hover highlight (only when not dragging).
            let white_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00FFFFFF));
            let _ = FrameRect(hdc, &entry.cell_rect, white_brush);
            for inset in 1..border {
                let r = RECT {
                    left: entry.cell_rect.left + inset, top: entry.cell_rect.top + inset,
                    right: entry.cell_rect.right - inset, bottom: entry.cell_rect.bottom - inset,
                };
                let _ = FrameRect(hdc, &r, white_brush);
            }
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(white_brush);
        }

        // Solid colored label bar at top of cell.
        let bg_color = color_for_number(entry.number);
        let label_bg_rect = RECT {
            left: entry.cell_rect.left + border,
            top: entry.cell_rect.top + border,
            right: entry.cell_rect.right - border,
            bottom: entry.cell_rect.top + border + label_h,
        };
        let label_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(bg_color));
        let _ = FillRect(hdc, &label_bg_rect, label_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(label_brush);

        let mut label_rect = RECT {
            left: label_bg_rect.left + dpi(8, d),
            top: label_bg_rect.top + dpi(4, d),
            right: label_bg_rect.right,
            bottom: label_bg_rect.bottom,
        };
        let mut text: Vec<u16> = entry.label.encode_utf16().collect();
        let _ = DrawTextW(hdc, &mut text, &mut label_rect, DT_LEFT | DT_SINGLELINE | DT_TOP);
    }

    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn label_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let (text, color) = state()
                .as_ref()
                .map(|s| (s.active_label_text.clone(), s.active_label_color))
                .unwrap_or((String::new(), LABEL_COLORS[0]));
            if !text.is_empty() {
                paint_label(hwnd, &text, color);
            } else {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let _ = SetLayeredWindowAttributes(hwnd, None, 25, LWA_ALPHA);
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let _ = SetLayeredWindowAttributes(hwnd, None, 255, LWA_ALPHA);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // Right-click on the main window label opens context menu.
            if let Some(s) = state().as_mut() {
                if let Some(active_pid) = s.active_pid {
                    let mut pt = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = ClientToScreen(hwnd, &mut pt);
                    show_char_menu(s, active_pid, pt);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN => {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ClientToScreen(hwnd, &mut pt);
            let below = WindowFromPoint(pt);
            if !below.is_invalid() && below != hwnd {
                let _ = PostMessageW(below, msg, wparam, LPARAM(
                    (pt.x as i16 as u16 as isize) | ((pt.y as i16 as u16 as isize) << 16)
                ));
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            // Show resize cursor when hovering the interior edge.
            if (lparam.0 & 0xFFFF) as u32 == 1 /* HTCLIENT */ {
                if let Some(s) = state().as_ref() {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(hwnd, &mut pt);
                    if resize_hit_test(s, pt) {
                        let cursor_id = if matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left) {
                            IDC_SIZEWE
                        } else {
                            IDC_SIZENS
                        };
                        let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
                }
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        WM_PAINT => {
            paint_overlay(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };

            // Handle resize drag.
            if let Some(ref resize) = s.resize_drag {
                let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                let new_size = if is_vertical {
                    let delta = pt.x - resize.start_pt.x;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Right) { -1 } else { 1 };
                    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
                    let min_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (resize.start_size + sign * delta).clamp(min_w, max_w)
                } else {
                    let delta = pt.y - resize.start_pt.y;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Bottom) { -1 } else { 1 };
                    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
                    let min_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (resize.start_size + sign * delta).clamp(min_h, max_h)
                };
                if Some(new_size) != s.custom_strip_width {
                    s.custom_strip_width = Some(new_size);
                    rebuild_thumbnails(s);
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                return LRESULT(0);
            }

            // Check if we've started dragging (exceeded threshold).
            if let Some(ref mut drag) = s.drag {
                if !drag.dragging {
                    let dx = (pt.x - drag.start_pt.x).abs();
                    let dy = (pt.y - drag.start_pt.y).abs();
                    let threshold = dpi(DRAG_THRESHOLD, s.dpi_scale);
                    if dx > threshold || dy > threshold {
                        drag.dragging = true;
                        let _ = SetCapture(hwnd);
                        // Dim the source thumbnail.
                        if let Some(entry) = s.thumbnails.get(drag.from_index) {
                            let props = DWM_THUMBNAIL_PROPERTIES {
                                dwFlags: DWM_TNP_OPACITY, opacity: 80, ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
                        }
                    }
                }
                if drag.dragging {
                    let new_target = hit_test(s, pt);
                    if s.drop_target != new_target {
                        s.drop_target = new_target;
                        let _ = InvalidateRect(s.overlay_hwnd, None, true);
                    }
                }
            }

            let idx = hit_test(s, pt);
            update_hover(s, idx);
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(s) = state().as_mut() {
                if s.drag.as_ref().map_or(true, |d| !d.dragging) {
                    s.drag = None;
                    s.drop_target = None;
                }
                update_hover(s, None);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            if resize_hit_test(s, pt) {
                let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                let start_size = if is_vertical { s.strip_width } else { s.strip_height };
                s.resize_drag = Some(ResizeDragState { start_pt: pt, start_size });
                let _ = SetCapture(hwnd);
            } else if let Some(idx) = hit_test(s, pt) {
                s.drag = Some(DragState {
                    from_index: idx,
                    start_pt: pt,
                    dragging: false,
                });
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ReleaseCapture();

            // Finalize resize drag.
            if s.resize_drag.take().is_some() {
                let mut cfg = config::Config::load();
                cfg.pip_strip_width = s.custom_strip_width.map(|v| v as u32);
                let _ = cfg.save();
                return LRESULT(0);
            }

            let drag = s.drag.take();
            s.drop_target = None;

            if let Some(drag) = drag {
                if drag.dragging {
                    // Restore source thumbnail opacity.
                    if let Some(entry) = s.thumbnails.get(drag.from_index) {
                        let props = DWM_THUMBNAIL_PROPERTIES {
                            dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_NORMAL, ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
                    }
                    // Drag completed — reorder pip_order.
                    if let Some(to_index) = hit_test(s, pt) {
                        if to_index != drag.from_index && to_index < s.pip_order.len() && drag.from_index < s.pip_order.len() {
                            s.pip_order.swap(drag.from_index, to_index);
                            rebuild_thumbnails(s);
                        }
                    }
                } else {
                    // Simple click — activate window.
                    if let Some(idx) = hit_test(s, pt) {
                        // Need to drop mutable borrow before swap_to re-borrows.
                        drop(s);
                        swap_to(idx);
                    }
                }
            } else if let Some(idx) = hit_test(s, pt) {
                drop(s);
                swap_to(idx);
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            if resize_hit_test(s, pt) {
                s.custom_strip_width = None;
                let mut cfg = config::Config::load();
                cfg.pip_strip_width = None;
                let _ = cfg.save();
                rebuild_thumbnails(s);
                update_visibility(s);
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            if let Some(idx) = hit_test(s, pt) {
                if let Some(entry) = s.thumbnails.get(idx) {
                    let pid = entry.pid;
                    let mut screen_pt = pt;
                    let _ = ClientToScreen(hwnd, &mut screen_pt);
                    show_char_menu(s, pid, screen_pt);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = (wparam.0 & 0xFFFF) as u32;
            if let Some(s) = state().as_mut() {
                if cmd_id == IDM_HIDE_OVERLAY {
                    s.hidden_by_user = true;
                    update_visibility(s);
                } else if cmd_id >= IDM_EDGE_BASE && cmd_id < IDM_EDGE_BASE + 4 {
                    handle_edge_assign(s, cmd_id);
                } else if cmd_id >= IDM_NUMBER_BASE && cmd_id < IDM_NUMBER_BASE + 100 {
                    let number = (cmd_id - IDM_NUMBER_BASE) as usize;
                    handle_number_assign(s, number);
                } else if cmd_id >= IDM_CHAR_BASE && cmd_id < IDM_CHAR_BASE + 100 {
                    handle_char_assign(s, cmd_id);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(s) = state().as_mut() {
                for entry in s.thumbnails.drain(..) {
                    let _ = DwmUnregisterThumbnail(entry.thumb);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Returns true if the foreground window is an EQ window or the overlay itself.
pub fn is_eq_active() -> bool {
    unsafe {
        let Some(s) = state().as_ref() else { return false };
        let fg = GetForegroundWindow();
        is_eq_or_overlay(fg, &s.eq_windows, s.overlay_hwnd, s.active_label_hwnd)
    }
}

/// Toggle user-hidden state for the overlay.
/// Returns true if the overlay is currently visible (not hidden by user).
pub fn is_visible() -> bool {
    unsafe {
        state().as_ref().map_or(true, |s| !s.hidden_by_user)
    }
}

pub fn toggle_hidden() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        s.hidden_by_user = !s.hidden_by_user;
        update_visibility(s);
    }
}

/// Reload config into overlay state and rebuild the strip layout.
pub fn force_rebuild() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        let cfg = config::Config::load();
        s.pip_edge = cfg.pip_edge;
        s.custom_strip_width = cfg.pip_strip_width.map(|v| v as i32);
        rebuild_thumbnails(s);
        update_visibility(s);
    }
}

pub fn cleanup() {
    unsafe {
        if let Some(s) = state().as_mut() {
            if !s.event_hook.is_invalid() {
                let _ = UnhookWinEvent(s.event_hook);
            }
            for entry in s.thumbnails.drain(..) {
                let _ = DwmUnregisterThumbnail(entry.thumb);
            }
            let _ = DestroyWindow(s.active_label_hwnd);
            let _ = DestroyWindow(s.overlay_hwnd);
        }
        *state() = None;
    }
}
