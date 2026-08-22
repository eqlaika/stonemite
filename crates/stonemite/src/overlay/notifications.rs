//! PiP notification state, animation geometry, Lucide icon drawing, and presentation.
//!
//! Static icon geometry is adapted from the Lucide and Lucide Animated frames
//! already used by the Stream Deck integration.

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DrawTextW, Ellipse, FrameRect, GetStockObject,
    GetTextExtentPoint32W, LineTo, MoveToEx, PolyBezierTo, RoundRect, SelectObject, SetTextColor,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FW_BOLD, HDC,
    NULL_BRUSH, PS_NULL, PS_SOLID,
};

use super::dpi;

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
struct BorderLine {
    from: (i32, i32),
    to: (i32, i32),
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

unsafe fn draw_frame(hdc: HDC, rect: RECT, width: i32, color: u32) {
    let brush = CreateSolidBrush(COLORREF(color));
    for inset in 0..width.max(1) {
        let frame = RECT {
            left: rect.left + inset,
            top: rect.top + inset,
            right: rect.right - inset,
            bottom: rect.bottom - inset,
        };
        let _ = FrameRect(hdc, &frame, brush);
    }
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush);
}

pub(super) unsafe fn draw_border(
    hdc: HDC,
    rect: RECT,
    border: i32,
    notification: &Notification,
    now_ms: u64,
    animations_enabled: bool,
) {
    let animation = notification.animation_frame(now_ms, animations_enabled);
    draw_frame(
        hdc,
        rect,
        border,
        if animation.is_some() {
            dim_colorref(notification.color)
        } else {
            notification.color
        },
    );

    let Some((lap, progress)) = animation else {
        return;
    };
    let inset = (border / 2).max(1);
    let path_width = (rect.right - rect.left - 1 - 2 * inset).max(1);
    let path_height = (rect.bottom - rect.top - 1 - 2 * inset).max(1);
    let perimeter = 2.0 * (path_width + path_height) as f64;
    let top_middle = path_width as f64 / 2.0;
    let (segment_start, segment_length) = border_highlight(lap, progress, perimeter, top_middle);
    let lines = perimeter_lines(path_width, path_height, segment_start, segment_length);
    let pen = CreatePen(PS_SOLID, border.max(2), COLORREF(notification.color));
    let old_pen = SelectObject(hdc, pen);
    for line in lines {
        let from = POINT {
            x: rect.left + inset + line.from.0,
            y: rect.top + inset + line.from.1,
        };
        let _ = MoveToEx(hdc, from.x, from.y, None);
        let _ = LineTo(
            hdc,
            rect.left + inset + line.to.0,
            rect.top + inset + line.to.1,
        );
    }
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(pen);
}

fn icon_point(rect: RECT, x: f64, y: f64) -> POINT {
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    POINT {
        x: rect.left + (x * width as f64 / 24.0).round() as i32,
        y: rect.top + (y * height as f64 / 24.0).round() as i32,
    }
}

unsafe fn select_icon_pen(
    hdc: HDC,
    rect: RECT,
    color: u32,
) -> (
    windows::Win32::Graphics::Gdi::HPEN,
    windows::Win32::Graphics::Gdi::HGDIOBJ,
    windows::Win32::Graphics::Gdi::HGDIOBJ,
) {
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    let pen = CreatePen(PS_SOLID, (width.min(height) / 12).max(2), COLORREF(color));
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    (pen, old_pen, old_brush)
}

unsafe fn restore_icon_pen(
    hdc: HDC,
    pen: windows::Win32::Graphics::Gdi::HPEN,
    old_pen: windows::Win32::Graphics::Gdi::HGDIOBJ,
    old_brush: windows::Win32::Graphics::Gdi::HGDIOBJ,
) {
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(pen);
}

unsafe fn draw_message_circle(hdc: HDC, rect: RECT, color: u32) {
    let (pen, old_pen, old_brush) = select_icon_pen(hdc, rect, color);
    let start = icon_point(rect, 7.9, 20.0);
    let _ = MoveToEx(hdc, start.x, start.y, None);
    let _ = PolyBezierTo(
        hdc,
        &[
            icon_point(rect, 12.5, 20.9),
            icon_point(rect, 17.6, 18.8),
            icon_point(rect, 20.1, 14.8),
            icon_point(rect, 22.8, 10.0),
            icon_point(rect, 20.0, 4.1),
            icon_point(rect, 15.1, 2.4),
            icon_point(rect, 10.2, 0.7),
            icon_point(rect, 5.0, 3.5),
            icon_point(rect, 3.3, 8.4),
            icon_point(rect, 2.4, 11.1),
            icon_point(rect, 2.7, 14.0),
            icon_point(rect, 4.0, 16.1),
        ],
    );
    let tail = icon_point(rect, 2.0, 22.0);
    let _ = LineTo(hdc, tail.x, tail.y);
    let _ = LineTo(hdc, start.x, start.y);
    restore_icon_pen(hdc, pen, old_pen, old_brush);
}

unsafe fn draw_user_plus(hdc: HDC, rect: RECT, color: u32, raid: bool) {
    let (pen, old_pen, old_brush) = select_icon_pen(hdc, rect, color);
    let head = RECT {
        left: icon_point(rect, 5.0, 3.0).x,
        top: icon_point(rect, 5.0, 3.0).y,
        right: icon_point(rect, 15.0, 13.0).x,
        bottom: icon_point(rect, 15.0, 13.0).y,
    };
    let _ = Ellipse(hdc, head.left, head.top, head.right, head.bottom);
    let start = icon_point(rect, 2.0, 21.0);
    let _ = MoveToEx(hdc, start.x, start.y, None);
    let _ = PolyBezierTo(
        hdc,
        &[
            icon_point(rect, 2.0, 16.6),
            icon_point(rect, 5.6, 13.0),
            icon_point(rect, 10.0, 13.0),
            icon_point(rect, 12.2, 13.0),
            icon_point(rect, 14.0, 13.7),
            icon_point(rect, 15.3, 15.0),
        ],
    );
    if raid {
        let side_start = icon_point(rect, 17.6, 3.7);
        let _ = MoveToEx(hdc, side_start.x, side_start.y, None);
        let _ = PolyBezierTo(
            hdc,
            &[
                icon_point(rect, 21.0, 5.0),
                icon_point(rect, 21.0, 10.0),
                icon_point(rect, 18.0, 12.0),
                icon_point(rect, 20.4, 13.8),
                icon_point(rect, 22.0, 16.8),
                icon_point(rect, 22.0, 20.0),
            ],
        );
    } else {
        let vertical_top = icon_point(rect, 19.0, 16.0);
        let vertical_bottom = icon_point(rect, 19.0, 22.0);
        let horizontal_left = icon_point(rect, 16.0, 19.0);
        let horizontal_right = icon_point(rect, 22.0, 19.0);
        let _ = MoveToEx(hdc, vertical_top.x, vertical_top.y, None);
        let _ = LineTo(hdc, vertical_bottom.x, vertical_bottom.y);
        let _ = MoveToEx(hdc, horizontal_left.x, horizontal_left.y, None);
        let _ = LineTo(hdc, horizontal_right.x, horizontal_right.y);
    }
    restore_icon_pen(hdc, pen, old_pen, old_brush);
}

unsafe fn draw_heart_pulse(hdc: HDC, rect: RECT, color: u32) {
    let (pen, old_pen, old_brush) = select_icon_pen(hdc, rect, color);
    let start = icon_point(rect, 12.0, 21.0);
    let _ = MoveToEx(hdc, start.x, start.y, None);
    let _ = PolyBezierTo(
        hdc,
        &[
            icon_point(rect, 10.0, 19.2),
            icon_point(rect, 3.0, 15.0),
            icon_point(rect, 2.0, 10.0),
            icon_point(rect, 1.4, 6.0),
            icon_point(rect, 4.0, 3.0),
            icon_point(rect, 7.5, 3.0),
            icon_point(rect, 9.8, 3.0),
            icon_point(rect, 11.2, 4.4),
            icon_point(rect, 12.0, 5.6),
            icon_point(rect, 12.8, 4.4),
            icon_point(rect, 14.2, 3.0),
            icon_point(rect, 16.5, 3.0),
            icon_point(rect, 20.0, 3.0),
            icon_point(rect, 22.6, 6.0),
            icon_point(rect, 22.0, 10.0),
            icon_point(rect, 21.2, 14.0),
            icon_point(rect, 14.0, 19.2),
            start,
        ],
    );
    let pulse = [
        (3.2, 13.0),
        (9.5, 13.0),
        (10.0, 12.0),
        (12.0, 16.5),
        (14.0, 9.5),
        (15.5, 13.0),
        (20.8, 13.0),
    ];
    let first = icon_point(rect, pulse[0].0, pulse[0].1);
    let _ = MoveToEx(hdc, first.x, first.y, None);
    for &(x, y) in &pulse[1..] {
        let point = icon_point(rect, x, y);
        let _ = LineTo(hdc, point.x, point.y);
    }
    restore_icon_pen(hdc, pen, old_pen, old_brush);
}

unsafe fn draw_bone(hdc: HDC, rect: RECT, color: u32) {
    let (pen, old_pen, old_brush) = select_icon_pen(hdc, rect, color);
    let start = icon_point(rect, 7.0, 17.0);
    let end = icon_point(rect, 17.0, 7.0);
    let _ = MoveToEx(hdc, start.x, start.y, None);
    let _ = LineTo(hdc, end.x, end.y);
    for &(cx, cy) in &[(5.0, 19.0), (19.0, 5.0)] {
        let left = icon_point(rect, cx - 2.5, cy - 2.5);
        let right = icon_point(rect, cx + 2.5, cy + 2.5);
        let _ = Ellipse(hdc, left.x, left.y, right.x, right.y);
    }
    restore_icon_pen(hdc, pen, old_pen, old_brush);
}

unsafe fn draw_icon(hdc: HDC, rect: RECT, color: u32, kind: Kind) {
    match kind {
        Kind::Tell => draw_message_circle(hdc, rect, color),
        Kind::GroupInvite => draw_user_plus(hdc, rect, color, false),
        Kind::RaidInvite => draw_user_plus(hdc, rect, color, true),
        Kind::Resurrection => draw_heart_pulse(hdc, rect, color),
        Kind::Death => draw_bone(hdc, rect, color),
    }
}

pub(super) unsafe fn draw_unread_dots(
    hdc: HDC,
    client: RECT,
    label_bottom: i32,
    scale: f64,
    notification: &Notification,
) -> i32 {
    let dot_diameter = dpi(UNREAD_DOT_DIAMETER, scale).max(6);
    let ring = dpi(UNREAD_DOT_RING, scale).max(1);
    let gap = dpi(UNREAD_DOT_GAP, scale).max(1);
    let stride = dot_diameter + 2 * ring + gap;
    let row_top = label_bottom + dpi(PREVIEW_GAP, scale);
    let dot_y = row_top + ring;
    let available_width = (client.right - client.left).max(0);
    let max_dots = ((available_width + gap) / stride).max(0) as usize;
    let ring_brush = CreateSolidBrush(COLORREF(PREVIEW_BACKGROUND));
    let dot_pen = CreatePen(PS_NULL, 0, COLORREF(0));
    let old_dot_pen = SelectObject(hdc, dot_pen);
    let old_dot_brush = SelectObject(hdc, ring_brush);
    let first_visible = notification.unread.len().saturating_sub(max_dots);
    for (index, unread) in notification.unread.iter().skip(first_visible).enumerate() {
        let dot_x = client.left + ring + index as i32 * stride;
        let _ = Ellipse(
            hdc,
            dot_x - ring,
            dot_y - ring,
            dot_x + dot_diameter + ring,
            dot_y + dot_diameter + ring,
        );
        let color_brush = CreateSolidBrush(COLORREF(unread.color));
        let _ = SelectObject(hdc, color_brush);
        let _ = Ellipse(
            hdc,
            dot_x,
            dot_y,
            dot_x + dot_diameter,
            dot_y + dot_diameter,
        );
        let _ = SelectObject(hdc, ring_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(color_brush);
    }
    let _ = SelectObject(hdc, old_dot_brush);
    let _ = SelectObject(hdc, old_dot_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(dot_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(ring_brush);
    row_top + dpi(UNREAD_ROW_HEIGHT, scale)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InviteButtonRects {
    accept: RECT,
    dismiss: RECT,
}

fn invite_button_rects(preview: RECT, scale: f64) -> Option<InviteButtonRects> {
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
    Some(InviteButtonRects {
        accept: RECT {
            left,
            top,
            right: left + accept_width,
            bottom: top + height,
        },
        dismiss: RECT {
            left: left + accept_width + gap,
            top,
            right: left + accept_width + gap + dismiss_width,
            bottom: top + height,
        },
    })
}

fn point_in_rect(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn available_invite_buttons(
    notification: &Notification,
    preview: RECT,
    scale: f64,
) -> Option<InviteButtonRects> {
    (notification.invite_actions && preview.bottom - preview.top >= dpi(84, scale))
        .then(|| invite_button_rects(preview, scale))
        .flatten()
}

fn invite_action_for_point(buttons: InviteButtonRects, point: POINT) -> Option<InviteAction> {
    if point_in_rect(buttons.accept, point) {
        Some(InviteAction::Accept)
    } else if point_in_rect(buttons.dismiss, point) {
        Some(InviteAction::Dismiss)
    } else {
        None
    }
}

fn invite_preview_interaction(
    preview: RECT,
    buttons: InviteButtonRects,
    point: POINT,
) -> Option<Option<InviteAction>> {
    point_in_rect(preview, point).then(|| invite_action_for_point(buttons, point))
}

pub(super) fn preview_bounds(
    client: RECT,
    minimum_top: i32,
    scale: f64,
    notification: &Notification,
) -> RECT {
    let minimum_top = minimum_top + dpi(PREVIEW_GAP, scale);
    let margin = dpi(PREVIEW_MARGIN, scale);
    let shadow_x = dpi(PREVIEW_SHADOW_X, scale);
    let shadow_y = dpi(PREVIEW_SHADOW_Y, scale);
    let preview_bottom = client.bottom - margin - shadow_y;
    let bounds_for_height = |height| RECT {
        left: client.left + margin,
        top: (preview_bottom - dpi(height, scale)).max(minimum_top),
        right: client.right - margin - shadow_x,
        bottom: preview_bottom,
    };
    let action_preview = bounds_for_height(ACTION_PREVIEW_HEIGHT);
    if available_invite_buttons(notification, action_preview, scale).is_some() {
        action_preview
    } else {
        bounds_for_height(PREVIEW_HEIGHT)
    }
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

struct InviteButtonStyle {
    base_color: u32,
    text_color: u32,
    hovered: bool,
    pressed: bool,
}

unsafe fn draw_invite_button(
    hdc: HDC,
    mut rect: RECT,
    scale: f64,
    label: &str,
    style: InviteButtonStyle,
) {
    let color = if style.pressed {
        adjust_color(style.base_color, -24)
    } else if style.hovered {
        adjust_color(style.base_color, 18)
    } else {
        style.base_color
    };
    let brush = CreateSolidBrush(COLORREF(color));
    let pen = CreatePen(
        PS_SOLID,
        dpi(1, scale).max(1),
        COLORREF(adjust_color(color, 28)),
    );
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);
    let radius = dpi(7, scale).max(3);
    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, old_pen);
    let _ = SelectObject(hdc, old_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(brush);

    let font = CreateFontW(
        dpi(16, scale),
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
    let _ = SetTextColor(hdc, COLORREF(style.text_color));
    let mut wide: Vec<u16> = label.encode_utf16().collect();
    let _ = DrawTextW(
        hdc,
        &mut wide,
        &mut rect,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
}

pub(super) unsafe fn draw_preview(
    hdc: HDC,
    available: RECT,
    scale: f64,
    notification: &Notification,
) {
    let available_width = available.right - available.left;
    let available_height = available.bottom - available.top;
    if available_width < dpi(120, scale) || available_height < dpi(32, scale) {
        return;
    }
    let available_buttons = available_invite_buttons(notification, available, scale);
    let show_actions = available_buttons.is_some();

    let font = CreateFontW(
        dpi(26, scale),
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
    let mut text_wide: Vec<u16> = notification.text.encode_utf16().collect();
    let mut text_size = SIZE::default();
    let old_font = SelectObject(hdc, font);
    let _ = GetTextExtentPoint32W(hdc, &text_wide, &mut text_size);

    let pad = dpi(PREVIEW_PADDING, scale);
    let icon_size = dpi(PREVIEW_ICON_SIZE, scale).min(available_height - 2 * pad);
    let gap = dpi(9, scale);
    let desired_width = 2 * pad + icon_size + gap + text_size.cx + dpi(12, scale);
    let preview_width = if show_actions || desired_width >= available_width - dpi(24, scale) {
        available_width
    } else {
        desired_width
            .max(dpi(PREVIEW_MIN_WIDTH, scale))
            .min(available_width)
    };
    let preview = RECT {
        left: available.left,
        top: available.top,
        right: available.left + preview_width,
        bottom: available.bottom,
    };
    let radius = dpi(10, scale).max(4);
    let far_shadow_brush = CreateSolidBrush(COLORREF(PREVIEW_SHADOW_FAR));
    let near_shadow_brush = CreateSolidBrush(COLORREF(PREVIEW_SHADOW_NEAR));
    let surface_brush = CreateSolidBrush(COLORREF(PREVIEW_BACKGROUND));
    let null_pen = CreatePen(PS_NULL, 0, COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, far_shadow_brush);
    let far_x = dpi(PREVIEW_SHADOW_X, scale);
    let far_y = dpi(PREVIEW_SHADOW_Y, scale);
    let _ = RoundRect(
        hdc,
        preview.left + far_x,
        preview.top + far_y,
        preview.right + far_x,
        preview.bottom + far_y,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, near_shadow_brush);
    let _ = RoundRect(
        hdc,
        preview.left + far_x / 2,
        preview.top + far_y / 2,
        preview.right + far_x / 2,
        preview.bottom + far_y / 2,
        radius * 2,
        radius * 2,
    );
    let _ = SelectObject(hdc, surface_brush);
    let _ = RoundRect(
        hdc,
        preview.left,
        preview.top,
        preview.right,
        preview.bottom,
        radius * 2,
        radius * 2,
    );

    let buttons = available_buttons;
    let content_bottom = buttons
        .map(|buttons| buttons.accept.top - dpi(4, scale))
        .unwrap_or(preview.bottom);
    let icon_left = preview.left + pad;
    let icon_top = preview.top + ((content_bottom - preview.top - icon_size) / 2);
    draw_icon(
        hdc,
        RECT {
            left: icon_left,
            top: icon_top,
            right: icon_left + icon_size,
            bottom: icon_top + icon_size,
        },
        notification.color,
        notification.kind,
    );

    let mut text_rect = RECT {
        left: icon_left + icon_size + gap,
        top: preview.top,
        right: preview.right - pad,
        bottom: content_bottom,
    };
    let _ = SelectObject(hdc, font);
    let _ = SetTextColor(hdc, COLORREF(notification.color));
    let _ = DrawTextW(
        hdc,
        &mut text_wide,
        &mut text_rect,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
    );

    if let Some(buttons) = buttons {
        draw_invite_button(
            hdc,
            buttons.accept,
            scale,
            "Accept",
            InviteButtonStyle {
                base_color: notification.color,
                text_color: contrasting_text_color(notification.color),
                hovered: notification.hovered_action == Some(InviteAction::Accept),
                pressed: notification.pressed_action == Some(InviteAction::Accept)
                    && notification.hovered_action == Some(InviteAction::Accept),
            },
        );
        draw_invite_button(
            hdc,
            buttons.dismiss,
            scale,
            "Dismiss",
            InviteButtonStyle {
                base_color: 0x00605040,
                text_color: 0x00FFFFFF,
                hovered: notification.hovered_action == Some(InviteAction::Dismiss),
                pressed: notification.pressed_action == Some(InviteAction::Dismiss)
                    && notification.hovered_action == Some(InviteAction::Dismiss),
            },
        );
    }

    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(surface_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(near_shadow_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(far_shadow_brush);
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
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
    windows::Win32::UI::WindowsAndMessaging::GetClientRect(pip.label_hwnd, &mut client).ok()?;
    let border = dpi(super::BORDER_WIDTH, state.dpi_scale);
    let label_point = POINT {
        x: point.x - border,
        y: point.y - border,
    };
    if !point_in_rect(client, label_point) {
        return None;
    }
    let label_bottom = dpi(state.label_height, state.dpi_scale)
        .min(client.bottom - client.top)
        .max(1);
    let unread_bottom = label_bottom + dpi(PREVIEW_GAP + UNREAD_ROW_HEIGHT, state.dpi_scale);
    let preview = preview_bounds(client, unread_bottom, state.dpi_scale, notification);
    let buttons = available_invite_buttons(notification, preview, state.dpi_scale)?;
    invite_preview_interaction(preview, buttons, label_point).map(|action| (pip.pid, action))
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
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(label_hwnd, None, false);
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
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(label_hwnd, None, false);
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
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(label_hwnd, None, false);
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
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(label_hwnd, None, false);
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
        if remove {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(
                pip.label_hwnd,
                COLORREF(super::LABEL_COLOR_KEY),
                state.label_alpha,
                windows::Win32::UI::WindowsAndMessaging::LWA_ALPHA
                    | windows::Win32::UI::WindowsAndMessaging::LWA_COLORKEY,
            );
        }
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(pip.hwnd, None, false);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(pip.label_hwnd, None, false);
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
                    let _ =
                        windows::Win32::Graphics::Gdi::InvalidateRect(pip.label_hwnd, None, false);
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
            let key = COLORREF(super::LABEL_COLOR_KEY);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(
                pip.label_hwnd,
                key,
                state.label_alpha.max(LABEL_MIN_ALPHA),
                windows::Win32::UI::WindowsAndMessaging::LWA_ALPHA
                    | windows::Win32::UI::WindowsAndMessaging::LWA_COLORKEY,
            );
            super::invalidate_pip_border(pip.hwnd, dpi(super::BORDER_WIDTH, state.dpi_scale));
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(pip.label_hwnd, None, false);
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
    let border = dpi(super::BORDER_WIDTH, state.dpi_scale);
    for pip in &state.pip_windows {
        if let Some(notification) = state.notifications.get_mut(&pip.pid) {
            let (redraw_border, redraw_preview) =
                notification.redraws_for_tick(now_ms, animations_enabled);
            if redraw_border {
                super::invalidate_pip_border(pip.hwnd, border);
            }
            if redraw_preview {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(pip.label_hwnd, None, false);
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
        let preview = preview_bounds(
            RECT {
                left: 0,
                top: 0,
                right: 420,
                bottom: 240,
            },
            50,
            1.0,
            &notification,
        );
        let buttons = available_invite_buttons(&notification, preview, 1.0).unwrap();
        assert_eq!(
            invite_action_for_point(
                buttons,
                POINT {
                    x: (buttons.accept.left + buttons.accept.right) / 2,
                    y: (buttons.accept.top + buttons.accept.bottom) / 2,
                },
            ),
            Some(InviteAction::Accept)
        );
        assert_eq!(
            invite_action_for_point(
                buttons,
                POINT {
                    x: (buttons.dismiss.left + buttons.dismiss.right) / 2,
                    y: (buttons.dismiss.top + buttons.dismiss.bottom) / 2,
                },
            ),
            Some(InviteAction::Dismiss)
        );
        assert_eq!(
            invite_preview_interaction(
                preview,
                buttons,
                POINT {
                    x: preview.left + 5,
                    y: preview.top + 5,
                },
            ),
            Some(None),
            "the whole preview must consume clicks instead of activating its PiP"
        );
        assert_eq!(
            invite_preview_interaction(preview, buttons, POINT { x: 5, y: 5 }),
            None
        );

        let narrow_preview = preview_bounds(
            RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 240,
            },
            50,
            1.0,
            &notification,
        );
        assert_eq!(narrow_preview.bottom - narrow_preview.top, PREVIEW_HEIGHT);
        assert_eq!(
            available_invite_buttons(&notification, narrow_preview, 1.0),
            None,
            "narrow previews must render and hit-test as non-interactive"
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
