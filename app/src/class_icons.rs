use std::collections::HashMap;
use std::sync::OnceLock;

use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, SelectObject, SetStretchBltMode,
    StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HALFTONE, SRCCOPY,
};

/// A decoded class icon ready for GDI blitting.
struct ClassIcon {
    hbitmap: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
}

// SAFETY: The HBITMAPs are created once during init and only read via StretchBlt afterwards.
unsafe impl Send for ClassIcon {}
unsafe impl Sync for ClassIcon {}

// Embed all 16 class icon PNGs.
const BRD_PNG: &[u8] = include_bytes!("../assets/class_icons/brd.png");
const BST_PNG: &[u8] = include_bytes!("../assets/class_icons/bst.png");
const BER_PNG: &[u8] = include_bytes!("../assets/class_icons/ber.png");
const CLR_PNG: &[u8] = include_bytes!("../assets/class_icons/clr.png");
const DRU_PNG: &[u8] = include_bytes!("../assets/class_icons/dru.png");
const ENC_PNG: &[u8] = include_bytes!("../assets/class_icons/enc.png");
const MAG_PNG: &[u8] = include_bytes!("../assets/class_icons/mag.png");
const MNK_PNG: &[u8] = include_bytes!("../assets/class_icons/mnk.png");
const NEC_PNG: &[u8] = include_bytes!("../assets/class_icons/nec.png");
const PAL_PNG: &[u8] = include_bytes!("../assets/class_icons/pal.png");
const RNG_PNG: &[u8] = include_bytes!("../assets/class_icons/rng.png");
const ROG_PNG: &[u8] = include_bytes!("../assets/class_icons/rog.png");
const SHK_PNG: &[u8] = include_bytes!("../assets/class_icons/shk.png");
const SHM_PNG: &[u8] = include_bytes!("../assets/class_icons/shm.png");
const WAR_PNG: &[u8] = include_bytes!("../assets/class_icons/war.png");
const WIZ_PNG: &[u8] = include_bytes!("../assets/class_icons/wiz.png");

static ICONS: OnceLock<HashMap<&'static str, ClassIcon>> = OnceLock::new();

/// Decode a PNG from memory and create a GDI HBITMAP (top-down BGRA DIB).
fn load_icon(png_bytes: &[u8]) -> ClassIcon {
    let img = image::load_from_memory(png_bytes)
        .expect("failed to decode class icon")
        .to_rgba8();
    let (w, h) = (img.width() as i32, img.height() as i32);

    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbitmap = CreateDIBSection(
            None,
            &bmi,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ).expect("CreateDIBSection failed for class icon");

        // Copy pixels, converting RGBA → BGRA.
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        for (src_px, dst_px) in img.pixels().zip(dst.chunks_exact_mut(4)) {
            let [r, g, b, a] = src_px.0;
            dst_px[0] = b;
            dst_px[1] = g;
            dst_px[2] = r;
            dst_px[3] = a;
        }

        ClassIcon { hbitmap, width: w, height: h }
    }
}

fn icons() -> &'static HashMap<&'static str, ClassIcon> {
    ICONS.get_or_init(|| {
        let pairs: Vec<(&str, &[u8])> = vec![
            ("BRD", BRD_PNG), ("BST", BST_PNG), ("BER", BER_PNG), ("CLR", CLR_PNG),
            ("DRU", DRU_PNG), ("ENC", ENC_PNG), ("MAG", MAG_PNG), ("MNK", MNK_PNG),
            ("NEC", NEC_PNG), ("PAL", PAL_PNG), ("RNG", RNG_PNG), ("ROG", ROG_PNG),
            ("SHK", SHK_PNG), ("SHM", SHM_PNG), ("WAR", WAR_PNG), ("WIZ", WIZ_PNG),
        ];
        pairs.into_iter().map(|(k, v)| (k, load_icon(v))).collect()
    })
}

/// Draw a class icon scaled to fit within a square of `size` pixels at (x, y).
/// Returns `true` if an icon was drawn.
pub unsafe fn draw_class_icon(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    class_abbrev: &str,
    x: i32,
    y: i32,
    size: i32,
) -> bool {
    let Some(icon) = icons().get(class_abbrev) else { return false; };

    let mem_dc = CreateCompatibleDC(hdc);
    let old = SelectObject(mem_dc, icon.hbitmap);
    let _ = SetStretchBltMode(hdc, HALFTONE);
    let _ = StretchBlt(
        hdc, x, y, size, size,
        mem_dc, 0, 0, icon.width, icon.height,
        SRCCOPY,
    );
    let _ = SelectObject(mem_dc, old);
    let _ = DeleteDC(mem_dc);
    true
}

