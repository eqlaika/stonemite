//! Renderer-independent label content, style, and geometry.
//!
//! The Win32 window layer decides where a label lives. Direct2D rendering
//! consumes these types without owning label state or hit geometry.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Color {
    /// Convert a Win32 COLORREF value (`0x00BBGGRR`) at the existing overlay
    /// boundary. Renderers use the explicit channels from this point onward.
    pub(super) const fn from_colorref(value: u32) -> Self {
        Self {
            red: (value & 0xff) as u8,
            green: ((value >> 8) & 0xff) as u8,
            blue: ((value >> 16) & 0xff) as u8,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FontSpec {
    pub family: String,
    /// Percentage of the established automatic character-name size.
    pub scale_percent: u32,
    /// Standard numeric font weight (400 regular through 900 heavy).
    pub weight: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LabelTheme {
    pub name_font: FontSpec,
    pub badge_font: FontSpec,
    pub text_color: Color,
    pub text_shadow_color: Color,
    pub badge_text_color: Color,
    pub corner_radius: i32,
    pub badge_inset: i32,
    pub badge_left_padding: i32,
    pub item_gap: i32,
    pub right_padding: i32,
    pub text_shadow_offset: i32,
    pub name_font_height_reduction: i32,
}

impl LabelTheme {
    pub(super) fn with_name_font(family: String, scale_percent: u32, weight: u16) -> Self {
        Self {
            name_font: FontSpec {
                family,
                scale_percent,
                weight,
            },
            badge_font: FontSpec {
                family: "Segoe UI".to_owned(),
                scale_percent,
                weight,
            },
            ..Self::default()
        }
    }
}

impl Default for LabelTheme {
    fn default() -> Self {
        Self {
            name_font: FontSpec {
                family: "Segoe UI".to_owned(),
                scale_percent: 100,
                weight: 700,
            },
            badge_font: FontSpec {
                family: "Segoe UI".to_owned(),
                scale_percent: 100,
                weight: 700,
            },
            text_color: Color {
                red: 255,
                green: 255,
                blue: 255,
            },
            text_shadow_color: Color {
                red: 0,
                green: 0,
                blue: 0,
            },
            badge_text_color: Color {
                red: 255,
                green: 255,
                blue: 255,
            },
            corner_radius: 8,
            badge_inset: 8,
            badge_left_padding: 8,
            item_gap: 8,
            right_padding: 8,
            text_shadow_offset: 1,
            name_font_height_reduction: 12,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LabelModel<'a> {
    pub text: &'a str,
    pub class: Option<&'a str>,
    pub number: usize,
    pub background: Color,
    pub badge_background: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LabelStyle {
    pub scale: f64,
    pub logical_height: i32,
}

impl LabelStyle {
    pub(super) const fn new(scale: f64, logical_height: i32) -> Self {
        Self {
            scale,
            logical_height,
        }
    }

    pub(super) fn pixels(self, logical: i32) -> i32 {
        (logical as f64 * self.scale).round() as i32
    }

    pub(super) fn height(self) -> i32 {
        self.pixels(self.logical_height)
    }

    pub(super) fn badge_font_height(self, theme: &LabelTheme) -> i32 {
        // Keep the window number optically aligned with the character name.
        // The badge retains a stable Segoe UI family for reliable digits, but
        // shares the configured name size and weight.
        self.name_font_height(theme)
    }

    pub(super) fn name_font_height(self, theme: &LabelTheme) -> i32 {
        let logical_height = self.logical_height - theme.name_font_height_reduction;
        (f64::from(logical_height) * f64::from(theme.name_font.scale_percent) / 100.0 * self.scale)
            .round() as i32
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub(super) const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub(super) const fn width(self) -> i32 {
        self.right - self.left
    }

    pub(super) const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub(super) const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub(super) fn inset(self, amount: i32) -> Self {
        let horizontal = amount.max(0).min(self.width().max(0) / 2);
        let vertical = amount.max(0).min(self.height().max(0) / 2);
        Self::new(
            self.left + horizontal,
            self.top + vertical,
            self.right - horizontal,
            self.bottom - vertical,
        )
    }

    pub(super) fn intersect(self, other: Self) -> Self {
        let left = self.left.max(other.left);
        let top = self.top.max(other.top);
        Self::new(
            left,
            top,
            self.right.min(other.right).max(left),
            self.bottom.min(other.bottom).max(top),
        )
    }

    pub(super) fn offset(self, x: i32, y: i32) -> Self {
        Self::new(self.left + x, self.top + y, self.right + x, self.bottom + y)
    }

    fn offset_origin(self, x: i32, y: i32) -> Self {
        Self::new(self.left + x, self.top + y, self.right, self.bottom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LabelLayout {
    pub background: Rect,
    pub badge: Rect,
    pub icon: Option<Rect>,
    pub text: Rect,
    pub text_shadow: Rect,
    pub corner_radius: i32,
}

impl LabelLayout {
    pub(super) fn new(
        bounds: Rect,
        style: LabelStyle,
        theme: &LabelTheme,
        has_class: bool,
    ) -> Self {
        let label_height = bounds.height().max(0);
        let item_size = (label_height - style.pixels(theme.badge_inset))
            .max(0)
            .min(bounds.width().max(0));
        let outer_padding = style.pixels(theme.badge_left_padding).max(0);
        let item_gap = style.pixels(theme.item_gap).max(0);
        let right_padding = style.pixels(theme.right_padding).max(0);
        let badge_left = (bounds.left + outer_padding).min(bounds.right);
        let badge_top = bounds.top + (label_height - item_size) / 2;
        let badge = Rect::new(
            badge_left,
            badge_top,
            (badge_left + item_size).min(bounds.right),
            (badge_top + item_size).min(bounds.bottom),
        );

        let after_badge = (badge.right + item_gap).min(bounds.right);
        let icon = has_class.then(|| {
            Rect::new(
                after_badge,
                badge.top,
                (after_badge + item_size).min(bounds.right),
                badge.bottom,
            )
        });
        let text_left = icon
            .map(|icon| (icon.right + item_gap).min(bounds.right))
            .unwrap_or(after_badge);
        let text_right = (bounds.right - right_padding).max(text_left);
        let text = Rect::new(text_left, bounds.top, text_right, bounds.bottom);

        Self {
            background: bounds,
            badge,
            icon,
            text,
            text_shadow: text.offset_origin(
                style.pixels(theme.text_shadow_offset),
                style.pixels(theme.text_shadow_offset),
            ),
            corner_radius: style.pixels(theme.corner_radius),
        }
    }
}

/// Preserve the existing label width formula while making it independent of
/// the text measurement backend.
pub(super) fn required_width(
    text_width: i32,
    style: LabelStyle,
    theme: &LabelTheme,
    has_class: bool,
    max_width: i32,
) -> i32 {
    let item_size = (style.height() - style.pixels(theme.badge_inset)).max(0);
    let gap = style.pixels(theme.item_gap).max(0);
    let item_count = if has_class { 2 } else { 1 };
    (style.pixels(theme.badge_left_padding).max(0)
        + item_count * item_size
        + item_count * gap
        + text_width.max(0)
        + style.pixels(theme.right_padding).max(0))
    .min(max_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: LabelStyle = LabelStyle::new(1.0, 48);

    #[test]
    fn lays_out_badge_and_text_without_class_icon() {
        let layout = LabelLayout::new(
            Rect::new(0, 0, 200, 48),
            STYLE,
            &LabelTheme::default(),
            false,
        );

        assert_eq!(layout.background, Rect::new(0, 0, 200, 48));
        assert_eq!(layout.badge, Rect::new(8, 4, 48, 44));
        assert_eq!(layout.icon, None);
        assert_eq!(layout.text, Rect::new(56, 0, 192, 48));
        assert_eq!(layout.text_shadow, Rect::new(57, 1, 192, 48));
        assert_eq!(layout.corner_radius, 8);
    }

    #[test]
    fn reserves_badge_sized_class_icon() {
        let layout = LabelLayout::new(
            Rect::new(10, 20, 230, 68),
            STYLE,
            &LabelTheme::default(),
            true,
        );

        assert_eq!(layout.badge, Rect::new(18, 24, 58, 64));
        assert_eq!(layout.icon, Some(Rect::new(66, 24, 106, 64)));
        assert_eq!(layout.text, Rect::new(114, 20, 222, 68));
        let icon = layout.icon.expect("class icon slot");
        assert_eq!(layout.badge.width(), icon.width());
        assert_eq!(layout.badge.top, icon.top);
        assert_eq!(layout.badge.bottom, icon.bottom);
        assert_eq!(layout.badge.left - 10, icon.left - layout.badge.right);
        assert_eq!(
            icon.left - layout.badge.right,
            layout.text.left - icon.right
        );
        assert_eq!(layout.text.left - icon.right, 230 - layout.text.right);
    }

    #[test]
    fn required_width_preserves_padding_and_cap() {
        let theme = LabelTheme::default();
        assert_eq!(required_width(100, STYLE, &theme, false, i32::MAX), 164);
        assert_eq!(required_width(100, STYLE, &theme, true, i32::MAX), 212);
        assert_eq!(required_width(100, STYLE, &theme, true, 180), 180);
    }

    #[test]
    fn scales_geometry_for_dpi() {
        let style = LabelStyle::new(1.5, 48);
        let layout = LabelLayout::new(
            Rect::new(0, 0, 300, 72),
            style,
            &LabelTheme::default(),
            true,
        );

        assert_eq!(style.height(), 72);
        assert_eq!(layout.badge, Rect::new(12, 6, 72, 66));
        assert_eq!(layout.icon, Some(Rect::new(84, 6, 144, 66)));
        assert_eq!(layout.text.left, 156);
        assert_eq!(layout.text.right, 288);
        assert_eq!(layout.corner_radius, 12);
    }

    #[test]
    fn typography_preserves_defaults_and_scales_character_names() {
        let default_theme = LabelTheme::default();
        assert_eq!(STYLE.name_font_height(&default_theme), 36);
        assert_eq!(STYLE.badge_font_height(&default_theme), 36);
        assert_eq!(
            default_theme.badge_font.weight,
            default_theme.name_font.weight
        );

        let larger = LabelTheme::with_name_font("Tahoma".to_owned(), 120, 600);
        assert_eq!(STYLE.name_font_height(&larger), 43);
        assert_eq!(STYLE.badge_font_height(&larger), 43);
        assert_eq!(larger.name_font.family, "Tahoma");
        assert_eq!(larger.name_font.weight, 600);
        assert_eq!(larger.badge_font.family, "Segoe UI");
        assert_eq!(larger.badge_font.weight, larger.name_font.weight);

        let fractional_dpi = LabelStyle::new(1.25, 48);
        let slightly_larger = LabelTheme::with_name_font("Segoe UI".to_owned(), 105, 700);
        assert_eq!(fractional_dpi.name_font_height(&slightly_larger), 47);
    }

    #[test]
    fn colorref_conversion_is_explicit() {
        assert_eq!(
            Color::from_colorref(0x00D4864A),
            Color {
                red: 0x4a,
                green: 0x86,
                blue: 0xd4,
            }
        );
    }
}
