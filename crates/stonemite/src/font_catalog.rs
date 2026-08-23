use windows::core::{Error as WindowsError, Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::{BOOL, E_FAIL};
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteLocalizedStrings, DWRITE_FACTORY_TYPE_SHARED,
};

const LOCALE_NAME_MAX_LENGTH: usize = 85;
const FALLBACK_LOCALE: &str = "en-us";

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
    let families = unsafe { directwrite_font_families() }.unwrap_or_default();
    normalize_font_families(families)
}

pub(crate) fn user_locale_name_wide() -> Vec<u16> {
    let mut locale = vec![0_u16; LOCALE_NAME_MAX_LENGTH];
    let length = unsafe { GetUserDefaultLocaleName(&mut locale) };
    if length > 1 {
        locale.truncate(length as usize);
        locale
    } else {
        FALLBACK_LOCALE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }
}

unsafe fn directwrite_font_families() -> WindowsResult<Vec<String>> {
    let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
    let user_locale = user_locale_name_wide();
    let mut collection = None;
    factory.GetSystemFontCollection(&mut collection, false)?;
    let collection = collection.ok_or_else(|| {
        WindowsError::new(
            E_FAIL,
            "DirectWrite did not return its system font collection",
        )
    })?;

    let mut families = Vec::with_capacity(collection.GetFontFamilyCount() as usize);
    for index in 0..collection.GetFontFamilyCount() {
        let Ok(family) = collection.GetFontFamily(index) else {
            continue;
        };
        let Ok(names) = family.GetFamilyNames() else {
            continue;
        };
        if let Ok(name) = localized_family_name(&names, &user_locale) {
            families.push(name);
        }
    }
    Ok(families)
}

unsafe fn localized_family_name(
    names: &IDWriteLocalizedStrings,
    user_locale: &[u16],
) -> WindowsResult<String> {
    let fallback_locale: Vec<u16> = FALLBACK_LOCALE
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut selected = None;
    for locale in [user_locale, fallback_locale.as_slice()] {
        let mut index = 0;
        let mut exists = BOOL::default();
        if names
            .FindLocaleName(PCWSTR(locale.as_ptr()), &mut index, &mut exists)
            .is_ok()
            && exists.as_bool()
        {
            selected = Some(index);
            break;
        }
    }

    let index = selected.unwrap_or(0);
    let length = names.GetStringLength(index)? as usize;
    let mut buffer = vec![0_u16; length + 1];
    names.GetString(index, &mut buffer)?;
    Ok(String::from_utf16_lossy(&buffer[..length]))
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
    fn user_locale_is_null_terminated_for_directwrite() {
        let locale = user_locale_name_wide();
        assert!(locale.len() > 1);
        assert_eq!(locale.last(), Some(&0));
    }

    #[test]
    fn font_families_are_sorted_deduplicated_and_hide_vertical_variants() {
        assert_eq!(
            normalize_font_families(vec![
                "Wingdings".to_owned(),
                "Verdana".to_owned(),
                "@Arial".to_owned(),
                "arial".to_owned(),
                "Arial".to_owned(),
                String::new(),
            ]),
            vec![
                "arial".to_owned(),
                "Verdana".to_owned(),
                "Wingdings".to_owned()
            ]
        );
    }

    #[test]
    fn directwrite_enumerates_real_system_font_families() {
        let families = unsafe { directwrite_font_families() }
            .expect("DirectWrite system font collection must be available");
        assert!(!families.is_empty());
    }

    #[test]
    fn installed_catalog_exposes_multiple_selectable_families() {
        let families = installed_font_families();
        assert!(families.len() > 1);
        assert!(families.iter().all(|family| !family.starts_with('@')));
        assert!(families
            .windows(2)
            .all(|pair| pair[0].to_lowercase() <= pair[1].to_lowercase()));

        // Wingdings is a supported configured family when Windows exposes it,
        // without assuming every test image installed that optional face.
        let wingdings = families
            .iter()
            .filter(|family| family.eq_ignore_ascii_case("Wingdings"))
            .count();
        assert!(wingdings <= 1);
    }

    #[test]
    fn font_family_fallback_contains_the_established_default() {
        let families = normalize_font_families(Vec::new());
        assert!(families.iter().any(|family| family == "Segoe UI"));
    }
}
