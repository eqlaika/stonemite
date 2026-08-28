//! Concrete Direct2D/DirectWrite drawing for complete authored overlay scenes.

use windows::core::Result as WindowsResult;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN,
    D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext, ID2D1GeometrySink, D2D1_ANTIALIAS_MODE,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_ELLIPSE, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, D2D1_LAYER_OPTIONS1_NONE,
    D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER,
};

use super::super::combat_awareness::{
    CombatBorderLayout, CombatStatus, CombatStatusLayout, CombatVisualLayout,
};
use super::super::labels::{Color, LabelLayout, Rect};
use super::super::notifications::{
    InviteAction, Kind, NotificationBorderLayout, NotificationContentLayout,
    NotificationPreviewLayout, NotificationVisualSnapshot,
};
use super::super::scenes::{
    ActiveLabelScene, CastingLayout, CastingScene, DpsScene, DpsSceneLayout, PipScene,
    PipSceneLayout, SceneColor, StatusBannerScene, StonemiteButtonScene, TimerLayout, TimerScene,
    ToastScene, UiTextRole, DPS_SURFACE_ALPHA,
};
use super::compositor::TextResources;

pub(super) unsafe fn draw_active_label(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    icon: Option<&ID2D1Bitmap1>,
    scene: &ActiveLabelScene<'_>,
) -> WindowsResult<()> {
    prepare_context(context);
    let layout = scene.layout();
    draw_label(context, text, icon, &scene.label, layout.label_bounds)?;
    if let (Some(timer), Some(timer_layout)) = (scene.timer, layout.timer) {
        draw_timer(context, text, timer, timer_layout)?;
    }
    Ok(())
}

pub(super) unsafe fn draw_dps_scene(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: &DpsScene,
) -> WindowsResult<()> {
    prepare_context(context);
    let layout = scene.layout();
    let scale = scene.scale();
    fill_rounded(
        context,
        layout.panel,
        pixels(8, scale),
        SceneColor::from_colorref(0x00181716, DPS_SURFACE_ALPHA),
    )?;
    draw_dps_header(context, text, scene, &layout)?;
    for (row, row_layout) in scene.rows.iter().zip(layout.rows.iter()) {
        if let Some(separator) = row_layout.separator {
            let line_y = separator.top + separator.height() / 2;
            fill_rect(
                context,
                Rect::new(separator.left, line_y, separator.right, line_y + 1),
                SceneColor::from_colorref(0x006D706F, 150),
            )?;
        }
        if !row_layout.bar.is_empty() {
            fill_rect(
                context,
                row_layout.bar,
                SceneColor::from_colorref(
                    if row.active_managed {
                        0x008D7460
                    } else {
                        0x006C4A3A
                    },
                    if row.active_managed { 86 } else { 58 },
                ),
            )?;
        }
        let role = UiTextRole::DpsRow;
        let font = role.font();
        let font_height = role
            .height(scale, 0)
            .min((row_layout.bounds.height() - 2).max(1));
        let primary = if row.active_managed {
            SceneColor::from_colorref(0x00FFF0D8, 255)
        } else {
            SceneColor::from_colorref(0x00FFFFFF, 255)
        };
        draw_text(
            context,
            text,
            &row.rank.to_string(),
            &font,
            font_height,
            row_layout.columns.rank,
            primary,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        draw_text(
            context,
            text,
            &row.name,
            &font,
            font_height,
            row_layout.columns.player,
            primary,
            DWRITE_TEXT_ALIGNMENT_LEADING,
            true,
        )?;
        for (value, bounds) in [
            (row.damage.as_ref(), row_layout.columns.damage),
            (row.dps.as_ref(), row_layout.columns.dps),
            (row.sdps.as_ref(), row_layout.columns.sdps),
        ] {
            draw_text(
                context,
                text,
                value,
                &font,
                font_height,
                bounds,
                primary,
                DWRITE_TEXT_ALIGNMENT_TRAILING,
                false,
            )?;
        }
    }
    if scene.edit_mode {
        fill_inward_frame(
            context,
            layout.panel,
            SceneColor::from_colorref(0x0000E5FF, 255),
        )?;
        fill_inward_frame(
            context,
            layout.panel.inset(1),
            SceneColor::from_colorref(0x0000E5FF, 255),
        )?;
    }
    Ok(())
}

unsafe fn draw_dps_header(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: &DpsScene,
    layout: &DpsSceneLayout,
) -> WindowsResult<()> {
    let scale = scene.scale();
    let title_role = UiTextRole::DpsTitle;
    draw_text(
        context,
        text,
        &scene.title,
        &title_role.font(),
        title_role.height(scale, 0),
        layout.title,
        SceneColor::from_colorref(0x00FFFFFF, 255),
        DWRITE_TEXT_ALIGNMENT_LEADING,
        true,
    )?;
    draw_text(
        context,
        text,
        &scene.duration,
        &title_role.font(),
        title_role.height(scale, 0),
        layout.duration,
        SceneColor::from_colorref(0x00D3D6D5, 255),
        DWRITE_TEXT_ALIGNMENT_TRAILING,
        false,
    )?;
    let role = UiTextRole::DpsColumn;
    let font = role.font();
    let height = role.height(scale, 0);
    let secondary = SceneColor::from_colorref(0x00B6B9B8, 255);
    for (value, bounds, alignment) in [
        ("#", layout.columns.rank, DWRITE_TEXT_ALIGNMENT_LEADING),
        (
            "Player",
            layout.columns.player,
            DWRITE_TEXT_ALIGNMENT_LEADING,
        ),
        (
            "Damage",
            layout.columns.damage,
            DWRITE_TEXT_ALIGNMENT_TRAILING,
        ),
        ("DPS", layout.columns.dps, DWRITE_TEXT_ALIGNMENT_TRAILING),
        ("SDPS", layout.columns.sdps, DWRITE_TEXT_ALIGNMENT_TRAILING),
    ] {
        draw_text(
            context, text, value, &font, height, bounds, secondary, alignment, false,
        )?;
    }
    Ok(())
}

pub(super) unsafe fn draw_pip_scene(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    icon: Option<&ID2D1Bitmap1>,
    scene: &PipScene<'_>,
    layout: &PipSceneLayout,
) -> WindowsResult<()> {
    prepare_context(context);
    let black = solid_brush(context, SceneColor::from_colorref(0x00000000, 255))?;
    for strip in layout.black_border_strips {
        if !strip.is_empty() {
            context.FillRectangle(&d2d_rect(strip), &black);
        }
    }
    if let Some(dim) = layout.dim_overlay {
        let brush = solid_brush(context, dim)?;
        context.FillRectangle(&d2d_rect(layout.content), &brush);
    }
    if let Some(combat) = &layout.combat {
        draw_combat_effects(context, combat)?;
    }
    for frame in &layout.indicator_frames {
        fill_inward_frame(context, frame.bounds, SceneColor::opaque(frame.color))?;
    }
    if let Some(combat) = &layout.combat {
        if let Some(border) = &combat.border {
            draw_combat_border(context, border)?;
        }
    }
    if let Some(border) = &layout.notification_border {
        draw_notification_border(context, border)?;
    }
    draw_pip_content_group(context, scene.content_alpha(), layout.canvas, || {
        draw_label(context, text, icon, &scene.label, layout.label_bounds)?;
        if let (Some(casting), Some(casting_layout)) = (scene.casting, layout.casting) {
            draw_pip_content_group(context, casting.alpha, casting_layout.panel, || {
                draw_casting(context, text, casting, casting_layout)
            })?;
        }
        if let (Some(timer), Some(timer_layout)) = (scene.timer, layout.timer) {
            draw_timer(context, text, timer, timer_layout)?;
        }
        if let (Some(snapshot), Some(content)) = (&scene.notification, &layout.notification_content)
        {
            draw_notification_content(context, text, snapshot, content, scene.scale)?;
        }
        Ok(())
    })?;
    if let Some(status) = layout
        .combat
        .as_ref()
        .and_then(|combat| combat.status.as_ref())
    {
        draw_combat_status(context, text, status, scene.scale)?;
    }
    Ok(())
}

pub(super) unsafe fn draw_status_banner(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: &StatusBannerScene<'_>,
) -> WindowsResult<()> {
    prepare_context(context);
    let radius = pixels(8, scene.scale);
    fill_rounded(
        context,
        scene.bounds,
        radius,
        SceneColor::opaque(scene.background),
    )?;
    let font = UiTextRole::StatusBanner.font();
    let font_height =
        UiTextRole::StatusBanner.height(scene.scale, (scene.logical_label_height - 12).max(1));
    let padding = pixels(10, scene.scale);
    let text_bounds = Rect::new(
        scene.bounds.left + padding,
        scene.bounds.top,
        scene.bounds.right,
        scene.bounds.bottom,
    );
    draw_text(
        context,
        text,
        scene.text,
        &font,
        font_height,
        text_bounds.offset(pixels(1, scene.scale), pixels(1, scene.scale)),
        SceneColor::from_colorref(0x00000044, 255),
        DWRITE_TEXT_ALIGNMENT_LEADING,
        false,
    )?;
    draw_text(
        context,
        text,
        scene.text,
        &font,
        font_height,
        text_bounds,
        SceneColor::from_colorref(0x00FFFFFF, 255),
        DWRITE_TEXT_ALIGNMENT_LEADING,
        false,
    )
}

pub(super) unsafe fn draw_stonemite_button(
    context: &ID2D1DeviceContext,
    icon: &ID2D1Bitmap1,
    scene: &StonemiteButtonScene,
) -> WindowsResult<()> {
    prepare_context(context);
    let icon_bounds = if scene.pressed {
        scene.icon_bounds.inset(1)
    } else {
        scene.icon_bounds
    };
    context.DrawBitmap(
        icon,
        Some(&d2d_rect(icon_bounds)),
        if scene.pressed {
            0.82
        } else if scene.hovered {
            1.0
        } else {
            0.94
        },
        D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
        None,
        None,
    );
    Ok(())
}

pub(super) unsafe fn draw_toast(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: &ToastScene<'_>,
) -> WindowsResult<()> {
    prepare_context(context);
    fill_rounded(
        context,
        scene.bounds,
        pixels(8, scene.scale),
        SceneColor::opaque(scene.background),
    )?;
    let font = UiTextRole::Toast.font();
    let font_height = UiTextRole::Toast.height(scene.scale, (scene.logical_height - 12).max(12));
    draw_text(
        context,
        text,
        scene.text,
        &font,
        font_height,
        scene
            .bounds
            .offset(pixels(1, scene.scale), pixels(1, scene.scale)),
        SceneColor::from_colorref(0x00000044, 255),
        DWRITE_TEXT_ALIGNMENT_CENTER,
        false,
    )?;
    draw_text(
        context,
        text,
        scene.text,
        &font,
        font_height,
        scene.bounds,
        SceneColor::from_colorref(0x00FFFFFF, 255),
        DWRITE_TEXT_ALIGNMENT_CENTER,
        false,
    )
}

unsafe fn draw_pip_content_group(
    context: &ID2D1DeviceContext,
    alpha: u8,
    bounds: Rect,
    draw: impl FnOnce() -> WindowsResult<()>,
) -> WindowsResult<()> {
    let layer = context.CreateLayer(None)?;
    let parameters = D2D1_LAYER_PARAMETERS1 {
        contentBounds: d2d_rect(bounds),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_ALIASED,
        maskTransform: Matrix3x2 {
            M11: 1.0,
            M22: 1.0,
            ..Default::default()
        },
        opacity: f32::from(alpha) / 255.0,
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        ..Default::default()
    };
    context.PushLayer(&parameters, &layer);
    let result = draw();
    context.PopLayer();
    result
}

struct ScopedAntialiasMode<'a> {
    context: &'a ID2D1DeviceContext,
    previous: D2D1_ANTIALIAS_MODE,
}

impl<'a> ScopedAntialiasMode<'a> {
    unsafe fn set(context: &'a ID2D1DeviceContext, mode: D2D1_ANTIALIAS_MODE) -> Self {
        let previous = context.GetAntialiasMode();
        context.SetAntialiasMode(mode);
        Self { context, previous }
    }
}

impl Drop for ScopedAntialiasMode<'_> {
    fn drop(&mut self) {
        unsafe { self.context.SetAntialiasMode(self.previous) };
    }
}

unsafe fn prepare_context(context: &ID2D1DeviceContext) {
    context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
}

unsafe fn draw_label(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    icon: Option<&ID2D1Bitmap1>,
    scene: &super::super::scenes::LabelScene<'_>,
    bounds: Rect,
) -> WindowsResult<()> {
    let layout = LabelLayout::new(
        bounds,
        scene.style,
        scene.theme,
        scene.model.class.is_some(),
    );
    fill_rounded(
        context,
        layout.background,
        layout.corner_radius,
        SceneColor::opaque(scene.model.background),
    )?;
    fill_ellipse(
        context,
        layout.badge,
        SceneColor::opaque(scene.model.badge_background),
    )?;
    draw_text(
        context,
        text,
        &scene.model.number.to_string(),
        &scene.theme.badge_font,
        scene.style.badge_font_height(scene.theme),
        layout.badge,
        SceneColor::opaque(scene.theme.badge_text_color),
        DWRITE_TEXT_ALIGNMENT_CENTER,
        false,
    )?;
    if let (Some(icon), Some(icon_bounds)) = (icon, layout.icon) {
        context.DrawBitmap(
            icon,
            Some(&d2d_rect(icon_bounds)),
            1.0,
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            None,
            None,
        );
    }
    if !scene.model.text.is_empty() {
        draw_text(
            context,
            text,
            scene.model.text,
            &scene.theme.name_font,
            scene.style.name_font_height(scene.theme),
            layout.text_shadow,
            SceneColor::opaque(scene.theme.text_shadow_color),
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
        draw_text(
            context,
            text,
            scene.model.text,
            &scene.theme.name_font,
            scene.style.name_font_height(scene.theme),
            layout.text,
            SceneColor::opaque(scene.theme.text_color),
            DWRITE_TEXT_ALIGNMENT_LEADING,
            false,
        )?;
    }
    Ok(())
}

unsafe fn draw_casting(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: CastingScene<'_>,
    layout: CastingLayout,
) -> WindowsResult<()> {
    let (panel, accent) = match scene.outcome {
        None => (
            Color {
                red: 18,
                green: 24,
                blue: 34,
            },
            Color {
                red: 91,
                green: 188,
                blue: 255,
            },
        ),
        Some(super::super::casting::CastingOutcome::Completed) => (
            Color {
                red: 15,
                green: 34,
                blue: 29,
            },
            Color {
                red: 101,
                green: 227,
                blue: 174,
            },
        ),
        Some(super::super::casting::CastingOutcome::Fizzled) => (
            Color {
                red: 39,
                green: 19,
                blue: 24,
            },
            Color {
                red: 255,
                green: 105,
                blue: 118,
            },
        ),
        Some(super::super::casting::CastingOutcome::Resisted) => (
            Color {
                red: 42,
                green: 31,
                blue: 15,
            },
            Color {
                red: 255,
                green: 191,
                blue: 83,
            },
        ),
        Some(super::super::casting::CastingOutcome::Interrupted) => (
            Color {
                red: 37,
                green: 18,
                blue: 21,
            },
            Color {
                red: 242,
                green: 89,
                blue: 100,
            },
        ),
    };
    fill_rounded(
        context,
        layout.panel,
        layout.corner_radius,
        SceneColor::opaque(panel),
    )?;
    fill_rounded(
        context,
        layout.track,
        (layout.track.height() / 2).max(1),
        SceneColor::with_alpha(
            Color {
                red: 5,
                green: 8,
                blue: 13,
            },
            238,
        ),
    )?;
    if !layout.fill.is_empty() {
        fill_rounded(
            context,
            layout.fill,
            (layout.fill.height() / 2).max(1),
            SceneColor::opaque(accent),
        )?;
    }
    let font = UiTextRole::Casting.font();
    draw_text(
        context,
        text,
        scene.spell_name,
        &font,
        layout.font_height,
        layout.spell_text,
        SceneColor::from_colorref(0x00FFFFFF, 255),
        DWRITE_TEXT_ALIGNMENT_LEADING,
        true,
    )?;
    if scene.outcome.is_some() {
        draw_text(
            context,
            text,
            scene.outcome_label(),
            &font,
            layout.font_height,
            layout.outcome_text,
            SceneColor::opaque(accent),
            DWRITE_TEXT_ALIGNMENT_TRAILING,
            false,
        )?;
    }
    Ok(())
}

unsafe fn draw_timer(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    scene: TimerScene<'_>,
    layout: TimerLayout,
) -> WindowsResult<()> {
    fill_rounded(
        context,
        layout.panel,
        layout.corner_radius,
        SceneColor::from_colorref(0x002A1F1A, 255),
    )?;
    fill_rect(
        context,
        layout.track,
        SceneColor::from_colorref(0x00120C09, 255),
    )?;
    if !layout.fill.is_empty() {
        fill_rect(
            context,
            layout.fill,
            SceneColor::opaque(Color {
                red: 93,
                green: 173,
                blue: 255,
            }),
        )?;
    }
    let font = UiTextRole::Timer.font();
    let foreground = SceneColor::from_colorref(0x00FFFFFF, 255);
    draw_text(
        context,
        text,
        scene.label,
        &font,
        layout.font_height,
        layout.label_text,
        foreground,
        DWRITE_TEXT_ALIGNMENT_LEADING,
        true,
    )?;
    draw_text(
        context,
        text,
        scene.remaining_text,
        &font,
        layout.font_height,
        layout.remaining_text,
        foreground,
        DWRITE_TEXT_ALIGNMENT_TRAILING,
        false,
    )
}

unsafe fn draw_combat_effects(
    context: &ID2D1DeviceContext,
    layout: &CombatVisualLayout,
) -> WindowsResult<()> {
    if let Some(fill) = layout.dead_tint {
        fill_rect(
            context,
            fill.bounds,
            SceneColor::with_alpha(fill.color, fill.alpha),
        )?;
    }
    if let Some(fill) = layout.blood_tint {
        fill_rect(
            context,
            fill.bounds,
            SceneColor::with_alpha(fill.color, fill.alpha),
        )?;
    }
    for frame in &layout.blood_frames {
        fill_inward_frame(
            context,
            frame.bounds,
            SceneColor::with_alpha(frame.color, frame.alpha),
        )?;
    }
    Ok(())
}

unsafe fn draw_combat_border(
    context: &ID2D1DeviceContext,
    layout: &CombatBorderLayout,
) -> WindowsResult<()> {
    for frame in &layout.frames {
        fill_inward_frame(context, *frame, SceneColor::opaque(layout.color))?;
    }
    Ok(())
}

unsafe fn draw_combat_status(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    layout: &CombatStatusLayout,
    scale: f64,
) -> WindowsResult<()> {
    fill_rounded(
        context,
        layout.surface,
        layout.radius,
        SceneColor::with_alpha(layout.surface_color, layout.alpha),
    )?;
    let role = if layout.status == CombatStatus::Dead {
        UiTextRole::CombatDead
    } else {
        UiTextRole::CombatStatus
    };
    draw_text(
        context,
        text,
        layout.status.label(),
        &role.font(),
        role.height(scale, 0),
        layout.text,
        SceneColor::opaque(layout.text_color),
        DWRITE_TEXT_ALIGNMENT_CENTER,
        true,
    )
}

unsafe fn draw_notification_border(
    context: &ID2D1DeviceContext,
    layout: &NotificationBorderLayout,
) -> WindowsResult<()> {
    for frame in &layout.frames {
        fill_inward_frame(context, *frame, SceneColor::opaque(layout.frame_color))?;
    }
    if !layout.highlight_lines.is_empty() {
        // Keep authored rectangles pixel-crisp, but allow the moving trace to
        // advance at subpixel positions. The guard restores the prior mode on
        // success and on any early return from fallible resource creation.
        let _antialias = ScopedAntialiasMode::set(context, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        let brush = solid_brush(context, SceneColor::opaque(layout.highlight_color))?;
        let stroke_width = layout.stroke_width as f32;
        let radius = stroke_width / 2.0;
        for line in &layout.highlight_lines {
            let from = point(line.from.0, line.from.1);
            let to = point(line.to.0, line.to.1);
            context.DrawLine(from, to, &brush, stroke_width, None);
            // Default Direct2D line caps are flat. Round each endpoint so
            // independently split edge segments join continuously at corners
            // and the moving head advances smoothly at subpixel positions.
            for center in [from, to] {
                context.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: center,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    &brush,
                );
            }
        }
    }
    Ok(())
}

unsafe fn draw_notification_content(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    snapshot: &NotificationVisualSnapshot,
    layout: &NotificationContentLayout,
    scale: f64,
) -> WindowsResult<()> {
    for dot in &layout.unread_dots {
        fill_ellipse(context, dot.ring, SceneColor::opaque(dot.ring_color))?;
        fill_ellipse(context, dot.dot, SceneColor::opaque(dot.color))?;
    }
    if let Some(preview) = &layout.preview {
        draw_notification_preview(context, text, snapshot, preview, scale)?;
    }
    Ok(())
}

unsafe fn draw_notification_preview(
    context: &ID2D1DeviceContext,
    text: &TextResources,
    snapshot: &NotificationVisualSnapshot,
    layout: &NotificationPreviewLayout,
    scale: f64,
) -> WindowsResult<()> {
    fill_rounded(
        context,
        layout.far_shadow,
        layout.radius,
        SceneColor::opaque(layout.far_shadow_color),
    )?;
    fill_rounded(
        context,
        layout.near_shadow,
        layout.radius,
        SceneColor::opaque(layout.near_shadow_color),
    )?;
    fill_rounded(
        context,
        layout.surface,
        layout.radius,
        SceneColor::opaque(layout.surface_color),
    )?;
    draw_notification_icon(
        context,
        snapshot.kind,
        layout.icon,
        SceneColor::opaque(snapshot.color),
    )?;
    let preview_font = UiTextRole::NotificationPreview.font();
    draw_text(
        context,
        text,
        &snapshot.text,
        &preview_font,
        UiTextRole::NotificationPreview.height(scale, 0),
        layout.text,
        SceneColor::opaque(snapshot.color),
        DWRITE_TEXT_ALIGNMENT_LEADING,
        true,
    )?;
    let button_font = UiTextRole::InviteButton.font();
    for button in &layout.buttons {
        fill_rounded(
            context,
            button.bounds,
            button.radius,
            SceneColor::opaque(button.fill_color),
        )?;
        let border = solid_brush(context, SceneColor::opaque(button.border_color))?;
        context.DrawRoundedRectangle(
            &rounded(button.bounds, button.radius),
            &border,
            pixels(1, scale).max(1) as f32,
            None,
        );
        draw_text(
            context,
            text,
            match button.action {
                InviteAction::Accept => "Accept",
                InviteAction::Dismiss => "Dismiss",
            },
            &button_font,
            UiTextRole::InviteButton.height(scale, 0),
            button.bounds,
            SceneColor::opaque(button.text_color),
            DWRITE_TEXT_ALIGNMENT_CENTER,
            false,
        )?;
    }
    Ok(())
}

unsafe fn draw_notification_icon(
    context: &ID2D1DeviceContext,
    kind: Kind,
    bounds: Rect,
    color: SceneColor,
) -> WindowsResult<()> {
    let brush = solid_brush(context, color)?;
    let width = (bounds.width().min(bounds.height()) / 12).max(2) as f32;
    match kind {
        Kind::Tell => {
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 7.9, 20.0);
                add_beziers(
                    sink,
                    bounds,
                    &[
                        (12.5, 20.9, 17.6, 18.8, 20.1, 14.8),
                        (22.8, 10.0, 20.0, 4.1, 15.1, 2.4),
                        (10.2, 0.7, 5.0, 3.5, 3.3, 8.4),
                        (2.4, 11.1, 2.7, 14.0, 4.0, 16.1),
                    ],
                );
                sink.AddLine(icon_point(bounds, 2.0, 22.0));
                sink.AddLine(icon_point(bounds, 7.9, 20.0));
                end(sink);
            })?;
        }
        Kind::GroupInvite | Kind::RaidInvite => {
            context.DrawEllipse(
                &ellipse(Rect::new(
                    icon_point(bounds, 5.0, 3.0).x as i32,
                    icon_point(bounds, 5.0, 3.0).y as i32,
                    icon_point(bounds, 15.0, 13.0).x as i32,
                    icon_point(bounds, 15.0, 13.0).y as i32,
                )),
                &brush,
                width,
                None,
            );
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 2.0, 21.0);
                add_beziers(
                    sink,
                    bounds,
                    &[
                        (2.0, 16.6, 5.6, 13.0, 10.0, 13.0),
                        (12.2, 13.0, 14.0, 13.7, 15.3, 15.0),
                    ],
                );
                end(sink);
            })?;
            if kind == Kind::RaidInvite {
                draw_path(context, &brush, width, |sink| {
                    begin(sink, bounds, 17.6, 3.7);
                    add_beziers(
                        sink,
                        bounds,
                        &[
                            (21.0, 5.0, 21.0, 10.0, 18.0, 12.0),
                            (20.4, 13.8, 22.0, 16.8, 22.0, 20.0),
                        ],
                    );
                    end(sink);
                })?;
            } else {
                context.DrawLine(
                    icon_point(bounds, 19.0, 16.0),
                    icon_point(bounds, 19.0, 22.0),
                    &brush,
                    width,
                    None,
                );
                context.DrawLine(
                    icon_point(bounds, 16.0, 19.0),
                    icon_point(bounds, 22.0, 19.0),
                    &brush,
                    width,
                    None,
                );
            }
        }
        Kind::Trade => {
            context.DrawLine(
                icon_point(bounds, 3.0, 7.0),
                icon_point(bounds, 21.0, 7.0),
                &brush,
                width,
                None,
            );
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 17.0, 3.0);
                sink.AddLine(icon_point(bounds, 21.0, 7.0));
                sink.AddLine(icon_point(bounds, 17.0, 11.0));
                end(sink);
            })?;
            context.DrawLine(
                icon_point(bounds, 21.0, 17.0),
                icon_point(bounds, 3.0, 17.0),
                &brush,
                width,
                None,
            );
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 7.0, 13.0);
                sink.AddLine(icon_point(bounds, 3.0, 17.0));
                sink.AddLine(icon_point(bounds, 7.0, 21.0));
                end(sink);
            })?;
        }
        Kind::Resurrection => {
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 12.0, 21.0);
                add_beziers(
                    sink,
                    bounds,
                    &[
                        (10.0, 19.2, 3.0, 15.0, 2.0, 10.0),
                        (1.4, 6.0, 4.0, 3.0, 7.5, 3.0),
                        (9.8, 3.0, 11.2, 4.4, 12.0, 5.6),
                        (12.8, 4.4, 14.2, 3.0, 16.5, 3.0),
                        (20.0, 3.0, 22.6, 6.0, 22.0, 10.0),
                        (21.2, 14.0, 14.0, 19.2, 12.0, 21.0),
                    ],
                );
                end(sink);
                begin(sink, bounds, 3.2, 13.0);
                for &(x, y) in &[
                    (9.5, 13.0),
                    (10.0, 12.0),
                    (12.0, 16.5),
                    (14.0, 9.5),
                    (15.5, 13.0),
                    (20.8, 13.0),
                ] {
                    sink.AddLine(icon_point(bounds, x, y));
                }
                end(sink);
            })?;
        }
        Kind::LevelGain | Kind::AlternateAdvancementGain => {
            context.DrawLine(
                icon_point(bounds, 12.0, 21.0),
                icon_point(bounds, 12.0, 4.0),
                &brush,
                width,
                None,
            );
            draw_path(context, &brush, width, |sink| {
                begin(sink, bounds, 6.0, 10.0);
                sink.AddLine(icon_point(bounds, 12.0, 4.0));
                sink.AddLine(icon_point(bounds, 18.0, 10.0));
                end(sink);
            })?;
            context.DrawLine(
                icon_point(bounds, 5.0, 21.0),
                icon_point(bounds, 19.0, 21.0),
                &brush,
                width,
                None,
            );
        }
        Kind::Death => {
            context.DrawLine(
                icon_point(bounds, 7.0, 17.0),
                icon_point(bounds, 17.0, 7.0),
                &brush,
                width,
                None,
            );
            for &(cx, cy) in &[(5.0, 19.0), (19.0, 5.0)] {
                let left = icon_point(bounds, cx - 2.5, cy - 2.5);
                let right = icon_point(bounds, cx + 2.5, cy + 2.5);
                context.DrawEllipse(
                    &D2D1_ELLIPSE {
                        point: point((left.x + right.x) / 2.0, (left.y + right.y) / 2.0),
                        radiusX: (right.x - left.x) / 2.0,
                        radiusY: (right.y - left.y) / 2.0,
                    },
                    &brush,
                    width,
                    None,
                );
            }
        }
    }
    Ok(())
}

unsafe fn draw_path(
    context: &ID2D1DeviceContext,
    brush: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    width: f32,
    build: impl FnOnce(&ID2D1GeometrySink),
) -> WindowsResult<()> {
    let geometry = context.GetFactory()?.CreatePathGeometry()?;
    let sink = geometry.Open()?;
    build(&sink);
    sink.Close()?;
    context.DrawGeometry(&geometry, brush, width, None);
    Ok(())
}

unsafe fn begin(sink: &ID2D1GeometrySink, bounds: Rect, x: f64, y: f64) {
    sink.BeginFigure(icon_point(bounds, x, y), D2D1_FIGURE_BEGIN_HOLLOW);
}

unsafe fn end(sink: &ID2D1GeometrySink) {
    sink.EndFigure(D2D1_FIGURE_END_OPEN);
}

unsafe fn add_beziers(
    sink: &ID2D1GeometrySink,
    bounds: Rect,
    segments: &[(f64, f64, f64, f64, f64, f64)],
) {
    let segments: Vec<_> = segments
        .iter()
        .map(|&(x1, y1, x2, y2, x3, y3)| D2D1_BEZIER_SEGMENT {
            point1: icon_point(bounds, x1, y1),
            point2: icon_point(bounds, x2, y2),
            point3: icon_point(bounds, x3, y3),
        })
        .collect();
    sink.AddBeziers(&segments);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_text(
    context: &ID2D1DeviceContext,
    resources: &TextResources,
    value: &str,
    font: &super::super::labels::FontSpec,
    height: i32,
    bounds: Rect,
    color: SceneColor,
    alignment: DWRITE_TEXT_ALIGNMENT,
    ellipsis: bool,
) -> WindowsResult<()> {
    if value.is_empty() || bounds.is_empty() {
        return Ok(());
    }
    let format = resources.text_format_with_alignment(font, height, alignment)?;
    if ellipsis {
        let sign = resources.factory.CreateEllipsisTrimmingSign(&format)?;
        format.SetTrimming(
            &DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            },
            &sign,
        )?;
    }
    let brush = solid_brush(context, color)?;
    let wide: Vec<u16> = value.encode_utf16().collect();
    context.DrawText(
        &wide,
        &format,
        &d2d_rect(bounds),
        &brush,
        D2D1_DRAW_TEXT_OPTIONS_CLIP,
        DWRITE_MEASURING_MODE_NATURAL,
    );
    Ok(())
}

unsafe fn solid_brush(
    context: &ID2D1DeviceContext,
    color: SceneColor,
) -> WindowsResult<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush> {
    context.CreateSolidColorBrush(&d2d_color(color), None)
}

unsafe fn fill_rect(
    context: &ID2D1DeviceContext,
    rect: Rect,
    color: SceneColor,
) -> WindowsResult<()> {
    if !rect.is_empty() {
        let brush = solid_brush(context, color)?;
        context.FillRectangle(&d2d_rect(rect), &brush);
    }
    Ok(())
}

/// Filled strips retain inward frame geometry and avoid losing the outer half
/// of a centered Direct2D stroke at target edges.
unsafe fn fill_inward_frame(
    context: &ID2D1DeviceContext,
    bounds: Rect,
    color: SceneColor,
) -> WindowsResult<()> {
    if bounds.is_empty() {
        return Ok(());
    }
    for strip in inward_frame_strips(bounds) {
        fill_rect(context, strip, color)?;
    }
    Ok(())
}

fn inward_frame_strips(bounds: Rect) -> [Rect; 4] {
    let bottom_top = (bounds.bottom - 1).max(bounds.top);
    let right_left = (bounds.right - 1).max(bounds.left);
    [
        Rect::new(
            bounds.left,
            bounds.top,
            bounds.right,
            (bounds.top + 1).min(bounds.bottom),
        ),
        Rect::new(bounds.left, bottom_top, bounds.right, bounds.bottom),
        Rect::new(
            bounds.left,
            bounds.top,
            (bounds.left + 1).min(bounds.right),
            bounds.bottom,
        ),
        Rect::new(right_left, bounds.top, bounds.right, bounds.bottom),
    ]
}

unsafe fn fill_rounded(
    context: &ID2D1DeviceContext,
    rect: Rect,
    radius: i32,
    color: SceneColor,
) -> WindowsResult<()> {
    if !rect.is_empty() {
        let brush = solid_brush(context, color)?;
        context.FillRoundedRectangle(&rounded(rect, radius), &brush);
    }
    Ok(())
}

unsafe fn fill_ellipse(
    context: &ID2D1DeviceContext,
    rect: Rect,
    color: SceneColor,
) -> WindowsResult<()> {
    if !rect.is_empty() {
        let brush = solid_brush(context, color)?;
        context.FillEllipse(&ellipse(rect), &brush);
    }
    Ok(())
}

fn d2d_color(color: SceneColor) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(color.color.red) / 255.0,
        g: f32::from(color.color.green) / 255.0,
        b: f32::from(color.color.blue) / 255.0,
        a: f32::from(color.alpha) / 255.0,
    }
}

fn d2d_rect(rect: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: rect.left as f32,
        top: rect.top as f32,
        right: rect.right as f32,
        bottom: rect.bottom as f32,
    }
}

fn rounded(rect: Rect, radius: i32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: d2d_rect(rect),
        radiusX: radius.max(0) as f32,
        radiusY: radius.max(0) as f32,
    }
}

fn ellipse(rect: Rect) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: point(
            (rect.left + rect.right) as f32 / 2.0,
            (rect.top + rect.bottom) as f32 / 2.0,
        ),
        radiusX: rect.width().max(0) as f32 / 2.0,
        radiusY: rect.height().max(0) as f32 / 2.0,
    }
}

fn icon_point(rect: Rect, x: f64, y: f64) -> D2D_POINT_2F {
    point(
        rect.left as f32 + (x * rect.width().max(1) as f64 / 24.0).round() as f32,
        rect.top as f32 + (y * rect.height().max(1) as f64 / 24.0).round() as f32,
    )
}

fn point(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn pixels(logical: i32, scale: f64) -> i32 {
    (f64::from(logical) * scale).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inward_frames_keep_all_edges_inside_supported_dpi_canvases() {
        for (width, height) in [(320, 180), (400, 225), (480, 270)] {
            let bounds = Rect::new(0, 0, width, height);
            let strips = inward_frame_strips(bounds);
            assert_eq!(strips[0], Rect::new(0, 0, width, 1));
            assert_eq!(strips[1], Rect::new(0, height - 1, width, height));
            assert_eq!(strips[2], Rect::new(0, 0, 1, height));
            assert_eq!(strips[3], Rect::new(width - 1, 0, width, height));
            assert!(strips.iter().all(|strip| *strip == strip.intersect(bounds)));
        }
    }
}
