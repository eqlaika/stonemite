use std::collections::HashMap;
use std::sync::OnceLock;

/// Immutable decoded class-icon pixels shared by render backends.
///
/// Pixels remain straight-alpha RGBA on the CPU. Direct2D upload converts them
/// to premultiplied BGRA for the device generation that owns the bitmap.
pub(crate) struct ClassIconData {
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
}

impl ClassIconData {
    pub(crate) fn premultiplied_bgra(&self) -> Vec<u8> {
        premultiplied_bgra(&self.rgba)
    }
}

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

const ICON_PNGS: &[(&str, &[u8])] = &[
    ("BRD", BRD_PNG),
    ("BST", BST_PNG),
    ("BER", BER_PNG),
    ("CLR", CLR_PNG),
    ("DRU", DRU_PNG),
    ("ENC", ENC_PNG),
    ("MAG", MAG_PNG),
    ("MNK", MNK_PNG),
    ("NEC", NEC_PNG),
    ("PAL", PAL_PNG),
    ("RNG", RNG_PNG),
    ("ROG", ROG_PNG),
    ("SHK", SHK_PNG),
    ("SHM", SHM_PNG),
    ("WAR", WAR_PNG),
    ("WIZ", WIZ_PNG),
];

static ICONS: OnceLock<HashMap<&'static str, ClassIconData>> = OnceLock::new();

fn decode_icon(png_bytes: &[u8]) -> ClassIconData {
    let image = image::load_from_memory(png_bytes)
        .expect("failed to decode class icon")
        .to_rgba8();
    ClassIconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw().into_boxed_slice(),
    }
}

fn icons() -> &'static HashMap<&'static str, ClassIconData> {
    ICONS.get_or_init(|| {
        ICON_PNGS
            .iter()
            .map(|&(abbreviation, png)| (abbreviation, decode_icon(png)))
            .collect()
    })
}

/// Look up a decoded icon and its stable catalog key.
pub(crate) fn class_icon(
    class_abbreviation: &str,
) -> Option<(&'static str, &'static ClassIconData)> {
    icons()
        .get_key_value(class_abbreviation)
        .map(|(&abbreviation, icon)| (abbreviation, icon))
}

/// Convert straight RGBA bytes to Direct2D's BGRA8 premultiplied layout.
pub(crate) fn premultiplied_bgra(rgba: &[u8]) -> Vec<u8> {
    assert_eq!(rgba.len() % 4, 0, "RGBA pixels must be four-byte aligned");
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        let premultiply = |component: u8| ((u16::from(component) * alpha + 127) / 255) as u8;
        bgra.extend_from_slice(&[
            premultiply(pixel[2]),
            premultiply(pixel[1]),
            premultiply(pixel[0]),
            pixel[3],
        ]);
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_every_embedded_class_icon() {
        assert_eq!(icons().len(), ICON_PNGS.len());
        for &(abbreviation, _) in ICON_PNGS {
            let (key, icon) = class_icon(abbreviation).expect("embedded class icon");
            assert_eq!(key, abbreviation);
            assert!(icon.width > 0);
            assert!(icon.height > 0);
            assert_eq!(
                icon.rgba.len(),
                (icon.width * icon.height * 4) as usize,
                "{abbreviation} must contain one straight RGBA pixel per texel"
            );
        }
        assert!(class_icon("UNKNOWN").is_none());
    }

    #[test]
    fn premultiplied_bgra_handles_opaque_transparent_and_partial_alpha() {
        let rgba = [
            10, 20, 30, 255, // opaque
            200, 100, 50, 128, // half alpha
            255, 127, 63, 0, // transparent color channels are cleared
        ];
        assert_eq!(
            premultiplied_bgra(&rgba),
            vec![30, 20, 10, 255, 25, 50, 100, 128, 0, 0, 0, 0]
        );
    }

    #[test]
    fn icon_method_uses_the_same_premultiplied_conversion() {
        let (_, icon) = class_icon("WAR").expect("warrior icon");
        let converted = icon.premultiplied_bgra();
        assert_eq!(converted.len(), icon.rgba.len());
        assert_eq!(converted.len(), (icon.width * icon.height * 4) as usize);
    }
}
