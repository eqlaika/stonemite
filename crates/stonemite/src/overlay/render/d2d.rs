//! Direct2D/DirectWrite implementation of the shared label renderer.
//!
//! A single DC render target is rebound to the current paint HDC. This keeps
//! target ownership independent of HWND rebuilds and lets the existing GDI
//! notification and class-icon paths continue to draw after `EndDraw`.

use std::sync::{Mutex, OnceLock};

use windows::core::{w, Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;

use super::super::labels::{
    required_width, Color, FontSpec, LabelLayout, LabelModel, LabelStyle, LabelTheme, Rect,
};

struct Renderer {
    d2d_factory: ID2D1Factory,
    write_factory: IDWriteFactory,
    target: ID2D1DCRenderTarget,
}

impl Renderer {
    unsafe fn new() -> WindowsResult<Self> {
        let d2d_factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let write_factory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let target = create_target(&d2d_factory)?;
        Ok(Self {
            d2d_factory,
            write_factory,
            target,
        })
    }

    unsafe fn recreate_target(&mut self) -> WindowsResult<()> {
        self.target = create_target(&self.d2d_factory)?;
        Ok(())
    }

    unsafe fn text_format(
        &self,
        spec: &FontSpec,
        height: i32,
        centered: bool,
    ) -> WindowsResult<IDWriteTextFormat> {
        let family: Vec<u16> = spec
            .family
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let format = self.write_factory.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            DWRITE_FONT_WEIGHT(i32::from(spec.weight)),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            height.max(1) as f32,
            w!("en-us"),
        )?;
        format.SetTextAlignment(if centered {
            DWRITE_TEXT_ALIGNMENT_CENTER
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        })?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(format)
    }

    unsafe fn measure_text(&self, text: &str, spec: &FontSpec, height: i32) -> WindowsResult<i32> {
        if text.is_empty() {
            return Ok(0);
        }
        let format = self.text_format(spec, height, false)?;
        let text: Vec<u16> = text.encode_utf16().collect();
        let layout = self
            .write_factory
            .CreateTextLayout(&text, &format, 100_000.0, 100_000.0)?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics)?;
        Ok(metrics.widthIncludingTrailingWhitespace.ceil() as i32)
    }

    unsafe fn draw(
        &mut self,
        hdc: HDC,
        canvas_bounds: RECT,
        label_bounds: RECT,
        model: &LabelModel<'_>,
        style: LabelStyle,
        theme: &LabelTheme,
        transparent_color: Color,
    ) -> WindowsResult<()> {
        self.target.BindDC(hdc, &canvas_bounds)?;

        let layout = LabelLayout::new(
            model_rect(label_bounds),
            style,
            theme,
            model.class.is_some(),
        );
        let background_brush = self
            .target
            .CreateSolidColorBrush(&d2d_color(model.background), None)?;
        let badge_brush = self
            .target
            .CreateSolidColorBrush(&d2d_color(model.badge_background), None)?;
        let badge_text_brush = self
            .target
            .CreateSolidColorBrush(&d2d_color(theme.badge_text_color), None)?;
        let shadow_brush = self
            .target
            .CreateSolidColorBrush(&d2d_color(theme.text_shadow_color), None)?;
        let text_brush = self
            .target
            .CreateSolidColorBrush(&d2d_color(theme.text_color), None)?;
        let badge_format =
            self.text_format(&theme.badge_font, style.badge_font_height(theme), true)?;
        let name_format =
            self.text_format(&theme.name_font, style.name_font_height(theme), false)?;
        let badge_text: Vec<u16> = model.number.to_string().encode_utf16().collect();
        let text: Vec<u16> = model.text.encode_utf16().collect();

        self.target.BeginDraw();
        self.target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        self.target
            .SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        self.target.Clear(Some(&d2d_color(transparent_color)));

        let background = rounded_rect(layout.background, layout.corner_radius);
        self.target
            .FillRoundedRectangle(&background, &background_brush);

        let badge = ellipse(layout.badge);
        self.target.FillEllipse(&badge, &badge_brush);
        self.target.DrawText(
            &badge_text,
            &badge_format,
            &d2d_rect(layout.badge),
            &badge_text_brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );

        if !text.is_empty() {
            self.target.DrawText(
                &text,
                &name_format,
                &d2d_rect(layout.text_shadow),
                &shadow_brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            self.target.DrawText(
                &text,
                &name_format,
                &d2d_rect(layout.text),
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }

        self.target.EndDraw(None, None)?;

        if let (Some(class), Some(icon)) = (model.class, layout.icon) {
            let _ =
                crate::class_icons::draw_class_icon(hdc, class, icon.left, icon.top, icon.width());
        }
        Ok(())
    }
}

unsafe fn create_target(factory: &ID2D1Factory) -> WindowsResult<ID2D1DCRenderTarget> {
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        // LabelStyle already converts every coordinate to physical pixels.
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    factory.CreateDCRenderTarget(&properties)
}

fn renderer() -> &'static Mutex<Option<Renderer>> {
    static RENDERER: OnceLock<Mutex<Option<Renderer>>> = OnceLock::new();
    RENDERER.get_or_init(|| Mutex::new(None))
}

unsafe fn with_renderer<T>(operation: impl FnOnce(&mut Renderer) -> WindowsResult<T>) -> Option<T> {
    let mut renderer = renderer().lock().ok()?;
    if renderer.is_none() {
        *renderer = Renderer::new().ok();
    }
    let renderer = renderer.as_mut()?;
    match operation(renderer) {
        Ok(value) => Some(value),
        Err(_) => {
            let _ = renderer.recreate_target();
            None
        }
    }
}

pub(super) unsafe fn measure_label_width(
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    max_width: i32,
) -> Option<i32> {
    with_renderer(|renderer| {
        let text_width =
            renderer.measure_text(model.text, &theme.name_font, style.name_font_height(theme))?;
        Ok(required_width(
            text_width,
            style,
            theme,
            model.class.is_some(),
            max_width,
        ))
    })
}

pub(super) unsafe fn draw_label(
    hdc: HDC,
    canvas_bounds: RECT,
    label_bounds: RECT,
    model: &LabelModel<'_>,
    style: LabelStyle,
    theme: &LabelTheme,
    transparent_color: Color,
) -> bool {
    with_renderer(|renderer| {
        renderer.draw(
            hdc,
            canvas_bounds,
            label_bounds,
            model,
            style,
            theme,
            transparent_color,
        )
    })
    .is_some()
}

fn d2d_color(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(color.red) / 255.0,
        g: f32::from(color.green) / 255.0,
        b: f32::from(color.blue) / 255.0,
        a: 1.0,
    }
}

fn model_rect(value: RECT) -> Rect {
    Rect::new(value.left, value.top, value.right, value.bottom)
}

fn d2d_rect(value: Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: value.left as f32,
        top: value.top as f32,
        right: value.right as f32,
        bottom: value.bottom as f32,
    }
}

fn rounded_rect(value: Rect, radius: i32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: d2d_rect(value),
        radiusX: radius as f32,
        radiusY: radius as f32,
    }
}

fn ellipse(value: Rect) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: D2D_POINT_2F {
            x: (value.left + value.right) as f32 / 2.0,
            y: (value.top + value.bottom) as f32 / 2.0,
        },
        radiusX: value.width() as f32 / 2.0,
        radiusY: value.height() as f32 / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };

    fn sample_model() -> LabelModel<'static> {
        LabelModel {
            text: "Bilka",
            class: None,
            number: 1,
            background: Color {
                red: 74,
                green: 134,
                blue: 212,
            },
            badge_background: Color {
                red: 48,
                green: 104,
                blue: 176,
            },
        }
    }

    #[test]
    fn initializes_directwrite_and_measures_a_label() {
        let width = unsafe {
            measure_label_width(
                &sample_model(),
                LabelStyle::new(1.0, 48),
                &LabelTheme::default(),
                500,
            )
        };
        assert!(width.is_some_and(|width| width > 64));
    }

    #[test]
    fn binds_a_dc_target_and_draws_the_shared_layout() {
        const WIDTH: i32 = 220;
        const HEIGHT: i32 = 48;
        unsafe {
            let hdc = CreateCompatibleDC(None);
            assert!(!hdc.0.is_null());
            let bitmap_info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: WIDTH,
                    biHeight: -HEIGHT,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
                .expect("create Direct2D test bitmap");
            let old_bitmap = SelectObject(hdc, bitmap);
            let bounds = RECT {
                left: 0,
                top: 0,
                right: WIDTH,
                bottom: HEIGHT,
            };

            assert!(draw_label(
                hdc,
                bounds,
                bounds,
                &sample_model(),
                LabelStyle::new(1.0, HEIGHT),
                &LabelTheme::default(),
                Color {
                    red: 255,
                    green: 0,
                    blue: 255,
                },
            ));
            let pixels =
                std::slice::from_raw_parts(bits.cast::<u8>(), (WIDTH * HEIGHT * 4) as usize);
            let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
            assert_ne!(&pixels[center..center + 3], &[255, 0, 255]);

            let _ = SelectObject(hdc, old_bitmap);
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(hdc);
        }
    }

    #[test]
    fn converts_renderer_independent_geometry_and_color() {
        let color = d2d_color(Color {
            red: 0x40,
            green: 0x80,
            blue: 0xff,
        });
        assert!((color.r - 64.0 / 255.0).abs() < f32::EPSILON);
        assert!((color.g - 128.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(color.b, 1.0);
        assert_eq!(color.a, 1.0);

        let rect = Rect::new(4, 6, 44, 26);
        assert_eq!(
            ellipse(rect),
            D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: 24.0, y: 16.0 },
                radiusX: 20.0,
                radiusY: 10.0,
            }
        );
    }
}
