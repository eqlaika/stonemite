use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

mod labels;
mod notifications;
mod render;
mod scenes;
mod timers;

use labels::{Color, LabelModel, LabelStyle, LabelTheme, Rect};
use notifications::{EnabledKinds, Notification};
use render::Compositor;
use scenes::{
    ActiveLabelScene, LabelScene, PipInteractionScene, PipScene, StatusBannerScene, TimerScene,
    ToastScene, UiTextRole,
};
use timers::TimerOverlayState;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, RPC_E_CHANGED_MODE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{ClientToScreen, InvalidateRect, ScreenToClient, ValidateRect};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
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

/// Thumbnail opacity while hovered and maximum opacity while reordering.
const THUMB_OPACITY_HOVER: u8 = 255;
const THUMB_OPACITY_DRAG_MAX: u8 = 80;

/// Border thickness for hover highlight.
const BORDER_WIDTH: i32 = 3;

/// Default height of the character name label overlay.
const DEFAULT_LABEL_HEIGHT: i32 = 48;

/// Default label opacity percentage (0–100).
const DEFAULT_LABEL_OPACITY: u32 = 80;

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
/// Timer ID and cadence for visible display-only countdowns.
const TIMER_OVERLAY_TICK: usize = 44;
const TIMER_OVERLAY_INTERVAL_MS: u32 = 100;
const TIMER_PANEL_GAP: i32 = 4;
const TIMER_PANEL_HEIGHT: i32 = 42;
const WM_CLEAR_INVITE_CAPTURE: u32 = WM_USER + 44;
const WM_SERVICE_COMPOSITOR_RECOVERY: u32 = WM_USER + 45;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ToastPhase {
    Hidden,
    FadingIn,
    Visible,
    FadingOut,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ToastFadeEffect {
    None,
    UpdateOpacity(u8),
    HideAndStop,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ToastFadeTransition {
    phase: ToastPhase,
    alpha: u8,
    phase_start: u64,
    effect: ToastFadeEffect,
}

fn toast_publication_allowed(
    phase: ToastPhase,
    scene_ready: bool,
    overlay_visible: bool,
    surface_attached: bool,
) -> bool {
    phase != ToastPhase::Hidden && scene_ready && overlay_visible && surface_attached
}

fn advance_toast_fade(
    phase: ToastPhase,
    alpha: u8,
    phase_start: u64,
    duration_ms: u32,
    now: u64,
) -> ToastFadeTransition {
    match phase {
        ToastPhase::FadingIn => {
            let alpha = alpha.saturating_add(TOAST_ALPHA_STEP).min(TOAST_MAX_ALPHA);
            let (phase, phase_start) = if alpha == TOAST_MAX_ALPHA {
                (ToastPhase::Visible, now)
            } else {
                (ToastPhase::FadingIn, phase_start)
            };
            ToastFadeTransition {
                phase,
                alpha,
                phase_start,
                effect: ToastFadeEffect::UpdateOpacity(alpha),
            }
        }
        ToastPhase::Visible if now.saturating_sub(phase_start) >= u64::from(duration_ms) => {
            ToastFadeTransition {
                phase: ToastPhase::FadingOut,
                alpha,
                phase_start,
                effect: ToastFadeEffect::None,
            }
        }
        ToastPhase::Visible => ToastFadeTransition {
            phase,
            alpha,
            phase_start,
            effect: ToastFadeEffect::None,
        },
        ToastPhase::FadingOut => {
            let alpha = alpha.saturating_sub(TOAST_ALPHA_STEP);
            if alpha == 0 {
                ToastFadeTransition {
                    phase: ToastPhase::Hidden,
                    alpha,
                    phase_start,
                    effect: ToastFadeEffect::HideAndStop,
                }
            } else {
                ToastFadeTransition {
                    phase,
                    alpha,
                    phase_start,
                    effect: ToastFadeEffect::UpdateOpacity(alpha),
                }
            }
        }
        ToastPhase::Hidden => ToastFadeTransition {
            phase,
            alpha: 0,
            phase_start,
            effect: ToastFadeEffect::HideAndStop,
        },
    }
}

struct ToastState {
    hwnd: HWND,
    text: String,
    phase: ToastPhase,
    /// True only after the current staged text has completed Present1 and attach.
    scene_ready: bool,
    alpha: u8,
    phase_start: u64,
    duration_ms: u32,
    height: i32,
    enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveSceneKey {
    text: String,
    class: Option<String>,
    color: u32,
    number: usize,
    label_height: i32,
    label_alpha: u8,
    dpi_bits: u64,
    theme: LabelTheme,
    timer_label: Option<String>,
    timer_start: Option<Instant>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BannerSceneKey {
    text: String,
    background: Color,
    label_height: i32,
    label_alpha: u8,
    dpi_bits: u64,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct ReorderCancellation {
    dimmed_source: Option<usize>,
    old_target: Option<usize>,
}

fn take_reorder_cancellation(
    reorder_drag: &mut Option<ReorderDragState>,
    drop_target: &mut Option<usize>,
) -> ReorderCancellation {
    let dimmed_source = reorder_drag
        .take()
        .filter(|drag| drag.dragging)
        .map(|drag| drag.from_index);
    ReorderCancellation {
        dimmed_source,
        old_target: drop_target.take(),
    }
}

struct ComApartment {
    usable: bool,
    uninitialize: bool,
}

impl ComApartment {
    unsafe fn initialize() -> Self {
        match CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() {
            Ok(()) => Self {
                usable: true,
                uninitialize: true,
            },
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Self {
                usable: true,
                uninitialize: false,
            },
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition COM initialization failed: {error}"
                ));
                Self {
                    usable: false,
                    uninitialize: false,
                }
            }
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

struct OverlayState {
    /// The hardware compositor drops before its balanced COM apartment owner.
    compositor: Option<Compositor>,
    com_apartment: ComApartment,
    pip_windows: Vec<PipWindowEntry>,
    /// Hidden composition HWNDs retained until their registered surfaces can
    /// be detached safely and the HWNDs can then be destroyed.
    pending_composition_destroys: Vec<HWND>,
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
    active_label_hovered: bool,
    active_scene_key: Option<ActiveSceneKey>,
    banner_scene_key: Option<BannerSceneKey>,
    /// Broadcast banner window, shown next to the active label when broadcasting.
    broadcast_label_hwnd: HWND,
    /// Configured label height (logical pixels).
    label_height: i32,
    /// Configured label alpha (0–255).
    label_alpha: u8,
    /// Renderer-independent typography and visual metrics for character labels.
    label_theme: LabelTheme,
    event_hook: HWINEVENTHOOK,
    monitor_rect: RECT,
    dpi_scale: f64,
    /// Which screen edge the PiP strip is anchored to.
    pip_edge: config::PipEdge,
    /// User-configured strip width override (pixels). None = auto.
    custom_strip_width: Option<i32>,
    /// Configured normal DWM thumbnail alpha (0-255); hover remains opaque.
    thumbnail_alpha: u8,
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
    /// Passive display-only timers started by log trigger activations.
    timers: TimerOverlayState,
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
    static SERVICING_COMPOSITOR_RECOVERY: Cell<bool> = const { Cell::new(false) };
    static COMPOSITOR_RECOVERY_POSTED: Cell<bool> = const { Cell::new(false) };
    static REDRAW_PENDING: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
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
    f64::from(GetDpiForSystem().max(96)) / 96.0
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

fn label_model<'a>(
    text: &'a str,
    class: Option<&'a str>,
    number: usize,
    background: u32,
) -> LabelModel<'a> {
    LabelModel {
        text,
        class,
        number,
        background: Color::from_colorref(background),
        badge_background: Color::from_colorref(badge_color_for_number(number)),
    }
}

fn label_font_weight(weight: config::LabelFontWeight) -> u16 {
    match weight {
        config::LabelFontWeight::Regular => 400,
        config::LabelFontWeight::Semibold => 600,
        config::LabelFontWeight::Bold => 700,
        config::LabelFontWeight::Heavy => 900,
    }
}

fn opacity_percent_to_alpha(percent: u32) -> u8 {
    ((percent.clamp(0, 100) as u16 * 255) / 100) as u8
}

fn reorder_thumbnail_alpha(normal_alpha: u8) -> u8 {
    normal_alpha.min(THUMB_OPACITY_DRAG_MAX)
}

unsafe fn cancel_reorder_drag(s: &mut OverlayState) {
    let cancellation = take_reorder_cancellation(&mut s.reorder_drag, &mut s.drop_target);
    if let Some(source) = cancellation.dimmed_source {
        if let Some(pip) = s.pip_windows.get(source) {
            let properties = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_OPACITY,
                opacity: s.thumbnail_alpha,
                ..Default::default()
            };
            let _ = DwmUpdateThumbnailProperties(pip.thumb, &properties);
            request_redraw(pip.label_hwnd);
        }
    }
    if let Some(target) = cancellation.old_target {
        if Some(target) != cancellation.dimmed_source {
            if let Some(pip) = s.pip_windows.get(target) {
                request_redraw(pip.label_hwnd);
            }
        }
    }
}

fn configured_label_theme(cfg: &config::Config) -> LabelTheme {
    LabelTheme::with_name_font(
        cfg.effective_pip_label_font_family().to_owned(),
        cfg.effective_pip_label_font_scale(),
        label_font_weight(cfg.effective_pip_label_font_weight()),
    )
}

fn active_timer(s: &OverlayState, now: Instant) -> Option<&timers::TimerOverlay> {
    let source_id = s.active_pid.map(|pid| format!("pid:{pid}"));
    s.timers.visible_for(source_id.as_deref(), now)
}

fn format_timer_remaining(remaining: Duration) -> String {
    let tenths = (remaining.as_millis() + 99) / 100;
    format!("{}.{:01}s", tenths / 10, tenths % 10)
}

fn timer_owner_hwnds(s: &OverlayState, now: Instant) -> Vec<HWND> {
    let mut owners = Vec::new();
    let active_source = s.active_pid.map(|pid| format!("pid:{pid}"));
    if s.timers
        .visible_for(active_source.as_deref(), now)
        .is_some()
    {
        owners.push(s.active_label_hwnd);
    }
    for pip in &s.pip_windows {
        let source_id = format!("pid:{}", pip.pid);
        if s.timers.visible_for(Some(&source_id), now).is_some() {
            owners.push(pip.label_hwnd);
        }
    }
    owners
}

fn timer_tick_redraw_targets(
    expired: bool,
    active_label_hwnd: HWND,
    pip_label_hwnds: &[HWND],
    previous_owners: &[HWND],
    current_owners: &[HWND],
) -> Vec<HWND> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |hwnd: HWND| {
        if !hwnd.is_invalid()
            && !(expired && hwnd == active_label_hwnd)
            && seen.insert(hwnd.0 as isize)
        {
            targets.push(hwnd);
        }
    };
    if expired {
        // Expired timers are already excluded by visible_for. Redrawing the
        // bounded PiP set clears whichever client owned the expired scene.
        pip_label_hwnds.iter().copied().for_each(&mut add);
    } else {
        previous_owners
            .iter()
            .chain(current_owners)
            .copied()
            .for_each(add);
    }
    targets
}

unsafe fn invalidate_timer_labels(s: &OverlayState) {
    for hwnd in timer_owner_hwnds(s, Instant::now()) {
        if hwnd != s.active_label_hwnd {
            request_redraw(hwnd);
        }
    }
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
        || s.pending_composition_destroys.contains(&hwnd)
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
    if is_our_window(hwnd, s)
        || unsafe { crate::settings_dialog::foreground_window_is_settings(hwnd) }
    {
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
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&pip_label_wc);

    // Register label window class.
    let label_class = w!("StonemiteLabelClass");
    let label_wc = WNDCLASSW {
        lpfnWndProc: Some(label_wnd_proc),
        lpszClassName: label_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&label_wc);

    let label_hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP,
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
    let label_alpha = opacity_percent_to_alpha(label_opacity);

    // Register broadcast banner window class.
    let bc_class = w!("StonemiteBroadcastClass");
    let bc_wc = WNDCLASSW {
        lpfnWndProc: Some(broadcast_label_wnd_proc),
        lpszClassName: bc_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&bc_wc);

    let bc_hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
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

    // Register toast notification window class.
    let toast_class = w!("StonemiteToastClass");
    let toast_wc = WNDCLASSW {
        lpfnWndProc: Some(toast_wnd_proc),
        lpszClassName: toast_class,
        hCursor: cursor,
        ..Default::default()
    };
    RegisterClassW(&toast_wc);

    let toast_hwnd = CreateWindowExW(
        WS_EX_TOPMOST
            | WS_EX_TOOLWINDOW
            | WS_EX_TRANSPARENT
            | WS_EX_NOACTIVATE
            | WS_EX_NOREDIRECTIONBITMAP,
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
    let label_theme = configured_label_theme(&cfg);
    let com_apartment = ComApartment::initialize();
    let compositor = if com_apartment.usable {
        match Compositor::new() {
            Ok(compositor) => Some(compositor),
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition compositor initialization failed: {error}"
                ));
                None
            }
        }
    } else {
        None
    };

    *state_unguarded() = Some(OverlayState {
        compositor,
        com_apartment,
        pip_windows: Vec::new(),
        pending_composition_destroys: Vec::new(),
        eq_windows: Vec::new(),
        pip_order: Vec::new(),
        preferred_box_order: cfg.box_order.clone(),
        active_pid: None,
        active_label_hwnd: label_hwnd,
        active_label_text: String::new(),
        active_label_class: None,
        active_label_color: LABEL_COLORS[0],
        active_label_number: 0,
        active_label_hovered: false,
        active_scene_key: None,
        banner_scene_key: None,
        broadcast_label_hwnd: bc_hwnd,
        label_height,
        label_alpha,
        label_theme,
        event_hook: hook,
        monitor_rect: RECT::default(),
        dpi_scale: get_dpi_scale(label_hwnd),
        pip_edge: cfg.pip_edge,
        custom_strip_width: cfg.pip_strip_width.map(|v| v as i32),
        thumbnail_alpha: opacity_percent_to_alpha(cfg.effective_pip_opacity()),
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
            scene_ready: false,
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
        timers: TimerOverlayState::default(),
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
        service_compositor_recovery(s);
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
    service_compositor_recovery(s);
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
    let mut timers_changed = false;
    let now = Instant::now();
    for batch in batches {
        for diagnostic in batch.diagnostics {
            debug_log(&format!("eq_logs: {diagnostic}"));
        }
        for envelope in batch.envelopes {
            timers_changed |= s
                .timers
                .apply_activations(&envelope.trigger_activations, now);
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
    if timers_changed {
        unsafe {
            if s.timers.is_empty() {
                let _ = KillTimer(s.active_label_hwnd, TIMER_OVERLAY_TICK);
            } else {
                let _ = SetTimer(
                    s.active_label_hwnd,
                    TIMER_OVERLAY_TICK,
                    TIMER_OVERLAY_INTERVAL_MS,
                    None,
                );
            }
            update_active_label(s);
            invalidate_timer_labels(s);
        }
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
    cancel_reorder_drag(s);
    // Destroy existing PiP windows only after authored composition surfaces
    // are detached. DWM thumbnail ownership remains on the host HWND.
    let previous = std::mem::take(&mut s.pip_windows);
    for pw in previous {
        let composition_detached = unregister_composition_surface(s, pw.label_hwnd);
        if pw.thumb != 0 {
            let _ = DwmUnregisterThumbnail(pw.thumb);
        }
        if !pw.label_hwnd.is_invalid() {
            if composition_detached {
                let _ = DestroyWindow(pw.label_hwnd);
            } else {
                let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
                if !s.pending_composition_destroys.contains(&pw.label_hwnd) {
                    s.pending_composition_destroys.push(pw.label_hwnd);
                }
            }
        }
        let _ = DestroyWindow(pw.hwnd);
    }

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

    let pip_order = s.pip_order.clone();
    for (i, pid) in pip_order.into_iter().enumerate() {
        let Some(eq_win) = s.eq_windows.iter().find(|w| w.pid == pid).cloned() else {
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
            opacity: s.thumbnail_alpha,
            fSourceClientAreaOnly: true.into(),
            ..Default::default()
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);

        // One full-host transparent composition sibling owns every authored
        // PiP visual. The DWM host remains thumbnail-only beneath it.
        let label_text = format_label(&eq_win);
        let pip_label_class = w!("StonemitePipLabelClass");
        let lbl_hwnd = CreateWindowExW(
            WS_EX_TOPMOST
                | WS_EX_TOOLWINDOW
                | WS_EX_TRANSPARENT
                | WS_EX_NOACTIVATE
                | WS_EX_NOREDIRECTIONBITMAP,
            pip_label_class,
            w!("StonemitePipComposition"),
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
        .expect("Failed to create PiP composition window");
        SetWindowLongPtrW(lbl_hwnd, GWLP_USERDATA, (i + 1) as isize);

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
        render_pip_surface(s, s.pip_windows.len() - 1);
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
    let style = LabelStyle::new(d, lh);
    let label_h = style.height();
    let active_timer = active_timer(s, Instant::now());
    let timer_label = active_timer.map(|timer| timer.label.to_string());
    let timer_start = active_timer.map(|timer| timer.start_time);
    let timer_height = timer_label
        .as_ref()
        .map(|_| dpi(TIMER_PANEL_GAP + TIMER_PANEL_HEIGHT, d))
        .unwrap_or(0);
    let active_scene_key = ActiveSceneKey {
        text: s.active_label_text.clone(),
        class: s.active_label_class.clone(),
        color: s.active_label_color,
        number: s.active_label_number,
        label_height: s.label_height,
        label_alpha: s.label_alpha,
        dpi_bits: s.dpi_scale.to_bits(),
        theme: s.label_theme.clone(),
        timer_label,
        timer_start,
    };
    let active_scene_changed = s.active_scene_key.as_ref() != Some(&active_scene_key);
    if !ensure_compositor(s) {
        return;
    }
    let model = label_model(
        &s.active_label_text,
        s.active_label_class.as_deref(),
        s.active_label_number,
        s.active_label_color,
    );
    let text_width = match s
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .measure_label_width(&model, style, &s.label_theme, i32::MAX)
    {
        Ok(width) => width,
        Err(error) => {
            debug_log(&format!(
                "DirectWrite active-label measurement failed: {error}"
            ));
            return;
        }
    };

    // When PiP edge is left, anchor the label at top-right so the strip doesn't cover it.
    let label_x = if matches!(s.pip_edge, config::PipEdge::Left) {
        top_right.x - text_width
    } else {
        top_left.x
    };

    let active_size = (text_width, label_h + timer_height);
    if (active_scene_changed || !surface_is_ready(s, s.active_label_hwnd))
        && !render_active_label_surface_for_size(s, active_size.0, active_size.1)
    {
        return;
    }
    s.active_scene_key = Some(active_scene_key);
    position_window_if_changed(
        s.active_label_hwnd,
        HWND_TOPMOST,
        label_x,
        top_left.y,
        active_size.0,
        active_size.1,
    );

    // Position the explicit keyboard/mouse input indicator next to the active label.
    if let Some(bc_text) = input_indicator_text() {
        let bc_width = match s
            .compositor
            .as_ref()
            .expect("compositor ensured")
            .measure_text(
                bc_text,
                &UiTextRole::StatusBanner.font(),
                UiTextRole::StatusBanner.height(d, (lh - 12).max(1)),
            ) {
            Ok(width) => width + dpi(20, d),
            Err(error) => {
                debug_log(&format!(
                    "DirectWrite status-banner measurement failed: {error}"
                ));
                return;
            }
        };
        let bc_x = if matches!(s.pip_edge, config::PipEdge::Left) {
            label_x - bc_width - dpi(4, d)
        } else {
            label_x + text_width + dpi(4, d)
        };
        let banner_scene_key = BannerSceneKey {
            text: bc_text.to_owned(),
            background: input_indicator_background(),
            label_height: s.label_height,
            label_alpha: s.label_alpha,
            dpi_bits: s.dpi_scale.to_bits(),
        };
        let banner_scene_changed = s.banner_scene_key.as_ref() != Some(&banner_scene_key);
        if (banner_scene_changed || !surface_is_ready(s, s.broadcast_label_hwnd))
            && !render_banner_surface_for_size(s, bc_width, label_h)
        {
            return;
        }
        s.banner_scene_key = Some(banner_scene_key);
        position_window_if_changed(
            s.broadcast_label_hwnd,
            HWND_TOPMOST,
            bc_x,
            top_left.y,
            bc_width,
            label_h,
        );
    } else {
        s.banner_scene_key = None;
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

fn overlay_visibility_policy(
    hidden_by_user: bool,
    has_pip: bool,
    context_menu_open: bool,
    foreground_is_eq_or_ours: bool,
) -> bool {
    !hidden_by_user && has_pip && (context_menu_open || foreground_is_eq_or_ours)
}

unsafe fn overlay_visibility_allowed(s: &OverlayState) -> bool {
    let has_pip = !s.pip_order.is_empty();
    let foreground_matches = if s.hidden_by_user || !has_pip {
        false
    } else {
        is_eq_or_ours(GetForegroundWindow(), s)
    };
    overlay_visibility_policy(
        s.hidden_by_user,
        has_pip,
        s.context_menu_open,
        foreground_matches,
    )
}

unsafe fn apply_pip_pair_visibility(s: &OverlayState, visible: bool) {
    let was_guarded = IN_OVERLAY.replace(true);
    for pip in &s.pip_windows {
        if visible && surface_is_ready(s, pip.label_hwnd) {
            let _ = ShowWindow(pip.hwnd, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(pip.label_hwnd, SW_SHOWNOACTIVATE);
            // Interactions with a PiP can promote it above its sibling.
            let _ = SetWindowPos(
                pip.label_hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        } else {
            let _ = ShowWindow(pip.hwnd, SW_HIDE);
            let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
            if visible {
                request_redraw(pip.label_hwnd);
            }
        }
    }
    IN_OVERLAY.set(was_guarded);
}

unsafe fn update_visibility(s: &mut OverlayState) {
    if overlay_visibility_allowed(s) {
        // Publish any state accumulated while hidden before exposing a visual.
        for index in 0..s.pip_windows.len() {
            let hwnd = s.pip_windows[index].label_hwnd;
            if has_redraw_request(hwnd) {
                render_pip_surface(s, index);
            }
        }
        if has_redraw_request(s.active_label_hwnd) {
            render_active_label_surface(s);
        }
        if has_redraw_request(s.broadcast_label_hwnd) {
            render_banner_surface(s);
        }

        // A thumbnail host and its full-host authored sibling are one visible
        // pair. DWM remains registered while both wait for a complete frame.
        apply_pip_pair_visibility(s, true);
        if !s.active_label_text.is_empty() && surface_is_ready(s, s.active_label_hwnd) {
            let _ = ShowWindow(s.active_label_hwnd, SW_SHOWNOACTIVATE);
        }
        if input_indicator_text().is_some() && surface_is_ready(s, s.broadcast_label_hwnd) {
            let _ = ShowWindow(s.broadcast_label_hwnd, SW_SHOWNOACTIVATE);
        } else {
            let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        }
    } else {
        for pw in &mut s.pip_windows {
            if std::mem::take(&mut pw.hovered) {
                request_redraw(pw.label_hwnd);
            }
            let _ = ShowWindow(pw.hwnd, SW_HIDE);
            let _ = ShowWindow(pw.label_hwnd, SW_HIDE);
        }
        if std::mem::take(&mut s.active_label_hovered) {
            let alpha = s.label_alpha;
            set_composition_opacity(s, s.active_label_hwnd, alpha);
        }
        let _ = ShowWindow(s.active_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.broadcast_label_hwnd, SW_HIDE);
        let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
        s.toast.phase = ToastPhase::Hidden;
        s.toast.scene_ready = false;
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
// DirectComposition scene publication and redraw scheduling
// ---------------------------------------------------------------------------

unsafe fn position_pip_pair(pip: &PipWindowEntry, x: i32, y: i32, width: i32, height: i32) {
    let deferred = (|| -> windows::core::Result<()> {
        let batch = BeginDeferWindowPos(2)?;
        let batch = DeferWindowPos(
            batch,
            pip.hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
        let batch = DeferWindowPos(
            batch,
            pip.label_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        )?;
        EndDeferWindowPos(batch)
    })();
    if deferred.is_err() {
        let _ = SetWindowPos(
            pip.hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            pip.label_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        );
    }
}

unsafe fn position_window_if_changed(
    hwnd: HWND,
    insert_after: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> bool {
    let mut current = RECT::default();
    let matches = GetWindowRect(hwnd, &mut current).is_ok()
        && current.left == x
        && current.top == y
        && current.right - current.left == width
        && current.bottom - current.top == height;
    matches || SetWindowPos(hwnd, insert_after, x, y, width, height, SWP_NOACTIVATE).is_ok()
}

fn client_scene_rect(hwnd: HWND) -> Option<Rect> {
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client).ok()? };
    let width = (client.right - client.left).max(0);
    let height = (client.bottom - client.top).max(0);
    (width > 0 && height > 0).then(|| Rect::new(0, 0, width, height))
}

unsafe fn hide_unready_pip_pairs(s: &OverlayState) {
    let was_guarded = IN_OVERLAY.replace(true);
    for pip in &s.pip_windows {
        if !surface_is_ready(s, pip.label_hwnd) {
            let _ = ShowWindow(pip.hwnd, SW_HIDE);
            let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
        }
    }
    IN_OVERLAY.set(was_guarded);
}

unsafe fn hide_all_pip_pairs(s: &OverlayState) {
    let was_guarded = IN_OVERLAY.replace(true);
    for pip in &s.pip_windows {
        let _ = ShowWindow(pip.hwnd, SW_HIDE);
        let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
    }
    IN_OVERLAY.set(was_guarded);
}

unsafe fn schedule_compositor_recovery(s: &OverlayState) {
    let redraws = match s
        .compositor
        .as_ref()
        .map(|compositor| compositor.recovery_redraw_hwnds())
    {
        Some(Ok(redraws)) => redraws,
        Some(Err(error)) => {
            debug_log(&format!(
                "DirectComposition recovery scheduling failed: {error}"
            ));
            return;
        }
        None => return,
    };
    if redraws.is_empty() {
        return;
    }

    hide_unready_pip_pairs(s);
    for hwnd in redraws {
        request_redraw(hwnd);
    }
    if SERVICING_COMPOSITOR_RECOVERY.get() {
        return;
    }
    let first = COMPOSITOR_RECOVERY_POSTED.replace(true);
    if !first
        && PostMessageW(
            s.active_label_hwnd,
            WM_SERVICE_COMPOSITOR_RECOVERY,
            WPARAM(0),
            LPARAM(0),
        )
        .is_err()
    {
        COMPOSITOR_RECOVERY_POSTED.set(false);
    }
}

unsafe fn ensure_compositor(s: &mut OverlayState) -> bool {
    if !s.com_apartment.usable {
        s.com_apartment = ComApartment::initialize();
    }
    if s.compositor.is_none() && s.com_apartment.usable {
        match Compositor::new() {
            Ok(compositor) => s.compositor = Some(compositor),
            Err(error) => {
                debug_log(&format!(
                    "DirectComposition compositor retry failed: {error}"
                ));
                hide_all_pip_pairs(s);
                return false;
            }
        }
    }
    let Some(compositor) = s.compositor.as_mut() else {
        hide_all_pip_pairs(s);
        return false;
    };
    if let Err(error) = compositor.recover_if_needed() {
        debug_log(&format!("DirectComposition device retry failed: {error}"));
        hide_all_pip_pairs(s);
        return false;
    }
    schedule_compositor_recovery(s);
    true
}

unsafe fn unregister_composition_surface(s: &mut OverlayState, hwnd: HWND) -> bool {
    let Some(compositor) = s.compositor.as_mut() else {
        return true;
    };
    match compositor.unregister_surface(hwnd) {
        Ok(()) => {
            clear_redraw_request(hwnd);
            true
        }
        Err(first_error) => {
            debug_log(&format!(
                "DirectComposition surface teardown retry: {first_error}"
            ));
            if let Err(error) = compositor.recover_device() {
                debug_log(&format!(
                    "DirectComposition teardown recovery failed: {error}"
                ));
                return false;
            }
            match compositor.unregister_surface(hwnd) {
                Ok(()) => {
                    clear_redraw_request(hwnd);
                    true
                }
                Err(error) => {
                    debug_log(&format!(
                        "DirectComposition surface teardown failed: {error}"
                    ));
                    false
                }
            }
        }
    }
}

unsafe fn retry_pending_composition_destroys(s: &mut OverlayState) {
    let pending = std::mem::take(&mut s.pending_composition_destroys);
    for hwnd in pending {
        let detached = unregister_composition_surface(s, hwnd);
        if detached && IN_OVERLAY.get() {
            // DestroyWindow pumps messages, so only destroy while the outer
            // overlay transaction guard blocks re-entrant state access.
            let _ = DestroyWindow(hwnd);
        } else {
            s.pending_composition_destroys.push(hwnd);
        }
    }
}

unsafe fn ensure_surface(
    s: &mut OverlayState,
    hwnd: HWND,
    width: i32,
    height: i32,
    opacity: f32,
) -> bool {
    if !ensure_compositor(s) {
        retain_redraw_request(hwnd);
        return false;
    }
    let result = {
        let compositor = s.compositor.as_mut().expect("compositor ensured");
        match compositor.has_surface(hwnd) {
            Ok(true) => Ok(()),
            Ok(false) => compositor.register_surface(
                hwnd,
                width.max(1) as u32,
                height.max(1) as u32,
                opacity,
            ),
            Err(error) => Err(error),
        }
    };
    match result {
        Ok(()) => true,
        Err(error) => {
            debug_log(&format!(
                "DirectComposition surface registration/lookup failed: {error}"
            ));
            retain_redraw_request(hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

unsafe fn set_composition_opacity(s: &mut OverlayState, hwnd: HWND, alpha: u8) {
    if !ensure_compositor(s) {
        retain_redraw_request(hwnd);
        return;
    }
    let result = {
        let compositor = s.compositor.as_mut().expect("compositor ensured");
        compositor
            .set_surface_opacity(hwnd, f32::from(alpha) / 255.0)
            .and_then(|()| compositor.flush())
    };
    if let Err(error) = result {
        debug_log(&format!("DirectComposition opacity commit failed: {error}"));
        retain_redraw_request(hwnd);
        service_compositor_recovery(s);
    }
}

fn surface_is_ready(s: &OverlayState, hwnd: HWND) -> bool {
    s.compositor
        .as_ref()
        .and_then(|compositor| compositor.surface_is_attached(hwnd).ok())
        .unwrap_or(false)
}

fn mark_redraw_pending(pending: &mut HashSet<isize>, hwnd: HWND) -> bool {
    !hwnd.is_invalid() && pending.insert(hwnd.0 as isize)
}

fn take_redraw_pending(pending: &mut HashSet<isize>, hwnd: HWND) -> bool {
    !hwnd.is_invalid() && pending.remove(&(hwnd.0 as isize))
}

unsafe fn request_redraw(hwnd: HWND) {
    let first = REDRAW_PENDING.with(|pending| mark_redraw_pending(&mut pending.borrow_mut(), hwnd));
    if first {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn retain_redraw_request(hwnd: HWND) {
    REDRAW_PENDING.with(|pending| {
        mark_redraw_pending(&mut pending.borrow_mut(), hwnd);
    });
}

fn has_redraw_request(hwnd: HWND) -> bool {
    REDRAW_PENDING.with(|pending| pending.borrow().contains(&(hwnd.0 as isize)))
}

fn take_redraw_request(hwnd: HWND) -> bool {
    REDRAW_PENDING.with(|pending| take_redraw_pending(&mut pending.borrow_mut(), hwnd))
}

fn clear_redraw_request(hwnd: HWND) {
    REDRAW_PENDING.with(|pending| {
        pending.borrow_mut().remove(&(hwnd.0 as isize));
    });
}

unsafe fn service_compositor_recovery(s: &mut OverlayState) {
    if SERVICING_COMPOSITOR_RECOVERY.replace(true) {
        return;
    }
    COMPOSITOR_RECOVERY_POSTED.set(false);
    let was_guarded = IN_OVERLAY.replace(true);
    service_compositor_recovery_guarded(s);
    IN_OVERLAY.set(was_guarded);
    SERVICING_COMPOSITOR_RECOVERY.set(false);
}

unsafe fn suppress_toast_publication(s: &mut OverlayState) {
    s.toast.scene_ready = false;
    let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
    let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
}

unsafe fn service_compositor_recovery_guarded(s: &mut OverlayState) {
    if !ensure_compositor(s) {
        suppress_toast_publication(s);
        return;
    }
    retry_pending_composition_destroys(s);
    let redraws = match s
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .recovery_redraw_hwnds()
    {
        Ok(redraws) => redraws,
        Err(error) => {
            debug_log(&format!(
                "DirectComposition recovery handshake failed: {error}"
            ));
            suppress_toast_publication(s);
            return;
        }
    };
    for hwnd in redraws {
        retain_redraw_request(hwnd);
    }

    // Each role gets at most one complete-frame attempt in this recovery pass.
    // A second device loss retains dirty work for the normal polling retry
    // rather than recursing indefinitely on persistent hardware failure.
    if has_redraw_request(s.active_label_hwnd) {
        render_active_label_surface(s);
    }
    if has_redraw_request(s.broadcast_label_hwnd) {
        render_banner_surface(s);
    }
    if has_redraw_request(s.toast.hwnd) {
        render_toast_surface(s);
    }
    for index in 0..s.pip_windows.len() {
        if has_redraw_request(s.pip_windows[index].label_hwnd) {
            render_pip_surface(s, index);
        }
    }

    apply_pip_pair_visibility(s, overlay_visibility_allowed(s));
    if toast_publication_allowed(
        s.toast.phase,
        s.toast.scene_ready,
        overlay_visibility_allowed(s),
        surface_is_ready(s, s.toast.hwnd),
    ) {
        let _ = ShowWindow(s.toast.hwnd, SW_SHOWNOACTIVATE);
        let _ = SetTimer(s.toast.hwnd, TIMER_TOAST_FADE, TOAST_FADE_STEP_MS, None);
    } else {
        let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
        let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
    }
}

fn timer_scene_values(timer: &timers::TimerOverlay, now: Instant) -> (String, String, f32) {
    (
        timer.label.to_string(),
        format_timer_remaining(timer.remaining_time(now)),
        timer.progress(now),
    )
}

unsafe fn render_active_label_surface(s: &mut OverlayState) {
    let Some(canvas) = client_scene_rect(s.active_label_hwnd) else {
        return;
    };
    let _ = render_active_label_surface_for_size(s, canvas.width(), canvas.height());
}

unsafe fn render_active_label_surface_for_size(
    s: &mut OverlayState,
    width: i32,
    height: i32,
) -> bool {
    if s.active_label_text.is_empty() {
        return false;
    }
    let canvas = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(s, s.active_label_hwnd, canvas.width(), canvas.height(), 1.0) {
        return false;
    }
    let now = Instant::now();
    let timer_values = active_timer(s, now).map(|timer| timer_scene_values(timer, now));
    let timer = timer_values
        .as_ref()
        .map(|(label, remaining, progress)| TimerScene {
            label,
            remaining_text: remaining,
            progress: *progress,
        });
    let model = label_model(
        &s.active_label_text,
        s.active_label_class.as_deref(),
        s.active_label_number,
        s.active_label_color,
    );
    let scene = ActiveLabelScene {
        canvas,
        label: LabelScene {
            model,
            style: LabelStyle::new(s.dpi_scale, s.label_height),
            theme: &s.label_theme,
            alpha: if s.active_label_hovered {
                s.label_alpha / 2
            } else {
                s.label_alpha
            },
        },
        timer,
    };
    match s
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_active_label(s.active_label_hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.active_label_hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition active-label render failed: {error}"
            ));
            retain_redraw_request(s.active_label_hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

unsafe fn render_pip_surface(s: &mut OverlayState, pip_index: usize) {
    let Some(pip) = s.pip_windows.get(pip_index) else {
        return;
    };
    let Some(canvas) = client_scene_rect(pip.label_hwnd) else {
        return;
    };
    let _ = render_pip_surface_for_size(s, pip_index, canvas.width(), canvas.height());
}

unsafe fn render_pip_surface_for_size(
    s: &mut OverlayState,
    pip_index: usize,
    width: i32,
    height: i32,
) -> bool {
    let Some(pip) = s.pip_windows.get(pip_index) else {
        return false;
    };
    let hwnd = pip.label_hwnd;
    let canvas = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(s, hwnd, canvas.width(), canvas.height(), 1.0) {
        return false;
    }
    let pip = &s.pip_windows[pip_index];
    let now = Instant::now();
    let source_id = format!("pid:{}", pip.pid);
    let timer_values = s
        .timers
        .visible_for(Some(&source_id), now)
        .map(|timer| timer_scene_values(timer, now));
    let timer = timer_values
        .as_ref()
        .map(|(label, remaining, progress)| TimerScene {
            label,
            remaining_text: remaining,
            progress: *progress,
        });
    let now_ms = windows::Win32::System::SystemInformation::GetTickCount64();
    let notification = s
        .notifications
        .get(&pip.pid)
        .map(|notification| notification.visual_snapshot(now_ms, s.animations_enabled));
    let reorder_dragging = s.reorder_drag.as_ref().is_some_and(|drag| drag.dragging);
    let drag_source =
        reorder_dragging && s.reorder_drag.as_ref().map(|drag| drag.from_index) == Some(pip_index);
    let drop_target = reorder_dragging
        && s.drop_target == Some(pip_index)
        && s.reorder_drag.as_ref().map(|drag| drag.from_index) != Some(pip_index);
    let model = label_model(
        &pip.label,
        pip.class.as_deref(),
        pip.number,
        color_for_number(pip.number),
    );
    let scene = PipScene {
        canvas,
        border_width: dpi(BORDER_WIDTH, s.dpi_scale),
        scale: s.dpi_scale,
        label: LabelScene {
            model,
            style: LabelStyle::new(s.dpi_scale, s.label_height),
            theme: &s.label_theme,
            alpha: s.label_alpha,
        },
        timer,
        notification,
        interaction: PipInteractionScene {
            hovered: pip.hovered,
            edit_mode: s.edit_mode,
            reorder_dragging,
            drag_source,
            drop_target,
        },
    };
    match s
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_pip_scene(hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition PiP scene render failed: {error}"
            ));
            retain_redraw_request(hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

fn input_indicator_background() -> Color {
    let value = if crate::broadcast::is_active() {
        0x002030CC
    } else {
        match crate::broadcast::mouse_clutch_status() {
            crate::broadcast::MouseClutchStatus::Inactive => 0x002030CC,
            crate::broadcast::MouseClutchStatus::Active => 0x00906A28,
            crate::broadcast::MouseClutchStatus::Releasing => 0x002080C8,
        }
    };
    Color::from_colorref(value)
}

unsafe fn render_banner_surface(s: &mut OverlayState) {
    let Some(bounds) = client_scene_rect(s.broadcast_label_hwnd) else {
        return;
    };
    let _ = render_banner_surface_for_size(s, bounds.width(), bounds.height());
}

unsafe fn render_banner_surface_for_size(s: &mut OverlayState, width: i32, height: i32) -> bool {
    let Some(text) = input_indicator_text() else {
        return false;
    };
    let bounds = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(
        s,
        s.broadcast_label_hwnd,
        bounds.width(),
        bounds.height(),
        1.0,
    ) {
        return false;
    }
    let scene = StatusBannerScene {
        bounds,
        text,
        background: input_indicator_background(),
        alpha: s.label_alpha,
        scale: s.dpi_scale,
        logical_label_height: s.label_height,
    };
    match s
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_status_banner(s.broadcast_label_hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.broadcast_label_hwnd);
            true
        }
        Err(error) => {
            debug_log(&format!(
                "DirectComposition status-banner render failed: {error}"
            ));
            retain_redraw_request(s.broadcast_label_hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

unsafe fn render_toast_surface(s: &mut OverlayState) {
    s.toast.scene_ready = false;
    let Some(bounds) = client_scene_rect(s.toast.hwnd) else {
        suppress_toast_publication(s);
        return;
    };
    let _ = render_toast_surface_for_size(s, bounds.width(), bounds.height());
}

unsafe fn render_toast_surface_for_size(s: &mut OverlayState, width: i32, height: i32) -> bool {
    // An attachment left by an older toast cannot make this staged scene
    // publishable. Only this render's successful Present1 restores readiness.
    s.toast.scene_ready = false;
    let bounds = Rect::new(0, 0, width.max(1), height.max(1));
    if !ensure_surface(s, s.toast.hwnd, bounds.width(), bounds.height(), 1.0) {
        return false;
    }
    let scene = ToastScene {
        bounds,
        text: &s.toast.text,
        background: Color::from_colorref(TOAST_BG_COLOR),
        alpha: s.toast.alpha,
        scale: s.dpi_scale,
        logical_height: s.toast.height,
    };
    match s
        .compositor
        .as_mut()
        .expect("compositor ensured")
        .render_toast(s.toast.hwnd, &scene)
    {
        Ok(()) => {
            clear_redraw_request(s.toast.hwnd);
            s.toast.scene_ready = true;
            true
        }
        Err(error) => {
            debug_log(&format!("DirectComposition toast render failed: {error}"));
            retain_redraw_request(s.toast.hwnd);
            service_compositor_recovery(s);
            false
        }
    }
}

unsafe fn validate_composition_paint(hwnd: HWND) {
    let _ = ValidateRect(hwnd, None);
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
    // Coalesce one complete composition redraw per PiP scene.
    for pw in &s.pip_windows {
        request_redraw(pw.label_hwnd);
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
// Composition window procedures
// ---------------------------------------------------------------------------

unsafe extern "system" fn pip_label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !IN_OVERLAY.get() && take_redraw_request(hwnd);
            if !IN_OVERLAY.get() {
                let raw_idx = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
                if raw_idx > 0 {
                    if let Some(s) = state().as_mut() {
                        if requested || !surface_is_ready(s, hwnd) {
                            render_pip_surface(s, raw_idx - 1);
                        }
                    }
                }
            }
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
    if IN_OVERLAY.get()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !IN_OVERLAY.get() && take_redraw_request(hwnd);
            if !IN_OVERLAY.get() {
                if let Some(s) = state().as_mut() {
                    if requested || !surface_is_ready(s, hwnd) {
                        render_banner_surface(s);
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn toast_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !IN_OVERLAY.get() && take_redraw_request(hwnd);
            if !IN_OVERLAY.get() {
                if let Some(s) = state().as_mut() {
                    if requested || !surface_is_ready(s, hwnd) {
                        render_toast_surface(s);
                        if !toast_publication_allowed(
                            s.toast.phase,
                            s.toast.scene_ready,
                            overlay_visibility_allowed(s),
                            surface_is_ready(s, hwnd),
                        ) {
                            suppress_toast_publication(s);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_TOAST_FADE => {
            let Some(s) = state().as_mut() else {
                let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
                return LRESULT(0);
            };
            if !toast_publication_allowed(
                s.toast.phase,
                s.toast.scene_ready,
                overlay_visibility_allowed(s),
                surface_is_ready(s, hwnd),
            ) {
                s.toast.phase = ToastPhase::Hidden;
                suppress_toast_publication(s);
                return LRESULT(0);
            }
            let now = windows::Win32::System::SystemInformation::GetTickCount64();
            let transition = advance_toast_fade(
                s.toast.phase,
                s.toast.alpha,
                s.toast.phase_start,
                s.toast.duration_ms,
                now,
            );
            s.toast.phase = transition.phase;
            s.toast.alpha = transition.alpha;
            s.toast.phase_start = transition.phase_start;
            match transition.effect {
                ToastFadeEffect::None => {}
                ToastFadeEffect::UpdateOpacity(alpha) => {
                    set_composition_opacity(s, hwnd, alpha);
                }
                ToastFadeEffect::HideAndStop => {
                    s.toast.scene_ready = false;
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    let _ = KillTimer(hwnd, TIMER_TOAST_FADE);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_toast_inner(s: &mut OverlayState, text: &str) {
    if !s.toast.enabled || !overlay_visibility_allowed(s) {
        return;
    }

    // Complete every fallible model/geometry preparation step while the
    // currently published toast remains authoritative.
    let active_hwnd = s
        .active_pid
        .and_then(|pid| s.eq_windows.iter().find(|w| w.pid == pid))
        .map(|w| w.hwnd);
    let Some(eq_hwnd) = active_hwnd else {
        return;
    };

    let mut eq_rect = RECT::default();
    if GetClientRect(eq_hwnd, &mut eq_rect).is_err() {
        return;
    }
    let mut top_left = POINT {
        x: eq_rect.left,
        y: eq_rect.top,
    };
    if !ClientToScreen(eq_hwnd, &mut top_left).as_bool() {
        return;
    }

    let d = s.dpi_scale;
    let toast_h = dpi(s.toast.height, d);
    if !ensure_compositor(s) {
        return;
    }
    let text_width = match s
        .compositor
        .as_ref()
        .expect("compositor ensured")
        .measure_text(
            text,
            &UiTextRole::Toast.font(),
            UiTextRole::Toast.height(d, (s.toast.height - 12).max(12)),
        ) {
        Ok(width) => width,
        Err(error) => {
            debug_log(&format!("DirectWrite toast measurement failed: {error}"));
            return;
        }
    };
    let pad = dpi(20, d);
    let toast_w = text_width + pad * 2;
    let eq_client_w = eq_rect.right - eq_rect.left;
    let toast_x = top_left.x + (eq_client_w - toast_w) / 2;
    let eq_client_h = eq_rect.bottom - eq_rect.top;
    let toast_y = top_left.y + eq_client_h / 3;

    // Replacement is fail-closed: no older attachment may remain visible once
    // the new toast starts staging.
    let _ = ShowWindow(s.toast.hwnd, SW_HIDE);
    let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
    s.toast.phase = ToastPhase::Hidden;
    s.toast.scene_ready = false;
    if !position_window_if_changed(
        s.toast.hwnd,
        HWND_TOPMOST,
        toast_x,
        toast_y,
        toast_w,
        toast_h,
    ) {
        return;
    }

    s.toast.text = text.to_string();
    s.toast.alpha = 0;
    s.toast.phase_start = 0;
    if !render_toast_surface_for_size(s, toast_w, toast_h) {
        s.toast.scene_ready = false;
        suppress_toast_publication(s);
        return;
    }

    // Foreground hooks can run while DirectComposition pumps Win32 messages.
    // Publish only this successfully presented scene and only while it is
    // still valid for the current overlay visibility policy.
    if !overlay_visibility_allowed(s) {
        s.toast.phase = ToastPhase::Hidden;
        suppress_toast_publication(s);
        return;
    }
    s.toast.phase = ToastPhase::FadingIn;
    s.toast.phase_start = windows::Win32::System::SystemInformation::GetTickCount64();
    if !toast_publication_allowed(
        s.toast.phase,
        s.toast.scene_ready,
        true,
        surface_is_ready(s, s.toast.hwnd),
    ) {
        s.toast.phase = ToastPhase::Hidden;
        suppress_toast_publication(s);
        return;
    }
    let _ = ShowWindow(s.toast.hwnd, SW_SHOWNOACTIVATE);
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
    // Skip state access during rebuilds while still validating paint without
    // asking the host to erase or author pixels.
    if IN_OVERLAY.get() {
        return match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                validate_composition_paint(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        };
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
                let _ = ScreenToClient(hwnd, &mut point);
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
                    let _ = ScreenToClient(hwnd, &mut client_pt);

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

        WM_ERASEBKGND => LRESULT(1),

        WM_PAINT => {
            // The host is exclusively a DWM thumbnail target. Authored pixels
            // are presented by its aligned composition sibling.
            validate_composition_paint(hwnd);
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
                        position_pip_pair(pw, sx, sy, w, h);
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

                    if render_pip_surface_for_size(s, idx, nw, nh) {
                        let pw = &s.pip_windows[idx];
                        position_pip_pair(pw, new_rect.left, new_rect.top, nw, nh);

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
                    for i in 0..s.pip_windows.len() {
                        if let Some(rect) = rects.get(i).copied() {
                            let cw = rect.right - rect.left;
                            let ch = rect.bottom - rect.top;
                            if render_pip_surface_for_size(s, i, cw, ch) {
                                let pw = &s.pip_windows[i];
                                position_pip_pair(pw, rect.left, rect.top, cw, ch);
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
                            }
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
                                opacity: reorder_thumbnail_alpha(s.thumbnail_alpha),
                                ..Default::default()
                            };
                            let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                            request_redraw(pw.label_hwnd);
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
                                request_redraw(pw.label_hwnd);
                            }
                        }
                        s.drop_target = new_target;
                        if let Some(new_t) = new_target {
                            if let Some(pw) = s.pip_windows.get(new_t) {
                                request_redraw(pw.label_hwnd);
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
                    request_redraw(pw.label_hwnd);
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
                        opacity: s.thumbnail_alpha,
                        ..Default::default()
                    };
                    let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                    request_redraw(pw.label_hwnd);
                }
            }

            // A potential click that never became a drag is no longer active.
            if s.reorder_drag.as_ref().is_some_and(|drag| !drag.dragging) {
                cancel_reorder_drag(s);
            }

            LRESULT(0)
        }

        WM_CANCELMODE => {
            let Some(s) = state().as_mut() else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let had_invite_press = notifications::invite_action_pressed(s, pip_idx);
            let had_reorder = s.reorder_drag.is_some();
            if had_invite_press {
                notifications::clear_invite_interaction(s, pip_idx);
            }
            if had_reorder {
                cancel_reorder_drag(s);
            }
            if had_invite_press || had_reorder {
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
                cancel_reorder_drag(s);
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
                cancel_reorder_drag(s);
                let _ = SetCapture(hwnd);
                return LRESULT(0);
            }

            cancel_reorder_drag(s);
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
                    cancel_reorder_drag(s);
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
                            opacity: s.thumbnail_alpha,
                            ..Default::default()
                        };
                        let _ = DwmUpdateThumbnailProperties(pw.thumb, &props);
                        request_redraw(pw.label_hwnd);
                    }
                    if let Some(target) = old_drop_target {
                        if let Some(pw) = s.pip_windows.get(target) {
                            request_redraw(pw.label_hwnd);
                        }
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

fn forwards_active_label_mouse_message(message: u32) -> bool {
    matches!(message, WM_LBUTTONDOWN | WM_LBUTTONUP)
}

unsafe fn tick_notification_animation(timer_hwnd: HWND) {
    let Some(s) = state().as_mut() else {
        let _ = KillTimer(timer_hwnd, notifications::TIMER_ID);
        return;
    };
    notifications::tick(s, timer_hwnd);
}

unsafe fn tick_timer_overlay(timer_hwnd: HWND) {
    let Some(s) = state().as_mut() else {
        let _ = KillTimer(timer_hwnd, TIMER_OVERLAY_TICK);
        return;
    };
    let now = Instant::now();
    let previous_owners = timer_owner_hwnds(s, now);
    let expired = s.timers.remove_expired(now);
    if s.timers.is_empty() {
        let _ = KillTimer(timer_hwnd, TIMER_OVERLAY_TICK);
    }
    if expired {
        // Timer appearance/expiry is structural for the active-label HWND;
        // ordinary countdown ticks never move or resize any HWND.
        update_active_label(s);
    }
    let current_owners = timer_owner_hwnds(s, now);
    let pip_label_hwnds = s
        .pip_windows
        .iter()
        .map(|pip| pip.label_hwnd)
        .collect::<Vec<_>>();
    for hwnd in timer_tick_redraw_targets(
        expired,
        s.active_label_hwnd,
        &pip_label_hwnds,
        &previous_owners,
        &current_owners,
    ) {
        request_redraw(hwnd);
    }
}

unsafe extern "system" fn label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if IN_OVERLAY.get() {
        return match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                validate_composition_paint(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        };
    }
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_SERVICE_COMPOSITOR_RECOVERY => {
            if let Some(s) = state().as_mut() {
                service_compositor_recovery(s);
            } else {
                COMPOSITOR_RECOVERY_POSTED.set(false);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == notifications::TIMER_ID => {
            tick_notification_animation(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_OVERLAY_TICK => {
            tick_timer_overlay(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = take_redraw_request(hwnd);
            if let Some(s) = state().as_mut() {
                if requested || !surface_is_ready(s, hwnd) {
                    render_active_label_surface(s);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(s) = state().as_mut() {
                if !s.active_label_hovered {
                    s.active_label_hovered = true;
                    let alpha = s.label_alpha / 2;
                    set_composition_opacity(s, hwnd, alpha);
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
            if let Some(s) = state().as_mut() {
                s.active_label_hovered = false;
                let alpha = s.label_alpha;
                set_composition_opacity(s, hwnd, alpha);
            }
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
        message if forwards_active_label_mouse_message(message) => {
            let mut pt = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ClientToScreen(hwnd, &mut pt);
            let target = state().as_ref().and_then(|s| {
                let pid = s.active_pid?;
                s.eq_windows
                    .iter()
                    .find(|window| window.pid == pid)
                    .map(|window| window.hwnd)
            });
            if let Some(target) = target.filter(|target| IsWindow(*target).as_bool()) {
                let mut client = pt;
                let _ = ScreenToClient(target, &mut client);
                let packed =
                    (client.x as i16 as u16 as isize) | ((client.y as i16 as u16 as isize) << 16);
                let _ = PostMessageW(target, msg, wparam, LPARAM(packed));
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
        s.thumbnail_alpha = opacity_percent_to_alpha(cfg.effective_pip_opacity());
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
        s.label_alpha = opacity_percent_to_alpha(opacity);
        s.label_theme = configured_label_theme(&cfg);
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
            let pips = std::mem::take(&mut s.pip_windows);
            let pending_composition_destroys = std::mem::take(&mut s.pending_composition_destroys);
            if let Some(mut compositor) = s.compositor.take() {
                for pip in &pips {
                    if let Err(error) = compositor.unregister_surface(pip.label_hwnd) {
                        debug_log(&format!("DirectComposition PiP cleanup failed: {error}"));
                    }
                }
                for hwnd in pending_composition_destroys.iter().copied().chain([
                    s.active_label_hwnd,
                    s.broadcast_label_hwnd,
                    s.toast.hwnd,
                ]) {
                    if let Err(error) = compositor.unregister_surface(hwnd) {
                        debug_log(&format!("DirectComposition cleanup failed: {error}"));
                    }
                }
                // shutdown detaches any registration whose individual commit
                // failed before its owning HWND is destroyed below.
                if let Err(error) = compositor.shutdown() {
                    debug_log(&format!("DirectComposition shutdown failed: {error}"));
                }
            }
            for pw in pips {
                let _ = DwmUnregisterThumbnail(pw.thumb);
                let _ = DestroyWindow(pw.label_hwnd);
                let _ = DestroyWindow(pw.hwnd);
            }
            for hwnd in pending_composition_destroys {
                let _ = DestroyWindow(hwnd);
            }
            let _ = KillTimer(s.active_label_hwnd, notifications::TIMER_ID);
            let _ = KillTimer(s.active_label_hwnd, TIMER_OVERLAY_TICK);
            let _ = DestroyWindow(s.active_label_hwnd);
            let _ = DestroyWindow(s.broadcast_label_hwnd);
            let _ = KillTimer(s.toast.hwnd, TIMER_TOAST_FADE);
            let _ = DestroyWindow(s.toast.hwnd);
        }
        *state_unguarded() = None;
        COMPOSITOR_RECOVERY_POSTED.set(false);
        SERVICING_COMPOSITOR_RECOVERY.set(false);
        REDRAW_PENDING.with(|pending| pending.borrow_mut().clear());
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
    fn redraw_requests_coalesce_until_one_frame_claims_the_hwnd() {
        let hwnd = HWND(0x1234usize as *mut _);
        let mut pending = HashSet::new();
        assert!(mark_redraw_pending(&mut pending, hwnd));
        assert!(!mark_redraw_pending(&mut pending, hwnd));
        assert!(take_redraw_pending(&mut pending, hwnd));
        assert!(!take_redraw_pending(&mut pending, hwnd));
        assert!(!mark_redraw_pending(&mut pending, HWND::default()));
    }

    #[test]
    fn staged_toast_cannot_publish_a_previous_attached_frame() {
        assert!(!toast_publication_allowed(
            ToastPhase::Hidden,
            false,
            true,
            true,
        ));
        // Even an accidentally advanced phase cannot compensate for a frame
        // that did not complete rendering for the staged toast.
        assert!(!toast_publication_allowed(
            ToastPhase::FadingIn,
            false,
            true,
            true,
        ));
    }

    #[test]
    fn completed_toast_frame_requires_every_publication_gate() {
        assert!(toast_publication_allowed(
            ToastPhase::FadingIn,
            true,
            true,
            true,
        ));
        assert!(!toast_publication_allowed(
            ToastPhase::Hidden,
            true,
            true,
            true,
        ));
        assert!(!toast_publication_allowed(
            ToastPhase::FadingIn,
            true,
            false,
            true,
        ));
        assert!(!toast_publication_allowed(
            ToastPhase::FadingIn,
            true,
            true,
            false,
        ));
    }

    #[test]
    fn toast_fade_in_saturates_and_starts_visible_duration_at_that_tick() {
        assert_eq!(
            advance_toast_fade(ToastPhase::FadingIn, 25, 10, 2_000, 40),
            ToastFadeTransition {
                phase: ToastPhase::FadingIn,
                alpha: 50,
                phase_start: 10,
                effect: ToastFadeEffect::UpdateOpacity(50),
            }
        );
        assert_eq!(
            advance_toast_fade(ToastPhase::FadingIn, 210, 10, 2_000, 40),
            ToastFadeTransition {
                phase: ToastPhase::Visible,
                alpha: TOAST_MAX_ALPHA,
                phase_start: 40,
                effect: ToastFadeEffect::UpdateOpacity(TOAST_MAX_ALPHA),
            }
        );
    }

    #[test]
    fn toast_visible_duration_transitions_at_the_inclusive_threshold() {
        assert_eq!(
            advance_toast_fade(ToastPhase::Visible, 220, 100, 2_000, 2_099).phase,
            ToastPhase::Visible
        );
        let transition = advance_toast_fade(ToastPhase::Visible, 220, 100, 2_000, 2_100);
        assert_eq!(transition.phase, ToastPhase::FadingOut);
        assert_eq!(transition.effect, ToastFadeEffect::None);
    }

    #[test]
    fn toast_fade_out_updates_opacity_then_hides_and_stops_at_zero() {
        assert_eq!(
            advance_toast_fade(ToastPhase::FadingOut, 50, 100, 2_000, 200).effect,
            ToastFadeEffect::UpdateOpacity(25)
        );
        assert_eq!(
            advance_toast_fade(ToastPhase::FadingOut, 25, 100, 2_000, 200),
            ToastFadeTransition {
                phase: ToastPhase::Hidden,
                alpha: 0,
                phase_start: 100,
                effect: ToastFadeEffect::HideAndStop,
            }
        );
    }

    #[test]
    fn hidden_toast_always_requests_hide_and_timer_stop() {
        assert_eq!(
            advance_toast_fade(ToastPhase::Hidden, 12, 100, 2_000, 200),
            ToastFadeTransition {
                phase: ToastPhase::Hidden,
                alpha: 0,
                phase_start: 100,
                effect: ToastFadeEffect::HideAndStop,
            }
        );
    }

    #[test]
    fn timer_countdown_rounds_up_to_the_next_tenth() {
        assert_eq!(format_timer_remaining(Duration::from_secs(10)), "10.0s");
        assert_eq!(format_timer_remaining(Duration::from_millis(9901)), "10.0s");
        assert_eq!(format_timer_remaining(Duration::from_millis(9900)), "9.9s");
        assert_eq!(format_timer_remaining(Duration::from_millis(1)), "0.1s");
        assert_eq!(format_timer_remaining(Duration::ZERO), "0.0s");
    }

    #[test]
    fn expired_timer_redraws_every_pip_even_after_visible_owners_are_empty() {
        let active = HWND(1usize as *mut _);
        let first_pip = HWND(2usize as *mut _);
        let expired_owner = HWND(3usize as *mut _);

        assert_eq!(
            timer_tick_redraw_targets(true, active, &[first_pip, expired_owner], &[], &[],),
            vec![first_pip, expired_owner]
        );
        assert_eq!(
            timer_tick_redraw_targets(
                false,
                active,
                &[first_pip, expired_owner],
                &[expired_owner],
                &[expired_owner],
            ),
            vec![expired_owner]
        );
    }

    #[test]
    fn overlay_visibility_policy_requires_user_client_and_foreground_permission() {
        assert!(overlay_visibility_policy(false, true, false, true));
        assert!(overlay_visibility_policy(false, true, true, false));
        assert!(!overlay_visibility_policy(true, true, true, true));
        assert!(!overlay_visibility_policy(false, false, true, true));
        assert!(!overlay_visibility_policy(false, true, false, false));
    }

    #[test]
    fn thumbnail_opacity_scales_percent_and_reorder_never_brightens_it() {
        assert_eq!(opacity_percent_to_alpha(10), 25);
        assert_eq!(opacity_percent_to_alpha(80), 204);
        assert_eq!(opacity_percent_to_alpha(100), 255);
        assert_eq!(reorder_thumbnail_alpha(25), 25);
        assert_eq!(reorder_thumbnail_alpha(204), THUMB_OPACITY_DRAG_MAX);
    }

    #[test]
    fn reorder_cancellation_takes_dimmed_source_and_old_target_atomically() {
        let mut drag = Some(ReorderDragState {
            from_index: 2,
            start_pt: POINT::default(),
            dragging: true,
        });
        let mut target = Some(4);
        assert_eq!(
            take_reorder_cancellation(&mut drag, &mut target),
            ReorderCancellation {
                dimmed_source: Some(2),
                old_target: Some(4),
            }
        );
        assert!(drag.is_none());
        assert!(target.is_none());

        let mut pending_click = Some(ReorderDragState {
            from_index: 1,
            start_pt: POINT::default(),
            dragging: false,
        });
        assert_eq!(
            take_reorder_cancellation(&mut pending_click, &mut None),
            ReorderCancellation {
                dimmed_source: None,
                old_target: None,
            }
        );
    }

    #[test]
    fn active_label_forwards_only_matched_left_button_messages() {
        assert!(forwards_active_label_mouse_message(WM_LBUTTONDOWN));
        assert!(forwards_active_label_mouse_message(WM_LBUTTONUP));
        assert!(!forwards_active_label_mouse_message(WM_RBUTTONDOWN));
        assert!(!forwards_active_label_mouse_message(WM_RBUTTONUP));
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
