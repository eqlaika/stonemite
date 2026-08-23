use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW,
};

const FALLBACK_FAMILIES: &[&str] = &[
    "Arial",
    "Consolas",
    "Georgia",
    "Segoe UI",
    "Tahoma",
    "Trebuchet MS",
    "Verdana",
];

pub fn installed_font_families() -> Vec<String> {
    let mut families = Vec::new();
    unsafe {
        let hdc = GetDC(HWND::default());
        if !hdc.0.is_null() {
            let request = LOGFONTW {
                lfCharSet: DEFAULT_CHARSET,
                ..Default::default()
            };
            EnumFontFamiliesExW(
                hdc,
                &request,
                Some(collect_font_family),
                LPARAM((&mut families as *mut Vec<String>) as isize),
                0,
            );
            let _ = ReleaseDC(HWND::default(), hdc);
        }
    }
    normalize_font_families(families)
}

unsafe extern "system" fn collect_font_family(
    logfont: *const LOGFONTW,
    _text_metrics: *const TEXTMETRICW,
    _font_type: u32,
    families: LPARAM,
) -> i32 {
    if logfont.is_null() || families.0 == 0 {
        return 1;
    }

    let face_name = &(*logfont).lfFaceName;
    let length = face_name
        .iter()
        .position(|code_unit| *code_unit == 0)
        .unwrap_or(face_name.len());
    let family = String::from_utf16_lossy(&face_name[..length]);
    (*(families.0 as *mut Vec<String>)).push(family);
    1
}

fn normalize_font_families(mut families: Vec<String>) -> Vec<String> {
    families.retain(|family| !family.is_empty() && !family.starts_with('@'));
    families.sort_by_key(|family| family.to_lowercase());
    families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));

    if families.is_empty() {
        FALLBACK_FAMILIES
            .iter()
            .map(|family| (*family).to_owned())
            .collect()
    } else {
        families
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_families_are_sorted_deduplicated_and_hide_vertical_variants() {
        assert_eq!(
            normalize_font_families(vec![
                "Verdana".to_owned(),
                "@Arial".to_owned(),
                "arial".to_owned(),
                "Arial".to_owned(),
                String::new(),
            ]),
            vec!["arial".to_owned(), "Verdana".to_owned()]
        );
    }

    #[test]
    fn installed_catalog_exposes_multiple_selectable_families() {
        let families = installed_font_families();
        assert!(families.len() > 1);
        assert!(families.iter().all(|family| !family.starts_with('@')));
    }

    #[test]
    fn font_family_fallback_contains_the_established_default() {
        let families = normalize_font_families(Vec::new());
        assert!(families.iter().any(|family| family == "Segoe UI"));
    }
}
