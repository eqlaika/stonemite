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
    BeginPaint, ClientToScreen, CreateFontW, CreateSolidBrush, DrawTextW, EndPaint, FillRect,
    FrameRect, GetStockObject, InvalidateRect, SelectObject, SetBkMode, SetTextColor,
    BACKGROUND_MODE, DT_LEFT, DT_SINGLELINE, DT_TOP, FW_BOLD, HBRUSH, BLACK_BRUSH,
    PAINTSTRUCT,
};
use windows::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::{config, eq_characters, eq_windows};
use crate::eq_windows::EqWindow;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Gap between thumbnails in strip layout (pixels).
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
/// Menu ID for edit/lock layout toggle.
const IDM_EDIT_MODE: u32 = 7200;
/// Menu ID for resetting to auto layout.
const IDM_RESET_LAYOUT: u32 = 7300;

/// Distinct background colors for per-number labels (COLORREF = 0x00BBGGRR).
const LABEL_COLORS: &[u32] = &[
    0x00F6A893, // soft blue     (rgb #93A8F6)
    0x0098D6A3, // mint green    (rgb #A3D698)
    0x009393F4, // soft rose     (rgb #F49393)
    0x0080CEF5, // warm peach    (rgb #F5CE80)
    0x00F4A8C8, // soft lavender (rgb #C8A8F4)
    0x00D4E898, // pale cyan     (rgb #98E8D4)
];

/// Minimum pixel distance before a click becomes a drag.
const DRAG_THRESHOLD: i32 = 8;

/// Snap distance in pixels for monitor edges and PiP-to-PiP snapping.
const SNAP_DISTANCE: i32 = 12;

/// Pixel zone around PiP edges for resize detection in edit mode.
const RESIZE_ZONE: i32 = 8;

/// Color for edit mode border indicator (bright cyan, COLORREF).
const EDIT_BORDER_COLOR: u32 = 0x00FFFF00;

/// VK_SHIFT virtual key code.
const VK_SHIFT_CODE: i32 = 0x10;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

struct PipWindowEntry {
    hwnd: HWND,
    pid: u32,
    thumb: isize,
    label: String,
    number: usize,
    hovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResizeEdge {
    N, S, E, W, NE, NW, SE, SW,
}

struct MoveDragState {
    pip_index: usize,
    start_cursor: POINT,
    start_rect: RECT,
}

struct PipResizeDragState {
    pip_index: usize,
    edge: ResizeEdge,
    start_cursor: POINT,
    start_rect: RECT,
}

struct StripResizeDragState {
    #[allow(dead_code)]
    pip_index: usize,
    start_pt: POINT,
    start_size: i32,
}

struct ReorderDragState {
    from_index: usize,
    start_pt: POINT,
    dragging: bool,
}

struct OverlayState {
    pip_windows: Vec<PipWindowEntry>,
    /// All tracked EQ windows with stable numbers and character assignments.
    eq_windows: Vec<EqWindow>,
    /// PIDs in PiP strip order. Positions are fixed; on swap, two PIDs exchange.
    pip_order: Vec<u32>,
    /// PID of the currently active (foreground) window.
    active_pid: Option<u32>,
    /// Floating label window for the active (foreground) EQ window.
    active_label_hwnd: HWND,
    active_label_text: String,
    active_label_color: u32,
    event_hook: HWINEVENTHOOK,
    monitor_rect: RECT,
    dpi_scale: f64,
    /// Which screen edge the PiP strip is anchored to.
    pip_edge: config::PipEdge,
    /// User-configured strip width override (pixels). None = auto.
    custom_strip_width: Option<i32>,
    /// User has toggled overlay hidden via hotkey.
    hidden_by_user: bool,
    context_menu_target_pid: Option<u32>,
    context_menu_candidates: Vec<eq_characters::CharCandidate>,
    /// Edit mode: PiPs can be freely moved/resized.
    edit_mode: bool,
    /// Snap grid size in pixels (0 = disabled).
    snap_grid: i32,
    /// True when pip_positions is non-empty in config.
    has_custom_positions: bool,
    /// Move drag state (edit mode).
    move_drag: Option<MoveDragState>,
    /// Per-PiP resize drag state (edit mode).
    pip_resize_drag: Option<PipResizeDragState>,
    /// Strip resize drag state (auto-layout use mode).
    strip_resize_drag: Option<StripResizeDragState>,
    /// Reorder drag state (use mode drag-to-swap).
    reorder_drag: Option<ReorderDragState>,
    /// Visual indicator: index where the dragged item would be dropped.
    drop_target: Option<usize>,
    /// Current strip dimensions (for strip resize).
    strip_width: i32,
    strip_height: i32,
}

// ---------------------------------------------------------------------------
// Static state
// ---------------------------------------------------------------------------

struct OverlayCell(UnsafeCell<Option<OverlayState>>);
unsafe impl Sync for OverlayCell {}

static OVERLAY: OverlayCell = OverlayCell(UnsafeCell::new(None));

fn state() -> &'static mut Option<OverlayState> {
    unsafe { &mut *OVERLAY.0.get() }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the lowest positive integer not already used by any tracked window.
fn next_available_number(eq_windows: &[EqWindow]) -> usize {
    let mut n = 1;
    while eq_windows.iter().any(|w| w.number == n) {
        n += 1;
    }
    n
}

unsafe fn get_dpi_scale(hwnd: HWND) -> f64 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    let dpi = GetDpiForWindow(hwnd);
    if dpi > 0 {
        return dpi as f64 / 96.0;
    }
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSY};
    let dc = GetDC(HWND::default());
    let val = GetDeviceCaps(dc, LOGPIXELSY);
    let _ = ReleaseDC(HWND::default(), dc);
    val as f64 / 96.0
}

fn dpi(val: i32, scale: f64) -> i32 {
    (val as f64 * scale).round() as i32
}

fn color_for_number(number: usize) -> u32 {
    if number == 0 { return LABEL_COLORS[0]; }
    LABEL_COLORS[(number - 1) % LABEL_COLORS.len()]
}

fn format_label(w: &EqWindow) -> String {
    let name = w.character.as_deref().unwrap_or("(right-click)");
    format!("{}: {}", w.number, name)
}

#[allow(dead_code)]
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

fn is_our_window(hwnd: HWND, s: &OverlayState) -> bool {
    if hwnd == s.active_label_hwnd { return true; }
    s.pip_windows.iter().any(|pw| pw.hwnd == hwnd)
}

fn is_eq_or_ours(hwnd: HWND, s: &OverlayState) -> bool {
    is_our_window(hwnd, s) || s.eq_windows.iter().any(|w| w.hwnd == hwnd)
}

/// Detect resize edge/corner in edit mode from client-area point.
fn edit_resize_edge_hit_test(pt: POINT, w: i32, h: i32, zone: i32) -> Option<ResizeEdge> {
    let on_left = pt.x < zone;
    let on_right = pt.x >= w - zone;
    let on_top = pt.y < zone;
    let on_bottom = pt.y >= h - zone;
    match (on_left, on_right, on_top, on_bottom) {
        (true, _, true, _) => Some(ResizeEdge::NW),
        (true, _, _, true) => Some(ResizeEdge::SW),
        (_, true, true, _) => Some(ResizeEdge::NE),
        (_, true, _, true) => Some(ResizeEdge::SE),
        (true, _, _, _) => Some(ResizeEdge::W),
        (_, true, _, _) => Some(ResizeEdge::E),
        (_, _, true, _) => Some(ResizeEdge::N),
        (_, _, _, true) => Some(ResizeEdge::S),
        _ => None,
    }
}

/// Check if a client-coordinate point is in the strip resize zone (interior edge).
fn strip_resize_hit_test(pt: POINT, w: i32, h: i32, pip_edge: config::PipEdge, handle_w: i32) -> bool {
    match pip_edge {
        config::PipEdge::Right => pt.x < handle_w,
        config::PipEdge::Left => pt.x >= w - handle_w,
        config::PipEdge::Top => pt.y >= h - handle_w,
        config::PipEdge::Bottom => pt.y < handle_w,
    }
}

fn cursor_for_resize_edge(edge: ResizeEdge) -> *const u16 {
    match edge {
        ResizeEdge::N | ResizeEdge::S => IDC_SIZENS.0 as *const u16,
        ResizeEdge::E | ResizeEdge::W => IDC_SIZEWE.0 as *const u16,
        ResizeEdge::NW | ResizeEdge::SE => IDC_SIZENWSE.0 as *const u16,
        ResizeEdge::NE | ResizeEdge::SW => IDC_SIZENESW.0 as *const u16,
    }
}

// ---------------------------------------------------------------------------
// Snap logic
// ---------------------------------------------------------------------------

/// Snap a position (x, y) for a window of size (w, h) to grid, monitor edges,
/// and other PiP windows. Hold Shift to bypass snapping.
fn snap_point(
    x: i32, y: i32, w: i32, h: i32,
    others: &[RECT], monitor: RECT, grid: i32,
) -> (i32, i32) {
    // Check Shift key to bypass snapping.
    let shift_down = unsafe { GetKeyState(VK_SHIFT_CODE) < 0 };
    if shift_down {
        return (x, y);
    }

    let mut sx = x;
    let mut sy = y;

    // Grid snap.
    if grid > 0 {
        sx = ((sx as f64 / grid as f64).round() as i32) * grid;
        sy = ((sy as f64 / grid as f64).round() as i32) * grid;
    }

    // Monitor edge snap.
    if (sx - monitor.left).abs() < SNAP_DISTANCE { sx = monitor.left; }
    if (sx + w - monitor.right).abs() < SNAP_DISTANCE { sx = monitor.right - w; }
    if (sy - monitor.top).abs() < SNAP_DISTANCE { sy = monitor.top; }
    if (sy + h - monitor.bottom).abs() < SNAP_DISTANCE { sy = monitor.bottom - h; }

    // PiP-to-PiP edge snap.
    for other in others {
        // Left edge of moving window → left/right edge of other.
        if (sx - other.left).abs() < SNAP_DISTANCE { sx = other.left; }
        if (sx - other.right).abs() < SNAP_DISTANCE { sx = other.right; }
        // Right edge of moving window → left/right edge of other.
        if (sx + w - other.left).abs() < SNAP_DISTANCE { sx = other.left - w; }
        if (sx + w - other.right).abs() < SNAP_DISTANCE { sx = other.right - w; }
        // Top edge.
        if (sy - other.top).abs() < SNAP_DISTANCE { sy = other.top; }
        if (sy - other.bottom).abs() < SNAP_DISTANCE { sy = other.bottom; }
        // Bottom edge.
        if (sy + h - other.top).abs() < SNAP_DISTANCE { sy = other.top - h; }
        if (sy + h - other.bottom).abs() < SNAP_DISTANCE { sy = other.bottom - h; }
    }

    (sx, sy)
}

/// Compute the correct cell height for a given cell width, maintaining 16:9
/// thumbnail aspect ratio with the label bar and border overhead.
fn aspect_height_for_width(cell_w: i32, border: i32, label_h: i32) -> i32 {
    let thumb_w = cell_w - 2 * border;
    let thumb_h = (thumb_w as f64 * 9.0 / 16.0).round() as i32;
    thumb_h + label_h + 2 * border
}

/// Compute the correct cell width for a given cell height, maintaining 16:9
/// thumbnail aspect ratio with the label bar and border overhead.
fn aspect_width_for_height(cell_h: i32, border: i32, label_h: i32) -> i32 {
    let thumb_h = cell_h - label_h - 2 * border;
    let thumb_w = (thumb_h as f64 * 16.0 / 9.0).round() as i32;
    thumb_w + 2 * border
}

/// Apply snap to a resize operation, enforcing 16:9 thumbnail aspect ratio.
/// The dragged edge(s) determine whether width or height is the driving dimension.
fn snap_resize(
    edge: ResizeEdge,
    start_rect: RECT,
    dx: i32, dy: i32,
    _others: &[RECT], _monitor: RECT, grid: i32,
    border: i32, label_h: i32,
) -> RECT {
    let shift_down = unsafe { GetKeyState(VK_SHIFT_CODE) < 0 };
    let min_w: i32 = 80;

    let mut r = start_rect;

    // Apply raw delta to the dragged edges.
    match edge {
        ResizeEdge::E | ResizeEdge::NE | ResizeEdge::SE => r.right += dx,
        ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => r.left += dx,
        _ => {}
    }
    match edge {
        ResizeEdge::S | ResizeEdge::SE | ResizeEdge::SW => r.bottom += dy,
        ResizeEdge::N | ResizeEdge::NE | ResizeEdge::NW => r.top += dy,
        _ => {}
    }

    // Grid snap the dragged edges (before aspect correction).
    if !shift_down && grid > 0 {
        let g = grid;
        match edge {
            ResizeEdge::E | ResizeEdge::NE | ResizeEdge::SE =>
                r.right = ((r.right as f64 / g as f64).round() as i32) * g,
            ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW =>
                r.left = ((r.left as f64 / g as f64).round() as i32) * g,
            _ => {}
        }
        // Only snap vertical edges when height is the driving axis (pure N/S).
        match edge {
            ResizeEdge::N => r.top = ((r.top as f64 / g as f64).round() as i32) * g,
            ResizeEdge::S => r.bottom = ((r.bottom as f64 / g as f64).round() as i32) * g,
            _ => {}
        }
    }

    // Enforce minimum width.
    let w = r.right - r.left;
    if w < min_w {
        match edge {
            ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => r.left = r.right - min_w,
            _ => r.right = r.left + min_w,
        }
    }

    // Enforce 16:9 aspect ratio on the thumbnail area.
    // Horizontal or diagonal drags: width drives height.
    // Pure vertical drags (N/S): height drives width.
    match edge {
        ResizeEdge::N => {
            // Height changed → adjust width, keep left edge fixed.
            let h = r.bottom - r.top;
            let new_w = aspect_width_for_height(h, border, label_h).max(min_w);
            r.right = r.left + new_w;
        }
        ResizeEdge::S => {
            let h = r.bottom - r.top;
            let new_w = aspect_width_for_height(h, border, label_h).max(min_w);
            r.right = r.left + new_w;
        }
        _ => {
            // Width drives height.
            let w = r.right - r.left;
            let new_h = aspect_height_for_width(w, border, label_h);
            match edge {
                ResizeEdge::NW | ResizeEdge::NE | ResizeEdge::N =>
                    r.top = r.bottom - new_h,
                _ =>
                    r.bottom = r.top + new_h,
            }
        }
    }

    r
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

pub fn init() -> HWND {
    unsafe { init_inner() }
}

unsafe fn init_inner() -> HWND {
    // Register per-PiP window class.
    let pip_class = w!("StonemitePipClass");
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(pip_wnd_proc),
        lpszClassName: pip_class.into(),
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        style: CS_DBLCLKS,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Register label window class.
    let label_class = w!("StonemiteLabelClass");
    let label_wc = WNDCLASSW {
        lpfnWndProc: Some(label_wnd_proc),
        lpszClassName: label_class.into(),
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        ..Default::default()
    };
    RegisterClassW(&label_wc);

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
    let has_custom = !cfg.pip_positions.is_empty();

    *state() = Some(OverlayState {
        pip_windows: Vec::new(),
        eq_windows: Vec::new(),
        pip_order: Vec::new(),
        active_pid: None,
        active_label_hwnd: label_hwnd,
        active_label_text: String::new(),
        active_label_color: LABEL_COLORS[0],
        event_hook: hook,
        monitor_rect: RECT::default(),
        dpi_scale: get_dpi_scale(label_hwnd),
        pip_edge: cfg.pip_edge,
        custom_strip_width: cfg.pip_strip_width.map(|v| v as i32),
        hidden_by_user: false,
        context_menu_target_pid: None,
        context_menu_candidates: Vec::new(),
        edit_mode: false,
        snap_grid: cfg.snap_grid as i32,
        has_custom_positions: has_custom,
        move_drag: None,
        pip_resize_drag: None,
        strip_resize_drag: None,
        reorder_drag: None,
        drop_target: None,
        strip_width: 0,
        strip_height: 0,
    });

    label_hwnd
}

// ---------------------------------------------------------------------------
// Poll
// ---------------------------------------------------------------------------

pub fn poll() {
    unsafe { poll_inner() }
}

unsafe fn poll_inner() {
    let Some(s) = state().as_mut() else { return };

    let new_windows = eq_windows::find_eq_windows();
    let old_pids: HashSet<u32> = s.eq_windows.iter().map(|w| w.pid).collect();
    let new_pids: HashSet<u32> = new_windows.iter().map(|w| w.pid).collect();

    if old_pids == new_pids {
        for nw in &new_windows {
            if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
                ow.hwnd = nw.hwnd;
            }
        }
        return;
    }

    let added: Vec<u32> = new_pids.difference(&old_pids).copied().collect();
    let removed: Vec<u32> = old_pids.difference(&new_pids).copied().collect();

    for pid in &removed {
        s.eq_windows.retain(|w| w.pid != *pid);
        s.pip_order.retain(|p| *p != *pid);
        if s.active_pid == Some(*pid) {
            s.active_pid = s.pip_order.first().copied();
            if let Some(promoted) = s.active_pid {
                s.pip_order.retain(|p| *p != promoted);
            }
        }
    }

    let fg_hwnd = GetForegroundWindow();
    let fg_pid = new_windows.iter().find(|w| w.hwnd == fg_hwnd).map(|w| w.pid);

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
            if fg_pid == Some(nw.pid) || fg_pid.is_none() {
                s.active_pid = Some(nw.pid);
            } else {
                s.pip_order.push(nw.pid);
            }
        } else {
            s.pip_order.push(nw.pid);
        }
    }

    if s.active_pid.is_none() {
        if let Some(first) = s.pip_order.first().copied() {
            s.active_pid = Some(first);
            s.pip_order.retain(|p| *p != first);
        }
    }

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

    s.pip_order.truncate(MAX_PIPS);

    for nw in &new_windows {
        if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
            ow.hwnd = nw.hwnd;
        }
    }

    rebuild_thumbnails(s);
    update_visibility(s);
}

// ---------------------------------------------------------------------------
// Position computation
// ---------------------------------------------------------------------------

/// Compute strip layout positions as screen-coordinate RECTs.
unsafe fn compute_strip_positions(s: &OverlayState) -> Vec<RECT> {
    let d = s.dpi_scale;
    let gap = dpi(THUMB_GAP, d);
    let border = dpi(BORDER_WIDTH, d);
    let label_h = dpi(LABEL_HEIGHT, d);

    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
    let n = s.pip_order.len() as i32;
    if n == 0 { return Vec::new(); }

    let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);

    let (strip_x, strip_y, cell_w, cell_h);

    if is_vertical {
        let max_strip_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

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

        let thumb_w = effective_strip_w - 2 * border;
        let thumb_h = (thumb_w as f64 * 9.0 / 16.0).round() as i32;
        cell_w = effective_strip_w;
        cell_h = thumb_h + label_h + 2 * border;
        strip_x = match s.pip_edge {
            config::PipEdge::Left => s.monitor_rect.left,
            _ => s.monitor_rect.right - cell_w,
        };
        strip_y = s.monitor_rect.top;
    } else {
        let max_strip_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

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

        let thumb_h = effective_strip_h - label_h - 2 * border;
        let thumb_w = (thumb_h as f64 * 16.0 / 9.0).round() as i32;
        cell_w = thumb_w + 2 * border;
        cell_h = effective_strip_h;
        let total_strip_w = n * cell_w + (n - 1).max(0) * gap;
        strip_x = s.monitor_rect.right - total_strip_w;
        strip_y = match s.pip_edge {
            config::PipEdge::Top => s.monitor_rect.top,
            _ => s.monitor_rect.bottom - cell_h,
        };
    }

    let mut rects = Vec::new();
    for i in 0..n {
        let (x_off, y_off) = if is_vertical {
            (0, i * (cell_h + gap))
        } else {
            (i * (cell_w + gap), 0)
        };
        rects.push(RECT {
            left: strip_x + x_off,
            top: strip_y + y_off,
            right: strip_x + x_off + cell_w,
            bottom: strip_y + y_off + cell_h,
        });
    }

    rects
}

/// Compute final positions: custom positions override strip positions where available.
unsafe fn compute_positions(s: &OverlayState) -> (Vec<RECT>, i32, i32) {
    let strip_rects = compute_strip_positions(s);

    // Compute strip dimensions for resize handle.
    let mut sw = 0i32;
    let mut sh = 0i32;
    if !strip_rects.is_empty() {
        let first = &strip_rects[0];
        let last = &strip_rects[strip_rects.len() - 1];
        sw = last.right - first.left;
        sh = last.bottom - first.top;
    }

    if !s.has_custom_positions {
        return (strip_rects, sw, sh);
    }

    let cfg = config::Config::load();
    let mut result = strip_rects;

    for pip_pos in &cfg.pip_positions {
        if pip_pos.slot < result.len() {
            result[pip_pos.slot] = RECT {
                left: pip_pos.x,
                top: pip_pos.y,
                right: pip_pos.x + pip_pos.width as i32,
                bottom: pip_pos.y + pip_pos.height as i32,
            };
        }
    }

    (result, sw, sh)
}

// ---------------------------------------------------------------------------
// Rebuild
// ---------------------------------------------------------------------------

unsafe fn rebuild_thumbnails(s: &mut OverlayState) {
    // Destroy existing PiP windows and unregister thumbnails.
    for pw in s.pip_windows.drain(..) {
        let _ = DwmUnregisterThumbnail(pw.thumb);
        let _ = DestroyWindow(pw.hwnd);
    }
    s.drop_target = None;

    if s.pip_order.is_empty() {
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }

    let reference = s.eq_windows.first().map(|w| w.hwnd);
    s.monitor_rect = eq_windows::get_monitor_work_area(reference);
    // Use label_hwnd for DPI since we no longer have a single overlay_hwnd.
    s.dpi_scale = get_dpi_scale(s.active_label_hwnd);

    let (rects, sw, sh) = compute_positions(s);
    s.strip_width = sw;
    s.strip_height = sh;

    let d = s.dpi_scale;
    let border = dpi(BORDER_WIDTH, d);
    let label_h = dpi(LABEL_HEIGHT, d);

    let pip_class = w!("StonemitePipClass");

    for (i, &pid) in s.pip_order.iter().enumerate() {
        let Some(eq_win) = s.eq_windows.iter().find(|w| w.pid == pid) else { continue };
        let Some(rect) = rects.get(i) else { continue };

        let cw = rect.right - rect.left;
        let ch = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            pip_class, w!("StonemitePip"), WS_POPUP,
            rect.left, rect.top, cw, ch,
            None, None, None, None,
        ).expect("Failed to create PiP window");

        // Store 1-based index so 0 = uninitialized.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (i + 1) as isize);

        // Register DWM thumbnail.
        let thumb_rect = RECT {
            left: border,
            top: border + label_h,
            right: cw - border,
            bottom: ch - border,
        };

        let thumb = match DwmRegisterThumbnail(hwnd, eq_win.hwnd) {
            Ok(t) => t,
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                continue;
            }
        };

        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: thumb_rect,
            fVisible: true.into(),
            opacity: THUMB_OPACITY_NORMAL,
            fSourceClientAreaOnly: true.into(),
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);

        s.pip_windows.push(PipWindowEntry {
            hwnd,
            pid,
            thumb,
            label: format_label(eq_win),
            number: eq_win.number,
            hovered: false,
        });
    }

    update_active_label(s);
}

// ---------------------------------------------------------------------------
// Active label
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

unsafe fn update_visibility(s: &mut OverlayState) {
    if s.hidden_by_user {
        for pw in &mut s.pip_windows {
            pw.hovered = false;
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
        }
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }

    let has_pip = !s.pip_order.is_empty();
    let fg = GetForegroundWindow();

    if has_pip && is_eq_or_ours(fg, s) {
        for pw in &s.pip_windows {
            let _ = ShowWindow(pw.hwnd, SW_SHOWNOACTIVATE);
        }
        if !s.active_label_text.is_empty() {
            let _ = ShowWindow(s.active_label_hwnd, SW_SHOWNOACTIVATE);
        }
    } else {
        for pw in &mut s.pip_windows {
            pw.hovered = false;
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
        }
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
    }
}

// ---------------------------------------------------------------------------
// Foreground event hook
// ---------------------------------------------------------------------------

unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK, _event: u32, _hwnd: HWND,
    _id_object: i32, _id_child: i32, _id_event_thread: u32, _dw_ms_event_time: u32,
) {
    let Some(s) = state().as_mut() else { return };

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

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

unsafe fn swap_to(pip_index: usize) {
    let Some(s) = state().as_mut() else { return };

    if pip_index >= s.pip_order.len() { return; }
    let Some(old_active_pid) = s.active_pid else { return };
    let new_active_pid = s.pip_order[pip_index];

    s.pip_order[pip_index] = old_active_pid;
    s.active_pid = Some(new_active_pid);

    if let Some(w) = s.eq_windows.iter().find(|w| w.pid == new_active_pid) {
        let _ = SetWindowPos(w.hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(w.hwnd);
    }

    rebuild_thumbnails(s);
    // Show PiP windows after swap.
    for pw in &s.pip_windows {
        let _ = ShowWindow(pw.hwnd, SW_SHOWNOACTIVATE);
    }
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

unsafe fn show_char_menu(s: &mut OverlayState, target_pid: u32, screen_pt: POINT, owner_hwnd: HWND) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let candidates = eq_characters::find_active_characters(&eq_dir, Duration::from_secs(86400));

    let hmenu = CreatePopupMenu().unwrap();

    // Character assignment submenu, grouped by server.
    let char_menu = CreatePopupMenu().unwrap();
    let mut servers: Vec<String> = Vec::new();
    for c in &candidates {
        if !servers.contains(&c.server) {
            servers.push(c.server.clone());
        }
    }

    if servers.len() == 1 {
        for (i, c) in candidates.iter().enumerate() {
            let label = format!("{}\0", c.character);
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(char_menu, MF_STRING,
                (IDM_CHAR_BASE + i as u32) as usize,
                windows::core::PCWSTR(wide.as_ptr()));
        }
    } else {
        for server in &servers {
            let server_menu = CreatePopupMenu().unwrap();
            for (i, c) in candidates.iter().enumerate() {
                if c.server != *server { continue; }
                let label = format!("{}\0", c.character);
                let wide: Vec<u16> = label.encode_utf16().collect();
                let _ = AppendMenuW(server_menu, MF_STRING,
                    (IDM_CHAR_BASE + i as u32) as usize,
                    windows::core::PCWSTR(wide.as_ptr()));
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
        let _ = AppendMenuW(num_menu, MF_STRING,
            (IDM_NUMBER_BASE + n as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()));
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
        let _ = AppendMenuW(edge_menu, flags,
            (IDM_EDGE_BASE + i as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()));
    }
    let edge_label: Vec<u16> = "PiP edge\0".encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_POPUP, edge_menu.0 as usize,
        windows::core::PCWSTR(edge_label.as_ptr()));

    // Edit/Lock layout toggle.
    let edit_label = if s.edit_mode {
        "Lock layout\0"
    } else {
        "Edit layout\0"
    };
    let edit_wide: Vec<u16> = edit_label.encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_STRING, IDM_EDIT_MODE as usize,
        windows::core::PCWSTR(edit_wide.as_ptr()));

    // Reset to auto layout (only when custom positions exist).
    if s.has_custom_positions {
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_RESET_LAYOUT as usize,
            w!("Reset to auto layout"));
    }

    // Hide overlay item with hotkey hint.
    let hide_label = format!("Hide overlay\t{}\0", cfg.hide_hotkey);
    let hide_wide: Vec<u16> = hide_label.encode_utf16().collect();
    let _ = AppendMenuW(hmenu, MF_STRING, IDM_HIDE_OVERLAY as usize,
        windows::core::PCWSTR(hide_wide.as_ptr()));

    s.context_menu_target_pid = Some(target_pid);
    s.context_menu_candidates = candidates;

    let _ = SetForegroundWindow(owner_hwnd);
    let _ = TrackPopupMenu(hmenu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        screen_pt.x, screen_pt.y, 0, owner_hwnd, None);
    let _ = DestroyMenu(hmenu);
    let _ = PostMessageW(owner_hwnd, WM_NULL, WPARAM(0), LPARAM(0));
}

unsafe fn handle_menu_command(cmd_id: u32) {
    let Some(s) = state().as_mut() else { return };

    if cmd_id == IDM_HIDE_OVERLAY {
        s.hidden_by_user = true;
        update_visibility(s);
    } else if cmd_id == IDM_EDIT_MODE {
        toggle_edit_mode_inner(s);
    } else if cmd_id == IDM_RESET_LAYOUT {
        let mut cfg = config::Config::load();
        cfg.pip_positions.clear();
        let _ = cfg.save();
        s.has_custom_positions = false;
        s.edit_mode = false;
        rebuild_thumbnails(s);
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
    // Switching edge resets to strip auto-layout: clear custom positions and
    // strip width (especially when changing orientation).
    s.pip_edge = edge;
    s.custom_strip_width = None;
    s.has_custom_positions = false;
    s.edit_mode = false;
    let mut cfg = config::Config::load();
    cfg.pip_edge = edge;
    cfg.pip_strip_width = None;
    cfg.pip_positions.clear();
    let _ = cfg.save();
    rebuild_thumbnails(s);
    update_visibility(s);
}

// ---------------------------------------------------------------------------
// Edit mode toggle
// ---------------------------------------------------------------------------

unsafe fn toggle_edit_mode_inner(s: &mut OverlayState) {
    if s.edit_mode {
        // Locking: save positions from current window positions.
        let mut positions = Vec::new();
        for (i, pw) in s.pip_windows.iter().enumerate() {
            let mut rect = RECT::default();
            let _ = GetWindowRect(pw.hwnd, &mut rect);
            positions.push(config::PipPosition {
                slot: i,
                x: rect.left,
                y: rect.top,
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            });
        }
        let mut cfg = config::Config::load();
        cfg.pip_positions = positions;
        let _ = cfg.save();
        s.has_custom_positions = true;
        s.edit_mode = false;
    } else {
        s.edit_mode = true;
    }
    // Repaint all PiP windows to show/hide edit indicators.
    for pw in &s.pip_windows {
        let _ = InvalidateRect(pw.hwnd, None, true);
    }
}

/// Public toggle for edit mode (called from tray menu).
pub fn toggle_edit_mode() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        toggle_edit_mode_inner(s);
    }
}

/// Public query for edit mode state.
pub fn is_edit_mode() -> bool {
    state().as_ref().map_or(false, |s| s.edit_mode)
}

// ---------------------------------------------------------------------------
// Paint functions
// ---------------------------------------------------------------------------

unsafe fn paint_pip_window(hwnd: HWND, pip_idx: usize) {
    let Some(s) = state().as_ref() else {
        let mut ps = PAINTSTRUCT::default();
        let _ = BeginPaint(hwnd, &mut ps);
        let _ = EndPaint(hwnd, &ps);
        return;
    };
    let Some(pw) = s.pip_windows.get(pip_idx) else {
        let mut ps = PAINTSTRUCT::default();
        let _ = BeginPaint(hwnd, &mut ps);
        let _ = EndPaint(hwnd, &ps);
        return;
    };

    let d = s.dpi_scale;
    let border = dpi(BORDER_WIDTH, d);
    let label_h = dpi(LABEL_HEIGHT, d);

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    // Black background.
    let black_brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);
    let _ = FillRect(hdc, &ps.rcPaint, black_brush);

    let mut client_rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut client_rect);

    // Determine drag visual state.
    let is_reorder_dragging = s.reorder_drag.as_ref().map_or(false, |d| d.dragging);
    let is_drag_source = is_reorder_dragging && s.reorder_drag.as_ref().map(|d| d.from_index) == Some(pip_idx);
    let is_drop_target = is_reorder_dragging && s.drop_target == Some(pip_idx)
        && s.reorder_drag.as_ref().map(|d| d.from_index) != Some(pip_idx);

    // Dimmed source during drag.
    if is_drag_source {
        let dim_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00333333));
        let _ = FillRect(hdc, &client_rect, dim_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(dim_brush);
    }

    // Drop target highlight (yellow border).
    if is_drop_target {
        let swap_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x0000CCFF));
        let _ = FrameRect(hdc, &client_rect, swap_brush);
        for inset in 1..border + 1 {
            let r = RECT {
                left: client_rect.left + inset, top: client_rect.top + inset,
                right: client_rect.right - inset, bottom: client_rect.bottom - inset,
            };
            let _ = FrameRect(hdc, &r, swap_brush);
        }
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(swap_brush);
    } else if pw.hovered && !is_reorder_dragging && !s.edit_mode {
        // Normal hover highlight.
        let white_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00FFFFFF));
        let _ = FrameRect(hdc, &client_rect, white_brush);
        for inset in 1..border {
            let r = RECT {
                left: client_rect.left + inset, top: client_rect.top + inset,
                right: client_rect.right - inset, bottom: client_rect.bottom - inset,
            };
            let _ = FrameRect(hdc, &r, white_brush);
        }
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(white_brush);
    }

    // Edit mode border indicator.
    if s.edit_mode {
        let edit_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(EDIT_BORDER_COLOR));
        let _ = FrameRect(hdc, &client_rect, edit_brush);
        let r2 = RECT {
            left: client_rect.left + 1, top: client_rect.top + 1,
            right: client_rect.right - 1, bottom: client_rect.bottom - 1,
        };
        let _ = FrameRect(hdc, &r2, edit_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(edit_brush);
    }

    // Colored label bar.
    let bg_color = color_for_number(pw.number);
    let label_bg_rect = RECT {
        left: client_rect.left + border,
        top: client_rect.top + border,
        right: client_rect.right - border,
        bottom: client_rect.top + border + label_h,
    };
    let label_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(bg_color));
    let _ = FillRect(hdc, &label_bg_rect, label_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(label_brush);

    // Label text.
    let font = CreateFontW(
        dpi(LABEL_HEIGHT - 8, d), 0, 0, 0, FW_BOLD.0 as i32,
        0, 0, 0, 0, 0, 0, 0, 0, w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x0048372D));
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let mut label_rect = RECT {
        left: label_bg_rect.left + dpi(8, d),
        top: label_bg_rect.top + dpi(4, d),
        right: label_bg_rect.right,
        bottom: label_bg_rect.bottom,
    };
    let mut text: Vec<u16> = pw.label.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut text, &mut label_rect, DT_LEFT | DT_SINGLELINE | DT_TOP);

    // Edit mode grip indicator (three dots in label bar).
    if s.edit_mode {
        let grip_color = windows::Win32::Foundation::COLORREF(0x0048372D);
        let grip_brush = CreateSolidBrush(grip_color);
        let cx = label_bg_rect.right - dpi(16, d);
        let cy = (label_bg_rect.top + label_bg_rect.bottom) / 2;
        let dot_size = dpi(3, d);
        let dot_gap = dpi(5, d);
        for j in 0..3 {
            let dot_rect = RECT {
                left: cx, top: cy - dot_gap + j * dot_gap - dot_size / 2,
                right: cx + dot_size, bottom: cy - dot_gap + j * dot_gap + dot_size / 2,
            };
            let _ = FillRect(hdc, &dot_rect, grip_brush);
        }
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(grip_brush);
    }

    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = EndPaint(hwnd, &ps);
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
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x0048372D));
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

// ---------------------------------------------------------------------------
// PiP window proc
// ---------------------------------------------------------------------------

unsafe extern "system" fn pip_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    // Decode pip index from GWLP_USERDATA (1-based, 0 = not yet set).
    let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    if raw_idx == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let pip_idx = raw_idx - 1;

    match msg {
        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == 1 /* HTCLIENT */ {
                if let Some(s) = state().as_ref() {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let mut client_pt = pt;
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client_pt);

                    let mut cr = RECT::default();
                    let _ = GetClientRect(hwnd, &mut cr);

                    if s.edit_mode {
                        let zone = dpi(RESIZE_ZONE, s.dpi_scale);
                        if let Some(edge) = edit_resize_edge_hit_test(client_pt, cr.right, cr.bottom, zone) {
                            let cursor_id = windows::core::PCWSTR(cursor_for_resize_edge(edge));
                            let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
                            SetCursor(cursor);
                            return LRESULT(1);
                        }
                        // Body → move cursor.
                        let cursor = LoadCursorW(None, IDC_SIZEALL).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    } else if !s.has_custom_positions {
                        // Strip resize cursor on interior edge.
                        let handle_w = dpi(RESIZE_HANDLE_WIDTH, s.dpi_scale);
                        if strip_resize_hit_test(client_pt, cr.right, cr.bottom, s.pip_edge, handle_w) {
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
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_PAINT => {
            paint_pip_window(hwnd, pip_idx);
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };

            // --- Edit mode move/resize drag ---
            if s.edit_mode {
                if let Some(ref md) = s.move_drag {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let dx = cursor.x - md.start_cursor.x;
                    let dy = cursor.y - md.start_cursor.y;
                    let new_x = md.start_rect.left + dx;
                    let new_y = md.start_rect.top + dy;
                    let w = md.start_rect.right - md.start_rect.left;
                    let h = md.start_rect.bottom - md.start_rect.top;

                    // Collect other pip rects for snapping.
                    let idx = md.pip_index;
                    let others: Vec<RECT> = s.pip_windows.iter().enumerate()
                        .filter(|(i, _)| *i != idx)
                        .map(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            r
                        })
                        .collect();

                    let (sx, sy) = snap_point(new_x, new_y, w, h, &others, s.monitor_rect, s.snap_grid);

                    if let Some(pw) = s.pip_windows.get(idx) {
                        let _ = SetWindowPos(pw.hwnd, HWND::default(), sx, sy, 0, 0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                        // Update DWM thumbnail (position relative to window doesn't change).
                    }
                    return LRESULT(0);
                }

                if let Some(ref rd) = s.pip_resize_drag {
                    let mut cursor = POINT::default();
                    let _ = GetCursorPos(&mut cursor);
                    let dx = cursor.x - rd.start_cursor.x;
                    let dy = cursor.y - rd.start_cursor.y;
                    let idx = rd.pip_index;
                    let edge = rd.edge;

                    let others: Vec<RECT> = s.pip_windows.iter().enumerate()
                        .filter(|(i, _)| *i != idx)
                        .map(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            r
                        })
                        .collect();

                    let d = s.dpi_scale;
                    let border = dpi(BORDER_WIDTH, d);
                    let label_h = dpi(LABEL_HEIGHT, d);
                    let new_rect = snap_resize(edge, rd.start_rect, dx, dy,
                        &others, s.monitor_rect, s.snap_grid, border, label_h);
                    let nw = new_rect.right - new_rect.left;
                    let nh = new_rect.bottom - new_rect.top;

                    if let Some(pw) = s.pip_windows.get(idx) {
                        let _ = SetWindowPos(pw.hwnd, HWND::default(),
                            new_rect.left, new_rect.top, nw, nh,
                            SWP_NOZORDER | SWP_NOACTIVATE);

                        // Update DWM thumbnail destination.
                        let thumb_rect = RECT {
                            left: border,
                            top: border + label_h,
                            right: nw - border,
                            bottom: nh - border,
                        };
                        let props = DWM_THUMBNAIL_PROPERTIES {
                            dwFlags: DWM_TNP_RECTDESTINATION,
                            rcDestination: thumb_rect,
                            ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                        let _ = InvalidateRect(pw.hwnd, None, true);
                    }
                    return LRESULT(0);
                }

                // Track mouse for leave.
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                return LRESULT(0);
            }

            // --- Use mode: strip resize drag ---
            if let Some(ref srd) = s.strip_resize_drag {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                let new_size = if is_vertical {
                    let delta = cursor.x - srd.start_pt.x;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Right) { -1 } else { 1 };
                    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
                    let min_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_w, max_w)
                } else {
                    let delta = cursor.y - srd.start_pt.y;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Bottom) { -1 } else { 1 };
                    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
                    let min_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_h, max_h)
                };
                if Some(new_size) != s.custom_strip_width {
                    s.custom_strip_width = Some(new_size);
                    rebuild_thumbnails(s);
                    // Show windows after rebuild during resize.
                    for pw in &s.pip_windows {
                        let _ = ShowWindow(pw.hwnd, SW_SHOWNOACTIVATE);
                    }
                }
                return LRESULT(0);
            }

            // --- Use mode: reorder drag ---
            if let Some(ref mut drag) = s.reorder_drag {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);

                if !drag.dragging {
                    let dx = (cursor.x - drag.start_pt.x).abs();
                    let dy = (cursor.y - drag.start_pt.y).abs();
                    let threshold = dpi(DRAG_THRESHOLD, s.dpi_scale);
                    if dx > threshold || dy > threshold {
                        drag.dragging = true;
                        let _ = SetCapture(hwnd);
                        // Dim the source thumbnail.
                        if let Some(pw) = s.pip_windows.get(drag.from_index) {
                            let props = DWM_THUMBNAIL_PROPERTIES {
                                dwFlags: DWM_TNP_OPACITY, opacity: 80, ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            let _ = InvalidateRect(pw.hwnd, None, true);
                        }
                    }
                }

                if drag.dragging {
                    // Find which pip is under cursor (screen coords).
                    let new_target = s.pip_windows.iter().enumerate().find(|(_, pw)| {
                        let mut r = RECT::default();
                        let _ = GetWindowRect(pw.hwnd, &mut r);
                        cursor.x >= r.left && cursor.x < r.right && cursor.y >= r.top && cursor.y < r.bottom
                    }).map(|(i, _)| i);

                    if s.drop_target != new_target {
                        // Invalidate old and new target.
                        if let Some(old_t) = s.drop_target {
                            if let Some(pw) = s.pip_windows.get(old_t) {
                                let _ = InvalidateRect(pw.hwnd, None, true);
                            }
                        }
                        s.drop_target = new_target;
                        if let Some(new_t) = new_target {
                            if let Some(pw) = s.pip_windows.get(new_t) {
                                let _ = InvalidateRect(pw.hwnd, None, true);
                            }
                        }
                    }
                }

                return LRESULT(0);
            }

            // --- Use mode: hover ---
            if let Some(pw) = s.pip_windows.get_mut(pip_idx) {
                if !pw.hovered {
                    pw.hovered = true;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_HOVER, ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }

            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };

            // Clear hover.
            if let Some(pw) = s.pip_windows.get_mut(pip_idx) {
                if pw.hovered {
                    pw.hovered = false;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_NORMAL, ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }

            // Cancel non-dragging reorder on leave.
            if s.reorder_drag.as_ref().map_or(false, |d| !d.dragging) {
                s.reorder_drag = None;
                s.drop_target = None;
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

            if s.edit_mode {
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);
                let zone = dpi(RESIZE_ZONE, s.dpi_scale);

                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                let mut win_rect = RECT::default();
                let _ = GetWindowRect(hwnd, &mut win_rect);

                if let Some(edge) = edit_resize_edge_hit_test(pt, cr.right, cr.bottom, zone) {
                    // Start resize drag.
                    s.pip_resize_drag = Some(PipResizeDragState {
                        pip_index: pip_idx,
                        edge,
                        start_cursor: cursor,
                        start_rect: win_rect,
                    });
                    let _ = SetCapture(hwnd);
                } else {
                    // Start move drag.
                    s.move_drag = Some(MoveDragState {
                        pip_index: pip_idx,
                        start_cursor: cursor,
                        start_rect: win_rect,
                    });
                    let _ = SetCapture(hwnd);
                }
            } else {
                // Use mode.
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);

                if !s.has_custom_positions {
                    // Check for strip resize hit.
                    let handle_w = dpi(RESIZE_HANDLE_WIDTH, s.dpi_scale);
                    if strip_resize_hit_test(pt, cr.right, cr.bottom, s.pip_edge, handle_w) {
                        let mut cursor = POINT::default();
                        let _ = GetCursorPos(&mut cursor);
                        let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                        let start_size = if is_vertical { s.strip_width } else { s.strip_height };
                        s.strip_resize_drag = Some(StripResizeDragState {
                            pip_index: pip_idx,
                            start_pt: cursor,
                            start_size,
                        });
                        let _ = SetCapture(hwnd);
                        return LRESULT(0);
                    }
                }

                // Start potential reorder drag.
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                s.reorder_drag = Some(ReorderDragState {
                    from_index: pip_idx,
                    start_pt: cursor,
                    dragging: false,
                });
            }

            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let _ = ReleaseCapture();

            // --- Edit mode: finalize move/resize ---
            if s.move_drag.take().is_some() {
                return LRESULT(0);
            }
            if s.pip_resize_drag.take().is_some() {
                return LRESULT(0);
            }

            // --- Strip resize finalize ---
            if s.strip_resize_drag.take().is_some() {
                let mut cfg = config::Config::load();
                cfg.pip_strip_width = s.custom_strip_width.map(|v| v as u32);
                let _ = cfg.save();
                return LRESULT(0);
            }

            // --- Reorder drag finalize ---
            let drag = s.reorder_drag.take();
            let old_drop_target = s.drop_target.take();

            if let Some(drag) = drag {
                if drag.dragging {
                    // Restore source thumbnail opacity.
                    if let Some(pw) = s.pip_windows.get(drag.from_index) {
                        let props = DWM_THUMBNAIL_PROPERTIES {
                            dwFlags: DWM_TNP_OPACITY, opacity: THUMB_OPACITY_NORMAL, ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    }
                    // Perform swap if target is valid.
                    if let Some(to_index) = old_drop_target {
                        if to_index != drag.from_index && to_index < s.pip_order.len() && drag.from_index < s.pip_order.len() {
                            s.pip_order.swap(drag.from_index, to_index);
                            rebuild_thumbnails(s);
                            // Show after rebuild.
                            for pw in &s.pip_windows {
                                let _ = ShowWindow(pw.hwnd, SW_SHOWNOACTIVATE);
                            }
                        }
                    }
                } else {
                    // Simple click → activate window.
                    let idx = drag.from_index;
                    let _ = s; // release borrow before swap_to re-borrows state
                    swap_to(idx);
                }
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

            if !s.edit_mode && !s.has_custom_positions {
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);
                let handle_w = dpi(RESIZE_HANDLE_WIDTH, s.dpi_scale);
                if strip_resize_hit_test(pt, cr.right, cr.bottom, s.pip_edge, handle_w) {
                    s.custom_strip_width = None;
                    let mut cfg = config::Config::load();
                    cfg.pip_strip_width = None;
                    let _ = cfg.save();
                    rebuild_thumbnails(s);
                    update_visibility(s);
                }
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

            if let Some(pw) = s.pip_windows.get(pip_idx) {
                let pid = pw.pid;
                let mut screen_pt = pt;
                let _ = ClientToScreen(hwnd, &mut screen_pt);
                show_char_menu(s, pid, screen_pt, hwnd);
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            let cmd_id = (wparam.0 & 0xFFFF) as u32;
            handle_menu_command(cmd_id);
            LRESULT(0)
        }

        WM_DPICHANGED | WM_DISPLAYCHANGE => {
            if let Some(s) = state().as_mut() {
                s.dpi_scale = get_dpi_scale(hwnd);
                rebuild_thumbnails(s);
                update_visibility(s);
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            // Individual PiP cleanup is handled by rebuild_thumbnails / cleanup.
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Label window proc
// ---------------------------------------------------------------------------

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
            if let Some(s) = state().as_mut() {
                if let Some(active_pid) = s.active_pid {
                    let mut pt = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = ClientToScreen(hwnd, &mut pt);
                    show_char_menu(s, active_pid, pt, hwnd);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = (wparam.0 & 0xFFFF) as u32;
            handle_menu_command(cmd_id);
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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns true if the foreground window is an EQ window or one of our windows.
pub fn is_eq_active() -> bool {
    unsafe {
        let Some(s) = state().as_ref() else { return false };
        let fg = GetForegroundWindow();
        is_eq_or_ours(fg, s)
    }
}

/// Returns true if the overlay is currently visible (not hidden by user).
pub fn is_visible() -> bool {
    state().as_ref().map_or(true, |s| !s.hidden_by_user)
}

pub fn toggle_hidden() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        s.hidden_by_user = !s.hidden_by_user;
        update_visibility(s);
    }
}

/// Reload config into overlay state and rebuild the layout.
pub fn force_rebuild() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        let cfg = config::Config::load();
        s.pip_edge = cfg.pip_edge;
        s.custom_strip_width = cfg.pip_strip_width.map(|v| v as i32);
        s.has_custom_positions = !cfg.pip_positions.is_empty();
        s.snap_grid = cfg.snap_grid as i32;
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
            for pw in s.pip_windows.drain(..) {
                let _ = DwmUnregisterThumbnail(pw.thumb);
                let _ = DestroyWindow(pw.hwnd);
            }
            let _ = DestroyWindow(s.active_label_hwnd);
        }
        *state() = None;
    }
}
