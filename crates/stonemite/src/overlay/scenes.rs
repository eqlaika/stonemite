//! Renderer-neutral authored overlay scenes and their physical-pixel geometry.
//!
//! These are role-specific snapshots, not a generic drawing API. Win32 owns
//! placement and interaction; the Direct2D compositor consumes complete scenes.

use super::labels::{Color, FontSpec, LabelModel, LabelStyle, LabelTheme, Rect};
use super::notifications::{
    NotificationBorderLayout, NotificationContentLayout, NotificationVisualSnapshot,
};
pub(super) use super::scene_layout::TimerLayout;
use super::scene_layout::{pip_content_stack_layout, TIMER_PANEL_GAP};
/// The DWM thumbnail is already reduced to its legacy drag opacity. This
/// restrained overlay adds the gray drag cue without obscuring live imagery.
pub(super) const REORDER_DIM_ALPHA: u8 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SceneColor {
    pub color: Color,
    pub alpha: u8,
}

impl SceneColor {
    pub(super) const fn opaque(color: Color) -> Self {
        Self { color, alpha: 255 }
    }

    pub(super) const fn with_alpha(color: Color, alpha: u8) -> Self {
        Self { color, alpha }
    }

    pub(super) const fn from_colorref(value: u32, alpha: u8) -> Self {
        Self::with_alpha(Color::from_colorref(value), alpha)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UiTextRole {
    Timer,
    NotificationPreview,
    InviteButton,
    StatusBanner,
    Toast,
}

impl UiTextRole {
    pub(super) fn font(self) -> FontSpec {
        FontSpec {
            family: "Segoe UI".to_owned(),
            scale_percent: 100,
            weight: match self {
                Self::Timer | Self::NotificationPreview | Self::InviteButton | Self::Toast => 700,
                Self::StatusBanner => 900,
            },
        }
    }

    pub(super) fn height(self, scale: f64, role_height: i32) -> i32 {
        let logical = match self {
            Self::Timer | Self::InviteButton => 16,
            Self::NotificationPreview => 26,
            Self::StatusBanner | Self::Toast => role_height,
        };
        (f64::from(logical) * scale).round().max(1.0) as i32
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LabelScene<'a> {
    pub model: LabelModel<'a>,
    pub style: LabelStyle,
    pub theme: &'a LabelTheme,
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TimerScene<'a> {
    pub label: &'a str,
    pub remaining_text: &'a str,
    /// Elapsed fraction; the fill intentionally visualizes `1.0 - progress`.
    pub progress: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PipInteractionScene {
    pub hovered: bool,
    pub edit_mode: bool,
    pub reorder_dragging: bool,
    pub drag_source: bool,
    pub drop_target: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PipScene<'a> {
    pub canvas: Rect,
    pub border_width: i32,
    pub scale: f64,
    pub label: LabelScene<'a>,
    pub timer: Option<TimerScene<'a>>,
    pub notification: Option<NotificationVisualSnapshot>,
    pub interaction: PipInteractionScene,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FrameVisual {
    pub bounds: Rect,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PipSceneLayout {
    pub canvas: Rect,
    pub content: Rect,
    pub black_border_strips: [Rect; 4],
    pub dim_overlay: Option<SceneColor>,
    pub indicator_frames: Vec<FrameVisual>,
    pub label_bounds: Rect,
    pub timer: Option<TimerLayout>,
    pub notification_border: Option<NotificationBorderLayout>,
    pub notification_content: Option<NotificationContentLayout>,
}

impl PipScene<'_> {
    pub(super) fn content_alpha(&self) -> u8 {
        if self.notification.is_some() {
            self.label.alpha.max(super::notifications::LABEL_MIN_ALPHA)
        } else {
            self.label.alpha
        }
    }

    pub(super) fn layout(
        &self,
        measured_label_width: i32,
        measured_preview_text_width: i32,
    ) -> PipSceneLayout {
        let border = self.border_width.max(0);
        let canvas = self.canvas;
        let stack = pip_content_stack_layout(
            canvas,
            border,
            self.label.style,
            measured_label_width,
            self.timer.map(|timer| timer.progress),
        );
        let content = stack.content;
        let strips = border_strips(canvas, content);
        let dim_overlay = self
            .interaction
            .drag_source
            .then(|| SceneColor::from_colorref(0x00333333, REORDER_DIM_ALPHA));
        let indicator_frames = if self.interaction.drop_target {
            frame_visuals(canvas, border + 1, Color::from_colorref(0x0000CCFF))
        } else if self.interaction.hovered
            && !self.interaction.reorder_dragging
            && !self.interaction.edit_mode
        {
            frame_visuals(canvas, border, Color::from_colorref(0x00FFFFFF))
        } else if self.interaction.edit_mode {
            frame_visuals(canvas, 2, Color::from_colorref(0x00FFFF00))
        } else {
            Vec::new()
        };
        let notification_border = (!self.interaction.reorder_dragging
            && !self.interaction.edit_mode)
            .then(|| {
                self.notification
                    .as_ref()
                    .map(|notification| notification.border_layout(canvas, border))
            })
            .flatten();
        let notification_content = self.notification.as_ref().map(|notification| {
            notification.content_layout(
                content,
                stack.content_bottom,
                self.scale,
                measured_preview_text_width,
            )
        });
        PipSceneLayout {
            canvas,
            content,
            black_border_strips: strips,
            dim_overlay,
            indicator_frames,
            label_bounds: stack.label_bounds,
            timer: stack.timer,
            notification_border,
            notification_content,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ActiveLabelScene<'a> {
    pub canvas: Rect,
    pub label: LabelScene<'a>,
    pub timer: Option<TimerScene<'a>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ActiveLabelLayout {
    pub label_bounds: Rect,
    pub timer: Option<TimerLayout>,
}

impl ActiveLabelScene<'_> {
    pub(super) fn surface_opacity(&self) -> f32 {
        alpha_opacity(self.label.alpha)
    }

    pub(super) fn layout(&self) -> ActiveLabelLayout {
        let height = self.label.style.height().min(self.canvas.height()).max(0);
        let label_bounds = Rect::new(
            self.canvas.left,
            self.canvas.top,
            self.canvas.right,
            self.canvas.top + height,
        );
        let timer = self.timer.and_then(|timer| {
            let top = label_bounds.bottom + self.label.style.pixels(TIMER_PANEL_GAP);
            (top < self.canvas.bottom).then(|| {
                TimerLayout::new(
                    Rect::new(
                        label_bounds.left,
                        top,
                        label_bounds.right,
                        self.canvas.bottom,
                    ),
                    self.label.style.scale,
                    timer.progress,
                )
            })
        });
        ActiveLabelLayout {
            label_bounds,
            timer,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct StatusBannerScene<'a> {
    pub bounds: Rect,
    pub text: &'a str,
    pub background: Color,
    pub alpha: u8,
    pub scale: f64,
    pub logical_label_height: i32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ToastScene<'a> {
    pub bounds: Rect,
    pub text: &'a str,
    pub background: Color,
    pub alpha: u8,
    pub scale: f64,
    pub logical_height: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StonemiteButtonScene {
    pub bounds: Rect,
    pub icon_bounds: Rect,
    pub hovered: bool,
    pub pressed: bool,
}

impl StatusBannerScene<'_> {
    pub(super) fn surface_opacity(&self) -> f32 {
        alpha_opacity(self.alpha)
    }
}

impl ToastScene<'_> {
    pub(super) fn surface_opacity(&self) -> f32 {
        alpha_opacity(self.alpha)
    }
}

fn alpha_opacity(alpha: u8) -> f32 {
    f32::from(alpha) / 255.0
}

fn frame_visuals(bounds: Rect, count: i32, color: Color) -> Vec<FrameVisual> {
    (0..count.max(0))
        .map(|inset| FrameVisual {
            bounds: bounds.inset(inset),
            color,
        })
        .collect()
}

fn border_strips(canvas: Rect, content: Rect) -> [Rect; 4] {
    [
        Rect::new(canvas.left, canvas.top, canvas.right, content.top),
        Rect::new(canvas.left, content.bottom, canvas.right, canvas.bottom),
        Rect::new(canvas.left, content.top, content.left, content.bottom),
        Rect::new(content.right, content.top, canvas.right, content.bottom),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timer(bounds: Rect, scale: f64, progress: f32) -> TimerLayout {
        TimerLayout::new(bounds, scale, progress)
    }

    #[test]
    fn timer_layout_preserves_remaining_fraction_and_clamps_progress() {
        let bounds = Rect::new(0, 0, 200, 42);
        let start = timer(bounds, 1.0, -1.0);
        let middle = timer(bounds, 1.0, 0.25);
        let expired = timer(bounds, 1.0, 2.0);
        assert_eq!(start.track, Rect::new(8, 30, 192, 34));
        assert_eq!(start.fill, start.track);
        assert_eq!(middle.fill.width(), 138);
        assert_eq!(expired.fill.width(), 0);
        assert_eq!(start.font_height, 16);
        assert_eq!(start.label_text.right, 138);
    }

    #[test]
    fn timer_geometry_scales_at_supported_dpi_values() {
        for (scale, expected_inset, expected_font) in
            [(1.0_f64, 8, 16), (1.25_f64, 10, 20), (1.5_f64, 12, 24)]
        {
            let height = (42.0 * scale).round() as i32;
            let layout = timer(Rect::new(0, 0, 300, height), scale, 0.5);
            assert_eq!(layout.track.left, expected_inset);
            assert_eq!(layout.track.right, 300 - expected_inset);
            assert_eq!(layout.font_height, expected_font);
            assert_eq!(layout.fill.width(), layout.track.width() / 2);
        }
    }

    #[test]
    fn clipped_timer_geometry_never_escapes_its_panel() {
        let layout = timer(Rect::new(10, 20, 24, 28), 1.5, 0.5);
        for rect in [
            layout.track,
            layout.fill,
            layout.label_text,
            layout.remaining_text,
        ] {
            assert_eq!(rect, rect.intersect(layout.panel));
            assert!(!rect.is_empty() || rect.width() == 0 || rect.height() == 0);
        }
    }

    #[test]
    fn full_host_content_insets_produce_four_black_border_strips() {
        for (scale, border) in [(1.0_f64, 3), (1.25_f64, 4), (1.5_f64, 5)] {
            let canvas = Rect::new(0, 0, 320, (180.0 * scale).round() as i32);
            let content = canvas.inset(border);
            let strips = border_strips(canvas, content);
            assert_eq!(content.left, border);
            assert_eq!(content.top, border);
            assert_eq!(content.right, canvas.right - border);
            assert_eq!(strips[0].height(), border);
            assert_eq!(strips[2].width(), border);
            assert!(strips.iter().all(|strip| *strip == strip.intersect(canvas)));
        }
    }

    #[test]
    fn pip_layout_combines_inset_label_and_timer_without_overlap() {
        let theme = LabelTheme::default();
        let scene = PipScene {
            canvas: Rect::new(0, 0, 320, 180),
            border_width: 3,
            scale: 1.0,
            label: LabelScene {
                model: LabelModel {
                    text: "Bilka",
                    class: Some("WAR"),
                    number: 1,
                    background: Color::from_colorref(0x00D4864A),
                    badge_background: Color::from_colorref(0x00B06830),
                },
                style: LabelStyle::new(1.0, 48),
                theme: &theme,
                alpha: 204,
            },
            timer: Some(TimerScene {
                label: "Mez",
                remaining_text: "9.9s",
                progress: 0.25,
            }),
            notification: None,
            interaction: PipInteractionScene::default(),
        };
        let layout = scene.layout(220, 0);
        assert_eq!(scene.content_alpha(), 204);
        assert_eq!(layout.content, Rect::new(3, 3, 317, 177));
        assert_eq!(layout.label_bounds, Rect::new(3, 3, 223, 51));
        assert_eq!(
            layout.timer.map(|timer| timer.panel),
            Some(Rect::new(3, 55, 223, 97))
        );
        assert!(layout.indicator_frames.is_empty());
    }

    #[test]
    fn notification_presence_enforces_the_established_content_alpha_floor() {
        let theme = LabelTheme::default();
        let scene = PipScene {
            canvas: Rect::new(0, 0, 320, 180),
            border_width: 3,
            scale: 1.0,
            label: LabelScene {
                model: LabelModel {
                    text: "Bilka",
                    class: None,
                    number: 1,
                    background: Color::from_colorref(0x00D4864A),
                    badge_background: Color::from_colorref(0x00B06830),
                },
                style: LabelStyle::new(1.0, 48),
                theme: &theme,
                alpha: 100,
            },
            timer: None,
            notification: Some(NotificationVisualSnapshot {
                kind: super::super::notifications::Kind::Tell,
                text: "hello".to_owned(),
                color: Color::from_colorref(0x00BE28BE),
                unread_colors: vec![Color::from_colorref(0x00BE28BE)],
                invite_actions: false,
                hovered_action: None,
                pressed_action: None,
                animation: None,
                preview_visible: true,
            }),
            interaction: PipInteractionScene::default(),
        };
        assert_eq!(
            scene.content_alpha(),
            super::super::notifications::LABEL_MIN_ALPHA
        );
    }

    #[test]
    fn reorder_dim_is_deliberately_translucent_over_the_dwm_thumbnail() {
        assert_eq!(REORDER_DIM_ALPHA, 64);
    }

    #[test]
    fn timer_constrained_invite_layout_has_no_invisible_button_hits() {
        let snapshot = NotificationVisualSnapshot {
            kind: super::super::notifications::Kind::GroupInvite,
            text: "Honka invited you".to_owned(),
            color: Color::from_colorref(0x0060B06A),
            unread_colors: Vec::new(),
            invite_actions: true,
            hovered_action: None,
            pressed_action: None,
            animation: None,
            preview_visible: true,
        };
        let canvas = Rect::new(0, 0, 420, 180);
        let style = LabelStyle::new(1.0, 48);
        let unconstrained = super::super::notifications::notification_content_layout(
            &snapshot, canvas, 3, style, None, 180,
        );
        assert_eq!(
            unconstrained.preview.as_ref().unwrap().buttons.len(),
            2,
            "the same PiP has action buttons without a timer"
        );

        let constrained = super::super::notifications::notification_content_layout(
            &snapshot,
            canvas,
            3,
            style,
            Some(0.5),
            180,
        );
        let preview = constrained.preview.expect("plain fallback remains visible");
        assert!(preview.buttons.is_empty());
        assert_eq!(preview.surface.height(), 49);
    }

    #[test]
    fn whole_surface_scenes_expose_one_visual_opacity() {
        let theme = LabelTheme::default();
        let active = ActiveLabelScene {
            canvas: Rect::new(0, 0, 220, 48),
            label: LabelScene {
                model: LabelModel {
                    text: "Bilka",
                    class: None,
                    number: 1,
                    background: Color::from_colorref(0x00D4864A),
                    badge_background: Color::from_colorref(0x00B06830),
                },
                style: LabelStyle::new(1.0, 48),
                theme: &theme,
                alpha: 204,
            },
            timer: None,
        };
        assert!((active.surface_opacity() - 0.8).abs() < f32::EPSILON);
        let banner = StatusBannerScene {
            bounds: Rect::new(0, 0, 220, 48),
            text: "Broadcasting",
            background: Color::from_colorref(0x002030CC),
            alpha: 128,
            scale: 1.0,
            logical_label_height: 48,
        };
        assert!((banner.surface_opacity() - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn active_timer_starts_only_after_the_label_gap() {
        let theme = LabelTheme::default();
        let scene = ActiveLabelScene {
            canvas: Rect::new(0, 0, 220, 94),
            label: LabelScene {
                model: LabelModel {
                    text: "Bilka",
                    class: None,
                    number: 1,
                    background: Color::from_colorref(0x00D4864A),
                    badge_background: Color::from_colorref(0x00B06830),
                },
                style: LabelStyle::new(1.0, 48),
                theme: &theme,
                alpha: 204,
            },
            timer: Some(TimerScene {
                label: "Mez",
                remaining_text: "9.9s",
                progress: 0.5,
            }),
        };
        let layout = scene.layout();
        assert_eq!(layout.label_bounds.bottom, 48);
        assert_eq!(layout.timer.unwrap().panel.top, 52);
    }

    #[test]
    fn rect_intersection_and_inset_are_clipped_for_tiny_surfaces() {
        let tiny = Rect::new(10, 20, 13, 22);
        assert_eq!(tiny.inset(50), Rect::new(11, 21, 12, 21));
        assert_eq!(
            tiny.intersect(Rect::new(50, 50, 60, 60)),
            Rect::new(50, 50, 50, 50)
        );
    }
}
