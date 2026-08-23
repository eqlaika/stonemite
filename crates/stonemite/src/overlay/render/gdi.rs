//! GDI implementation of the shared label renderer.
//!
//! Window ownership, placement, and event handling remain in `overlay.rs`.
//! This module is intentionally the only label code that knows about HDCs,
//! GDI fonts, brushes, or Win32 text drawing.

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DrawTextW, Ellipse, FillRect, GetTextExtentPoint32W,
    RoundRect, SelectObject, SetBkMode, SetTextColor, BACKGROUND_MODE, DT_CENTER, DT_LEFT,
    DT_SINGLELINE, DT_VCENTER, FW_BOLD, FW_HEAVY, HDC, PS_NULL,
};

use super::super::labels::{required_width, Color, LabelLayout, LabelModel, LabelStyle, Rect};

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

pub(in crate::overlay) unsafe fn measure_label_width(
    hdc: HDC,
    model: &LabelModel<'_>,
    style: LabelStyle,
    max_width: i32,
) -> i32 {
    let font = CreateFontW(
        style.name_font_height(),
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
    let wide: Vec<u16> = model.text.encode_utf16().collect();
    let mut text_size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut text_size);
    let _ = SelectObject(hdc, old_font);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(font);

    required_width(text_size.cx, style, model.class.is_some(), max_width)
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
    transparent_color: Color,
) {
    let transparent_brush = CreateSolidBrush(colorref(transparent_color));
    let _ = FillRect(hdc, &canvas_bounds, transparent_brush);
    let _ = windows::Win32::Graphics::Gdi::DeleteObject(transparent_brush);

    let layout = LabelLayout::new(model_rect(label_bounds), style, model.class.is_some());

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

    let badge_font = CreateFontW(
        style.badge_font_height(),
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
    let mut badge_text_bounds = badge;
    let mut badge_text: Vec<u16> = model.number.to_string().encode_utf16().collect();
    let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
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

    let name_font = CreateFontW(
        style.name_font_height(),
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
    let old_name_font = SelectObject(hdc, name_font);
    let mut text: Vec<u16> = model.text.encode_utf16().collect();
    if !text.is_empty() {
        let mut shadow_bounds = rect(layout.text_shadow);
        let _ = SetTextColor(hdc, COLORREF(0x00000000));
        let _ = DrawTextW(
            hdc,
            &mut text,
            &mut shadow_bounds,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        let mut text_bounds = rect(layout.text);
        let _ = SetTextColor(hdc, COLORREF(0x00FFFFFF));
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
