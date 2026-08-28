use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::geometry::scale;
use super::runtime::{is_busy, try_with_state, try_with_state_mut};
use super::scenes::{DpsScene, DpsSceneRow, DPS_MIN_ROW_DIP, DPS_ROW_DIP};
use super::state::OverlayState;
use super::surfaces::{
    position_window_if_changed, render_dps_surface, request_redraw, surface_is_ready,
    take_redraw_request, validate_composition_paint,
};
use crate::config::{
    Config, DpsOverlayPlacement, DEFAULT_DPS_OVERLAY_WIDTH_DIP, MIN_DPS_OVERLAY_WIDTH_DIP,
};

const DEFAULT_OFFSET_DIP: i32 = 24;
const RESIZE_ZONE_DIP: i32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SceneKey {
    encounter: Option<(eqcombat::EncounterId, u64)>,
    active_source: Option<eqlog::LogSourceId>,
    edit_mode: bool,
    top_rows: u8,
    preview: bool,
}

#[derive(Clone, Copy)]
enum DragKind {
    Move,
    ResizeRight,
}

#[derive(Clone, Copy)]
struct DragState {
    kind: DragKind,
    start_cursor: POINT,
    start_rect: RECT,
}

pub(super) struct DpsOverlayController {
    pub(super) enabled: bool,
    pub(super) top_rows: u8,
    pub(super) placement: Option<DpsOverlayPlacement>,
    pub(super) book: Option<Arc<eqcombat::EncounterBookSnapshot>>,
    pub(super) scene: Option<DpsScene>,
    scene_key: Option<SceneKey>,
    runtime_rect: Option<RECT>,
    last_monitor: RECT,
    drag: Option<DragState>,
}

impl DpsOverlayController {
    pub(super) fn new(config: &Config) -> Self {
        Self {
            enabled: config.dps_overlay_enabled,
            top_rows: config.effective_dps_overlay_top_rows(),
            placement: config.dps_overlay_placement.clone(),
            book: None,
            scene: None,
            scene_key: None,
            runtime_rect: None,
            last_monitor: RECT::default(),
            drag: None,
        }
    }

    pub(super) fn apply_config(&mut self, config: &Config) {
        self.enabled = config.dps_overlay_enabled;
        self.top_rows = config.effective_dps_overlay_top_rows();
        if self.placement != config.dps_overlay_placement {
            self.placement = config.dps_overlay_placement.clone();
            self.runtime_rect = None;
        }
        self.scene_key = None;
    }
}

pub(super) unsafe fn apply_book(
    state: &mut OverlayState,
    book: Arc<eqcombat::EncounterBookSnapshot>,
) {
    state.dps.book = Some(book);
    reconcile(state);
}

pub(super) unsafe fn reconcile(state: &mut OverlayState) {
    let monitor = state.layout.monitor_rect;
    if monitor_width(monitor) <= 0 || monitor_height(monitor) <= 0 {
        return;
    }
    if !same_rect(state.dps.last_monitor, monitor) {
        state.dps.last_monitor = monitor;
        state.dps.runtime_rect = None;
    }
    let active_source = state
        .clients
        .active_pid()
        .map(|pid| eqlog::LogSourceId::new(format!("pid:{pid}")));
    let selected = state.dps.book.as_ref().and_then(|book| {
        let id = eqcombat::select_presented(book, active_source.as_ref())?
            .id
            .clone();
        book.encounters
            .iter()
            .find(|encounter| encounter.id == id)
            .cloned()
    });
    let preview = state.interaction.edit_mode && selected.is_none();
    let encounter_key = selected
        .as_ref()
        .map(|encounter| (encounter.id.clone(), encounter.revision));
    let key = SceneKey {
        encounter: encounter_key,
        active_source: active_source.clone(),
        edit_mode: state.interaction.edit_mode,
        top_rows: state.dps.top_rows,
        preview,
    };

    if selected.is_none() && !preview {
        state.dps.scene = None;
        state.dps.scene_key = Some(key);
        state.presentation.dps_scene_ready = false;
        let _ = ShowWindow(state.presentation.dps_hwnd, SW_HIDE);
        return;
    }

    let rows = selected
        .as_ref()
        .map(|encounter| {
            eqcombat::project_visible_rows(&encounter.rows, usize::from(state.dps.top_rows))
        })
        .unwrap_or_else(preview_rows);
    let active_identity = state.clients.active_pid().and_then(|pid| {
        state
            .clients
            .windows
            .iter()
            .find(|window| window.pid == pid)
            .and_then(|window| Some((window.server.as_deref()?, window.character.as_deref()?)))
    });
    let scene_rows = rows
        .iter()
        .map(|row| DpsSceneRow {
            rank: row.rank,
            name: Arc::from(if row.has_pet_damage {
                format!("{} +Pets", row.display_name)
            } else if row.provisional_pet {
                format!("{} (Pet)", row.display_name)
            } else {
                row.display_name.to_string()
            }),
            damage: Arc::from(eqcombat::format_grouped_ascii(row.damage)),
            dps: Arc::from(eqcombat::format_grouped_ascii(row.dps)),
            sdps: Arc::from(eqcombat::format_grouped_ascii(row.sdps)),
            contribution_millionths: row.contribution_millionths,
            has_pet_damage: row.has_pet_damage,
            managed_extra: row.managed && row.rank > usize::from(state.dps.top_rows),
            active_managed: active_identity.is_some_and(|(server, character)| {
                row.participant.server.eq_ignore_ascii_case(server)
                    && row
                        .participant
                        .canonical_name
                        .eq_ignore_ascii_case(character)
            }),
        })
        .collect::<Vec<_>>();
    let managed_separator = scene_rows.iter().any(|row| row.managed_extra);
    let scale_factor = state.layout.dpi_scale;
    let mut rect = state
        .dps
        .runtime_rect
        .unwrap_or_else(|| restored_rect(state, monitor, scale_factor));
    let max_height = monitor_height(monitor);
    let row_height = DpsScene::row_height_for(
        scene_rows.len(),
        managed_separator,
        max_height,
        scale_factor,
    );
    let height = DpsScene::content_height(
        scene_rows.len(),
        managed_separator,
        row_height,
        scale_factor,
    )
    .min(max_height);
    rect.bottom = rect.top.saturating_add(height);
    rect = clamp_rect(
        rect,
        monitor,
        scale(MIN_DPS_OVERLAY_WIDTH_DIP as i32, scale_factor),
    );
    state.dps.runtime_rect = Some(rect);
    let width = monitor_width(rect);
    let height = monitor_height(rect);
    position_window_if_changed(
        state.presentation.dps_hwnd,
        HWND_TOPMOST,
        rect.left,
        rect.top,
        width,
        height,
    );

    let (title, duration) = selected.as_ref().map_or_else(
        || (Arc::from("DPS overlay preview"), Arc::from("0:42")),
        |encounter| {
            (
                encounter.title.clone(),
                Arc::from(format_duration(encounter.encounter_seconds)),
            )
        },
    );
    let scene = DpsScene {
        bounds: super::labels::Rect::new(0, 0, width, height),
        scale_millionths: (scale_factor * 1_000_000.0).round().max(1.0) as u32,
        title,
        duration,
        rows: scene_rows.into(),
        row_height: row_height.clamp(
            scale(DPS_MIN_ROW_DIP, scale_factor).max(1),
            scale(DPS_ROW_DIP, scale_factor).max(1),
        ),
        edit_mode: state.interaction.edit_mode,
        preview,
    };
    if state.dps.scene_key.as_ref() != Some(&key) || state.dps.scene.as_ref() != Some(&scene) {
        state.dps.scene_key = Some(key);
        state.dps.scene = Some(scene);
        state.presentation.dps_scene_ready = false;
        let _ = ShowWindow(state.presentation.dps_hwnd, SW_HIDE);
        request_redraw(state.presentation.dps_hwnd);
    }
}

pub(super) unsafe fn set_edit_mode(state: &mut OverlayState, edit_mode: bool) {
    let hwnd = state.presentation.dps_hwnd;
    let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    let transparent = WS_EX_TRANSPARENT.0;
    let updated = if edit_mode {
        current & !transparent
    } else {
        current | transparent
    };
    if updated != current {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, updated as isize);
    }
    if !edit_mode {
        state.dps.drag = None;
        let _ = ReleaseCapture();
    }
    state.dps.scene_key = None;
    reconcile(state);
}

pub(super) unsafe fn save_placement(state: &mut OverlayState) {
    let mut rect = RECT::default();
    if GetWindowRect(state.presentation.dps_hwnd, &mut rect).is_err() {
        return;
    }
    let monitor = state.layout.monitor_rect;
    let scale = state.layout.dpi_scale.max(0.01);
    let placement = DpsOverlayPlacement {
        x_dip: (f64::from(rect.left - monitor.left) / scale).round() as i32,
        y_dip: (f64::from(rect.top - monitor.top) / scale).round() as i32,
        width_dip: (f64::from(monitor_width(rect)) / scale)
            .round()
            .max(f64::from(MIN_DPS_OVERLAY_WIDTH_DIP)) as u32,
    };
    state.dps.placement = Some(placement.clone());
    state.dps.runtime_rect = Some(rect);
    let _ = Config::update(move |config| {
        config.dps_overlay_placement = Some(placement);
    });
}

fn restored_rect(state: &OverlayState, monitor: RECT, scale_factor: f64) -> RECT {
    let placement = state.dps.placement.clone().unwrap_or(DpsOverlayPlacement {
        x_dip: DEFAULT_OFFSET_DIP,
        y_dip: DEFAULT_OFFSET_DIP,
        width_dip: DEFAULT_DPS_OVERLAY_WIDTH_DIP,
    });
    let width_dip = placement.width_dip.max(MIN_DPS_OVERLAY_WIDTH_DIP) as i32;
    let left = monitor.left + scale(placement.x_dip, scale_factor);
    let top = monitor.top + scale(placement.y_dip, scale_factor);
    RECT {
        left,
        top,
        right: left + scale(width_dip, scale_factor),
        bottom: top + 1,
    }
}

fn preview_rows() -> Vec<eqcombat::DpsRowSnapshot> {
    [
        ("Aria", 1_284_900u128),
        ("Bront", 1_041_220),
        ("Caela", 884_510),
        ("Dorin", 726_440),
        ("Your box", 512_300),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, damage))| eqcombat::DpsRowSnapshot {
        rank: if index == 4 { 17 } else { index + 1 },
        participant: eqcombat::ParticipantId::new("preview", name),
        display_name: Arc::from(name),
        managed: index == 4,
        has_pet_damage: index == 2,
        provisional_pet: false,
        damage,
        active_seconds: 40,
        dps: damage / 40,
        sdps: damage / 42,
        contribution_millionths: ((damage * 1_000_000) / 4_449_370) as u32,
        source_quality: eqcombat::SourceQuality::CompleteObserver,
        elected_source: eqlog::LogSourceId::new("preview"),
    })
    .collect()
}

fn format_duration(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn same_rect(left: RECT, right: RECT) -> bool {
    left.left == right.left
        && left.top == right.top
        && left.right == right.right
        && left.bottom == right.bottom
}

fn monitor_width(rect: RECT) -> i32 {
    (rect.right - rect.left).max(0)
}

fn monitor_height(rect: RECT) -> i32 {
    (rect.bottom - rect.top).max(0)
}

fn clamp_rect(mut rect: RECT, work: RECT, minimum_width: i32) -> RECT {
    let work_width = monitor_width(work).max(1);
    let work_height = monitor_height(work).max(1);
    let width = monitor_width(rect).clamp(minimum_width.min(work_width), work_width);
    let height = monitor_height(rect).min(work_height).max(1);
    rect.left = rect.left.clamp(work.left, work.right - width);
    rect.top = rect.top.clamp(work.top, work.bottom - height);
    rect.right = rect.left + width;
    rect.bottom = rect.top + height;
    rect
}

unsafe fn point_from_lparam(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xffff) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    }
}

unsafe fn begin_drag(state: &mut OverlayState, hwnd: HWND, point: POINT) {
    if !state.interaction.edit_mode {
        return;
    }
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return;
    }
    let mut cursor = POINT::default();
    if GetCursorPos(&mut cursor).is_err() {
        return;
    }
    let resize_zone = scale(RESIZE_ZONE_DIP, state.layout.dpi_scale).max(4);
    let kind = if point.x >= monitor_width(rect) - resize_zone {
        DragKind::ResizeRight
    } else {
        DragKind::Move
    };
    state.dps.drag = Some(DragState {
        kind,
        start_cursor: cursor,
        start_rect: rect,
    });
    let _ = SetCapture(hwnd);
}

unsafe fn update_drag(state: &mut OverlayState, hwnd: HWND) {
    let Some(drag) = state.dps.drag else {
        return;
    };
    let mut cursor = POINT::default();
    if GetCursorPos(&mut cursor).is_err() {
        return;
    }
    let dx = cursor.x - drag.start_cursor.x;
    let dy = cursor.y - drag.start_cursor.y;
    let mut rect = drag.start_rect;
    match drag.kind {
        DragKind::Move => {
            rect.left += dx;
            rect.right += dx;
            rect.top += dy;
            rect.bottom += dy;
        }
        DragKind::ResizeRight => {
            rect.right += dx;
        }
    }
    rect = clamp_rect(
        rect,
        state.layout.monitor_rect,
        scale(MIN_DPS_OVERLAY_WIDTH_DIP as i32, state.layout.dpi_scale).max(1),
    );
    state.dps.runtime_rect = Some(rect);
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        rect.left,
        rect.top,
        monitor_width(rect),
        monitor_height(rect),
        SWP_NOACTIVATE,
    );
    state.dps.scene_key = None;
    reconcile(state);
}

pub(super) unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if is_busy()
        && !matches!(
            msg,
            WM_PAINT | WM_ERASEBKGND | WM_NCHITTEST | WM_MOUSEACTIVATE
        )
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => {
            let edit = try_with_state(|state| state.interaction.edit_mode).unwrap_or(false);
            if edit {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTTRANSPARENT as isize)
            }
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            validate_composition_paint(hwnd);
            let requested = !is_busy() && take_redraw_request(hwnd);
            if !is_busy() {
                let _ = try_with_state_mut(|state| unsafe {
                    if requested || !surface_is_ready(state, hwnd) {
                        render_dps_surface(state);
                    }
                });
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let point = point_from_lparam(lparam);
            let _ = try_with_state_mut(|state| unsafe { begin_drag(state, hwnd, point) });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let _ = try_with_state_mut(|state| unsafe { update_drag(state, hwnd) });
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            let _ = try_with_state_mut(|state| {
                state.dps.drag = None;
            });
            let _ = ReleaseCapture();
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let resize = try_with_state(|state| {
                let mut point = POINT::default();
                let _ = unsafe { GetCursorPos(&mut point) };
                let _ = unsafe { ScreenToClient(hwnd, &mut point) };
                let mut rect = RECT::default();
                let _ = unsafe { GetClientRect(hwnd, &mut rect) };
                state.interaction.edit_mode
                    && point.x >= rect.right - scale(RESIZE_ZONE_DIP, state.layout.dpi_scale)
            })
            .unwrap_or(false);
            SetCursor(
                LoadCursorW(None, if resize { IDC_SIZEWE } else { IDC_SIZEALL })
                    .unwrap_or_default(),
            );
            LRESULT(1)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_clamps_complete_panel_to_work_area() {
        let work = RECT {
            left: 100,
            top: 50,
            right: 740,
            bottom: 590,
        };
        let clamped = clamp_rect(
            RECT {
                left: -500,
                top: 900,
                right: 1_500,
                bottom: 1_500,
            },
            work,
            360,
        );
        assert_eq!(clamped.left, 100);
        assert_eq!(clamped.right, 740);
        assert_eq!(clamped.top, 50);
        assert_eq!(clamped.bottom, 590);
    }

    #[test]
    fn duration_is_compact_and_stable() {
        assert_eq!(format_duration(1), "0:01");
        assert_eq!(format_duration(125), "2:05");
    }
}
