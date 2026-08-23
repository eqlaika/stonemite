//! PiP notification state, animation geometry, Lucide icon drawing, and presentation.
//!
//! Static icon geometry is adapted from the Lucide and Lucide Animated frames
//! already used by the Stream Deck integration.

use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT};

use super::dpi;
use super::labels::{Color, LabelStyle, Rect};

pub(super) const TIMER_ID: usize = 43;
pub(super) const ANIMATION_STEP_MS: u32 = 16;
pub(super) const LABEL_MIN_ALPHA: u8 = 245;
pub(super) const RESURRECTION_COLOR: u32 = 0x0089DF80;
pub(super) const DEATH_COLOR: u32 = 0x006F82FF;

const PREVIEW_DURATION_MS: u64 = 6000;
const ANIMATION_LAP_MS: u64 = 900;
const ANIMATION_LAPS: u64 = 3;
const ANIMATION_DURATION_MS: u64 = ANIMATION_LAP_MS * ANIMATION_LAPS;
const PREVIEW_HEIGHT: i32 = 54;
const ACTION_PREVIEW_HEIGHT: i32 = 98;
const PREVIEW_MIN_WIDTH: i32 = 260;
const PREVIEW_PADDING: i32 = 10;
const PREVIEW_ICON_SIZE: i32 = 20;
const ACTION_BUTTON_HEIGHT: i32 = 30;
const ACTION_BUTTON_GAP: i32 = 8;
const ACCEPT_BUTTON_WIDTH: i32 = 84;
const DISMISS_BUTTON_WIDTH: i32 = 88;
const PREVIEW_MARGIN: i32 = 6;
const PREVIEW_SHADOW_X: i32 = 3;
const PREVIEW_SHADOW_Y: i32 = 4;
const PREVIEW_SHADOW_FAR: u32 = 0x00140F0B;
const PREVIEW_SHADOW_NEAR: u32 = 0x00241B13;
const PREVIEW_BACKGROUND: u32 = 0x00403020;
const PREVIEW_GAP: i32 = 4;
const UNREAD_DOT_DIAMETER: i32 = 9;
const UNREAD_DOT_RING: i32 = 2;
const UNREAD_DOT_GAP: i32 = 4;
const UNREAD_ROW_HEIGHT: i32 = UNREAD_DOT_DIAMETER + 2 * UNREAD_DOT_RING;
const MAX_TRACKED_UNREAD: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Tell,
    GroupInvite,
    RaidInvite,
    Resurrection,
    Death,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EnabledKinds {
    pub tells: bool,
    pub group_invites: bool,
    pub raid_invites: bool,
    pub resurrections: bool,
    pub deaths: bool,
}

impl EnabledKinds {
    pub(super) fn contains(self, kind: Kind) -> bool {
        match kind {
            Kind::Tell => self.tells,
            Kind::GroupInvite => self.group_invites,
            Kind::RaidInvite => self.raid_invites,
            Kind::Resurrection => self.resurrections,
            Kind::Death => self.deaths,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Unread {
    kind: Kind,
    color: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InviteAction {
    Accept,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Notification {
    kind: Kind,
    text: String,
    color: u32,
    received_at_ms: u64,
    unread: Vec<Unread>,
    invite_actions: bool,
    hovered_action: Option<InviteAction>,
    pressed_action: Option<InviteAction>,
    invite_preview_pressed: bool,
    animation_complete: bool,
    preview_complete: bool,
}

impl Notification {
    pub(super) fn push(
        previous: Option<Self>,
        kind: Kind,
        text: String,
        color: u32,
        received_at_ms: u64,
        invite_actions: bool,
    ) -> Self {
        let mut unread = previous
            .map(|notification| notification.unread)
            .unwrap_or_default();
        if unread.len() == MAX_TRACKED_UNREAD {
            unread.remove(0);
        }
        unread.push(Unread { kind, color });
        Self {
            kind,
            text,
            color,
            received_at_ms,
            unread,
            invite_actions,
            hovered_action: None,
            pressed_action: None,
            invite_preview_pressed: false,
            animation_complete: false,
            preview_complete: false,
        }
    }

    fn elapsed_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.received_at_ms)
    }

    pub(super) fn preview_visible(&self, now_ms: u64) -> bool {
        !self.preview_complete && self.elapsed_ms(now_ms) < PREVIEW_DURATION_MS
    }

    fn animation_frame(&self, now_ms: u64, animations_enabled: bool) -> Option<(u64, f64)> {
        let elapsed = self.elapsed_ms(now_ms);
        (animations_enabled && !self.animation_complete && elapsed < ANIMATION_DURATION_MS).then(
            || {
                (
                    elapsed / ANIMATION_LAP_MS,
                    (elapsed % ANIMATION_LAP_MS) as f64 / ANIMATION_LAP_MS as f64,
                )
            },
        )
    }

    pub(super) fn redraws_for_tick(
        &mut self,
        now_ms: u64,
        animations_enabled: bool,
    ) -> (bool, bool) {
        let border = if self.animation_frame(now_ms, animations_enabled).is_some() {
            true
        } else if !self.animation_complete {
            self.animation_complete = true;
            true
        } else {
            false
        };
        let preview = if !self.preview_visible(now_ms) && !self.preview_complete {
            self.preview_complete = true;
            true
        } else {
            false
        };
        (border, preview)
    }

    /// Remove only the current group invitation, preserving older unread
    /// events and their steady indicator colors.
    fn dismiss_invite(&mut self) -> bool {
        if self.kind != Kind::GroupInvite {
            return false;
        }
        self.unread.pop();
        self.finish_invite_preview()
    }

    /// Clear every unread group invitation after EQ confirms that its pending
    /// invitation was accepted or declined. Unrelated unread events remain.
    fn resolve_group_invites(&mut self) -> bool {
        let current_was_invite = self.kind == Kind::GroupInvite;
        self.unread
            .retain(|unread| unread.kind != Kind::GroupInvite);
        if current_was_invite {
            self.finish_invite_preview()
        } else {
            self.unread.is_empty()
        }
    }

    fn finish_invite_preview(&mut self) -> bool {
        self.invite_actions = false;
        self.hovered_action = None;
        self.pressed_action = None;
        self.invite_preview_pressed = false;
        self.animation_complete = true;
        self.preview_complete = true;
        if let Some(previous) = self.unread.last().copied() {
            self.kind = previous.kind;
            self.color = previous.color;
            false
        } else {
            true
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BorderLine {
    pub from: (i32, i32),
    pub to: (i32, i32),
}

fn perimeter_point(width: i32, height: i32, distance: f64) -> (i32, i32) {
    let width = width.max(1) as f64;
    let height = height.max(1) as f64;
    let perimeter = 2.0 * (width + height);
    let distance = distance.rem_euclid(perimeter);
    if distance <= width {
        (distance.round() as i32, 0)
    } else if distance <= width + height {
        (width as i32, (distance - width).round() as i32)
    } else if distance <= 2.0 * width + height {
        (
            (2.0 * width + height - distance).round() as i32,
            height as i32,
        )
    } else {
        (0, (perimeter - distance).round() as i32)
    }
}

fn perimeter_lines(width: i32, height: i32, start: f64, length: f64) -> Vec<BorderLine> {
    let width = width.max(1) as f64;
    let height = height.max(1) as f64;
    let perimeter = 2.0 * (width + height);
    let boundaries = [width, width + height, 2.0 * width + height, perimeter];
    let mut cursor = start.rem_euclid(perimeter);
    let mut remaining = length.clamp(0.0, perimeter);
    let mut lines = Vec::with_capacity(5);

    while remaining > 0.01 && lines.len() < 5 {
        let edge_end = boundaries
            .iter()
            .copied()
            .find(|boundary| *boundary > cursor + 0.01)
            .unwrap_or(perimeter);
        let step = remaining.min(edge_end - cursor);
        let from = perimeter_point(width as i32, height as i32, cursor);
        let to = if cursor + step >= perimeter - 0.01 {
            (0, 0)
        } else {
            perimeter_point(width as i32, height as i32, cursor + step)
        };
        if from != to {
            lines.push(BorderLine { from, to });
        }
        remaining -= step;
        cursor += step;
        if cursor >= perimeter - 0.01 {
            cursor = 0.0;
        }
    }
    lines
}

fn dim_colorref(color: u32) -> u32 {
    let scale = |component: u32| ((component as f64 * 0.38).round() as u32).min(255);
    scale(color & 0xff) | (scale((color >> 8) & 0xff) << 8) | (scale((color >> 16) & 0xff) << 16)
}

fn border_highlight(lap: u64, progress: f64, perimeter: f64, origin: f64) -> (f64, f64) {
    let segment_fraction = 0.22;
    if lap + 1 == ANIMATION_LAPS {
        (
            origin,
            perimeter * (segment_fraction + progress * (1.0 - segment_fraction)),
        )
    } else {
        (
            (origin + progress * perimeter).rem_euclid(perimeter),
            perimeter * segment_fraction,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NotificationVisualSnapshot {
    pub kind: Kind,
    pub text: String,
    pub color: Color,
    pub unread_colors: Vec<Color>,
    pub invite_actions: bool,
    pub hovered_action: Option<InviteAction>,
    pub pressed_action: Option<InviteAction>,
    pub animation: Option<(u64, f64)>,
    pub preview_visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NotificationBorderLayout {
    pub frames: Vec<Rect>,
    pub frame_color: Color,
    pub highlight_color: Color,
    pub highlight_lines: Vec<BorderLine>,
    pub stroke_width: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UnreadDotLayout {
    pub ring: Rect,
    pub dot: Rect,
    pub ring_color: Color,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InviteButtonVisual {
    pub action: InviteAction,
    pub bounds: Rect,
    pub fill_color: Color,
    pub border_color: Color,
    pub text_color: Color,
    pub hovered: bool,
    pub pressed: bool,
    pub radius: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NotificationPreviewLayout {
    pub far_shadow: Rect,
    pub near_shadow: Rect,
    pub surface: Rect,
    pub icon: Rect,
    pub text: Rect,
    pub far_shadow_color: Color,
    pub near_shadow_color: Color,
    pub surface_color: Color,
    pub radius: i32,
    pub buttons: Vec<InviteButtonVisual>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NotificationContentLayout {
    pub unread_dots: Vec<UnreadDotLayout>,
    pub unread_bottom: i32,
    pub preview: Option<NotificationPreviewLayout>,
}

impl NotificationContentLayout {
    /// Hit-test exactly the button/surface rectangles selected by layout. A
    /// constrained plain-preview fallback has no buttons and is noninteractive.
    fn invite_interaction_at(&self, point: (i32, i32)) -> Option<Option<InviteAction>> {
        let preview = self.preview.as_ref()?;
        if preview.buttons.is_empty() || !rect_contains(preview.surface, point) {
            return None;
        }
        Some(
            preview
                .buttons
                .iter()
                .find(|button| rect_contains(button.bounds, point))
                .map(|button| button.action),
        )
    }
}

fn rect_contains(rect: Rect, point: (i32, i32)) -> bool {
    point.0 >= rect.left && point.0 < rect.right && point.1 >= rect.top && point.1 < rect.bottom
}

impl Notification {
    pub(super) fn visual_snapshot(
        &self,
        now_ms: u64,
        animations_enabled: bool,
    ) -> NotificationVisualSnapshot {
        NotificationVisualSnapshot {
            kind: self.kind,
            text: self.text.clone(),
            color: Color::from_colorref(self.color),
            unread_colors: self
                .unread
                .iter()
                .map(|unread| Color::from_colorref(unread.color))
                .collect(),
            invite_actions: self.invite_actions,
            hovered_action: self.hovered_action,
            pressed_action: self.pressed_action,
            animation: self.animation_frame(now_ms, animations_enabled),
            preview_visible: self.preview_visible(now_ms),
        }
    }
}

impl NotificationVisualSnapshot {
    pub(super) fn border_layout(&self, rect: Rect, border: i32) -> NotificationBorderLayout {
        let frame_color = if self.animation.is_some() {
            Color::from_colorref(dim_colorref(colorref(self.color)))
        } else {
            self.color
        };
        let frames = (0..border.max(1)).map(|inset| rect.inset(inset)).collect();
        let stroke_width = border.max(2);
        let Some((lap, progress)) = self.animation else {
            return NotificationBorderLayout {
                frames,
                frame_color,
                highlight_color: self.color,
                highlight_lines: Vec::new(),
                stroke_width,
            };
        };
        let inset = (border / 2).max(1);
        let path_width = (rect.width() - 1 - 2 * inset).max(1);
        let path_height = (rect.height() - 1 - 2 * inset).max(1);
        let perimeter = 2.0 * (path_width + path_height) as f64;
        let (start, length) = border_highlight(lap, progress, perimeter, path_width as f64 / 2.0);
        let highlight_lines = perimeter_lines(path_width, path_height, start, length)
            .into_iter()
            .map(|line| BorderLine {
                from: (
                    rect.left + inset + line.from.0,
                    rect.top + inset + line.from.1,
                ),
                to: (rect.left + inset + line.to.0, rect.top + inset + line.to.1),
            })
            .collect();
        NotificationBorderLayout {
            frames,
            frame_color,
            highlight_color: self.color,
            highlight_lines,
            stroke_width,
        }
    }

    pub(super) fn content_layout(
        &self,
        client: Rect,
        label_bottom: i32,
        scale: f64,
        measured_text_width: i32,
    ) -> NotificationContentLayout {
        let dot_diameter = dpi(UNREAD_DOT_DIAMETER, scale).max(6);
        let ring = dpi(UNREAD_DOT_RING, scale).max(1);
        let gap = dpi(UNREAD_DOT_GAP, scale).max(1);
        let stride = dot_diameter + 2 * ring + gap;
        let row_top = label_bottom + dpi(PREVIEW_GAP, scale);
        let dot_y = row_top + ring;
        let max_dots = ((client.width().max(0) + gap) / stride).max(0) as usize;
        let first_visible = self.unread_colors.len().saturating_sub(max_dots);
        let unread_dots = self
            .unread_colors
            .iter()
            .skip(first_visible)
            .enumerate()
            .map(|(index, color)| {
                let dot_x = client.left + ring + index as i32 * stride;
                UnreadDotLayout {
                    ring: Rect::new(
                        dot_x - ring,
                        dot_y - ring,
                        dot_x + dot_diameter + ring,
                        dot_y + dot_diameter + ring,
                    )
                    .intersect(client),
                    dot: Rect::new(dot_x, dot_y, dot_x + dot_diameter, dot_y + dot_diameter)
                        .intersect(client),
                    ring_color: Color::from_colorref(PREVIEW_BACKGROUND),
                    color: *color,
                }
            })
            .collect();
        let unread_bottom = unread_row_bottom(label_bottom, scale);
        let preview = self
            .preview_visible
            .then(|| preview_layout(client, unread_bottom, scale, self, measured_text_width))
            .flatten();
        NotificationContentLayout {
            unread_dots,
            unread_bottom,
            preview,
        }
    }
}

fn colorref(color: Color) -> u32 {
    u32::from(color.red) | (u32::from(color.green) << 8) | (u32::from(color.blue) << 16)
}

fn rect_from_win32(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right, rect.bottom)
}

fn invite_button_rects_neutral(preview: Rect, scale: f64) -> Option<(Rect, Rect)> {
    let pad = dpi(PREVIEW_PADDING, scale);
    let gap = dpi(ACTION_BUTTON_GAP, scale);
    let height = dpi(ACTION_BUTTON_HEIGHT, scale);
    let accept_width = dpi(ACCEPT_BUTTON_WIDTH, scale);
    let dismiss_width = dpi(DISMISS_BUTTON_WIDTH, scale);
    let left = preview.left + pad + dpi(PREVIEW_ICON_SIZE + 9, scale);
    let top = preview.bottom - pad - height;
    if top <= preview.top + pad || left + accept_width + gap + dismiss_width + pad > preview.right {
        return None;
    }
    Some((
        Rect::new(left, top, left + accept_width, top + height),
        Rect::new(
            left + accept_width + gap,
            top,
            left + accept_width + gap + dismiss_width,
            top + height,
        ),
    ))
}

fn unread_row_bottom(label_bottom: i32, scale: f64) -> i32 {
    label_bottom + dpi(PREVIEW_GAP, scale) + dpi(UNREAD_ROW_HEIGHT, scale)
}

fn preview_available_bounds(
    client: Rect,
    minimum_top: i32,
    scale: f64,
    invite_actions: bool,
) -> Rect {
    let minimum_top = minimum_top + dpi(PREVIEW_GAP, scale);
    let margin = dpi(PREVIEW_MARGIN, scale);
    let shadow_x = dpi(PREVIEW_SHADOW_X, scale);
    let shadow_y = dpi(PREVIEW_SHADOW_Y, scale);
    let preview_bottom = client.bottom - margin - shadow_y;
    let bounds_for_height = |height| {
        Rect::new(
            client.left + margin,
            (preview_bottom - dpi(height, scale)).max(minimum_top),
            client.right - margin - shadow_x,
            preview_bottom,
        )
    };
    let action_preview = bounds_for_height(ACTION_PREVIEW_HEIGHT);
    if invite_actions
        && action_preview.height() >= dpi(84, scale)
        && invite_button_rects_neutral(action_preview, scale).is_some()
    {
        action_preview
    } else {
        bounds_for_height(PREVIEW_HEIGHT)
    }
}

fn button_visual(
    action: InviteAction,
    bounds: Rect,
    base_color: u32,
    text_color: u32,
    scale: f64,
    snapshot: &NotificationVisualSnapshot,
) -> InviteButtonVisual {
    let hovered = snapshot.hovered_action == Some(action);
    let pressed = snapshot.pressed_action == Some(action) && hovered;
    let fill = if pressed {
        adjust_color(base_color, -24)
    } else if hovered {
        adjust_color(base_color, 18)
    } else {
        base_color
    };
    InviteButtonVisual {
        action,
        bounds,
        fill_color: Color::from_colorref(fill),
        border_color: Color::from_colorref(adjust_color(fill, 28)),
        text_color: Color::from_colorref(text_color),
        hovered,
        pressed,
        radius: dpi(7, scale).max(3),
    }
}

fn preview_layout(
    client: Rect,
    minimum_top: i32,
    scale: f64,
    snapshot: &NotificationVisualSnapshot,
    measured_text_width: i32,
) -> Option<NotificationPreviewLayout> {
    let available = preview_available_bounds(client, minimum_top, scale, snapshot.invite_actions);
    if available.width() < dpi(120, scale) || available.height() < dpi(32, scale) {
        return None;
    }
    let available_buttons = snapshot
        .invite_actions
        .then(|| invite_button_rects_neutral(available, scale))
        .flatten();
    let pad = dpi(PREVIEW_PADDING, scale);
    let icon_size = dpi(PREVIEW_ICON_SIZE, scale).min(available.height() - 2 * pad);
    let gap = dpi(9, scale);
    let desired_width = 2 * pad + icon_size + gap + measured_text_width + dpi(12, scale);
    let preview_width =
        if available_buttons.is_some() || desired_width >= available.width() - dpi(24, scale) {
            available.width()
        } else {
            desired_width
                .max(dpi(PREVIEW_MIN_WIDTH, scale))
                .min(available.width())
        };
    let surface = Rect::new(
        available.left,
        available.top,
        available.left + preview_width,
        available.bottom,
    );
    let far_x = dpi(PREVIEW_SHADOW_X, scale);
    let far_y = dpi(PREVIEW_SHADOW_Y, scale);
    let content_bottom = available_buttons
        .map(|(accept, _)| accept.top - dpi(4, scale))
        .unwrap_or(surface.bottom);
    let icon_left = surface.left + pad;
    let icon_top = surface.top + (content_bottom - surface.top - icon_size) / 2;
    let buttons = available_buttons
        .map(|(accept, dismiss)| {
            vec![
                button_visual(
                    InviteAction::Accept,
                    accept,
                    colorref(snapshot.color),
                    contrasting_text_color(colorref(snapshot.color)),
                    scale,
                    snapshot,
                ),
                button_visual(
                    InviteAction::Dismiss,
                    dismiss,
                    0x00605040,
                    0x00FFFFFF,
                    scale,
                    snapshot,
                ),
            ]
        })
        .unwrap_or_default();
    Some(NotificationPreviewLayout {
        far_shadow: surface.offset(far_x, far_y),
        near_shadow: surface.offset(far_x / 2, far_y / 2),
        surface,
        icon: Rect::new(
            icon_left,
            icon_top,
            icon_left + icon_size,
            icon_top + icon_size,
        ),
        text: Rect::new(
            icon_left + icon_size + gap,
            surface.top,
            surface.right - pad,
            content_bottom,
        ),
        far_shadow_color: Color::from_colorref(PREVIEW_SHADOW_FAR),
        near_shadow_color: Color::from_colorref(PREVIEW_SHADOW_NEAR),
        surface_color: Color::from_colorref(PREVIEW_BACKGROUND),
        radius: dpi(10, scale).max(4),
        buttons,
    })
}

fn adjust_color(color: u32, amount: i32) -> u32 {
    let adjust = |component: u32| (component as i32 + amount).clamp(0, 255) as u32;
    adjust(color & 0xff) | (adjust((color >> 8) & 0xff) << 8) | (adjust((color >> 16) & 0xff) << 16)
}

fn contrasting_text_color(background: u32) -> u32 {
    let red = background & 0xff;
    let green = (background >> 8) & 0xff;
    let blue = (background >> 16) & 0xff;
    if 299 * red + 587 * green + 114 * blue >= 145_000 {
        0x00201810
    } else {
        0x00FFFFFF
    }
}

/// Return the action-preview hit for one PiP. `Some(None)` means the pointer is
/// inside the preview but not over a button; the whole surface consumes clicks
/// so notification interaction can never fall through to PiP activation.
unsafe fn invite_interaction_at_point(
    state: &super::OverlayState,
    pip_index: usize,
    point: POINT,
) -> Option<(u32, Option<InviteAction>)> {
    if state.edit_mode {
        return None;
    }
    let pip = state.pip_windows.get(pip_index)?;
    let notification = state.notifications.get(&pip.pid)?;
    if !notification.invite_actions || notification.preview_complete {
        return None;
    }

    let mut client = RECT::default();
    windows::Win32::UI::WindowsAndMessaging::GetClientRect(pip.hwnd, &mut client).ok()?;
    let now = Instant::now();
    let source_id = format!("pid:{}", pip.pid);
    let timer_progress = state
        .timers
        .visible_for(Some(&source_id), now)
        .map(|timer| timer.progress(now));
    let snapshot = notification.visual_snapshot(
        windows::Win32::System::SystemInformation::GetTickCount64(),
        state.animations_enabled,
    );
    let layout = super::scenes::notification_content_layout(
        &snapshot,
        rect_from_win32(client),
        dpi(super::BORDER_WIDTH, state.dpi_scale),
        LabelStyle::new(state.dpi_scale, state.label_height),
        timer_progress,
        0,
    );
    layout
        .invite_interaction_at((point.x, point.y))
        .map(|action| (pip.pid, action))
}

pub(super) unsafe fn has_invite_preview_at(
    state: &super::OverlayState,
    pip_index: usize,
    point: POINT,
) -> bool {
    invite_interaction_at_point(state, pip_index, point).is_some()
}

pub(super) unsafe fn has_invite_action_at(
    state: &super::OverlayState,
    pip_index: usize,
    point: POINT,
) -> bool {
    invite_interaction_at_point(state, pip_index, point)
        .and_then(|(_, action)| action)
        .is_some()
}

pub(super) unsafe fn update_invite_hover(
    state: &mut super::OverlayState,
    pip_index: usize,
    point: POINT,
) {
    let hit = invite_interaction_at_point(state, pip_index, point);
    let Some(pip) = state.pip_windows.get(pip_index) else {
        return;
    };
    let pid = pip.pid;
    let label_hwnd = pip.label_hwnd;
    let hovered = hit
        .filter(|(hit_pid, _)| *hit_pid == pid)
        .and_then(|(_, action)| action);
    if let Some(notification) = state.notifications.get_mut(&pid) {
        if notification.hovered_action != hovered {
            notification.hovered_action = hovered;
            super::request_redraw(label_hwnd);
        }
    }
}

pub(super) unsafe fn clear_invite_interaction(state: &mut super::OverlayState, pip_index: usize) {
    let Some(pip) = state.pip_windows.get(pip_index) else {
        return;
    };
    let pid = pip.pid;
    let label_hwnd = pip.label_hwnd;
    if let Some(notification) = state.notifications.get_mut(&pid) {
        if notification.hovered_action.take().is_some()
            || notification.pressed_action.take().is_some()
            || std::mem::take(&mut notification.invite_preview_pressed)
        {
            super::request_redraw(label_hwnd);
        }
    }
}

pub(super) unsafe fn press_invite_action(
    state: &mut super::OverlayState,
    pip_index: usize,
    point: POINT,
) -> bool {
    let Some((pid, action)) = invite_interaction_at_point(state, pip_index, point) else {
        return false;
    };
    let Some(pip) = state.pip_windows.get(pip_index) else {
        return false;
    };
    let label_hwnd = pip.label_hwnd;
    if let Some(notification) = state.notifications.get_mut(&pid) {
        notification.invite_preview_pressed = true;
        notification.hovered_action = action;
        notification.pressed_action = action;
        super::request_redraw(label_hwnd);
        true
    } else {
        false
    }
}

pub(super) fn invite_action_pressed(state: &super::OverlayState, pip_index: usize) -> bool {
    state
        .pip_windows
        .get(pip_index)
        .and_then(|pip| state.notifications.get(&pip.pid))
        .is_some_and(|notification| notification.invite_preview_pressed)
}

pub(super) unsafe fn release_invite_action(
    state: &mut super::OverlayState,
    pip_index: usize,
    point: POINT,
) -> Option<(u32, InviteAction)> {
    let hit = invite_interaction_at_point(state, pip_index, point);
    let pip = state.pip_windows.get(pip_index)?;
    let pid = pip.pid;
    let label_hwnd = pip.label_hwnd;
    let notification = state.notifications.get_mut(&pid)?;
    if !std::mem::take(&mut notification.invite_preview_pressed) {
        return None;
    }
    let pressed = notification.pressed_action.take();
    let hovered = hit
        .filter(|(hit_pid, _)| *hit_pid == pid)
        .and_then(|(_, action)| action);
    notification.hovered_action = hovered;
    super::request_redraw(label_hwnd);
    hovered
        .filter(|action| Some(*action) == pressed)
        .map(|action| (pid, action))
}

unsafe fn remove_group_invites(
    state: &mut super::OverlayState,
    pid: u32,
    remove_from: fn(&mut Notification) -> bool,
) {
    let remove = state.notifications.get_mut(&pid).is_some_and(remove_from);
    if remove {
        state.notifications.remove(&pid);
    }
    if let Some(pip) = state.pip_windows.iter().find(|pip| pip.pid == pid) {
        super::request_redraw(pip.label_hwnd);
    }
}

unsafe fn dismiss_invite(state: &mut super::OverlayState, pid: u32) {
    remove_group_invites(state, pid, Notification::dismiss_invite);
}

fn resolve_group_invites(state: &mut super::OverlayState, source: &crate::log_watcher::LogSource) {
    let Some(pid) = pid_for_log_source(&state.eq_windows, source) else {
        return;
    };
    unsafe {
        remove_group_invites(state, pid, Notification::resolve_group_invites);
    }
}

pub(super) unsafe fn execute_invite_action(
    state: &mut super::OverlayState,
    pid: u32,
    action: InviteAction,
) {
    match action {
        InviteAction::Accept => match crate::control::send_invite_follow(pid) {
            Ok(()) => dismiss_invite(state, pid),
            Err(error) => {
                if let Some(notification) = state.notifications.get_mut(&pid) {
                    notification.invite_actions = false;
                    notification.hovered_action = None;
                    notification.pressed_action = None;
                    notification.invite_preview_pressed = false;
                }
                if let Some(pip) = state.pip_windows.iter().find(|pip| pip.pid == pid) {
                    super::request_redraw(pip.label_hwnd);
                }
                super::show_toast_inner(
                    state,
                    &format!("Could not accept the group invitation: {}", error.message),
                );
            }
        },
        InviteAction::Dismiss => dismiss_invite(state, pid),
    }
}

fn pid_for_log_source(
    windows: &[crate::eq_windows::EqWindow],
    source: &crate::log_watcher::LogSource,
) -> Option<u32> {
    if let Some(pid) = source.id.as_str().strip_prefix("pid:") {
        return pid
            .parse()
            .ok()
            .filter(|pid| windows.iter().any(|window| window.pid == *pid));
    }

    let mut matches = windows.iter().filter(|window| {
        window
            .character
            .as_deref()
            .is_some_and(|character| character.eq_ignore_ascii_case(&source.character))
            && window
                .server
                .as_deref()
                .is_some_and(|server| server.eq_ignore_ascii_case(&source.server))
    });
    let pid = matches.next()?.pid;
    matches.next().is_none().then_some(pid)
}

fn resolve_eq_color(
    state: &mut super::OverlayState,
    source: &crate::log_watcher::LogSource,
    id: crate::eq_chat_colors::ChatColorId,
    fallback: crate::eq_chat_colors::RgbColor,
) -> u32 {
    match state
        .chat_colors
        .resolve(id, &source.character, &source.server)
    {
        Ok(Some(color)) => color.colorref(),
        Ok(None) => fallback.colorref(),
        Err(error) => {
            super::debug_log(&format!("eq_chat_colors: {error}"));
            fallback.colorref()
        }
    }
}

pub(super) fn apply_log_event(
    state: &mut super::OverlayState,
    event: &crate::log_watcher::ParsedLogEvent,
) {
    if matches!(
        &event.event,
        crate::log_watcher::LogEvent::Notification(
            crate::log_watcher::NotificationEvent::GroupInviteAccepted
                | crate::log_watcher::NotificationEvent::GroupInviteDeclined { .. }
        )
    ) {
        resolve_group_invites(state, &event.source);
        return;
    }

    let (kind, text, color) = match &event.event {
        crate::log_watcher::LogEvent::Chat(crate::log_watcher::ChatEvent::IncomingTell(tell)) => (
            Kind::Tell,
            format!("{}: {}", tell.sender, tell.message),
            resolve_eq_color(
                state,
                &event.source,
                crate::eq_chat_colors::TELL_COLOR_ID,
                crate::eq_chat_colors::DEFAULT_TELL_COLOR,
            ),
        ),
        crate::log_watcher::LogEvent::Notification(
            crate::log_watcher::NotificationEvent::GroupInvite { inviter },
        ) => (
            Kind::GroupInvite,
            format!("{inviter} invited you to a group"),
            resolve_eq_color(
                state,
                &event.source,
                crate::eq_chat_colors::GROUP_COLOR_ID,
                crate::eq_chat_colors::DEFAULT_GROUP_COLOR,
            ),
        ),
        crate::log_watcher::LogEvent::Notification(
            crate::log_watcher::NotificationEvent::RaidInvite { inviter },
        ) => (
            Kind::RaidInvite,
            format!("{inviter} invited you to a raid"),
            resolve_eq_color(
                state,
                &event.source,
                crate::eq_chat_colors::RAID_COLOR_ID,
                crate::eq_chat_colors::DEFAULT_RAID_COLOR,
            ),
        ),
        crate::log_watcher::LogEvent::Notification(
            crate::log_watcher::NotificationEvent::ResurrectionOffered,
        ) => (
            Kind::Resurrection,
            "Resurrection offered".to_owned(),
            RESURRECTION_COLOR,
        ),
        crate::log_watcher::LogEvent::Notification(
            crate::log_watcher::NotificationEvent::CharacterSlain { killer },
        ) => (Kind::Death, format!("Slain by {killer}"), DEATH_COLOR),
        _ => return,
    };
    apply(state, &event.source, kind, text, color);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisualDelivery {
    Show,
    SuppressFocused,
    SuppressDisabled,
}

fn visual_delivery(
    target_pid: u32,
    focused_eq_pid: Option<u32>,
    visuals_enabled: bool,
) -> VisualDelivery {
    if focused_eq_pid == Some(target_pid) {
        VisualDelivery::SuppressFocused
    } else if visuals_enabled {
        VisualDelivery::Show
    } else {
        VisualDelivery::SuppressDisabled
    }
}

fn apply(
    state: &mut super::OverlayState,
    source: &crate::log_watcher::LogSource,
    kind: Kind,
    text: String,
    color: u32,
) {
    if !state.notification_kinds.contains(kind) {
        return;
    }
    let Some(pid) = pid_for_log_source(&state.eq_windows, source) else {
        super::debug_log(&format!(
            "eq_logs: notification source {} is no longer attached to an EQ window",
            source.id.as_str()
        ));
        return;
    };

    if state.tell_sound_enabled {
        let _ = crate::sound::play(&state.tell_sound);
    }

    let focused_eq_pid = unsafe {
        super::focused_foreground_pid(
            &state.eq_windows,
            windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow(),
            |hwnd| super::target_has_keyboard_focus(hwnd),
        )
    };
    match visual_delivery(pid, focused_eq_pid, state.tell_visual_enabled) {
        VisualDelivery::SuppressFocused => {
            state.notifications.remove(&pid);
            return;
        }
        VisualDelivery::SuppressDisabled => return,
        VisualDelivery::Show => {}
    }

    let invite_actions = kind == Kind::GroupInvite && crate::control::invite_follow_available(pid);
    let now_ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    let previous = state.notifications.remove(&pid);
    state.notifications.insert(
        pid,
        Notification::push(previous, kind, text, color, now_ms, invite_actions),
    );

    unsafe {
        if let Some(pip) = state.pip_windows.iter().find(|pip| pip.pid == pid) {
            super::request_redraw(pip.label_hwnd);
        }
        let _ = windows::Win32::UI::WindowsAndMessaging::SetTimer(
            state.active_label_hwnd,
            TIMER_ID,
            ANIMATION_STEP_MS,
            None,
        );
    }
}

pub(super) unsafe fn tick(state: &mut super::OverlayState, timer_hwnd: HWND) {
    let now_ms = windows::Win32::System::SystemInformation::GetTickCount64();
    let animations_enabled = state.animations_enabled;
    for pip in &state.pip_windows {
        if let Some(notification) = state.notifications.get_mut(&pip.pid) {
            let (redraw_border, redraw_preview) =
                notification.redraws_for_tick(now_ms, animations_enabled);
            if redraw_border || redraw_preview {
                super::request_redraw(pip.label_hwnd);
            }
        }
    }
    if !state
        .notifications
        .values()
        .any(|notification| notification.preview_visible(now_ms))
    {
        let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(timer_hwnd, TIMER_ID);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tell(received_at_ms: u64) -> Notification {
        Notification::push(
            None,
            Kind::Tell,
            "Laika: hello".to_owned(),
            0x00BE28BE,
            received_at_ms,
            false,
        )
    }

    #[test]
    fn unread_notifications_accumulate_colors_and_latest_content() {
        let first = tell(1_000);
        let second = Notification::push(
            Some(first),
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        assert_eq!(second.unread.len(), 2);
        assert_eq!(second.text, "Honka invited you to a group");
        assert_eq!(
            second
                .unread
                .iter()
                .map(|unread| unread.color)
                .collect::<Vec<_>>(),
            vec![0x00BE28BE, 0x0060B06A]
        );
    }

    #[test]
    fn unread_history_is_bounded_and_keeps_the_newest_events() {
        let mut notification = None;
        for index in 0..300u32 {
            notification = Some(Notification::push(
                notification,
                Kind::Tell,
                format!("message {index}"),
                index,
                index as u64,
                false,
            ));
        }
        let notification = notification.unwrap();
        assert_eq!(notification.unread.len(), MAX_TRACKED_UNREAD);
        assert_eq!(
            notification.unread.first().map(|unread| unread.color),
            Some(44)
        );
        assert_eq!(
            notification.unread.last().map(|unread| unread.color),
            Some(299)
        );
    }

    #[test]
    fn dismissing_an_invite_preserves_older_unread_events() {
        let first = tell(1_000);
        let mut invite = Notification::push(
            Some(first),
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        assert!(!invite.dismiss_invite());
        assert_eq!(invite.unread.len(), 1);
        assert_eq!(invite.kind, Kind::Tell);
        assert_eq!(invite.color, 0x00BE28BE);
        assert!(!invite.preview_visible(2_001));
        assert!(!invite.invite_actions);

        let mut lone_invite = Notification::push(
            None,
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        assert!(lone_invite.dismiss_invite());
    }

    #[test]
    fn log_resolution_clears_invites_but_preserves_other_unread_events() {
        let tell_before = tell(1_000);
        let invite = Notification::push(
            Some(tell_before),
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        let mut tell_after = Notification::push(
            Some(invite),
            Kind::Tell,
            "Laika: still here".to_owned(),
            0x00BE28BE,
            3_000,
            false,
        );
        assert!(!tell_after.resolve_group_invites());
        assert_eq!(
            tell_after
                .unread
                .iter()
                .map(|unread| unread.kind)
                .collect::<Vec<_>>(),
            vec![Kind::Tell, Kind::Tell]
        );
        assert_eq!(tell_after.kind, Kind::Tell);
        assert!(tell_after.preview_visible(3_001));

        let mut lone_invite = Notification::push(
            None,
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        assert!(lone_invite.resolve_group_invites());
    }

    #[test]
    fn invite_button_hit_targets_match_the_rendered_action_row() {
        let notification = Notification::push(
            None,
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        let snapshot = notification.visual_snapshot(2_001, false);
        let layout = snapshot.content_layout(Rect::new(0, 0, 420, 240), 50, 1.0, 180);
        let preview = layout.preview.as_ref().expect("action preview");
        assert_eq!(preview.buttons.len(), 2);
        for button in &preview.buttons {
            let center = (
                (button.bounds.left + button.bounds.right) / 2,
                (button.bounds.top + button.bounds.bottom) / 2,
            );
            assert_eq!(
                layout.invite_interaction_at(center),
                Some(Some(button.action))
            );
        }
        assert_eq!(
            layout.invite_interaction_at((preview.surface.left + 5, preview.surface.top + 5)),
            Some(None),
            "the whole rendered action preview consumes clicks"
        );
    }

    #[test]
    fn notification_source_pid_fails_closed_and_identity_fallback_requires_uniqueness() {
        let window = |pid| crate::eq_windows::EqWindow {
            hwnd: HWND::default(),
            pid,
            number: pid as usize,
            character: Some("Bilka".to_owned()),
            server: Some("xegony".to_owned()),
            class: None,
        };
        let windows = vec![window(7)];
        let stale_exact = crate::log_watcher::LogSource::new("pid:42", "Bilka", "xegony");
        assert_eq!(pid_for_log_source(&windows, &stale_exact), None);

        let legacy = crate::log_watcher::LogSource::new("legacy", "Bilka", "xegony");
        assert_eq!(pid_for_log_source(&windows, &legacy), Some(7));
        assert_eq!(pid_for_log_source(&[window(7), window(8)], &legacy), None);
    }

    #[test]
    fn visual_delivery_uses_real_focused_eq_state_not_the_stored_partition() {
        assert_eq!(
            visual_delivery(42, Some(42), true),
            VisualDelivery::SuppressFocused
        );
        assert_eq!(visual_delivery(42, None, true), VisualDelivery::Show);
        assert_eq!(visual_delivery(42, Some(99), true), VisualDelivery::Show);
        assert_eq!(
            visual_delivery(42, None, false),
            VisualDelivery::SuppressDisabled
        );
    }

    #[test]
    fn preview_and_animation_are_bounded_and_reduce_motion_safe() {
        let mut notification = tell(1_000);
        assert!(notification.preview_visible(1_000));
        assert!(notification.preview_visible(1_000 + PREVIEW_DURATION_MS - 1));
        assert!(!notification.preview_visible(1_000 + PREVIEW_DURATION_MS));
        assert_eq!(notification.animation_frame(1_450, true), Some((0, 0.5)));
        assert_eq!(notification.animation_frame(1_450, false), None);
        assert_eq!(
            notification.animation_frame(1_000 + 2 * ANIMATION_LAP_MS + 450, true),
            Some((2, 0.5))
        );
        assert_eq!(notification.redraws_for_tick(1_450, true), (true, false));
        assert_eq!(
            notification.redraws_for_tick(1_000 + ANIMATION_DURATION_MS, true),
            (true, false)
        );
        assert_eq!(
            notification.redraws_for_tick(1_000 + ANIMATION_DURATION_MS + 1, true),
            (false, false)
        );
        assert_eq!(
            notification.redraws_for_tick(1_000 + PREVIEW_DURATION_MS, true),
            (false, true)
        );
        assert_eq!(
            notification.redraws_for_tick(1_000 + PREVIEW_DURATION_MS + 1, true),
            (false, false)
        );
    }

    #[test]
    fn short_pip_timer_invite_plain_fallback_has_no_hit_target() {
        let notification = Notification::push(
            None,
            Kind::GroupInvite,
            "Honka invited you to a group".to_owned(),
            0x0060B06A,
            2_000,
            true,
        );
        let snapshot = notification.visual_snapshot(2_001, false);
        let layout = super::super::scenes::notification_content_layout(
            &snapshot,
            Rect::new(0, 0, 420, 180),
            3,
            LabelStyle::new(1.0, 48),
            Some(0.5),
            180,
        );
        let preview = layout.preview.as_ref().expect("plain preview fallback");
        assert!(preview.buttons.is_empty());
        assert_eq!(
            layout.invite_interaction_at((preview.surface.left + 10, preview.surface.bottom - 10)),
            None,
            "invisible action buttons must never consume or activate clicks"
        );
    }

    #[test]
    fn notification_dots_and_preview_scale_at_supported_dpi_values() {
        let notification = tell(1_000);
        let snapshot = notification.visual_snapshot(1_001, false);
        for (scale, expected_dot) in [(1.0_f64, 9), (1.25_f64, 11), (1.5_f64, 14)] {
            let layout = snapshot.content_layout(Rect::new(0, 0, 480, 300), 72, scale, 120);
            assert_eq!(layout.unread_dots.len(), 1);
            assert_eq!(layout.unread_dots[0].dot.width(), expected_dot);
            let preview = layout.preview.expect("preview at supported DPI");
            assert_eq!(preview.radius, dpi(10, scale).max(4));
            assert_eq!(preview.icon.width(), dpi(PREVIEW_ICON_SIZE, scale));
        }
    }

    #[test]
    fn notification_layout_clips_preview_on_tiny_surfaces() {
        let notification = tell(1_000);
        let snapshot = notification.visual_snapshot(1_001, false);
        let layout = snapshot.content_layout(Rect::new(0, 0, 80, 40), 35, 1.5, 500);
        assert!(layout.preview.is_none());
        assert!(layout
            .unread_dots
            .iter()
            .all(|dot| dot.dot == dot.dot.intersect(Rect::new(0, 0, 80, 40))));
    }

    #[test]
    fn final_border_lap_accumulates_from_top_middle() {
        let (moving_start, moving_length) = border_highlight(0, 0.5, 100.0, 25.0);
        assert!((moving_start - 75.0).abs() < f64::EPSILON);
        assert!((moving_length - 22.0).abs() < f64::EPSILON);
        let (fill_start, fill_length) = border_highlight(2, 0.5, 100.0, 25.0);
        assert!((fill_start - 25.0).abs() < f64::EPSILON);
        assert!((fill_length - 61.0).abs() < f64::EPSILON);
        let (_, complete_length) = border_highlight(2, 1.0, 100.0, 25.0);
        assert!((complete_length - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn border_segment_wraps_cleanly_around_the_perimeter() {
        let lines = perimeter_lines(100, 50, 295.0, 20.0);
        assert_eq!(
            lines,
            vec![
                BorderLine {
                    from: (0, 5),
                    to: (0, 0),
                },
                BorderLine {
                    from: (0, 0),
                    to: (15, 0),
                },
            ]
        );
    }
}
