//! Renderer-independent label content, style, and geometry.
//!
//! The Win32 window layer decides where a label lives. Rendering backends
//! consume these types so the current GDI implementation can later be replaced
//! without moving label state or hit geometry again.

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

    pub(super) fn badge_font_height(self) -> i32 {
        self.pixels(self.logical_height - 14)
    }

    pub(super) fn name_font_height(self) -> i32 {
        self.pixels(self.logical_height - 12)
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
    pub(super) fn new(bounds: Rect, style: LabelStyle, has_class: bool) -> Self {
        let label_height = bounds.height();
        let badge_diameter = label_height - style.pixels(6);
        let badge_left = bounds.left + style.pixels(4);
        let badge_top = bounds.top + (label_height - badge_diameter) / 2;
        let badge = Rect::new(
            badge_left,
            badge_top,
            badge_left + badge_diameter,
            badge_top + badge_diameter,
        );

        let after_badge = badge.right + style.pixels(6);
        let icon = has_class.then(|| {
            Rect::new(
                after_badge,
                badge.top,
                after_badge + badge_diameter,
                badge.bottom,
            )
        });
        let text_left = icon
            .map(|icon| icon.right + style.pixels(6))
            .unwrap_or(after_badge);
        let text = Rect::new(text_left, bounds.top, bounds.right, bounds.bottom);

        Self {
            background: bounds,
            badge,
            icon,
            text,
            text_shadow: text.offset_origin(style.pixels(1), style.pixels(1)),
            corner_radius: style.pixels(8),
        }
    }
}

/// Preserve the existing label width formula while making it independent of
/// the text measurement backend.
pub(super) fn required_width(
    text_width: i32,
    style: LabelStyle,
    has_class: bool,
    max_width: i32,
) -> i32 {
    let badge_width = style.height();
    let icon_width = if has_class {
        badge_width + style.pixels(6)
    } else {
        0
    };
    (badge_width + style.pixels(6) + icon_width + text_width + style.pixels(10)).min(max_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: LabelStyle = LabelStyle::new(1.0, 48);

    #[test]
    fn lays_out_badge_and_text_without_class_icon() {
        let layout = LabelLayout::new(Rect::new(0, 0, 200, 48), STYLE, false);

        assert_eq!(layout.background, Rect::new(0, 0, 200, 48));
        assert_eq!(layout.badge, Rect::new(4, 3, 46, 45));
        assert_eq!(layout.icon, None);
        assert_eq!(layout.text, Rect::new(52, 0, 200, 48));
        assert_eq!(layout.text_shadow, Rect::new(53, 1, 200, 48));
        assert_eq!(layout.corner_radius, 8);
    }

    #[test]
    fn reserves_badge_sized_class_icon() {
        let layout = LabelLayout::new(Rect::new(10, 20, 230, 68), STYLE, true);

        assert_eq!(layout.badge, Rect::new(14, 23, 56, 65));
        assert_eq!(layout.icon, Some(Rect::new(62, 23, 104, 65)));
        assert_eq!(layout.text, Rect::new(110, 20, 230, 68));
    }

    #[test]
    fn required_width_preserves_padding_and_cap() {
        assert_eq!(required_width(100, STYLE, false, i32::MAX), 164);
        assert_eq!(required_width(100, STYLE, true, i32::MAX), 218);
        assert_eq!(required_width(100, STYLE, true, 180), 180);
    }

    #[test]
    fn scales_geometry_for_dpi() {
        let style = LabelStyle::new(1.5, 48);
        let layout = LabelLayout::new(Rect::new(0, 0, 300, 72), style, true);

        assert_eq!(style.height(), 72);
        assert_eq!(layout.badge, Rect::new(6, 4, 69, 67));
        assert_eq!(layout.icon, Some(Rect::new(78, 4, 141, 67)));
        assert_eq!(layout.text.left, 150);
        assert_eq!(layout.corner_radius, 12);
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
