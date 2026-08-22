use std::cell::{Cell, UnsafeCell};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

mod notifications;

use notifications::{EnabledKinds, Notification};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, CreateFontW, CreatePen, CreateSolidBrush, DrawTextW, Ellipse,
    EndPaint, FillRect, FrameRect, GetStockObject, GetTextExtentPoint32W, InvalidateRect,
    RoundRect, SelectObject, SetBkMode, SetTextColor, BACKGROUND_MODE, BLACK_BRUSH, DT_CENTER,
    DT_LEFT, DT_SINGLELINE, DT_VCENTER, FW_BOLD, FW_HEAVY, HBRUSH, PAINTSTRUCT, PS_NULL,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::eq_windows::EqWindow;
use crate::{
    character_cache, config, eq_characters, eq_chat_colors, eq_windows, log_watcher, sound,
    trusik_shm,
};

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
const RESIZE_HANDLE_WIDTH: i32 = 12;

/// Thumbnail opacity (0-255). ~80% = 204.
const THUMB_OPACITY_NORMAL: u8 = 204;
const THUMB_OPACITY_HOVER: u8 = 255;

/// Border thickness for hover highlight.
const BORDER_WIDTH: i32 = 3;

/// Default height of the character name label overlay.
const DEFAULT_LABEL_HEIGHT: i32 = 48;

/// Default label opacity percentage (0–100).
const DEFAULT_LABEL_OPACITY: u32 = 80;

/// Color key for layered window transparency (magenta, COLORREF = 0x00BBGGRR).
const LABEL_COLOR_KEY: u32 = 0x00FF00FF;

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
/// Menu ID for broadcast toggle.
const IDM_BROADCAST_TOGGLE: u32 = 7400;
/// Menu ID for opening settings dialog.
const IDM_SETTINGS: u32 = 7500;

/// Distinct background colors for per-number labels (COLORREF = 0x00BBGGRR).
const LABEL_COLORS: &[u32] = &[
    0x00D4864A, // medium blue   (rgb #4A86D4)
    0x0060B06A, // forest green  (rgb #6AB060)
    0x005858D8, // warm rose     (rgb #D85858)
    0x0048B8E0, // amber         (rgb #E0B848)
    0x00C87CA0, // orchid        (rgb #A07CC8)
    0x00A8C858, // teal          (rgb #58C8A8)
];

/// Darker accent for number badge circles (COLORREF = 0x00BBGGRR).
const BADGE_COLORS: &[u32] = &[
    0x00B06830, // deep blue     (rgb #3068B0)
    0x00409048, // deep green    (rgb #489040)
    0x003838B8, // deep rose     (rgb #B83838)
    0x002898C0, // deep amber    (rgb #C09828)
    0x00A85C80, // deep orchid   (rgb #805CA8)
    0x0088A838, // deep teal     (rgb #38A888)
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

/// Default height of toast notification window (pixels).
const DEFAULT_TOAST_HEIGHT: i32 = 64;
/// Default toast visible duration (milliseconds).
const DEFAULT_TOAST_DURATION_MS: u32 = 2000;
/// Timer interval for toast fade animation (milliseconds).
const TOAST_FADE_STEP_MS: u32 = 30;
/// Alpha change per fade step.
const TOAST_ALPHA_STEP: u8 = 25;
/// Maximum alpha for the toast window.
const TOAST_MAX_ALPHA: u8 = 220;
/// Background color for toast window (dark neutral, COLORREF = 0x00BBGGRR).
const TOAST_BG_COLOR: u32 = 0x00403020;
/// Timer ID for toast animation.
const TIMER_TOAST_FADE: usize = 42;
const WM_CLEAR_INVITE_CAPTURE: u32 = WM_USER + 44;
/// Color key for toast layered window.
const TOAST_COLOR_KEY: u32 = 0x00FF00FF;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum ToastPhase {
    Hidden,
    FadingIn,
    Visible,
    FadingOut,
}

struct ToastState {
    hwnd: HWND,
    text: String,
    phase: ToastPhase,
    alpha: u8,
    phase_start: u64,
    duration_ms: u32,
    height: i32,
    enabled: bool,
}

struct PipWindowEntry {
    hwnd: HWND,
    label_hwnd: HWND,
    pid: u32,
    thumb: isize,
    label: String,
    class: Option<String>,
    number: usize,
    hovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResizeEdge {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
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
    /// Durable identity priority used to restore stable window numbers.
    preferred_box_order: Vec<config::BoxIdentity>,
    /// PID of the currently active (foreground) window.
    active_pid: Option<u32>,
    /// Floating label window for the active (foreground) EQ window.
    active_label_hwnd: HWND,
    active_label_text: String,
    active_label_class: Option<String>,
    active_label_color: u32,
    active_label_number: usize,
    /// Broadcast banner window, shown next to the active label when broadcasting.
    broadcast_label_hwnd: HWND,
    /// Configured label height (logical pixels).
    label_height: i32,
    /// Configured label alpha (0–255).
    label_alpha: u8,
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
    /// True while a context menu is open (suppresses visibility changes).
    context_menu_open: bool,
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
    /// Toast notification state.
    toast: ToastState,
    /// Whether to hide background EQ windows from Alt-Tab.
    hide_from_alt_tab: bool,
    /// Original extended styles for EQ windows, keyed by HWND as isize.
    original_ex_styles: HashMap<isize, isize>,
    /// Whether trusik character detection is enabled.
    trusik_enabled: bool,
    /// Last automatic identity observed for each process. Kept separate from
    /// the visible assignment so an unchanged SHM value cannot undo a manual
    /// assignment, while a genuinely new login can replace stale identity.
    trusik_identities: HashMap<u32, (String, String)>,
    /// Latest passive per-character state derived by the EQ log reducer.
    log_telemetry: HashMap<(String, String), log_watcher::CharacterTelemetry>,
    /// Durable unread notification state keyed by EQ process, surviving PiP rebuilds.
    notifications: HashMap<u32, Notification>,
    tell_visual_enabled: bool,
    tell_sound_enabled: bool,
    tell_sound: String,
    notification_kinds: EnabledKinds,
    animations_enabled: bool,
    chat_colors: eq_chat_colors::EqChatColorResolver,
    /// Persistent character knowledge (class, pets).
    character_cache: character_cache::CharacterCache,
}

// ---------------------------------------------------------------------------
// Static state
// ---------------------------------------------------------------------------

struct OverlayCell(UnsafeCell<Option<OverlayState>>);
unsafe impl Sync for OverlayCell {}

static OVERLAY: OverlayCell = OverlayCell(UnsafeCell::new(None));

// Guard against re-entrant access to overlay state.
// Win32 calls like CreateWindowExW pump messages, which can trigger
// foreground event hooks while we're already mutating state.
thread_local! {
    static IN_OVERLAY: Cell<bool> = const { Cell::new(false) };
}

// Dummy None value returned when re-entrant access is detected.
// This prevents callers from aliasing the real OVERLAY's &mut.
struct NoneCel(UnsafeCell<Option<OverlayState>>);
unsafe impl Sync for NoneCel {}
static REENTRANT_NONE: NoneCel = NoneCel(UnsafeCell::new(None));

/// Access overlay state. Returns a reference to a dummy `None` if called
/// re-entrantly (while IN_OVERLAY is set), preventing aliasing UB.
/// Use `state_unguarded()` only inside sections that already hold IN_OVERLAY.
fn state() -> &'static mut Option<OverlayState> {
    if IN_OVERLAY.get() {
        return unsafe { &mut *REENTRANT_NONE.0.get() };
    }
    unsafe { &mut *OVERLAY.0.get() }
}

/// Direct access to overlay state without the re-entrancy check.
/// Only safe to call from inside a section that already holds IN_OVERLAY
/// (poll_inner_guarded, foreground_event_proc, etc.).
fn state_unguarded() -> &'static mut Option<OverlayState> {
    unsafe { &mut *OVERLAY.0.get() }
}

fn publish_control_state(s: &OverlayState) {
    let sources = s
        .eq_windows
        .iter()
        .map(|window| trushar::control::SourceClient {
            private_key: u64::from(window.pid),
            character: window.character.clone(),
            server: window.server.clone(),
            class_code: window.class.clone(),
            window_number: window.number,
            active: s.active_pid == Some(window.pid),
            // The existing swap model supports the active client and at most
            // MAX_PIPS clients in pip_order. Extra discovered windows remain
            // visible in state but are explicitly not advertised as activatable.
            activatable: s.active_pid == Some(window.pid) || s.pip_order.contains(&window.pid),
            input_ready: crate::broadcast::is_target_ready(window.pid),
        })
        .collect();
    let mouse_clutch = trushar::control::MouseClutchState {
        phase: match crate::broadcast::mouse_clutch_status() {
            crate::broadcast::MouseClutchStatus::Inactive => {
                trushar::control::MouseClutchPhase::Inactive
            }
            crate::broadcast::MouseClutchStatus::Active => {
                trushar::control::MouseClutchPhase::Active
            }
            crate::broadcast::MouseClutchStatus::Releasing => {
                trushar::control::MouseClutchPhase::Releasing
            }
        },
        availability: match crate::broadcast::mouse_clutch_availability() {
            crate::broadcast::MouseClutchAvailability::Ready => {
                trushar::control::MouseClutchAvailability::Ready
            }
            crate::broadcast::MouseClutchAvailability::NoActiveClient => {
                trushar::control::MouseClutchAvailability::NoActiveClient
            }
            crate::broadcast::MouseClutchAvailability::NoCompatibleTargets => {
                trushar::control::MouseClutchAvailability::NoCompatibleTargets
            }
            crate::broadcast::MouseClutchAvailability::InputUnavailable => {
                trushar::control::MouseClutchAvailability::InputUnavailable
            }
        },
    };
    crate::control::publish(
        sources,
        trushar::control::BroadcastState {
            available: crate::broadcast::is_available(),
            enabled: crate::broadcast::is_active(),
        },
        mouse_clutch,
    );
}

/// Publish the current owner-thread state. Never call from the server thread.
pub fn publish_control_snapshot() {
    let Some(s) = state().as_ref() else { return };
    publish_control_state(s);
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

/// Assign contiguous window numbers from the configured identity priority.
/// Unlisted and not-yet-identified clients follow in their current relative order.
fn apply_preferred_box_order(
    eq_windows: &mut [EqWindow],
    preferred: &[config::BoxIdentity],
) -> bool {
    if preferred.is_empty() || eq_windows.is_empty() {
        return false;
    }

    let mut ordered: Vec<(u32, usize, Option<usize>)> = eq_windows
        .iter()
        .map(|window| {
            let rank = window
                .server
                .as_deref()
                .zip(window.character.as_deref())
                .and_then(|(server, character)| {
                    preferred
                        .iter()
                        .position(|identity| identity.matches(server, character))
                });
            (window.pid, window.number, rank)
        })
        .collect();
    ordered.sort_by_key(|(pid, number, rank)| {
        (rank.is_none(), rank.unwrap_or(*number), *number, *pid)
    });

    let mut changed = false;
    for (index, (pid, _, _)) in ordered.into_iter().enumerate() {
        let desired_number = index + 1;
        if let Some(window) = eq_windows.iter_mut().find(|window| window.pid == pid) {
            if window.number != desired_number {
                window.number = desired_number;
                changed = true;
            }
        }
    }
    changed
}

/// Exchange the stable window numbers of two loaded clients.
fn exchange_window_numbers(
    eq_windows: &mut [EqWindow],
    first_pid: u32,
    second_pid: u32,
) -> Option<(usize, usize)> {
    let first_index = eq_windows
        .iter()
        .position(|window| window.pid == first_pid)?;
    let second_index = eq_windows
        .iter()
        .position(|window| window.pid == second_pid)?;
    let first_number = eq_windows[first_index].number;
    let second_number = eq_windows[second_index].number;
    if first_index != second_index {
        eq_windows[first_index].number = second_number;
        eq_windows[second_index].number = first_number;
    }
    Some((first_number, second_number))
}

/// Exchange a PiP client with the active client while preserving the partition.
/// Returns false when the target is already active or is not currently a PiP.
fn focused_foreground_pid(
    windows: &[EqWindow],
    foreground: HWND,
    mut has_keyboard_focus: impl FnMut(HWND) -> bool,
) -> Option<u32> {
    windows
        .iter()
        .find(|window| window.hwnd == foreground && has_keyboard_focus(window.hwnd))
        .map(|window| window.pid)
}

fn exchange_active_with_pip(
    active_pid: &mut Option<u32>,
    pip_order: &mut Vec<u32>,
    target_pid: u32,
) -> bool {
    if *active_pid == Some(target_pid) {
        return false;
    }
    let Some(position) = pip_order.iter().position(|pid| *pid == target_pid) else {
        return false;
    };
    if let Some(old_active) = *active_pid {
        pip_order[position] = old_active;
    } else {
        pip_order.remove(position);
    }
    *active_pid = Some(target_pid);
    true
}

/// Sort pip_order so windows appear in slot-number order (1, 2, 3, …).
fn apply_auto_order(s: &mut OverlayState) {
    s.pip_order.sort_by_key(|pid| {
        s.eq_windows
            .iter()
            .find(|w| w.pid == *pid)
            .map_or(usize::MAX, |w| w.number)
    });
}

#[cfg(debug_assertions)]
fn debug_assert_client_partition(s: &OverlayState) {
    let known: HashSet<u32> = s.eq_windows.iter().map(|window| window.pid).collect();
    let unique_pips: HashSet<u32> = s.pip_order.iter().copied().collect();
    debug_assert_eq!(unique_pips.len(), s.pip_order.len());
    debug_assert!(s.active_pid.is_none_or(|pid| known.contains(&pid)));
    debug_assert!(s.active_pid.is_none_or(|pid| !unique_pips.contains(&pid)));
    debug_assert!(s.pip_order.iter().all(|pid| known.contains(pid)));
}

#[cfg(not(debug_assertions))]
fn debug_assert_client_partition(_s: &OverlayState) {}

/// Save original extended style and apply WS_EX_TOOLWINDOW to hide from Alt-Tab.
unsafe fn hide_window_from_alt_tab(s: &mut OverlayState, hwnd: HWND) {
    let key = hwnd.0 as isize;
    s.original_ex_styles
        .entry(key)
        .or_insert_with(|| GetWindowLongPtrW(hwnd, GWL_EXSTYLE));
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let new_style = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
    if new_style != style {
        // SetWindowLongPtrW sends WM_STYLECHANGING synchronously to the
        // target window. If it's hung (e.g. EQ zoning), that blocks our
        // UI thread. Fire-and-forget on a background thread instead.
        let h = hwnd.0 as usize;
        std::thread::spawn(move || {
            SetWindowLongPtrW(HWND(h as *mut _), GWL_EXSTYLE, new_style);
        });
    }
}

/// Restore a window's original extended style from the saved HashMap.
unsafe fn restore_window_ex_style(s: &mut OverlayState, hwnd: HWND) {
    let key = hwnd.0 as isize;
    if let Some(original) = s.original_ex_styles.remove(&key) {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if current != original {
            let h = hwnd.0 as usize;
            std::thread::spawn(move || {
                SetWindowLongPtrW(HWND(h as *mut _), GWL_EXSTYLE, original);
            });
        }
    }
}

/// Apply Alt-Tab hiding: hide background EQ windows, restore the active one.
unsafe fn apply_alt_tab_hiding(s: &mut OverlayState) {
    if !s.hide_from_alt_tab {
        return;
    }
    let active_pid = s.active_pid;
    let windows: Vec<(HWND, u32)> = s.eq_windows.iter().map(|w| (w.hwnd, w.pid)).collect();
    for (hwnd, pid) in &windows {
        if active_pid == Some(*pid) {
            restore_window_ex_style(s, *hwnd);
        } else {
            hide_window_from_alt_tab(s, *hwnd);
        }
    }
    // Prune stale entries for windows that no longer exist.
    let live_hwnds: HashSet<isize> = s.eq_windows.iter().map(|w| w.hwnd.0 as isize).collect();
    s.original_ex_styles.retain(|k, _| live_hwnds.contains(k));
}

/// Get the effective DPI scale for the monitor a window is on.
/// Uses GetDpiForMonitor (not GetDpiForWindow) so the result is correct
/// even when the window belongs to a DPI-unaware process like EQ.
unsafe fn get_dpi_scale(hwnd: HWND) -> f64 {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
    let mut dpi_x = 0u32;
    let mut dpi_y = 0u32;
    if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() && dpi_x > 0 {
        return dpi_x as f64 / 96.0;
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

unsafe fn client_animations_enabled() -> bool {
    let mut enabled = windows::Win32::Foundation::BOOL(1);
    SystemParametersInfoW(
        SPI_GETCLIENTAREAANIMATION,
        0,
        Some((&mut enabled as *mut windows::Win32::Foundation::BOOL).cast()),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    )
    .is_err()
        || enabled.as_bool()
}

fn pip_label_overlay_size(
    _s: &OverlayState,
    pip_width: i32,
    pip_height: i32,
    border: i32,
) -> (i32, i32) {
    let inner_width = (pip_width - 2 * border).max(0);
    let inner_height = (pip_height - 2 * border).max(0);
    (inner_width, inner_height)
}

fn color_for_number(number: usize) -> u32 {
    if number == 0 {
        return LABEL_COLORS[0];
    }
    LABEL_COLORS[(number - 1) % LABEL_COLORS.len()]
}

fn badge_color_for_number(number: usize) -> u32 {
    if number == 0 {
        return BADGE_COLORS[0];
    }
    BADGE_COLORS[(number - 1) % BADGE_COLORS.len()]
}

fn format_label(w: &EqWindow) -> String {
    match &w.character {
        Some(name) => name.clone(),
        None => String::new(),
    }
}

#[allow(dead_code)]
pub fn debug_log(msg: &str) {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let elapsed = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let log_path = std::path::Path::new(&appdata)
            .join("Stonemite")
            .join("debug.log");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
        {
            let _ = writeln!(f, "[{elapsed:>8.3}s] {msg}");
        }
    }
}

fn is_our_window(hwnd: HWND, s: &OverlayState) -> bool {
    if hwnd == s.active_label_hwnd {
        return true;
    }
    if hwnd == s.broadcast_label_hwnd {
        return true;
    }
    if hwnd == s.toast.hwnd {
        return true;
    }
    s.pip_windows
        .iter()
        .any(|pw| pw.hwnd == hwnd || pw.label_hwnd == hwnd)
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct MouseGeometry {
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
    dpi: u32,
}

unsafe fn mouse_geometry(hwnd: HWND) -> Option<MouseGeometry> {
    let mut client = RECT::default();
    GetClientRect(hwnd, &mut client).ok()?;
    let mut origin = POINT::default();
    if !ClientToScreen(hwnd, &mut origin).as_bool() {
        return None;
    }
    Some(MouseGeometry {
        origin_x: origin.x,
        origin_y: origin.y,
        width: client.right - client.left,
        height: client.bottom - client.top,
        dpi: GetDpiForWindow(hwnd),
    })
}

fn sync_mouse_eligibility(s: &OverlayState) {
    let eligible = unsafe {
        s.active_pid
            .and_then(|source_pid| {
                let source = s
                    .eq_windows
                    .iter()
                    .find(|window| window.pid == source_pid)?;
                let source_geometry = mouse_geometry(source.hwnd)?;
                Some(
                    s.eq_windows
                        .iter()
                        .filter(|window| mouse_geometry(window.hwnd) == Some(source_geometry))
                        .map(|window| window.pid)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default()
    };
    crate::broadcast::update_mouse_eligible_pids(&eligible);
}

fn is_eq_or_ours(hwnd: HWND, s: &OverlayState) -> bool {
    if is_our_window(hwnd, s) {
        return true;
    }
    // Check by HWND first (fast path), then fall back to PID check.
    // The PID fallback handles EQ recreating its window (new HWND, same PID)
    // before the next poll updates our cached HWNDs.
    if s.eq_windows.iter().any(|w| w.hwnd == hwnd) {
        return true;
    }
    if !s.eq_windows.is_empty() {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != 0 {
            return s.eq_windows.iter().any(|w| w.pid == pid);
        }
    }
    false
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
fn strip_resize_hit_test(
    pt: POINT,
    w: i32,
    h: i32,
    pip_edge: config::PipEdge,
    handle_w: i32,
) -> bool {
    match pip_edge {
        config::PipEdge::Right => pt.x < handle_w,
        config::PipEdge::Left => pt.x >= w - handle_w,
        config::PipEdge::Top => pt.y >= h - handle_w,
        config::PipEdge::Bottom => pt.y < handle_w,
    }
}

fn cursor_for_resize_edge(edge: ResizeEdge) -> *const u16 {
    match edge {
        ResizeEdge::N | ResizeEdge::S => IDC_SIZENS.0,
        ResizeEdge::E | ResizeEdge::W => IDC_SIZEWE.0,
        ResizeEdge::NW | ResizeEdge::SE => IDC_SIZENWSE.0,
        ResizeEdge::NE | ResizeEdge::SW => IDC_SIZENESW.0,
    }
}

// ---------------------------------------------------------------------------
// Snap logic
// ---------------------------------------------------------------------------

/// Snap a position (x, y) for a window of size (w, h) to grid, monitor edges,
/// and other PiP windows. Hold Shift to bypass snapping.
fn snap_point(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    others: &[RECT],
    monitor: RECT,
    grid: i32,
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
    if (sx - monitor.left).abs() < SNAP_DISTANCE {
        sx = monitor.left;
    }
    if (sx + w - monitor.right).abs() < SNAP_DISTANCE {
        sx = monitor.right - w;
    }
    if (sy - monitor.top).abs() < SNAP_DISTANCE {
        sy = monitor.top;
    }
    if (sy + h - monitor.bottom).abs() < SNAP_DISTANCE {
        sy = monitor.bottom - h;
    }

    // PiP-to-PiP edge snap.
    for other in others {
        // Left edge of moving window → left/right edge of other.
        if (sx - other.left).abs() < SNAP_DISTANCE {
            sx = other.left;
        }
        if (sx - other.right).abs() < SNAP_DISTANCE {
            sx = other.right;
        }
        // Right edge of moving window → left/right edge of other.
        if (sx + w - other.left).abs() < SNAP_DISTANCE {
            sx = other.left - w;
        }
        if (sx + w - other.right).abs() < SNAP_DISTANCE {
            sx = other.right - w;
        }
        // Top edge.
        if (sy - other.top).abs() < SNAP_DISTANCE {
            sy = other.top;
        }
        if (sy - other.bottom).abs() < SNAP_DISTANCE {
            sy = other.bottom;
        }
        // Bottom edge.
        if (sy + h - other.top).abs() < SNAP_DISTANCE {
            sy = other.top - h;
        }
        if (sy + h - other.bottom).abs() < SNAP_DISTANCE {
            sy = other.bottom - h;
        }
    }

    (sx, sy)
}

/// Compute the correct cell height for a given cell width, maintaining 16:9
/// thumbnail aspect ratio with the border overhead.
fn aspect_height_for_width(cell_w: i32, border: i32, _label_h: i32) -> i32 {
    let thumb_w = cell_w - 2 * border;
    let thumb_h = (thumb_w as f64 * 9.0 / 16.0).round() as i32;
    thumb_h + 2 * border
}

/// Compute the correct cell width for a given cell height, maintaining 16:9
/// thumbnail aspect ratio with the border overhead.
fn aspect_width_for_height(cell_h: i32, border: i32, _label_h: i32) -> i32 {
    let thumb_h = cell_h - 2 * border;
    let thumb_w = (thumb_h as f64 * 16.0 / 9.0).round() as i32;
    thumb_w + 2 * border
}

/// Apply snap to a resize operation, enforcing 16:9 thumbnail aspect ratio.
/// The dragged edge(s) determine whether width or height is the driving dimension.
#[allow(clippy::too_many_arguments)]
fn snap_resize(
    edge: ResizeEdge,
    start_rect: RECT,
    dx: i32,
    dy: i32,
    _others: &[RECT],
    _monitor: RECT,
    grid: i32,
    border: i32,
    label_h: i32,
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
            ResizeEdge::E | ResizeEdge::NE | ResizeEdge::SE => {
                r.right = ((r.right as f64 / g as f64).round() as i32) * g
            }
            ResizeEdge::W | ResizeEdge::NW | ResizeEdge::SW => {
                r.left = ((r.left as f64 / g as f64).round() as i32) * g
            }
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
                ResizeEdge::NW | ResizeEdge::NE | ResizeEdge::N => r.top = r.bottom - new_h,
                _ => r.bottom = r.top + new_h,
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
        lpszClassName: pip_class,
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        style: CS_DBLCLKS,
        ..Default::default()
    };
    RegisterClassW(&wc);

    // Register PiP label overlay window class.
    let pip_label_class = w!("StonemitePipLabelClass");
    let pip_label_wc = WNDCLASSW {
        lpfnWndProc: Some(pip_label_wnd_proc),
        lpszClassName: pip_label_class,
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&pip_label_wc);

    // Register label window class.
    let label_class = w!("StonemiteLabelClass");
    let label_wc = WNDCLASSW {
        lpfnWndProc: Some(label_wnd_proc),
        lpszClassName: label_class,
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&label_wc);

    let label_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        label_class,
        w!("StonemiteLabel"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create label window");

    let cfg = config::Config::load();
    let label_opacity = cfg
        .pip_label_opacity
        .unwrap_or(DEFAULT_LABEL_OPACITY)
        .min(100);
    let label_alpha = ((label_opacity as u16 * 255) / 100) as u8;

    let label_key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
    let _ =
        SetLayeredWindowAttributes(label_hwnd, label_key, label_alpha, LWA_ALPHA | LWA_COLORKEY);

    // Register broadcast banner window class.
    let bc_class = w!("StonemiteBroadcastClass");
    let bc_wc = WNDCLASSW {
        lpfnWndProc: Some(broadcast_label_wnd_proc),
        lpszClassName: bc_class,
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&bc_wc);

    let bc_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        bc_class,
        w!("StonemiteBroadcast"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create broadcast label window");

    let _ = SetLayeredWindowAttributes(bc_hwnd, label_key, label_alpha, LWA_ALPHA | LWA_COLORKEY);

    // Register toast notification window class.
    let toast_class = w!("StonemiteToastClass");
    let toast_wc = WNDCLASSW {
        lpfnWndProc: Some(toast_wnd_proc),
        lpszClassName: toast_class,
        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&toast_wc);

    let toast_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
        toast_class,
        w!("StonemiteToast"),
        WS_POPUP,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .expect("Failed to create toast window");

    let toast_key = windows::Win32::Foundation::COLORREF(TOAST_COLOR_KEY);
    let _ = SetLayeredWindowAttributes(toast_hwnd, toast_key, 0, LWA_ALPHA | LWA_COLORKEY);

    let hook = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        None,
        Some(foreground_event_proc),
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    );

    let has_custom = !cfg.pip_positions.is_empty();
    let label_height = cfg
        .pip_label_height
        .map(|v| v as i32)
        .unwrap_or(DEFAULT_LABEL_HEIGHT);

    *state_unguarded() = Some(OverlayState {
        pip_windows: Vec::new(),
        eq_windows: Vec::new(),
        pip_order: Vec::new(),
        preferred_box_order: cfg.box_order.clone(),
        active_pid: None,
        active_label_hwnd: label_hwnd,
        active_label_text: String::new(),
        active_label_class: None,
        active_label_color: LABEL_COLORS[0],
        active_label_number: 0,
        broadcast_label_hwnd: bc_hwnd,
        label_height,
        label_alpha,
        event_hook: hook,
        monitor_rect: RECT::default(),
        dpi_scale: get_dpi_scale(label_hwnd),
        pip_edge: cfg.pip_edge,
        custom_strip_width: cfg.pip_strip_width.map(|v| v as i32),
        hidden_by_user: false,
        context_menu_target_pid: None,
        context_menu_candidates: Vec::new(),
        context_menu_open: false,
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
        toast: ToastState {
            hwnd: toast_hwnd,
            text: String::new(),
            phase: ToastPhase::Hidden,
            alpha: 0,
            phase_start: 0,
            duration_ms: cfg
                .toast_duration
                .map(|d| (d * 1000.0) as u32)
                .unwrap_or(DEFAULT_TOAST_DURATION_MS),
            height: cfg
                .toast_height
                .map(|h| h as i32)
                .unwrap_or(DEFAULT_TOAST_HEIGHT),
            enabled: cfg.toast_enabled,
        },
        hide_from_alt_tab: cfg.hide_from_alt_tab,
        original_ex_styles: HashMap::new(),
        trusik_enabled: cfg.trusik,
        trusik_identities: HashMap::new(),
        log_telemetry: HashMap::new(),
        notifications: HashMap::new(),
        tell_visual_enabled: cfg.tell_visual_enabled,
        tell_sound_enabled: cfg.tell_sound_enabled,
        tell_sound: sound::normalized_id(&cfg.tell_sound).to_owned(),
        notification_kinds: EnabledKinds {
            tells: cfg.notify_tells,
            group_invites: cfg.notify_group_invites,
            raid_invites: cfg.notify_raid_invites,
            resurrections: cfg.notify_resurrections,
            deaths: cfg.notify_deaths,
        },
        animations_enabled: client_animations_enabled(),
        chat_colors: eq_chat_colors::EqChatColorResolver::new(cfg.eq_directory()),
        character_cache: character_cache::CharacterCache::load(),
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
    if IN_OVERLAY.get() {
        return;
    }
    IN_OVERLAY.set(true);
    poll_inner_guarded();
    IN_OVERLAY.set(false);
}

unsafe fn poll_inner_guarded() {
    let Some(s) = state_unguarded().as_mut() else {
        return;
    };

    let new_windows = eq_windows::find_eq_windows();
    let old_pids: HashSet<u32> = s.eq_windows.iter().map(|w| w.pid).collect();
    let new_pids: HashSet<u32> = new_windows.iter().map(|w| w.pid).collect();

    if old_pids == new_pids {
        let mut hwnd_changed = false;
        for nw in &new_windows {
            if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
                if ow.hwnd != nw.hwnd {
                    ow.hwnd = nw.hwnd;
                    hwnd_changed = true;
                }
            }
        }
        // A foreground WinEvent can be suppressed while an overlay transaction
        // is guarded. Reconcile on every poll so that skipped callbacks cannot
        // leave the active/PiP partition stale indefinitely.
        let foreground_pid = focused_foreground_pid(&s.eq_windows, GetForegroundWindow(), |hwnd| {
            target_has_keyboard_focus(hwnd)
        });
        let foreground_changed = foreground_pid.is_some_and(|pid| {
            let changed = exchange_active_with_pip(&mut s.active_pid, &mut s.pip_order, pid);
            if changed && config::Config::load().auto_order {
                apply_auto_order(s);
            }
            crate::broadcast::set_active_pid(pid);
            sync_mouse_eligibility(s);
            acknowledge_notification(s, pid);
            changed
        });
        // Poll trusik shared memory for character names.
        if s.trusik_enabled {
            trusik_poll_characters(s);
        }
        // Publish identity changes to the event-driven log worker. No log
        // filesystem I/O occurs on the UI polling path.
        publish_log_sources_inner(s);
        // Update broadcast targets and the strict identical-geometry Mouse Clutch set.
        let pids: Vec<u32> = s.eq_windows.iter().map(|w| w.pid).collect();
        crate::broadcast::update_targets(&pids, s.active_pid);
        sync_mouse_eligibility(s);
        // Re-derive DPI from the EQ window; if it changed (e.g. monitor
        // reconnect moved EQ to a different-DPI display), rebuild everything.
        // Also rebuild if any HWND changed (e.g. EQ recreated its window
        // during login), since DWM thumbnails are bound to specific HWNDs.
        let dpi_hwnd = s
            .eq_windows
            .first()
            .map(|w| w.hwnd)
            .unwrap_or(s.active_label_hwnd);
        let new_dpi = get_dpi_scale(dpi_hwnd);
        if foreground_changed || hwnd_changed || (new_dpi - s.dpi_scale).abs() > 0.001 {
            s.dpi_scale = new_dpi;
            rebuild_thumbnails(s);
        } else {
            update_active_label(s);
        }
        apply_alt_tab_hiding(s);
        update_visibility(s);
        debug_assert_client_partition(s);
        publish_control_state(s);
        return;
    }

    let added: Vec<u32> = new_pids.difference(&old_pids).copied().collect();
    let removed: Vec<u32> = old_pids.difference(&new_pids).copied().collect();
    let mut last_closed_label = None;
    for pid in &removed {
        // Capture info before removing for toast.
        if let Some(w) = s.eq_windows.iter().find(|w| w.pid == *pid) {
            last_closed_label = Some(format!("Window #{} closed", w.number));
        }
        s.eq_windows.retain(|w| w.pid != *pid);
        s.trusik_identities.remove(pid);
        s.notifications.remove(pid);
        s.pip_order.retain(|p| *p != *pid);
        if s.active_pid == Some(*pid) {
            s.active_pid = s.pip_order.first().copied();
            if let Some(promoted) = s.active_pid {
                s.pip_order.retain(|p| *p != promoted);
            }
        }
    }
    if let Some(label) = last_closed_label {
        show_toast_inner(s, &label);
    }

    let fg_hwnd = GetForegroundWindow();
    let fg_pid = focused_foreground_pid(&new_windows, fg_hwnd, |hwnd| {
        target_has_keyboard_focus(hwnd)
    });

    for pid in &added {
        let nw = new_windows.iter().find(|w| w.pid == *pid).unwrap();
        let number = next_available_number(&s.eq_windows);
        s.eq_windows.push(EqWindow {
            hwnd: nw.hwnd,
            pid: nw.pid,
            number,
            character: None,
            server: None,
            class: None,
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
        if exchange_active_with_pip(&mut s.active_pid, &mut s.pip_order, fg)
            && config::Config::load().auto_order
        {
            apply_auto_order(s);
        }
        crate::broadcast::set_active_pid(fg);
        sync_mouse_eligibility(s);
        acknowledge_notification(s, fg);
    }

    s.pip_order.truncate(MAX_PIPS);

    for nw in &new_windows {
        if let Some(ow) = s.eq_windows.iter_mut().find(|w| w.pid == nw.pid) {
            ow.hwnd = nw.hwnd;
        }
    }

    // Poll trusik shared memory for character names.
    if s.trusik_enabled {
        trusik_poll_characters(s);
    }
    // Publish identity changes to the event-driven log worker. No log
    // filesystem I/O occurs on the UI polling path.
    publish_log_sources_inner(s);

    // Update broadcast targets and the strict identical-geometry Mouse Clutch set.
    let pids: Vec<u32> = s.eq_windows.iter().map(|w| w.pid).collect();
    crate::broadcast::update_targets(&pids, s.active_pid);
    sync_mouse_eligibility(s);

    apply_preferred_box_order(&mut s.eq_windows, &s.preferred_box_order);
    if config::Config::load().auto_order {
        apply_auto_order(s);
    }

    apply_alt_tab_hiding(s);
    rebuild_thumbnails(s);
    update_visibility(s);
    debug_assert_client_partition(s);
    publish_control_state(s);
}

/// Check trusik shared memory for a newly published identity for each process.
fn trusik_poll_characters(s: &mut OverlayState) {
    let mut changed = false;
    for ew in &mut s.eq_windows {
        if let Some((name, server)) = trusik_shm::read_character(ew.pid) {
            let class = s
                .character_cache
                .get_class(&server, &name)
                .map(String::from);
            let identity_changed = reconcile_trusik_identity(
                &mut s.trusik_identities,
                ew,
                name.clone(),
                server.clone(),
                class,
            );
            if identity_changed {
                s.character_cache.remember(&server, &name);
                changed = true;
            }
        }
    }
    if changed {
        s.character_cache.save();
        if apply_preferred_box_order(&mut s.eq_windows, &s.preferred_box_order)
            && config::Config::load().auto_order
        {
            apply_auto_order(s);
        }
        unsafe { rebuild_thumbnails(s) };
    }
}

fn reconcile_trusik_identity(
    observed: &mut HashMap<u32, (String, String)>,
    window: &mut EqWindow,
    character: String,
    server: String,
    class: Option<String>,
) -> bool {
    let unchanged = observed
        .get(&window.pid)
        .is_some_and(|(old_character, old_server)| {
            old_character.eq_ignore_ascii_case(&character)
                && old_server.eq_ignore_ascii_case(&server)
        });
    if unchanged {
        return false;
    }

    observed.insert(window.pid, (character.clone(), server.clone()));
    window.character = Some(character);
    window.server = Some(server);
    window.class = class;
    true
}

fn publish_log_sources_inner(s: &OverlayState) {
    let logs_dir = config::Config::load().eq_directory().join("Logs");
    let sources = s
        .eq_windows
        .iter()
        .filter_map(|window| {
            Some(log_watcher::LogSource::new(
                format!("pid:{}", window.pid),
                window.character.as_ref()?.as_str(),
                window.server.as_ref()?.as_str(),
            ))
        })
        .collect();
    log_watcher::replace_sources(logs_dir, sources);
}

/// Publish the current identity snapshot after the log worker starts or the EQ
/// directory changes. This is last-write-wins and performs no filesystem I/O.
pub fn publish_log_sources() {
    if IN_OVERLAY.get() {
        return;
    }
    IN_OVERLAY.set(true);
    if let Some(s) = state_unguarded().as_ref() {
        publish_log_sources_inner(s);
    }
    IN_OVERLAY.set(false);
}

/// Drain a bounded number of parsed log batches on the owner thread. Returns
/// false when a re-entrant Win32 callback should repost the wake message.
pub fn drain_log_events() -> bool {
    if IN_OVERLAY.get() {
        return false;
    }
    IN_OVERLAY.set(true);
    let batches = log_watcher::drain_ready();
    if let Some(s) = state_unguarded().as_mut() {
        apply_log_batches(s, batches);
    }
    IN_OVERLAY.set(false);
    true
}

fn apply_log_batches(s: &mut OverlayState, batches: Vec<log_watcher::LogBatch>) {
    let mut class_changed = false;
    for batch in batches {
        for diagnostic in batch.diagnostics {
            debug_log(&format!("eq_logs: {diagnostic}"));
        }
        for envelope in batch.envelopes {
            for event in envelope.events.iter() {
                notifications::apply_log_event(s, event);
            }
            for change in envelope.telemetry_changes.iter() {
                let server = change.character.server.as_ref();
                let character = change.character.character.as_ref();
                s.log_telemetry.insert(
                    (server.to_ascii_lowercase(), character.to_ascii_lowercase()),
                    change.telemetry.clone(),
                );

                if let Some(class_code) = &change.telemetry.class_code {
                    s.character_cache
                        .set_class(server, character, class_code.as_ref());
                    for window in &mut s.eq_windows {
                        if let (Some(name), Some(window_server)) =
                            (&window.character, &window.server)
                        {
                            if name.eq_ignore_ascii_case(character)
                                && window_server.eq_ignore_ascii_case(server)
                            {
                                let new_class = Some(class_code.to_string());
                                if window.class != new_class {
                                    window.class = new_class;
                                    class_changed = true;
                                }
                            }
                        }
                    }
                }
                if let Some(pet) = &change.telemetry.pet {
                    s.character_cache.set_pet(server, character, pet.as_ref());
                }
            }
        }
    }
    s.character_cache.save();

    if class_changed {
        unsafe { rebuild_thumbnails(s) };
    }
}

fn acknowledge_notification(s: &mut OverlayState, pid: u32) -> bool {
    s.notifications.remove(&pid).is_some()
}

// ---------------------------------------------------------------------------
// Position computation
// ---------------------------------------------------------------------------

/// Compute strip layout positions as screen-coordinate RECTs.
unsafe fn compute_strip_positions(s: &OverlayState) -> Vec<RECT> {
    let d = s.dpi_scale;
    let gap = dpi(THUMB_GAP, d);
    let border = dpi(BORDER_WIDTH, d);

    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
    let n = s.pip_order.len() as i32;
    if n == 0 {
        return Vec::new();
    }

    let is_vertical = matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);

    let (strip_x, strip_y, cell_w, cell_h);

    if is_vertical {
        let max_strip_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

        let auto_max_thumb_w = max_strip_w - 2 * border;
        let auto_max_thumb_h = (auto_max_thumb_w as f64 * 9.0 / 16.0).round() as i32;
        let auto_max_cell_h = (mon_h - (n - 1).max(0) * gap) / n;
        let auto_thumb_h = (auto_max_cell_h - 2 * border).clamp(dpi(40, d), auto_max_thumb_h);
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
        cell_h = thumb_h + 2 * border;
        strip_x = match s.pip_edge {
            config::PipEdge::Left => s.monitor_rect.left,
            _ => s.monitor_rect.right - cell_w,
        };
        strip_y = s.monitor_rect.top;
    } else {
        let max_strip_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
        let min_strip_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;

        let auto_max_thumb_h = max_strip_h - 2 * border;
        let auto_max_thumb_w = (auto_max_thumb_h as f64 * 16.0 / 9.0).round() as i32;
        let auto_max_cell_w = (mon_w - (n - 1).max(0) * gap) / n;
        let auto_thumb_w = (auto_max_cell_w - 2 * border).clamp(dpi(60, d), auto_max_thumb_w);
        let auto_thumb_h = (auto_thumb_w as f64 * 9.0 / 16.0).round() as i32;
        let auto_cell_h = auto_thumb_h + 2 * border;

        let effective_strip_h = if let Some(custom_h) = s.custom_strip_width {
            custom_h.clamp(min_strip_h, max_strip_h)
        } else {
            auto_cell_h
        };

        let thumb_h = effective_strip_h - 2 * border;
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
    // Every rebuild creates and destroys Win32 windows, which can pump messages.
    // Preserve an outer transaction guard or establish one for unguarded UI paths.
    let was_guarded = IN_OVERLAY.replace(true);
    rebuild_thumbnails_guarded(s);
    IN_OVERLAY.set(was_guarded);
}

unsafe fn rebuild_thumbnails_guarded(s: &mut OverlayState) {
    // Destroy existing PiP windows and unregister thumbnails.
    for pw in s.pip_windows.drain(..) {
        if pw.thumb != 0 {
            let _ = DwmUnregisterThumbnail(pw.thumb);
        }
        if !pw.label_hwnd.is_invalid() {
            let _ = DestroyWindow(pw.label_hwnd);
        }
        let _ = DestroyWindow(pw.hwnd);
    }
    s.drop_target = None;

    if s.pip_order.is_empty() {
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        return;
    }

    let reference = s.eq_windows.first().map(|w| w.hwnd);
    s.monitor_rect = eq_windows::get_monitor_work_area(reference);
    // Get DPI from the same monitor as the EQ windows, so it stays consistent
    // with monitor_rect after display changes (unplug/replug, DPI change).
    let dpi_hwnd = reference.unwrap_or(s.active_label_hwnd);
    s.dpi_scale = get_dpi_scale(dpi_hwnd);

    let (rects, sw, sh) = compute_positions(s);
    s.strip_width = sw;
    s.strip_height = sh;

    let d = s.dpi_scale;
    let border = dpi(BORDER_WIDTH, d);

    let pip_class = w!("StonemitePipClass");

    for (i, &pid) in s.pip_order.iter().enumerate() {
        let Some(eq_win) = s.eq_windows.iter().find(|w| w.pid == pid) else {
            continue;
        };
        if !IsWindow(eq_win.hwnd).as_bool() {
            continue;
        }
        let Some(rect) = rects.get(i) else { continue };

        let cw = rect.right - rect.left;
        let ch = rect.bottom - rect.top;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            pip_class,
            w!("StonemitePip"),
            WS_POPUP,
            rect.left,
            rect.top,
            cw,
            ch,
            None,
            None,
            None,
            None,
        )
        .expect("Failed to create PiP window");

        // Store 1-based index so 0 = uninitialized.
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (i + 1) as isize);

        // Register DWM thumbnail filling the window (label overlays on top).
        let thumb_rect = RECT {
            left: border,
            top: border,
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
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: thumb_rect,
            fVisible: true.into(),
            opacity: THUMB_OPACITY_NORMAL,
            fSourceClientAreaOnly: true.into(),
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);

        // Create layered label overlay window on top of the PiP.
        let label_text = format_label(eq_win);
        let (lbl_w, lbl_h) = pip_label_overlay_size(s, cw, ch, border);
        let pip_label_class = w!("StonemitePipLabelClass");
        let lbl_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            pip_label_class,
            w!("StonemitePipLabel"),
            WS_POPUP,
            rect.left + border,
            rect.top + border,
            lbl_w,
            lbl_h,
            None,
            None,
            None,
            None,
        )
        .expect("Failed to create PiP label window");
        SetWindowLongPtrW(lbl_hwnd, GWLP_USERDATA, (i + 1) as isize);
        let key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
        let label_alpha = if s.notifications.contains_key(&pid) {
            s.label_alpha.max(notifications::LABEL_MIN_ALPHA)
        } else {
            s.label_alpha
        };
        let _ = SetLayeredWindowAttributes(lbl_hwnd, key, label_alpha, LWA_ALPHA | LWA_COLORKEY);

        s.pip_windows.push(PipWindowEntry {
            hwnd,
            label_hwnd: lbl_hwnd,
            pid,
            thumb,
            label: label_text,
            class: eq_win.class.clone(),
            number: eq_win.number,
            hovered: false,
        });
    }

    update_active_label(s);
}

// ---------------------------------------------------------------------------
// Active label
// ---------------------------------------------------------------------------

fn input_indicator_text() -> Option<&'static str> {
    use crate::broadcast::MouseClutchStatus;
    match (
        crate::broadcast::is_active(),
        crate::broadcast::mouse_clutch_status(),
    ) {
        (false, MouseClutchStatus::Inactive) => None,
        (true, MouseClutchStatus::Inactive) => Some("Broadcasting"),
        (false, MouseClutchStatus::Active) => Some("Mouse Clutch"),
        (false, MouseClutchStatus::Releasing) => Some("Mouse Clutch · Releasing"),
        (true, MouseClutchStatus::Active) => Some("Broadcasting · Mouse Clutch"),
        (true, MouseClutchStatus::Releasing) => Some("Broadcasting · Mouse Clutch releasing"),
    }
}

unsafe fn update_active_label(s: &mut OverlayState) {
    let active = s
        .active_pid
        .and_then(|pid| s.eq_windows.iter().find(|w| w.pid == pid));
    s.active_label_text = active.map(format_label).unwrap_or_default();
    s.active_label_class = active.and_then(|w| w.class.clone());
    s.active_label_color = active
        .map(|w| color_for_number(w.number))
        .unwrap_or(LABEL_COLORS[0]);
    s.active_label_number = active.map(|w| w.number).unwrap_or(0);

    if s.active_label_text.is_empty() {
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        return;
    }

    let active_hwnd = active.unwrap().hwnd;
    let mut rect = RECT::default();
    let _ = GetClientRect(active_hwnd, &mut rect);
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let _ = ClientToScreen(active_hwnd, &mut top_left);
    let mut top_right = POINT {
        x: rect.right,
        y: rect.top,
    };
    let _ = ClientToScreen(active_hwnd, &mut top_right);

    let d = s.dpi_scale;
    let lh = s.label_height;
    let label_h = dpi(lh, d);

    // Measure text width using the actual font.
    let name_font = CreateFontW(
        dpi(lh - 12, d),
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let hdc = windows::Win32::Graphics::Gdi::GetDC(s.active_label_hwnd);
    let old_font = SelectObject(hdc, name_font);
    let wide: Vec<u16> = s.active_label_text.encode_utf16().collect();
    let mut text_size = windows::Win32::Foundation::SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut text_size);
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(name_font);
    let _ = windows::Win32::Graphics::Gdi::ReleaseDC(s.active_label_hwnd, hdc);

    // Badge width + optional class icon + padding + measured text + right padding.
    let badge_w = label_h;
    let icon_w = if s.active_label_class.is_some() {
        badge_w + dpi(6, d)
    } else {
        0
    };
    let text_width = badge_w + dpi(6, d) + icon_w + text_size.cx + dpi(10, d);

    // When PiP edge is left, anchor the label at top-right so the strip doesn't cover it.
    let label_x = if matches!(s.pip_edge, config::PipEdge::Left) {
        top_right.x - text_width
    } else {
        top_left.x
    };

    let _ = SetWindowPos(
        s.active_label_hwnd,
        HWND_TOPMOST,
        label_x,
        top_left.y,
        text_width,
        label_h,
        SWP_NOACTIVATE,
    );

    // Color key transparency: magenta pixels become fully transparent, rest gets alpha.
    let key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
    let _ = SetLayeredWindowAttributes(
        s.active_label_hwnd,
        key,
        s.label_alpha,
        LWA_ALPHA | LWA_COLORKEY,
    );

    let _ = InvalidateRect(s.active_label_hwnd, None, true);

    // Position the explicit keyboard/mouse input indicator next to the active label.
    if let Some(bc_text) = input_indicator_text() {
        let bc_font = CreateFontW(
            dpi(lh - 12, d),
            0,
            0,
            0,
            FW_HEAVY.0 as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            w!("Segoe UI"),
        );
        let bc_hdc = windows::Win32::Graphics::Gdi::GetDC(s.broadcast_label_hwnd);
        let bc_old = SelectObject(bc_hdc, bc_font);
        let bc_wide: Vec<u16> = bc_text.encode_utf16().collect();
        let mut bc_size = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(bc_hdc, &bc_wide, &mut bc_size);
        let _ = SelectObject(bc_hdc, bc_old);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(bc_font);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(s.broadcast_label_hwnd, bc_hdc);
        let bc_width = bc_size.cx + dpi(20, d);
        let bc_x = if matches!(s.pip_edge, config::PipEdge::Left) {
            label_x - bc_width - dpi(4, d)
        } else {
            label_x + text_width + dpi(4, d)
        };
        let _ = SetWindowPos(
            s.broadcast_label_hwnd,
            HWND_TOPMOST,
            bc_x,
            top_left.y,
            bc_width,
            label_h,
            SWP_NOACTIVATE,
        );
        let _ = SetLayeredWindowAttributes(
            s.broadcast_label_hwnd,
            key,
            s.label_alpha,
            LWA_ALPHA | LWA_COLORKEY,
        );
        let _ = InvalidateRect(s.broadcast_label_hwnd, None, true);
    } else {
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

unsafe fn update_visibility(s: &mut OverlayState) {
    if s.hidden_by_user {
        for pw in &mut s.pip_windows {
            pw.hovered = false;
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
            let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
        }
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
        s.toast.phase = ToastPhase::Hidden;
        return;
    }

    let has_pip = !s.pip_order.is_empty();
    let fg = GetForegroundWindow();
    let eq_or_ours = is_eq_or_ours(fg, s);
    let visible = has_pip && (s.context_menu_open || eq_or_ours);

    if visible {
        // Show PiP thumbnail windows first, then labels on top.
        // ShowWindow first, then SetWindowPos to re-assert z-order
        // (interactions with a PiP can promote it above its label).
        for pw in &s.pip_windows {
            let _ = ShowWindow(pw.hwnd, SW_SHOWNOACTIVATE);
        }
        for pw in &s.pip_windows {
            let _ = ShowWindow(pw.label_hwnd, SW_SHOWNOACTIVATE);
        }
        // Re-assert z-order: labels above thumbnails.
        for pw in &s.pip_windows {
            let _ = SetWindowPos(
                pw.label_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        if !s.active_label_text.is_empty() {
            let _ = ShowWindow(s.active_label_hwnd, SW_SHOWNOACTIVATE);
        }
        if input_indicator_text().is_some() {
            let _ = ShowWindow(s.broadcast_label_hwnd, SW_SHOWNOACTIVATE);
        } else {
            let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        }
    } else {
        for pw in &mut s.pip_windows {
            pw.hovered = false;
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
            let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
        }
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
        s.toast.phase = ToastPhase::Hidden;
    }
}

// ---------------------------------------------------------------------------
// Foreground event hook
// ---------------------------------------------------------------------------

unsafe extern "system" fn foreground_event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dw_ms_event_time: u32,
) {
    if IN_OVERLAY.get() {
        return;
    }
    IN_OVERLAY.set(true);

    let Some(s) = state_unguarded().as_mut() else {
        IN_OVERLAY.set(false);
        return;
    };

    let fg = GetForegroundWindow();
    if let Some(fg_pid) =
        focused_foreground_pid(&s.eq_windows, fg, |hwnd| target_has_keyboard_focus(hwnd))
    {
        acknowledge_notification(s, fg_pid);
        if exchange_active_with_pip(&mut s.active_pid, &mut s.pip_order, fg_pid) {
            if config::Config::load().auto_order {
                apply_auto_order(s);
            }
            apply_alt_tab_hiding(s);
            rebuild_thumbnails(s);
        }
        // Keep broadcast suppression synchronized with WinEvent foreground
        // changes immediately instead of waiting for the next process poll.
        crate::broadcast::set_active_pid(fg_pid);
        sync_mouse_eligibility(s);
    }

    update_visibility(s);
    debug_assert_client_partition(s);
    publish_control_state(s);
    IN_OVERLAY.set(false);
}

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForegroundRequest {
    Confirmed,
    TargetDisappeared,
    TargetUnresponsive,
    ForegroundDenied,
    FocusDenied,
}

/// A short-lived connection between two GUI input queues.
///
/// Keeping this guard scoped is important: attached queues process input as one
/// queue, and Windows resets their keyboard state when they are attached.
struct InputQueueAttachment {
    current_thread: u32,
    target_thread: u32,
    attached: bool,
}

impl InputQueueAttachment {
    unsafe fn attach(current_thread: u32, target_thread: u32) -> Option<Self> {
        if current_thread == target_thread {
            return Some(Self {
                current_thread,
                target_thread,
                attached: false,
            });
        }
        if !AttachThreadInput(current_thread, target_thread, true).as_bool() {
            return None;
        }
        Some(Self {
            current_thread,
            target_thread,
            attached: true,
        })
    }
}

impl Drop for InputQueueAttachment {
    fn drop(&mut self) {
        if self.attached {
            unsafe {
                let _ = AttachThreadInput(self.current_thread, self.target_thread, false);
            }
        }
    }
}

unsafe fn window_is_responsive(hwnd: HWND) -> bool {
    let mut result = 0usize;
    SendMessageTimeoutW(
        hwnd,
        WM_NULL,
        WPARAM(0),
        LPARAM(0),
        SMTO_ABORTIFHUNG,
        50,
        Some(&mut result),
    )
    .0 != 0
}

unsafe fn target_has_keyboard_focus(hwnd: HWND) -> bool {
    if GetForegroundWindow() != hwnd {
        return false;
    }
    let target_thread = GetWindowThreadProcessId(hwnd, None);
    if target_thread == 0 {
        return false;
    }
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if GetGUIThreadInfo(target_thread, &mut info).is_err() || info.hwndActive != hwnd {
        return false;
    }
    info.hwndFocus == hwnd
        || (!info.hwndFocus.is_invalid() && IsChild(hwnd, info.hwndFocus).as_bool())
}

/// Once the target is foreground, repair a missing keyboard-focus assignment.
/// SetFocus may target another process only while its input queue is attached
/// to ours, so the attachment is kept to this single call and always released.
unsafe fn repair_keyboard_focus(hwnd: HWND) -> bool {
    let target_thread = GetWindowThreadProcessId(hwnd, None);
    if target_thread == 0 {
        return false;
    }
    let Some(attachment) = InputQueueAttachment::attach(GetCurrentThreadId(), target_thread) else {
        return false;
    };
    let _ = SetFocus(hwnd);
    drop(attachment);
    target_has_keyboard_focus(hwnd)
}

/// EQ's DirectInput mouse can remain unacquired after a programmatic
/// activation even when Windows confirms foreground and keyboard focus. Mouse
/// Clutch showed that a fresh WM_ACTIVATEAPP(TRUE) makes EQ run its input
/// activation path again. Queue it only after real activation is confirmed;
/// this message does not grant foreground or focus by itself.
unsafe fn reassert_eq_mouse_activation(hwnd: HWND) {
    let _ = PostMessageW(hwnd, WM_ACTIVATEAPP, WPARAM(1), LPARAM(0));
}

unsafe fn confirm_foreground_and_focus(hwnd: HWND) -> ForegroundRequest {
    if !IsWindow(hwnd).as_bool() {
        return ForegroundRequest::TargetDisappeared;
    }
    if GetForegroundWindow() != hwnd {
        return ForegroundRequest::ForegroundDenied;
    }
    if target_has_keyboard_focus(hwnd) {
        return ForegroundRequest::Confirmed;
    }
    if !window_is_responsive(hwnd) {
        return if IsWindow(hwnd).as_bool() {
            ForegroundRequest::TargetUnresponsive
        } else {
            ForegroundRequest::TargetDisappeared
        };
    }
    if repair_keyboard_focus(hwnd) {
        ForegroundRequest::Confirmed
    } else if !IsWindow(hwnd).as_bool() {
        ForegroundRequest::TargetDisappeared
    } else if GetForegroundWindow() != hwnd {
        ForegroundRequest::ForegroundDenied
    } else {
        ForegroundRequest::FocusDenied
    }
}

/// Bring one live EQ HWND to the actual Windows foreground and ensure its
/// window tree owns keyboard focus without changing Stonemite's active/PiP
/// model. The model is committed only after both properties are confirmed.
unsafe fn denied_or_disappeared(hwnd: HWND) -> ForegroundRequest {
    if IsWindow(hwnd).as_bool() {
        ForegroundRequest::ForegroundDenied
    } else {
        ForegroundRequest::TargetDisappeared
    }
}

unsafe fn request_foreground(hwnd: HWND) -> ForegroundRequest {
    if !IsWindow(hwnd).as_bool() {
        return ForegroundRequest::TargetDisappeared;
    }
    if GetForegroundWindow() == hwnd {
        return confirm_foreground_and_focus(hwnd);
    }

    if IsIconic(hwnd).as_bool() {
        let _ = ShowWindowAsync(hwnd, SW_RESTORE);
    }
    let _ = SetForegroundWindow(hwnd);
    if GetForegroundWindow() == hwnd {
        return confirm_foreground_and_focus(hwnd);
    }

    // A remote integration command does not carry Windows foreground rights.
    // For responsive windows only, briefly share the current foreground input
    // queue, retry synchronously, and always detach before returning.
    if !window_is_responsive(hwnd) {
        return if IsWindow(hwnd).as_bool() {
            ForegroundRequest::TargetUnresponsive
        } else {
            ForegroundRequest::TargetDisappeared
        };
    }
    let foreground = GetForegroundWindow();
    if foreground.is_invalid() || !window_is_responsive(foreground) {
        return denied_or_disappeared(hwnd);
    }
    let current_thread = GetCurrentThreadId();
    let foreground_thread = GetWindowThreadProcessId(foreground, None);
    if foreground_thread == 0 {
        return denied_or_disappeared(hwnd);
    }
    let Some(attachment) = InputQueueAttachment::attach(current_thread, foreground_thread) else {
        return denied_or_disappeared(hwnd);
    };

    let _ = BringWindowToTop(hwnd);
    let _ = SetForegroundWindow(hwnd);
    drop(attachment);

    if GetForegroundWindow() == hwnd {
        confirm_foreground_and_focus(hwnd)
    } else {
        denied_or_disappeared(hwnd)
    }
}

fn foreground_request_error(request: ForegroundRequest) -> trushar::control::ControlError {
    let (code, message) = match request {
        ForegroundRequest::TargetDisappeared => (
            trushar::control::ErrorCode::TargetDisappeared,
            "the target EQ window is no longer loaded",
        ),
        ForegroundRequest::TargetUnresponsive => (
            trushar::control::ErrorCode::ActivationFailed,
            "the target EQ window is not responding",
        ),
        ForegroundRequest::ForegroundDenied => (
            trushar::control::ErrorCode::ActivationFailed,
            "Windows did not bring the target EQ window to the foreground",
        ),
        ForegroundRequest::FocusDenied => (
            trushar::control::ErrorCode::ActivationFailed,
            "Windows foregrounded the target EQ window but did not give it keyboard focus",
        ),
        ForegroundRequest::Confirmed => (
            trushar::control::ErrorCode::ActivationFailed,
            "the target EQ window activation failed",
        ),
    };
    trushar::control::ControlError::new(code, message)
}

/// Swap to the window with the given stable number (1-based).
/// Called from hotkey handlers.
pub unsafe fn swap_to_number(number: usize) {
    let target_pid = state().as_ref().and_then(|s| {
        s.eq_windows
            .iter()
            .find(|w| w.number == number)
            .map(|w| w.pid)
    });
    if let Some(target_pid) = target_pid {
        let _ = activate_pid(target_pid);
    }
}

/// Swap the selected client's stable window number with the active client's number.
/// The foreground client does not change.
pub unsafe fn swap_active_window_numbers(
    target_pid: u32,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    if IN_OVERLAY.replace(true) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is already handling a window transition",
        ));
    }
    let result = swap_active_window_numbers_guarded(target_pid);
    IN_OVERLAY.set(false);
    result
}

unsafe fn swap_active_window_numbers_guarded(
    target_pid: u32,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    let Some(s) = state_unguarded().as_mut() else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is unavailable",
        ));
    };
    let Some(active_pid) = s.active_pid else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::WindowNumberSwapFailed,
            "there is no active EQ client whose window number can be swapped",
        ));
    };
    if !s.eq_windows.iter().any(|window| window.pid == target_pid) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the selected EQ window is no longer loaded",
        ));
    }
    let Some((active_previous_number, selected_previous_number)) =
        exchange_window_numbers(&mut s.eq_windows, active_pid, target_pid)
    else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::WindowNumberSwapFailed,
            "the active EQ window is no longer loaded",
        ));
    };

    if active_pid != target_pid {
        if config::Config::load().auto_order {
            apply_auto_order(s);
        }
        rebuild_thumbnails_guarded(s);
        update_visibility(s);
        show_toast_inner(
            s,
            &format!(
                "Swapped window numbers #{} and #{}",
                active_previous_number, selected_previous_number
            ),
        );
        debug_assert_client_partition(s);
        publish_control_state(s);
    }

    Ok(trushar::control::CommandOutcome::WindowNumbersSwapped {
        active_previous_number,
        selected_previous_number,
    })
}

/// Authoritative semantic activation operation used by local UI and trushar.
pub unsafe fn activate_pid(
    target_pid: u32,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    let Some(s) = state().as_ref() else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is unavailable",
        ));
    };
    let Some(target_window) = s.eq_windows.iter().find(|window| window.pid == target_pid) else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target is no longer loaded",
        ));
    };
    if s.active_pid == Some(target_pid) {
        let target_hwnd = target_window.hwnd;
        let _ = s;
        let result = reassert_active_foreground(target_hwnd);
        if result.is_ok() {
            if let Some(s) = state().as_mut() {
                acknowledge_notification(s, target_pid);
            }
        }
        return result;
    }
    let Some(pip_index) = s.pip_order.iter().position(|pid| *pid == target_pid) else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::ActivationFailed,
            "the loaded client is outside the supported activation set",
        ));
    };
    let _ = s;
    swap_to(pip_index)
}

unsafe fn reassert_active_foreground(
    target_hwnd: HWND,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    if IN_OVERLAY.replace(true) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is already handling a window transition",
        ));
    }
    let request = request_foreground(target_hwnd);
    IN_OVERLAY.set(false);
    if request != ForegroundRequest::Confirmed {
        return Err(foreground_request_error(request));
    }
    reassert_eq_mouse_activation(target_hwnd);
    Ok(trushar::control::CommandOutcome::Activated {
        status: trushar::control::ActivationStatus::AlreadyActive,
        foreground_confirmed: true,
    })
}

unsafe fn swap_to(
    pip_index: usize,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    // DWM and window-management calls below can pump messages. Guard the whole
    // state transition so a reentrant foreground callback cannot alias and
    // mutate OverlayState while the active/PiP partition is being rebuilt.
    if IN_OVERLAY.replace(true) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is already handling a window transition",
        ));
    }
    let result = swap_to_guarded(pip_index);
    IN_OVERLAY.set(false);
    result
}

unsafe fn swap_to_guarded(
    pip_index: usize,
) -> Result<trushar::control::CommandOutcome, trushar::control::ControlError> {
    let Some(s) = state_unguarded().as_mut() else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "overlay is unavailable",
        ));
    };

    if pip_index >= s.pip_order.len() {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared before activation",
        ));
    }
    if s.active_pid.is_none() {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::ActivationFailed,
            "there is no active EQ client to exchange",
        ));
    }
    let new_active_pid = s.pip_order[pip_index];
    let Some(new_active_hwnd) = s
        .eq_windows
        .iter()
        .find(|window| window.pid == new_active_pid)
        .map(|window| window.hwnd)
    else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared before activation",
        ));
    };

    let request = request_foreground(new_active_hwnd);
    if request != ForegroundRequest::Confirmed {
        return Err(foreground_request_error(request));
    }

    let previous_active_pid = s.active_pid;
    let previous_pip_order = s.pip_order.clone();
    if !exchange_active_with_pip(&mut s.active_pid, &mut s.pip_order, new_active_pid) {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::InternalError,
            "the active/PiP partition changed during activation",
        ));
    }
    // Confirm again after the pure partition exchange but before any detached
    // style work. Rollback is then limited to in-memory state and cannot race
    // asynchronous Alt-Tab style changes.
    let final_request = request_foreground(new_active_hwnd);
    if final_request != ForegroundRequest::Confirmed {
        s.active_pid = previous_active_pid;
        s.pip_order = previous_pip_order;
        return Err(foreground_request_error(final_request));
    }

    crate::broadcast::set_active_pid(new_active_pid);
    sync_mouse_eligibility(s);
    acknowledge_notification(s, new_active_pid);
    if config::Config::load().auto_order {
        apply_auto_order(s);
    }
    apply_alt_tab_hiding(s);
    rebuild_thumbnails(s);
    update_visibility(s);

    // Toast notification for the swap.
    let toast_label = if let Some(w) = s.eq_windows.iter().find(|w| w.pid == new_active_pid) {
        match &w.character {
            Some(name) => format!("Swapped to #{} {}", w.number, name),
            None => format!("Swapped to #{}", w.number),
        }
    } else {
        return Err(trushar::control::ControlError::new(
            trushar::control::ErrorCode::TargetDisappeared,
            "the target disappeared during activation",
        ));
    };
    show_toast_inner(s, &toast_label);
    debug_assert_client_partition(s);
    publish_control_state(s);
    // Reassert last, after all window churn, so EQ reacquires input while its
    // real foreground and focus state are stable.
    reassert_eq_mouse_activation(new_active_hwnd);
    Ok(trushar::control::CommandOutcome::Activated {
        status: trushar::control::ActivationStatus::Activated,
        foreground_confirmed: true,
    })
}

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------

unsafe fn show_char_menu(
    s: &mut OverlayState,
    target_pid: u32,
    screen_pt: POINT,
    owner_hwnd: HWND,
) {
    let cfg = config::Config::load();
    let eq_dir = cfg.eq_directory();
    let candidates = eq_characters::find_active_characters(&eq_dir, Duration::from_secs(86400));

    let Ok(hmenu) = CreatePopupMenu() else { return };

    // Character assignment submenu, grouped by server.
    if let Ok(char_menu) = CreatePopupMenu() {
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
                let _ = AppendMenuW(
                    char_menu,
                    MF_STRING,
                    (IDM_CHAR_BASE + i as u32) as usize,
                    windows::core::PCWSTR(wide.as_ptr()),
                );
            }
        } else {
            for server in &servers {
                let Ok(server_menu) = CreatePopupMenu() else {
                    continue;
                };
                for (i, c) in candidates.iter().enumerate() {
                    if c.server != *server {
                        continue;
                    }
                    let label = format!("{}\0", c.character);
                    let wide: Vec<u16> = label.encode_utf16().collect();
                    let _ = AppendMenuW(
                        server_menu,
                        MF_STRING,
                        (IDM_CHAR_BASE + i as u32) as usize,
                        windows::core::PCWSTR(wide.as_ptr()),
                    );
                }
                let server_label = format!("{server}\0");
                let wide: Vec<u16> = server_label.encode_utf16().collect();
                let _ = AppendMenuW(
                    char_menu,
                    MF_POPUP,
                    server_menu.0 as usize,
                    windows::core::PCWSTR(wide.as_ptr()),
                );
            }
        }

        if !candidates.is_empty() {
            let assign_label: Vec<u16> = "Assign character\0".encode_utf16().collect();
            let _ = AppendMenuW(
                hmenu,
                MF_POPUP,
                char_menu.0 as usize,
                windows::core::PCWSTR(assign_label.as_ptr()),
            );
        }
    }

    // Number reassignment submenu.
    if let Ok(num_menu) = CreatePopupMenu() {
        for n in 1..=s.eq_windows.len() {
            let label = format!("#{n}\0");
            let wide: Vec<u16> = label.encode_utf16().collect();
            let _ = AppendMenuW(
                num_menu,
                MF_STRING,
                (IDM_NUMBER_BASE + n as u32) as usize,
                windows::core::PCWSTR(wide.as_ptr()),
            );
        }
        let num_label: Vec<u16> = "Assign number\0".encode_utf16().collect();
        let _ = AppendMenuW(
            hmenu,
            MF_POPUP,
            num_menu.0 as usize,
            windows::core::PCWSTR(num_label.as_ptr()),
        );
    }

    // PiP Edge submenu.
    let Ok(edge_menu) = CreatePopupMenu() else {
        let _ = DestroyMenu(hmenu);
        return;
    };
    let edge_options = [
        (config::PipEdge::Right, "Right"),
        (config::PipEdge::Left, "Left"),
        (config::PipEdge::Top, "Top"),
        (config::PipEdge::Bottom, "Bottom"),
    ];
    for (i, (edge, label)) in edge_options.iter().enumerate() {
        let text = format!("{label}\0");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let flags = if *edge == s.pip_edge {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(
            edge_menu,
            flags,
            (IDM_EDGE_BASE + i as u32) as usize,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
    let edge_label: Vec<u16> = "PiP edge\0".encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_POPUP,
        edge_menu.0 as usize,
        windows::core::PCWSTR(edge_label.as_ptr()),
    );

    // Edit/Lock layout toggle.
    let edit_label = if s.edit_mode {
        "Lock layout\0"
    } else {
        "Edit layout\0"
    };
    let edit_wide: Vec<u16> = edit_label.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_EDIT_MODE as usize,
        windows::core::PCWSTR(edit_wide.as_ptr()),
    );

    // Reset to auto layout (only when custom positions exist).
    if s.has_custom_positions {
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_RESET_LAYOUT as usize,
            w!("Reset to auto layout"),
        );
    }

    // Broadcasting toggle (only shown if trusik is enabled).
    if cfg.trusik {
        let bc_label = if crate::broadcast::is_active() {
            format!("Broadcasting: on\t{}\0", cfg.broadcast_hotkey)
        } else {
            format!("Broadcasting: off\t{}\0", cfg.broadcast_hotkey)
        };
        let bc_wide: Vec<u16> = bc_label.encode_utf16().collect();
        let bc_flag = if crate::broadcast::is_active() {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | bc_flag,
            IDM_BROADCAST_TOGGLE as usize,
            windows::core::PCWSTR(bc_wide.as_ptr()),
        );
    }

    // Hide overlay item with hotkey hint.
    let hide_label = format!("Hide overlay\t{}\0", cfg.hide_hotkey);
    let hide_wide: Vec<u16> = hide_label.encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_HIDE_OVERLAY as usize,
        windows::core::PCWSTR(hide_wide.as_ptr()),
    );

    let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
    let settings_label: Vec<u16> = "Settings...\0".encode_utf16().collect();
    let _ = AppendMenuW(
        hmenu,
        MF_STRING,
        IDM_SETTINGS as usize,
        windows::core::PCWSTR(settings_label.as_ptr()),
    );

    s.context_menu_target_pid = Some(target_pid);
    s.context_menu_candidates = candidates;
    s.context_menu_open = true;

    let _ = SetForegroundWindow(owner_hwnd);
    // Re-assert PiP label z-order after SetForegroundWindow promoted the PiP window.
    update_visibility(s);
    let _ = TrackPopupMenu(
        hmenu,
        TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON,
        screen_pt.x,
        screen_pt.y,
        0,
        owner_hwnd,
        None,
    );

    if let Some(s) = state().as_mut() {
        s.context_menu_open = false;
        update_visibility(s);
    }
    let _ = DestroyMenu(hmenu);
    let _ = PostMessageW(owner_hwnd, WM_NULL, WPARAM(0), LPARAM(0));
}

unsafe fn handle_menu_command(cmd_id: u32) {
    let Some(s) = state().as_mut() else { return };

    if cmd_id == IDM_BROADCAST_TOGGLE {
        let _ = crate::broadcast::toggle();
        update_active_label(s);
        update_visibility(s);
        publish_control_state(s);
    } else if cmd_id == IDM_HIDE_OVERLAY {
        s.hidden_by_user = true;
        update_visibility(s);
    } else if cmd_id == IDM_SETTINGS {
        crate::settings_dialog::show();
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
    } else if (IDM_EDGE_BASE..IDM_EDGE_BASE + 4).contains(&cmd_id) {
        handle_edge_assign(s, cmd_id);
    } else if (IDM_NUMBER_BASE..IDM_NUMBER_BASE + 100).contains(&cmd_id) {
        let number = (cmd_id - IDM_NUMBER_BASE) as usize;
        handle_number_assign(s, number);
    } else if (IDM_CHAR_BASE..IDM_CHAR_BASE + 100).contains(&cmd_id) {
        handle_char_assign(s, cmd_id);
    }
}

unsafe fn handle_char_assign(s: &mut OverlayState, cmd_id: u32) {
    let char_idx = (cmd_id - IDM_CHAR_BASE) as usize;
    let Some(target_pid) = s.context_menu_target_pid.take() else {
        return;
    };
    let candidates = std::mem::take(&mut s.context_menu_candidates);

    let Some(candidate) = candidates.get(char_idx) else {
        return;
    };

    if let Some(w) = s.eq_windows.iter_mut().find(|w| w.pid == target_pid) {
        w.class = s
            .character_cache
            .get_class(&candidate.server, &candidate.character)
            .map(String::from);
        w.character = Some(candidate.character.clone());
        w.server = Some(candidate.server.clone());
    }
    s.character_cache
        .remember(&candidate.server, &candidate.character);
    s.character_cache.save();

    if apply_preferred_box_order(&mut s.eq_windows, &s.preferred_box_order)
        && config::Config::load().auto_order
    {
        apply_auto_order(s);
    }
    publish_log_sources_inner(s);
    rebuild_thumbnails(s);
    publish_control_state(s);
}

unsafe fn handle_number_assign(s: &mut OverlayState, new_number: usize) {
    let Some(target_pid) = s.context_menu_target_pid.take() else {
        return;
    };
    let _ = std::mem::take(&mut s.context_menu_candidates);

    let old_number = s
        .eq_windows
        .iter()
        .find(|w| w.pid == target_pid)
        .map(|w| w.number)
        .unwrap_or(0);
    // Swap numbers with any window that already has new_number.
    // If the target had no number (0), assign the displaced window the next available number.
    let replacement = if old_number > 0 {
        old_number
    } else {
        // Target was unassigned — give displaced window the next free number.
        let mut n = 1;
        while s.eq_windows.iter().any(|w| w.number == n) || n == new_number {
            n += 1;
        }
        n
    };
    if let Some(other) = s
        .eq_windows
        .iter_mut()
        .find(|w| w.number == new_number && w.pid != target_pid)
    {
        other.number = replacement;
    }
    if let Some(w) = s.eq_windows.iter_mut().find(|w| w.pid == target_pid) {
        w.number = new_number;
    }

    rebuild_thumbnails(s);
    publish_control_state(s);
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
        let _ = InvalidateRect(pw.label_hwnd, None, true);
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
    state().as_ref().is_some_and(|state| state.edit_mode)
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

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    // Black background.
    let black_brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);
    let _ = FillRect(hdc, &ps.rcPaint, black_brush);

    let mut client_rect = RECT::default();
    let _ = GetClientRect(hwnd, &mut client_rect);

    // Determine drag visual state.
    let is_reorder_dragging = s.reorder_drag.as_ref().is_some_and(|drag| drag.dragging);
    let is_drag_source =
        is_reorder_dragging && s.reorder_drag.as_ref().map(|d| d.from_index) == Some(pip_idx);
    let is_drop_target = is_reorder_dragging
        && s.drop_target == Some(pip_idx)
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
                left: client_rect.left + inset,
                top: client_rect.top + inset,
                right: client_rect.right - inset,
                bottom: client_rect.bottom - inset,
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
                left: client_rect.left + inset,
                top: client_rect.top + inset,
                right: client_rect.right - inset,
                bottom: client_rect.bottom - inset,
            };
            let _ = FrameRect(hdc, &r, white_brush);
        }
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(white_brush);
    }

    // Notifications own the normal frame, while edit and reorder modes retain
    // their stronger interaction indicators.
    if !is_reorder_dragging && !s.edit_mode {
        if let Some(notification) = s.notifications.get(&pw.pid) {
            notifications::draw_border(
                hdc,
                client_rect,
                border,
                notification,
                windows::Win32::System::SystemInformation::GetTickCount64(),
                s.animations_enabled,
            );
        }
    }

    // Edit mode border indicator.
    if s.edit_mode {
        let edit_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(EDIT_BORDER_COLOR));
        let _ = FrameRect(hdc, &client_rect, edit_brush);
        let r2 = RECT {
            left: client_rect.left + 1,
            top: client_rect.top + 1,
            right: client_rect.right - 1,
            bottom: client_rect.bottom - 1,
        };
        let _ = FrameRect(hdc, &r2, edit_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(edit_brush);
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe fn paint_label(hwnd: HWND, text: &str, class: Option<&str>, bg_color: u32) {
    let (d, number, lh) = state()
        .as_ref()
        .map(|s| (s.dpi_scale, s.active_label_number, s.label_height))
        .unwrap_or((1.0, 0, DEFAULT_LABEL_HEIGHT));
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let label_h = rc.bottom - rc.top;

    // Fill entire window with color key (becomes transparent via LWA_COLORKEY).
    let key_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY));
    let _ = FillRect(hdc, &rc, key_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(key_brush);

    // Rounded background.
    let radius = dpi(8, d);
    let bg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(bg_color));
    let null_pen = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, bg_brush);
    let _ = RoundRect(
        hdc,
        rc.left,
        rc.top,
        rc.right,
        rc.bottom,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg_brush);

    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    // Number badge circle.
    let badge_diameter = label_h - dpi(6, d);
    let badge_x = rc.left + dpi(4, d);
    let badge_y = rc.top + (label_h - badge_diameter) / 2;
    let badge_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(
        badge_color_for_number(number),
    ));
    let null_pen2 = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let op2 = SelectObject(hdc, null_pen2);
    let ob2 = SelectObject(hdc, badge_brush);
    let _ = Ellipse(
        hdc,
        badge_x,
        badge_y,
        badge_x + badge_diameter,
        badge_y + badge_diameter,
    );
    let _ = SelectObject(hdc, ob2);
    let _ = SelectObject(hdc, op2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_brush);

    // Number text in badge.
    let badge_font = CreateFontW(
        dpi(lh - 14, d),
        0,
        0,
        0,
        FW_HEAVY.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, badge_font);
    let mut badge_rect = RECT {
        left: badge_x,
        top: badge_y,
        right: badge_x + badge_diameter,
        bottom: badge_y + badge_diameter,
    };
    let num_str = format!("{number}");
    let mut num_wide: Vec<u16> = num_str.encode_utf16().collect();
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = DrawTextW(
        hdc,
        &mut num_wide,
        &mut badge_rect,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_font);

    // Class icon badge (same size as number badge).
    let mut after_badges = badge_x + badge_diameter + dpi(6, d);
    if let Some(cls) = class {
        let icon_x = after_badges;
        let icon_y = badge_y;
        let icon_size = badge_diameter;
        crate::class_icons::draw_class_icon(hdc, cls, icon_x, icon_y, icon_size);
        after_badges = icon_x + icon_size + dpi(6, d);
    }

    // Character name with shadow.
    let name_font = CreateFontW(
        dpi(lh - 12, d),
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font2 = SelectObject(hdc, name_font);
    let text_left = after_badges;
    let mut wide: Vec<u16> = text.encode_utf16().collect();

    if !wide.is_empty() {
        // Shadow.
        let mut shadow_rc = RECT {
            left: text_left + dpi(1, d),
            top: rc.top + dpi(1, d),
            right: rc.right,
            bottom: rc.bottom,
        };
        let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00000000));
        let _ = DrawTextW(
            hdc,
            &mut wide,
            &mut shadow_rc,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        // Main text (white).
        let mut text_rc = RECT {
            left: text_left,
            top: rc.top,
            right: rc.right,
            bottom: rc.bottom,
        };
        let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
        let _ = DrawTextW(
            hdc,
            &mut wide,
            &mut text_rc,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
    }

    let _ = SelectObject(hdc, old_font2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(name_font);
    let _ = EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------------------
// PiP label overlay helpers
// ---------------------------------------------------------------------------

/// Measure the width needed for a PiP label, capped at max_w.
unsafe fn measure_pip_label_size(
    hwnd: HWND,
    s: &OverlayState,
    text: &str,
    class: Option<&str>,
    max_w: i32,
) -> (i32, i32) {
    let d = s.dpi_scale;
    let lh = s.label_height;
    let label_h = dpi(lh, d);
    let font = CreateFontW(
        dpi(lh - 12, d),
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let hdc = windows::Win32::Graphics::Gdi::GetDC(hwnd);
    let old = SelectObject(hdc, font);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut sz = windows::Win32::Foundation::SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut sz);
    let _ = SelectObject(hdc, old);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = windows::Win32::Graphics::Gdi::ReleaseDC(hwnd, hdc);
    let badge_w = label_h;
    let icon_w = if class.is_some() {
        badge_w + dpi(6, d)
    } else {
        0
    };
    let w = (badge_w + dpi(6, d) + icon_w + sz.cx + dpi(10, d)).min(max_w);
    (w, label_h)
}

unsafe fn paint_pip_label(hwnd: HWND) {
    let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    if raw_idx == 0 {
        return;
    }
    let pip_idx = raw_idx - 1;

    let Some(s) = state().as_ref() else {
        return;
    };
    let Some(pw) = s.pip_windows.get(pip_idx) else {
        return;
    };

    let d = s.dpi_scale;
    let lh = s.label_height;
    let number = pw.number;
    let text = &pw.label;
    let notification = s.notifications.get(&pw.pid);
    let now_ms = windows::Win32::System::SystemInformation::GetTickCount64();

    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let label_h = dpi(lh, d).min(rc.bottom - rc.top).max(1);
    let max_label_w = (rc.right - rc.left).max(1);
    let (label_w, _) = measure_pip_label_size(hwnd, s, text, pw.class.as_deref(), max_label_w);
    let label_rc = RECT {
        left: rc.left,
        top: rc.top,
        right: rc.left + label_w,
        bottom: rc.top + label_h,
    };

    // The unused portion of this full-width overlay is color-key transparent.
    let key_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY));
    let _ = FillRect(hdc, &rc, key_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(key_brush);

    let radius = dpi(8, d);
    let bg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(color_for_number(
        number,
    )));
    let null_pen = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, bg_brush);
    let _ = RoundRect(
        hdc,
        label_rc.left,
        label_rc.top,
        label_rc.right,
        label_rc.bottom,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg_brush);
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let badge_diameter = label_h - dpi(6, d);
    let badge_x = label_rc.left + dpi(4, d);
    let badge_y = label_rc.top + (label_h - badge_diameter) / 2;
    let badge_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(
        badge_color_for_number(number),
    ));
    let null_pen2 = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let old_pen2 = SelectObject(hdc, null_pen2);
    let old_brush2 = SelectObject(hdc, badge_brush);
    let _ = Ellipse(
        hdc,
        badge_x,
        badge_y,
        badge_x + badge_diameter,
        badge_y + badge_diameter,
    );
    let _ = SelectObject(hdc, old_brush2);
    let _ = SelectObject(hdc, old_pen2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_brush);

    let badge_font = CreateFontW(
        dpi(lh - 14, d),
        0,
        0,
        0,
        FW_HEAVY.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, badge_font);
    let mut badge_rect = RECT {
        left: badge_x,
        top: badge_y,
        right: badge_x + badge_diameter,
        bottom: badge_y + badge_diameter,
    };
    let mut num_wide: Vec<u16> = number.to_string().encode_utf16().collect();
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = DrawTextW(
        hdc,
        &mut num_wide,
        &mut badge_rect,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_font);

    let mut after_badges = badge_x + badge_diameter + dpi(6, d);
    if let Some(cls) = &pw.class {
        let icon_x = after_badges;
        let icon_y = badge_y;
        crate::class_icons::draw_class_icon(hdc, cls, icon_x, icon_y, badge_diameter);
        after_badges = icon_x + badge_diameter + dpi(6, d);
    }

    let name_font = CreateFontW(
        dpi(lh - 12, d),
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font2 = SelectObject(hdc, name_font);
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if !wide.is_empty() {
        let mut shadow_rc = RECT {
            left: after_badges + dpi(1, d),
            top: label_rc.top + dpi(1, d),
            right: label_rc.right,
            bottom: label_rc.bottom,
        };
        let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00000000));
        let _ = DrawTextW(
            hdc,
            &mut wide,
            &mut shadow_rc,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        let mut text_rc = RECT {
            left: after_badges,
            top: label_rc.top,
            right: label_rc.right,
            bottom: label_rc.bottom,
        };
        let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
        let _ = DrawTextW(
            hdc,
            &mut wide,
            &mut text_rc,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
    }
    let _ = SelectObject(hdc, old_font2);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(name_font);

    if let Some(notification) = notification {
        let unread_bottom =
            notifications::draw_unread_dots(hdc, rc, label_rc.bottom, d, notification);
        if notification.preview_visible(now_ms) {
            notifications::draw_preview(
                hdc,
                notifications::preview_bounds(rc, unread_bottom, d, notification),
                d,
                notification,
            );
        }
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn pip_label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_PAINT => {
            paint_pip_label(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ---------------------------------------------------------------------------
// Broadcast banner window proc
// ---------------------------------------------------------------------------

unsafe extern "system" fn broadcast_label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_PAINT => {
            paint_broadcast_label(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint_broadcast_label(hwnd: HWND) {
    let (d, lh) = state()
        .as_ref()
        .map(|s| (s.dpi_scale, s.label_height))
        .unwrap_or((1.0, DEFAULT_LABEL_HEIGHT));
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    // Fill with color key for transparent corners.
    let key_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY));
    let _ = FillRect(hdc, &rc, key_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(key_brush);

    // Keyboard Broadcast keeps its reserved red. Mouse-only activity uses teal;
    // bounded release/drain uses amber. Text makes every state redundant.
    let background = if crate::broadcast::is_active() {
        0x002030CC
    } else {
        match crate::broadcast::mouse_clutch_status() {
            crate::broadcast::MouseClutchStatus::Inactive => 0x002030CC,
            crate::broadcast::MouseClutchStatus::Active => 0x00906A28,
            crate::broadcast::MouseClutchStatus::Releasing => 0x002080C8,
        }
    };
    let radius = dpi(8, d);
    let bg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(background));
    let null_pen = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, bg_brush);
    let _ = RoundRect(
        hdc,
        rc.left,
        rc.top,
        rc.right,
        rc.bottom,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg_brush);

    let font = CreateFontW(
        dpi(lh - 12, d),
        0,
        0,
        0,
        FW_HEAVY.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let text = input_indicator_text().unwrap_or("Broadcasting");
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let pad = dpi(10, d);

    // Shadow.
    let mut shadow_rc = RECT {
        left: rc.left + pad + dpi(1, d),
        top: rc.top + dpi(1, d),
        right: rc.right,
        bottom: rc.bottom,
    };
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00000044));
    let _ = DrawTextW(
        hdc,
        &mut wide,
        &mut shadow_rc,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );

    // Main text (white).
    let mut text_rc = RECT {
        left: rc.left + pad,
        top: rc.top,
        right: rc.right,
        bottom: rc.bottom,
    };
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = DrawTextW(
        hdc,
        &mut wide,
        &mut text_rc,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );

    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------------------
// Toast notification
// ---------------------------------------------------------------------------

unsafe fn paint_toast(hwnd: HWND) {
    let Some(s) = state().as_ref() else {
        let mut ps = PAINTSTRUCT::default();
        let _ = BeginPaint(hwnd, &mut ps);
        let _ = EndPaint(hwnd, &ps);
        return;
    };
    let d = s.dpi_scale;
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    // Fill with color key for transparent corners.
    let key_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(TOAST_COLOR_KEY));
    let _ = FillRect(hdc, &rc, key_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(key_brush);

    // Rounded dark background.
    let radius = dpi(8, d);
    let bg_brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(TOAST_BG_COLOR));
    let null_pen = CreatePen(PS_NULL, 0, windows::Win32::Foundation::COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, bg_brush);
    let _ = RoundRect(
        hdc,
        rc.left,
        rc.top,
        rc.right,
        rc.bottom,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(bg_brush);

    let font_h = dpi(s.toast.height - 12, d).max(dpi(12, d));
    let font = CreateFontW(
        font_h,
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let mut wide: Vec<u16> = s.toast.text.encode_utf16().collect();

    // Shadow.
    let mut shadow_rc = RECT {
        left: rc.left + dpi(1, d),
        top: rc.top + dpi(1, d),
        right: rc.right + dpi(1, d),
        bottom: rc.bottom + dpi(1, d),
    };
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00000044));
    let _ = DrawTextW(
        hdc,
        &mut wide,
        &mut shadow_rc,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );

    // Main text (white).
    let mut text_rc = rc;
    let _ = SetTextColor(hdc, windows::Win32::Foundation::COLORREF(0x00FFFFFF));
    let _ = DrawTextW(
        hdc,
        &mut wide,
        &mut text_rc,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );

    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn toast_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_PAINT => {
            paint_toast(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_TOAST_FADE => {
            let Some(s) = state().as_mut() else {
                let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
                return LRESULT(0);
            };
            let now = windows::Win32::System::SystemInformation::GetTickCount64();
            match s.toast.phase {
                ToastPhase::FadingIn => {
                    let new_alpha = s.toast.alpha.saturating_add(TOAST_ALPHA_STEP);
                    if new_alpha >= TOAST_MAX_ALPHA {
                        s.toast.alpha = TOAST_MAX_ALPHA;
                        s.toast.phase = ToastPhase::Visible;
                        s.toast.phase_start = now;
                    } else {
                        s.toast.alpha = new_alpha;
                    }
                    let _ = SetLayeredWindowAttributes(
                        hwnd,
                        windows::Win32::Foundation::COLORREF(TOAST_COLOR_KEY),
                        s.toast.alpha,
                        LWA_ALPHA | LWA_COLORKEY,
                    );
                }
                ToastPhase::Visible => {
                    if now - s.toast.phase_start >= s.toast.duration_ms as u64 {
                        s.toast.phase = ToastPhase::FadingOut;
                    }
                }
                ToastPhase::FadingOut => {
                    let new_alpha = s.toast.alpha.saturating_sub(TOAST_ALPHA_STEP);
                    if new_alpha == 0 {
                        s.toast.alpha = 0;
                        s.toast.phase = ToastPhase::Hidden;
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
                    } else {
                        s.toast.alpha = new_alpha;
                    }
                    let _ = SetLayeredWindowAttributes(
                        hwnd,
                        windows::Win32::Foundation::COLORREF(TOAST_COLOR_KEY),
                        s.toast.alpha,
                        LWA_ALPHA | LWA_COLORKEY,
                    );
                }
                ToastPhase::Hidden => {
                    let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_toast_inner(s: &mut OverlayState, text: &str) {
    if !s.toast.enabled {
        return;
    }

    s.toast.text = text.to_string();
    s.toast.alpha = 0;
    s.toast.phase = ToastPhase::FadingIn;
    s.toast.phase_start = windows::Win32::System::SystemInformation::GetTickCount64();

    // Position centered on the active EQ window, near the top.
    let active_hwnd = s
        .active_pid
        .and_then(|pid| s.eq_windows.iter().find(|w| w.pid == pid))
        .map(|w| w.hwnd);
    let Some(eq_hwnd) = active_hwnd else {
        return;
    };

    let mut eq_rect = RECT::default();
    let _ = GetClientRect(eq_hwnd, &mut eq_rect);
    let mut top_left = POINT {
        x: eq_rect.left,
        y: eq_rect.top,
    };
    let _ = ClientToScreen(eq_hwnd, &mut top_left);

    let d = s.dpi_scale;
    let toast_h = dpi(s.toast.height, d);

    // Measure text width.
    let font_h = dpi(s.toast.height - 12, d).max(dpi(12, d));
    let font = CreateFontW(
        font_h,
        0,
        0,
        0,
        FW_BOLD.0 as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        w!("Segoe UI"),
    );
    let hdc = windows::Win32::Graphics::Gdi::GetDC(s.toast.hwnd);
    let old_font = SelectObject(hdc, font);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut text_size = windows::Win32::Foundation::SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut text_size);
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
    let _ = windows::Win32::Graphics::Gdi::ReleaseDC(s.toast.hwnd, hdc);

    let pad = dpi(20, d);
    let toast_w = text_size.cx + pad * 2;
    let eq_client_w = eq_rect.right - eq_rect.left;
    let toast_x = top_left.x + (eq_client_w - toast_w) / 2;
    let eq_client_h = eq_rect.bottom - eq_rect.top;
    let toast_y = top_left.y + eq_client_h / 3;

    let _ = SetWindowPos(
        s.toast.hwnd,
        HWND_TOPMOST,
        toast_x,
        toast_y,
        toast_w,
        toast_h,
        SWP_NOACTIVATE,
    );

    let _ = SetLayeredWindowAttributes(
        s.toast.hwnd,
        windows::Win32::Foundation::COLORREF(TOAST_COLOR_KEY),
        0,
        LWA_ALPHA | LWA_COLORKEY,
    );

    let _ = ShowWindow(s.toast.hwnd, SW_SHOWNOACTIVATE);
    let _ = InvalidateRect(s.toast.hwnd, None, true);
    let _ = SetTimer(s.toast.hwnd, TIMER_TOAST_FADE, TOAST_FADE_STEP_MS, None);
}

pub fn show_toast(text: &str) {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        show_toast_inner(s, text);
    }
}

// ---------------------------------------------------------------------------
// PiP window proc
// ---------------------------------------------------------------------------

unsafe extern "system" fn pip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Skip if we're inside a rebuild/poll to avoid re-entrant state access.
    if IN_OVERLAY.get() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    // Decode pip index from GWLP_USERDATA (1-based, 0 = not yet set).
    let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
    if raw_idx == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let pip_idx = raw_idx - 1;

    match msg {
        WM_MOUSEACTIVATE => {
            if let Some(s) = state().as_ref() {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
                if notifications::has_invite_preview_at(s, pip_idx, point) {
                    return LRESULT(MA_NOACTIVATE as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_SETCURSOR => {
            if (lparam.0 & 0xFFFF) as u32 == 1
            /* HTCLIENT */
            {
                if let Some(s) = state().as_ref() {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let mut client_pt = pt;
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client_pt);

                    let mut cr = RECT::default();
                    let _ = GetClientRect(hwnd, &mut cr);

                    if s.edit_mode {
                        let zone = dpi(RESIZE_ZONE, s.dpi_scale);
                        if let Some(edge) =
                            edit_resize_edge_hit_test(client_pt, cr.right, cr.bottom, zone)
                        {
                            let cursor_id = windows::core::PCWSTR(cursor_for_resize_edge(edge));
                            let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
                            SetCursor(cursor);
                            return LRESULT(1);
                        }
                        // Body → move cursor.
                        let cursor = LoadCursorW(None, IDC_SIZEALL).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
                    if notifications::has_invite_action_at(s, pip_idx, client_pt) {
                        let cursor = LoadCursorW(None, IDC_HAND).unwrap_or_default();
                        SetCursor(cursor);
                        return LRESULT(1);
                    }
                    if !s.has_custom_positions {
                        // Strip resize cursor on interior edge.
                        let handle_w = dpi(RESIZE_HANDLE_WIDTH, s.dpi_scale);
                        if strip_resize_hit_test(
                            client_pt, cr.right, cr.bottom, s.pip_edge, handle_w,
                        ) {
                            let cursor_id = if matches!(
                                s.pip_edge,
                                config::PipEdge::Right | config::PipEdge::Left
                            ) {
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
                let border = dpi(BORDER_WIDTH, s.dpi_scale);
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
                    let others: Vec<RECT> = s
                        .pip_windows
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != idx)
                        .map(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            r
                        })
                        .collect();

                    let (sx, sy) =
                        snap_point(new_x, new_y, w, h, &others, s.monitor_rect, s.snap_grid);

                    if let Some(pw) = s.pip_windows.get(idx) {
                        let _ = SetWindowPos(
                            pw.hwnd,
                            HWND::default(),
                            sx,
                            sy,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        let _ = SetWindowPos(
                            pw.label_hwnd,
                            HWND_TOPMOST,
                            sx + border,
                            sy + border,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOACTIVATE,
                        );
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

                    let others: Vec<RECT> = s
                        .pip_windows
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != idx)
                        .map(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            r
                        })
                        .collect();

                    let d = s.dpi_scale;
                    let border = dpi(BORDER_WIDTH, d);
                    let label_h = dpi(s.label_height, d);
                    let new_rect = snap_resize(
                        edge,
                        rd.start_rect,
                        dx,
                        dy,
                        &others,
                        s.monitor_rect,
                        s.snap_grid,
                        border,
                        label_h,
                    );
                    let nw = new_rect.right - new_rect.left;
                    let nh = new_rect.bottom - new_rect.top;

                    if let Some(pw) = s.pip_windows.get(idx) {
                        let _ = SetWindowPos(
                            pw.hwnd,
                            HWND::default(),
                            new_rect.left,
                            new_rect.top,
                            nw,
                            nh,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );

                        // Update DWM thumbnail destination.
                        let thumb_rect = RECT {
                            left: border,
                            top: border,
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
                        // Reposition the full-width label and Tell overlay.
                        let (lw, lh) = pip_label_overlay_size(s, nw, nh, border);
                        let _ = SetWindowPos(
                            pw.label_hwnd,
                            HWND_TOPMOST,
                            new_rect.left + border,
                            new_rect.top + border,
                            lw,
                            lh,
                            SWP_NOACTIVATE,
                        );
                        let _ = InvalidateRect(pw.label_hwnd, None, true);
                    }
                    return LRESULT(0);
                }

                // Track mouse for leave.
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                return LRESULT(0);
            }

            // --- Use mode: strip resize drag ---
            if let Some(ref srd) = s.strip_resize_drag {
                let mut cursor = POINT::default();
                let _ = GetCursorPos(&mut cursor);
                let is_vertical =
                    matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                let new_size = if is_vertical {
                    let delta = cursor.x - srd.start_pt.x;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Right) {
                        -1
                    } else {
                        1
                    };
                    let mon_w = s.monitor_rect.right - s.monitor_rect.left;
                    let min_w = (mon_w as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_w = (mon_w as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_w, max_w)
                } else {
                    let delta = cursor.y - srd.start_pt.y;
                    let sign = if matches!(s.pip_edge, config::PipEdge::Bottom) {
                        -1
                    } else {
                        1
                    };
                    let mon_h = s.monitor_rect.bottom - s.monitor_rect.top;
                    let min_h = (mon_h as f64 * MIN_STRIP_WIDTH_FRACTION).round() as i32;
                    let max_h = (mon_h as f64 * MAX_STRIP_WIDTH_FRACTION).round() as i32;
                    (srd.start_size + sign * delta).clamp(min_h, max_h)
                };
                if Some(new_size) != s.custom_strip_width {
                    s.custom_strip_width = Some(new_size);
                    let (rects, sw, sh) = compute_positions(s);
                    s.strip_width = sw;
                    s.strip_height = sh;
                    let d = s.dpi_scale;
                    let border = dpi(BORDER_WIDTH, d);
                    for (i, pw) in s.pip_windows.iter().enumerate() {
                        if let Some(rect) = rects.get(i) {
                            let cw = rect.right - rect.left;
                            let ch = rect.bottom - rect.top;
                            let _ = SetWindowPos(
                                pw.hwnd,
                                HWND::default(),
                                rect.left,
                                rect.top,
                                cw,
                                ch,
                                SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                            let thumb_rect = RECT {
                                left: border,
                                top: border,
                                right: cw - border,
                                bottom: ch - border,
                            };
                            let props = DWM_THUMBNAIL_PROPERTIES {
                                dwFlags: DWM_TNP_RECTDESTINATION,
                                rcDestination: thumb_rect,
                                ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            let _ = InvalidateRect(pw.hwnd, None, true);
                            // Reposition the full-width label and Tell overlay.
                            let (lw, lh) = pip_label_overlay_size(s, cw, ch, border);
                            let _ = SetWindowPos(
                                pw.label_hwnd,
                                HWND_TOPMOST,
                                rect.left + border,
                                rect.top + border,
                                lw,
                                lh,
                                SWP_NOACTIVATE,
                            );
                            let _ = InvalidateRect(pw.label_hwnd, None, true);
                        }
                    }
                    update_active_label(s);
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
                                dwFlags: DWM_TNP_OPACITY,
                                opacity: 80,
                                ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            let _ = InvalidateRect(pw.hwnd, None, true);
                        }
                    }
                }

                if drag.dragging {
                    // Find which pip is under cursor (screen coords).
                    let new_target = s
                        .pip_windows
                        .iter()
                        .enumerate()
                        .find(|(_, pw)| {
                            let mut r = RECT::default();
                            let _ = GetWindowRect(pw.hwnd, &mut r);
                            cursor.x >= r.left
                                && cursor.x < r.right
                                && cursor.y >= r.top
                                && cursor.y < r.bottom
                        })
                        .map(|(i, _)| i);

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

            // --- Use mode: notification action and PiP hover ---
            let point = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            notifications::update_invite_hover(s, pip_idx, point);
            if let Some(pw) = s.pip_windows.get_mut(pip_idx) {
                if !pw.hovered {
                    pw.hovered = true;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY,
                        opacity: THUMB_OPACITY_HOVER,
                        ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }

            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }

        WM_MOUSELEAVE => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };

            // Clear notification and PiP hover.
            notifications::clear_invite_interaction(s, pip_idx);
            if let Some(pw) = s.pip_windows.get_mut(pip_idx) {
                if pw.hovered {
                    pw.hovered = false;
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_OPACITY,
                        opacity: THUMB_OPACITY_NORMAL,
                        ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    let _ = InvalidateRect(hwnd, None, true);
                }
            }

            // Cancel non-dragging reorder on leave.
            if s.reorder_drag.as_ref().is_some_and(|drag| !drag.dragging) {
                s.reorder_drag = None;
                s.drop_target = None;
            }

            LRESULT(0)
        }

        WM_CANCELMODE => {
            let had_invite_press = state()
                .as_ref()
                .is_some_and(|s| notifications::invite_action_pressed(s, pip_idx));
            if had_invite_press {
                if let Some(s) = state().as_mut() {
                    notifications::clear_invite_interaction(s, pip_idx);
                }
                let _ = ReleaseCapture();
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }

        WM_CAPTURECHANGED => {
            // Capture APIs can send this message synchronously while another
            // branch still owns `&mut OverlayState`. Defer state access until
            // the outer window-procedure invocation has returned.
            let _ = PostMessageW(hwnd, WM_CLEAR_INVITE_CAPTURE, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }

        WM_CLEAR_INVITE_CAPTURE => {
            if let Some(s) = state().as_mut() {
                notifications::clear_invite_interaction(s, pip_idx);
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

            if !s.edit_mode && notifications::press_invite_action(s, pip_idx, pt) {
                s.reorder_drag = None;
                s.drop_target = None;
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

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
                        let is_vertical =
                            matches!(s.pip_edge, config::PipEdge::Right | config::PipEdge::Left);
                        let start_size = if is_vertical {
                            s.strip_width
                        } else {
                            s.strip_height
                        };
                        s.strip_resize_drag = Some(StripResizeDragState {
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

            if !s.edit_mode {
                let point = POINT {
                    x: (lparam.0 & 0xFFFF) as i16 as i32,
                    y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let had_invite_press = notifications::invite_action_pressed(s, pip_idx);
                let invite_action = notifications::release_invite_action(s, pip_idx, point);
                if had_invite_press {
                    s.reorder_drag = None;
                    s.drop_target = None;
                    let _ = ReleaseCapture();
                    if let Some((pid, action)) = invite_action {
                        notifications::execute_invite_action(s, pid, action);
                    }
                    return LRESULT(0);
                }
            }
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
                            dwFlags: DWM_TNP_OPACITY,
                            opacity: THUMB_OPACITY_NORMAL,
                            ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    }
                    // Perform swap if target is valid.
                    if let Some(to_index) = old_drop_target {
                        if to_index != drag.from_index
                            && to_index < s.pip_order.len()
                            && drag.from_index < s.pip_order.len()
                        {
                            // When auto-order is on, swap window numbers so the
                            // sort keeps the user's intended arrangement.
                            if config::Config::load().auto_order {
                                let pid_a = s.pip_order[drag.from_index];
                                let pid_b = s.pip_order[to_index];
                                let num_a = s
                                    .eq_windows
                                    .iter()
                                    .find(|w| w.pid == pid_a)
                                    .map(|w| w.number);
                                let num_b = s
                                    .eq_windows
                                    .iter()
                                    .find(|w| w.pid == pid_b)
                                    .map(|w| w.number);
                                if let (Some(na), Some(nb)) = (num_a, num_b) {
                                    if let Some(wa) =
                                        s.eq_windows.iter_mut().find(|w| w.pid == pid_a)
                                    {
                                        wa.number = nb;
                                    }
                                    if let Some(wb) =
                                        s.eq_windows.iter_mut().find(|w| w.pid == pid_b)
                                    {
                                        wb.number = na;
                                    }
                                }
                            }
                            s.pip_order.swap(drag.from_index, to_index);
                            rebuild_thumbnails(s);
                            update_visibility(s);
                            publish_control_state(s);
                        }
                    }
                } else {
                    // Simple click → activate window.
                    let idx = drag.from_index;
                    let _ = s; // release borrow before swap_to re-borrows state
                    let _ = swap_to(idx);
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
                // Don't set dpi_scale from this PiP window — it may be on the
                // wrong monitor after a display change. rebuild_thumbnails
                // derives DPI from the EQ window (same source as monitor_rect).
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

unsafe fn invalidate_pip_border(hwnd: HWND, border: i32) {
    let mut client = RECT::default();
    if GetClientRect(hwnd, &mut client).is_err() || border <= 0 {
        return;
    }
    let border = border
        .min((client.right - client.left) / 2)
        .min((client.bottom - client.top) / 2);
    let regions = [
        RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: client.top + border,
        },
        RECT {
            left: client.left,
            top: client.bottom - border,
            right: client.right,
            bottom: client.bottom,
        },
        RECT {
            left: client.left,
            top: client.top + border,
            right: client.left + border,
            bottom: client.bottom - border,
        },
        RECT {
            left: client.right - border,
            top: client.top + border,
            right: client.right,
            bottom: client.bottom - border,
        },
    ];
    for region in regions {
        let _ = InvalidateRect(hwnd, Some(&region), false);
    }
}

unsafe fn tick_notification_animation(timer_hwnd: HWND) {
    let Some(s) = state().as_mut() else {
        let _ = KillTimer(timer_hwnd, notifications::TIMER_ID);
        return;
    };
    notifications::tick(s, timer_hwnd);
}

unsafe extern "system" fn label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_TIMER if wparam.0 == notifications::TIMER_ID => {
            tick_notification_animation(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
        WM_PAINT => {
            let (text, class, color) = state()
                .as_ref()
                .map(|s| {
                    (
                        s.active_label_text.clone(),
                        s.active_label_class.clone(),
                        s.active_label_color,
                    )
                })
                .unwrap_or((String::new(), None, LABEL_COLORS[0]));
            if !text.is_empty() {
                paint_label(hwnd, &text, class.as_deref(), color);
            } else {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
            let alpha = state().as_ref().map_or(204, |s| s.label_alpha);
            let _ = SetLayeredWindowAttributes(hwnd, key, alpha / 2, LWA_ALPHA | LWA_COLORKEY);
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
            let alpha = state().as_ref().map_or(204, |s| s.label_alpha);
            let _ = SetLayeredWindowAttributes(hwnd, key, alpha, LWA_ALPHA | LWA_COLORKEY);
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
            // Hide label so WindowFromPoint finds the window underneath.
            let _ = ShowWindow(hwnd, SW_HIDE);
            let below = WindowFromPoint(pt);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if !below.is_invalid() && below != hwnd {
                let _ = PostMessageW(
                    below,
                    msg,
                    wparam,
                    LPARAM((pt.x as i16 as u16 as isize) | ((pt.y as i16 as u16 as isize) << 16)),
                );
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
        let Some(s) = state().as_ref() else {
            return false;
        };
        let fg = GetForegroundWindow();
        is_eq_or_ours(fg, s)
    }
}

/// Returns true if the overlay is currently visible (not hidden by user).
pub fn is_visible() -> bool {
    state().as_ref().is_none_or(|state| !state.hidden_by_user)
}

pub fn toggle_hidden() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        s.hidden_by_user = !s.hidden_by_user;
        update_visibility(s);
    }
}

/// Refresh broadcast label visibility immediately.
pub fn refresh_broadcast_label() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        update_active_label(s);
        update_visibility(s);
    }
}

/// Reload config into overlay state and rebuild the layout.
pub fn force_rebuild() {
    unsafe {
        let Some(s) = state().as_mut() else { return };
        let cfg = config::Config::load();
        s.pip_edge = cfg.pip_edge;
        s.preferred_box_order = cfg.box_order.clone();
        apply_preferred_box_order(&mut s.eq_windows, &s.preferred_box_order);
        if cfg.auto_order {
            apply_auto_order(s);
        }
        s.custom_strip_width = cfg.pip_strip_width.map(|v| v as i32);
        s.has_custom_positions = !cfg.pip_positions.is_empty();
        s.snap_grid = cfg.snap_grid as i32;
        s.label_height = cfg
            .pip_label_height
            .map(|v| v as i32)
            .unwrap_or(DEFAULT_LABEL_HEIGHT);
        let opacity = cfg
            .pip_label_opacity
            .unwrap_or(DEFAULT_LABEL_OPACITY)
            .min(100);
        s.label_alpha = ((opacity as u16 * 255) / 100) as u8;
        // Update layered window attributes for floating labels.
        let key = windows::Win32::Foundation::COLORREF(LABEL_COLOR_KEY);
        let _ = SetLayeredWindowAttributes(
            s.active_label_hwnd,
            key,
            s.label_alpha,
            LWA_ALPHA | LWA_COLORKEY,
        );
        let _ = SetLayeredWindowAttributes(
            s.broadcast_label_hwnd,
            key,
            s.label_alpha,
            LWA_ALPHA | LWA_COLORKEY,
        );
        // Handle hide_from_alt_tab setting change.
        let old_hide = s.hide_from_alt_tab;
        s.hide_from_alt_tab = cfg.hide_from_alt_tab;
        if old_hide && !s.hide_from_alt_tab {
            // Setting turned off: restore all original styles.
            let hwnds: Vec<HWND> = s.eq_windows.iter().map(|w| w.hwnd).collect();
            for hwnd in hwnds {
                restore_window_ex_style(s, hwnd);
            }
        } else if s.hide_from_alt_tab {
            apply_alt_tab_hiding(s);
        }
        // Reload notification config and the EQ profile resolver.
        s.tell_visual_enabled = cfg.tell_visual_enabled;
        s.tell_sound_enabled = cfg.tell_sound_enabled;
        s.tell_sound = sound::normalized_id(&cfg.tell_sound).to_owned();
        let notification_kinds = EnabledKinds {
            tells: cfg.notify_tells,
            group_invites: cfg.notify_group_invites,
            raid_invites: cfg.notify_raid_invites,
            resurrections: cfg.notify_resurrections,
            deaths: cfg.notify_deaths,
        };
        let notification_selection_changed = s.notification_kinds != notification_kinds;
        s.notification_kinds = notification_kinds;
        s.animations_enabled = client_animations_enabled();
        s.chat_colors.set_eq_dir(cfg.eq_directory());
        if !s.tell_visual_enabled || notification_selection_changed {
            s.notifications.clear();
            let _ = KillTimer(s.active_label_hwnd, notifications::TIMER_ID);
        }
        // Reload toast config.
        s.toast.enabled = cfg.toast_enabled;
        s.toast.height = cfg
            .toast_height
            .map(|h| h as i32)
            .unwrap_or(DEFAULT_TOAST_HEIGHT);
        s.toast.duration_ms = cfg
            .toast_duration
            .map(|d| (d * 1000.0) as u32)
            .unwrap_or(DEFAULT_TOAST_DURATION_MS);
        rebuild_thumbnails(s);
        update_visibility(s);
    }
}

pub fn cleanup() {
    unsafe {
        if let Some(s) = state_unguarded().as_mut() {
            // Restore original ex styles on all EQ windows before shutting down.
            let hwnds: Vec<HWND> = s.eq_windows.iter().map(|w| w.hwnd).collect();
            for hwnd in hwnds {
                restore_window_ex_style(s, hwnd);
            }
            if !s.event_hook.is_invalid() {
                let _ = UnhookWinEvent(s.event_hook);
            }
            for pw in s.pip_windows.drain(..) {
                let _ = DwmUnregisterThumbnail(pw.thumb);
                let _ = DestroyWindow(pw.label_hwnd);
                let _ = DestroyWindow(pw.hwnd);
            }
            let _ = KillTimer(s.active_label_hwnd, notifications::TIMER_ID);
            let _ = DestroyWindow(s.active_label_hwnd);
            let _ = DestroyWindow(s.broadcast_label_hwnd);
            let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
            let _ = DestroyWindow(s.toast.hwnd);
        }
        *state_unguarded() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(pid: u32) -> EqWindow {
        EqWindow {
            hwnd: HWND::default(),
            pid,
            number: 1,
            character: None,
            server: None,
            class: None,
        }
    }

    fn identified_window(pid: u32, number: usize, server: &str, character: &str) -> EqWindow {
        let mut window = window(pid);
        window.number = number;
        window.server = Some(server.into());
        window.character = Some(character.into());
        window
    }

    #[test]
    fn preferred_box_order_is_global_case_insensitive_and_compacts_missing_entries() {
        let preferred = vec![
            config::BoxIdentity {
                server: "xegony".into(),
                character: "Laika".into(),
            },
            config::BoxIdentity {
                server: "bristlebane".into(),
                character: "Foo".into(),
            },
            config::BoxIdentity {
                server: "xegony".into(),
                character: "Kafka".into(),
            },
        ];
        let mut windows = vec![
            identified_window(10, 1, "XEGONY", "Kafka"),
            identified_window(20, 2, "Bristlebane", "foo"),
            identified_window(30, 3, "Xegony", "Unlisted"),
        ];

        assert!(apply_preferred_box_order(&mut windows, &preferred));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 2), (20, 1), (30, 3)]
        );
        assert!(!apply_preferred_box_order(&mut windows, &preferred));
    }

    #[test]
    fn unlisted_windows_retain_their_relative_order_after_ranked_windows() {
        let preferred = vec![config::BoxIdentity {
            server: "xegony".into(),
            character: "Laika".into(),
        }];
        let mut windows = vec![
            identified_window(10, 4, "Xegony", "UnknownOne"),
            identified_window(20, 2, "Xegony", "Laika"),
            identified_window(30, 3, "Teek", "UnknownTwo"),
        ];

        assert!(apply_preferred_box_order(&mut windows, &preferred));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 3), (20, 1), (30, 2)]
        );
    }

    #[test]
    fn empty_preference_preserves_existing_window_numbers() {
        let mut windows = vec![window(10), window(20)];
        windows[0].number = 4;
        windows[1].number = 2;

        assert!(!apply_preferred_box_order(&mut windows, &[]));
        assert_eq!(windows[0].number, 4);
        assert_eq!(windows[1].number, 2);
    }

    #[test]
    fn swapping_window_numbers_preserves_identity_and_supports_a_no_op() {
        let mut windows = vec![window(10), window(20), window(30)];
        windows[0].number = 1;
        windows[1].number = 2;
        windows[2].number = 3;

        assert_eq!(exchange_window_numbers(&mut windows, 10, 30), Some((1, 3)));
        assert_eq!(
            windows
                .iter()
                .map(|window| (window.pid, window.number))
                .collect::<Vec<_>>(),
            vec![(10, 3), (20, 2), (30, 1)]
        );
        assert_eq!(exchange_window_numbers(&mut windows, 20, 20), Some((2, 2)));
        assert!(exchange_window_numbers(&mut windows, 10, 99).is_none());
    }

    #[test]
    fn foreground_pid_requires_confirmed_keyboard_focus_during_client_set_changes() {
        let hwnd = HWND(42usize as *mut _);
        let mut candidate = window(42);
        candidate.hwnd = hwnd;
        let windows = vec![candidate];

        assert_eq!(focused_foreground_pid(&windows, hwnd, |_| false), None);
        assert_eq!(focused_foreground_pid(&windows, hwnd, |_| true), Some(42));
        assert_eq!(
            focused_foreground_pid(&windows, HWND(99usize as *mut _), |_| true),
            None
        );
    }

    #[test]
    fn rapid_out_of_order_foreground_changes_preserve_the_client_partition() {
        let mut active = Some(1);
        let mut pips = vec![2, 3, 4];

        for target in [2, 3, 2, 4, 3, 1, 4] {
            assert!(exchange_active_with_pip(&mut active, &mut pips, target));
            let mut partition = vec![active.expect("an active client")];
            partition.extend(pips.iter().copied());
            partition.sort_unstable();
            assert_eq!(partition, vec![1, 2, 3, 4]);
        }

        let before = (active, pips.clone());
        assert!(!exchange_active_with_pip(&mut active, &mut pips, 99));
        assert_eq!((active, pips), before);
    }

    #[test]
    fn new_automatic_identity_refreshes_but_unchanged_identity_preserves_manual_assignment() {
        let mut observed = HashMap::new();
        let mut window = window(42);

        assert!(reconcile_trusik_identity(
            &mut observed,
            &mut window,
            "Orlov".into(),
            "teek".into(),
            Some("SHK".into()),
        ));
        window.character = Some("Manual".into());
        window.server = Some("assignment".into());
        window.class = Some("CLR".into());

        assert!(!reconcile_trusik_identity(
            &mut observed,
            &mut window,
            "orlov".into(),
            "TEEK".into(),
            None,
        ));
        assert_eq!(window.character.as_deref(), Some("Manual"));

        assert!(reconcile_trusik_identity(
            &mut observed,
            &mut window,
            "Laika".into(),
            "xegony".into(),
            Some("SHM".into()),
        ));
        assert_eq!(window.character.as_deref(), Some("Laika"));
        assert_eq!(window.server.as_deref(), Some("xegony"));
        assert_eq!(window.class.as_deref(), Some("SHM"));
    }
}
