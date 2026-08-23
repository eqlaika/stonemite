use super::labels::{LabelStyle, Rect};

pub(super) const TIMER_PANEL_GAP: i32 = 4;
pub(super) const TIMER_PANEL_HEIGHT: i32 = 42;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TimerLayout {
    pub panel: Rect,
    pub track: Rect,
    pub fill: Rect,
    pub label_text: Rect,
    pub remaining_text: Rect,
    pub corner_radius: i32,
    pub font_height: i32,
}

impl TimerLayout {
    pub(super) fn new(bounds: Rect, scale: f64, progress: f32) -> Self {
        let pixels = |logical: i32| (f64::from(logical) * scale).round() as i32;
        let panel = bounds;
        let inset = pixels(8).max(1);
        let bar_height = pixels(4).max(1);
        let track_bottom = (panel.bottom - inset).max(panel.top);
        let track = Rect::new(
            (panel.left + inset).min(panel.right),
            (track_bottom - bar_height).max(panel.top),
            (panel.right - inset).max((panel.left + inset).min(panel.right)),
            track_bottom,
        )
        .intersect(panel);
        let remaining_fraction = 1.0 - progress.clamp(0.0, 1.0);
        let fill_right = track.left
            + ((track.width().max(0) as f32 * remaining_fraction).round() as i32)
                .clamp(0, track.width().max(0));
        let fill = Rect::new(track.left, track.top, fill_right, track.bottom);
        let text_bottom = (track.top - pixels(2)).max(panel.top);
        let remaining_width = pixels(62).min(panel.width().max(0));
        let split = (panel.right - remaining_width).max(panel.left + inset);
        Self {
            panel,
            track,
            fill,
            label_text: Rect::new(
                (panel.left + inset).min(panel.right),
                panel.top,
                split.min(panel.right),
                text_bottom,
            )
            .intersect(panel),
            remaining_text: Rect::new(
                split.min(panel.right),
                panel.top,
                (panel.right - inset).max(split.min(panel.right)),
                text_bottom,
            )
            .intersect(panel),
            corner_radius: pixels(7).max(1),
            font_height: pixels(16).max(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PipContentStackLayout {
    pub content: Rect,
    pub label_bounds: Rect,
    pub timer: Option<TimerLayout>,
    pub content_bottom: i32,
}

/// Shared vertical layout for drawing and interaction. In particular, timer
/// fit decides the notification's minimum top in exactly one place.
pub(super) fn pip_content_stack_layout(
    canvas: Rect,
    border_width: i32,
    style: LabelStyle,
    measured_label_width: i32,
    timer_progress: Option<f32>,
) -> PipContentStackLayout {
    let content = canvas.inset(border_width.max(0));
    let label_height = style.height().min(content.height()).max(0);
    let label_bounds = Rect::new(
        content.left,
        content.top,
        content.left + measured_label_width.clamp(0, content.width().max(0)),
        content.top + label_height,
    );
    let timer = timer_progress.and_then(|progress| {
        let top = label_bounds.bottom + style.pixels(TIMER_PANEL_GAP);
        let bottom = top + style.pixels(TIMER_PANEL_HEIGHT);
        (bottom <= content.bottom).then(|| {
            TimerLayout::new(
                Rect::new(label_bounds.left, top, label_bounds.right, bottom),
                style.scale,
                progress,
            )
        })
    });
    PipContentStackLayout {
        content,
        label_bounds,
        timer,
        content_bottom: timer.map_or(label_bounds.bottom, |timer| timer.panel.bottom),
    }
}
