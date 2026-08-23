//! Overlay rendering backends.

mod d2d;
mod gdi;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::HDC;

use super::labels::{Color, LabelModel, LabelStyle, LabelTheme};

pub(super) unsafe fn measure_label_width(
    hdc: HDC,
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    max_width: i32,
) -> i32 {
    d2d::measure_label_width(model, style, theme, max_width)
        .unwrap_or_else(|| gdi::measure_label_width(hdc, model, style, theme, max_width))
}

pub(super) use gdi::draw_timer_overlay;

pub(super) unsafe fn draw_label(
    hdc: HDC,
    canvas_bounds: RECT,
    label_bounds: RECT,
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    transparent_color: Color,
) {
    if !d2d::draw_label(
        hdc,
        canvas_bounds,
        label_bounds,
        model,
        style,
        theme,
        transparent_color,
    ) {
        gdi::draw_label(
            hdc,
            canvas_bounds,
            label_bounds,
            model,
            style,
            theme,
            transparent_color,
        );
    }
}
