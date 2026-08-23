use super::labels::{Color, LabelModel, LabelTheme};
use crate::config;
use crate::eq_windows::EqWindow;

pub(super) const BORDER_WIDTH: i32 = 3;
pub(super) const DEFAULT_LABEL_HEIGHT: i32 = 48;
pub(super) const DEFAULT_LABEL_OPACITY: u32 = 80;

/// Distinct background colors for per-number labels (COLORREF = 0x00BBGGRR).
pub(super) const LABEL_COLORS: &[u32] = &[
    0x00D4864A, // medium blue   (rgb #4A86D4)
    0x0060B06A, // forest green  (rgb #6AB060)
    0x005858D8, // warm rose     (rgb #D85858)
    0x0048B8E0, // amber         (rgb #E0B848)
    0x00C87CA0, // orchid        (rgb #A07CC8)
    0x00A8C858, // teal          (rgb #58C8A8)
];

const BADGE_COLORS: &[u32] = &[
    0x00B06830, // deep blue     (rgb #3068B0)
    0x00409048, // deep green    (rgb #489040)
    0x003838B8, // deep rose     (rgb #B83838)
    0x002898C0, // deep amber    (rgb #C09828)
    0x00A85C80, // deep orchid   (rgb #805CA8)
    0x0088A838, // deep teal     (rgb #38A888)
];

pub(super) fn color_for_number(number: usize) -> u32 {
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

pub(super) fn label_model<'a>(
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

pub(super) fn configured_label_theme(cfg: &config::Config) -> LabelTheme {
    LabelTheme::with_name_font(
        cfg.effective_pip_label_font_family().to_owned(),
        cfg.effective_pip_label_font_scale(),
        label_font_weight(cfg.effective_pip_label_font_weight()),
    )
}

pub(super) fn opacity_percent_to_alpha(percent: u32) -> u8 {
    ((percent.clamp(0, 100) as u16 * 255) / 100) as u8
}

pub(super) fn format_label(window: &EqWindow) -> String {
    window.character.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_percent_scales_to_alpha() {
        assert_eq!(opacity_percent_to_alpha(10), 25);
        assert_eq!(opacity_percent_to_alpha(80), 204);
        assert_eq!(opacity_percent_to_alpha(100), 255);
    }
}
