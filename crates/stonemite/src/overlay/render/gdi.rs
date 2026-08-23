//! GDI implementation of the shared label renderer.
//!
//! Window ownership, placement, and event handling remain in `overlay.rs`.
//! This module is intentionally the only label code that knows about HDCs,
//! GDI fonts, brushes, or Win32 text drawing.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DrawTextW, Ellipse, FillRect, GetTextExtentPoint32W,
    RoundRect, SelectObject, SetBkMode, SetTextColor, BACKGROUND_MODE, DT_CENTER, DT_LEFT,
    DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC, HFONT, PS_NULL,
};

use super::super::labels::{
    required_width, Color, FontSpec, LabelLayout, LabelModel, LabelStyle, LabelTheme, Rect,
};

fn colorref(color: Color) -> COLORREF {
    COLORREF(u32::from(color.red) | (u32::from(color.green) << 8) | (u32::from(color.blue) << 16))
}

fn rect(value: Rect) -> RECT {
    RECT {
        left: value.left,
        top: value.top,
        right: value.right,
        bottom: value.bottom,
    }
}

fn model_rect(value: RECT) -> Rect {
    Rect::new(value.left, value.top, value.right, value.bottom)
}

unsafe fn create_font(height: i32, spec: &FontSpec) -> HFONT {
    let family: Vec<u16> = spec
        .family
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    CreateFontW(
        height,
        0,
        0,
        0,
        i32::from(spec.weight),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        PCWSTR(family.as_ptr()),
    )
}

pub(in crate::overlay) unsafe fn measure_label_width(
    hdc: HDC,
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    max_width: i32,
) -> i32 {
    let font = create_font(style.name_font_height(theme), &theme.name_font);
    let old_font = SelectObject(hdc, font);
    let wide: Vec<u16> = model.text.encode_utf16().collect();
    let mut text_size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut text_size);
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);

    required_width(text_size.cx, style, theme, model.class.is_some(), max_width)
}

/// Draw a label inside a larger transparent canvas. Active labels pass the
/// same rectangle for both values; PiP labels use the complete overlay as the
/// canvas and only the measured label rectangle as `label_bounds`.
pub(in crate::overlay) unsafe fn draw_label(
    hdc: HDC,
    canvas_bounds: RECT,
    label_bounds: RECT,
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    transparent_color: Color,
) {
    let transparent_brush = CreateSolidBrush(colorref(transparent_color));
    let _ = FillRect(hdc, &canvas_bounds, transparent_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(transparent_brush);

    let layout = LabelLayout::new(
        model_rect(label_bounds),
        style,
        theme,
        model.class.is_some(),
    );

    let background = rect(layout.background);
    let background_brush = CreateSolidBrush(colorref(model.background));
    let null_pen = CreatePen(PS_NULL, 0, COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, background_brush);
    let _ = RoundRect(
        hdc,
        background.left,
        background.top,
        background.right,
        background.bottom,
        layout.corner_radius * 2,
        layout.corner_radius * 2,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(background_brush);
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));

    let badge = rect(layout.badge);
    let badge_brush = CreateSolidBrush(colorref(model.badge_background));
    let badge_pen = CreatePen(PS_NULL, 0, COLORREF(0));
    let old_badge_pen = SelectObject(hdc, badge_pen);
    let old_badge_brush = SelectObject(hdc, badge_brush);
    let _ = Ellipse(hdc, badge.left, badge.top, badge.right, badge.bottom);
    let _ = SelectObject(hdc, old_badge_brush);
    let _ = SelectObject(hdc, old_badge_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_brush);

    let badge_font = create_font(style.badge_font_height(theme), &theme.badge_font);
    let old_font = SelectObject(hdc, badge_font);
    let mut badge_text_bounds = badge;
    let mut badge_text: Vec<u16> = model.number.to_string().encode_utf16().collect();
    let _ = SetTextColor(hdc, colorref(theme.badge_text_color));
    let _ = DrawTextW(
        hdc,
        &mut badge_text,
        &mut badge_text_bounds,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(badge_font);

    if let (Some(class), Some(icon)) = (model.class, layout.icon) {
        let _ = crate::class_icons::draw_class_icon(hdc, class, icon.left, icon.top, icon.width());
    }

    let name_font = create_font(style.name_font_height(theme), &theme.name_font);
    let old_name_font = SelectObject(hdc, name_font);
    let mut text: Vec<u16> = model.text.encode_utf16().collect();
    if !text.is_empty() {
        let mut shadow_bounds = rect(layout.text_shadow);
        let _ = SetTextColor(hdc, colorref(theme.text_shadow_color));
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut shadow_bounds,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        let mut text_bounds = rect(layout.text);
        let _ = SetTextColor(hdc, colorref(theme.text_color));
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut text_bounds,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
    }
    let _ = SelectObject(hdc, old_name_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(name_font);
}

pub(in crate::overlay) unsafe fn draw_timer_overlay(
    hdc: HDC,
    bounds: RECT,
    label: &str,
    remaining_text: &str,
    progress: f32,
    scale: f64,
) {
    let pixels = |logical: i32| (f64::from(logical) * scale).round() as i32;
    let panel_color = Color {
        red: 26,
        green: 31,
        blue: 42,
    };
    let track_color = Color {
        red: 9,
        green: 12,
        blue: 18,
    };
    let progress_color = Color {
        red: 93,
        green: 173,
        blue: 255,
    };

    let panel_brush = CreateSolidBrush(colorref(panel_color));
    let null_pen = CreatePen(PS_NULL, 0, COLORREF(0));
    let old_pen = SelectObject(hdc, null_pen);
    let old_brush = SelectObject(hdc, panel_brush);
    let radius = pixels(7) * 2;
    let _ = RoundRect(
        hdc,
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom,
        radius,
        radius,
    );
    let _ = SelectObject(hdc, old_brush);
    let _ = SelectObject(hdc, old_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(null_pen);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(panel_brush);

    let inset = pixels(8);
    let bar_height = pixels(4).max(1);
    let track = RECT {
        left: bounds.left + inset,
        top: bounds.bottom - inset - bar_height,
        right: bounds.right - inset,
        bottom: bounds.bottom - inset,
    };
    let track_brush = CreateSolidBrush(colorref(track_color));
    let _ = FillRect(hdc, &track, track_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(track_brush);

    let remaining_fraction = (1.0 - progress).clamp(0.0, 1.0);
    let fill = RECT {
        right: track.left + ((track.right - track.left) as f32 * remaining_fraction).round() as i32,
        ..track
    };
    if fill.right > fill.left {
        let progress_brush = CreateSolidBrush(colorref(progress_color));
        let _ = FillRect(hdc, &fill, progress_brush);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(progress_brush);
    }

    let font = create_font(
        pixels(16),
        &FontSpec {
            family: "Segoe UI".to_owned(),
            scale_percent: 100,
            weight: 700,
        },
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, BACKGROUND_MODE(1));
    let _ = SetTextColor(
        hdc,
        colorref(Color {
            red: 255,
            green: 255,
            blue: 255,
        }),
    );
    let text_bottom = track.top - pixels(2);
    let mut label_bounds = RECT {
        left: bounds.left + inset,
        top: bounds.top,
        right: bounds.right - pixels(62),
        bottom: text_bottom,
    };
    let mut remaining_bounds = RECT {
        left: label_bounds.right,
        top: bounds.top,
        right: bounds.right - inset,
        bottom: text_bottom,
    };
    let mut label: Vec<u16> = label.encode_utf16().collect();
    let mut remaining: Vec<u16> = remaining_text.encode_utf16().collect();
    let _ = DrawTextW(
        hdc,
        &mut label,
        &mut label_bounds,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
    );
    let _ = DrawTextW(
        hdc,
        &mut remaining,
        &mut remaining_bounds,
        DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);
}
