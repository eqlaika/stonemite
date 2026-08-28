//! Resolve semantic EverQuest actions through the effective `[KeyMaps]` profile.
//!
//! Live EQ stores global overrides in `eqclient.ini`. Since October 2025,
//! character/persona profiles can store sparse overriding keymaps in
//! `<Character>_<server>_<class>.ini`. An older `<Character>_<server>.ini`
//! profile is also supported as an intermediate compatibility layer.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use trushar::control::{EqAction, EqMappingName, MAX_HOTBAR_BUTTONS, MAX_SPELL_GEMS};

const ALT_FLAG: u32 = 0x1000_0000;
const CONTROL_FLAG: u32 = 0x2000_0000;
const SHIFT_FLAG: u32 = 0x4000_0000;
const KNOWN_MAPPING_MASK: u32 = ALT_FLAG | CONTROL_FLAG | SHIFT_FLAG | 0x0000_ffff;

const LEFT_CONTROL_SCAN: u8 = 0x1d;
const LEFT_SHIFT_SCAN: u8 = 0x2a;
const LEFT_ALT_SCAN: u8 = 0x38;

#[derive(Clone, Copy, Debug, Default)]
pub struct ClientIdentity<'a> {
    pub character: Option<&'a str>,
    pub server: Option<&'a str>,
    pub class_code: Option<&'a str>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ResolveError {
    Unbound,
    Read(String),
    Malformed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedBinding {
    pub scans: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KeymapFile {
    values: HashMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Clone, Debug)]
struct CachedFile {
    stamp: FileStamp,
    keymaps: Option<KeymapFile>,
}

pub struct EqKeymapResolver {
    eq_dir: PathBuf,
    cache: HashMap<PathBuf, CachedFile>,
}

impl EqKeymapResolver {
    pub fn new(eq_dir: PathBuf) -> Self {
        Self {
            eq_dir,
            cache: HashMap::new(),
        }
    }

    pub fn resolve(
        &mut self,
        action: &EqAction,
        identity: ClientIdentity<'_>,
    ) -> Result<ResolvedBinding, ResolveError> {
        action
            .validate()
            .map_err(|error| ResolveError::Malformed(error.message))?;
        self.resolve_mapping(&action.mapping_name(), identity)
    }

    pub fn resolve_mapping(
        &mut self,
        mapping: &EqMappingName,
        identity: ClientIdentity<'_>,
    ) -> Result<ResolvedBinding, ResolveError> {
        let keymaps = self.effective_keymaps(identity)?;
        resolve_from_keymaps(mapping, keymaps.as_ref())
    }

    pub fn mapped_actions(
        &mut self,
        identity: ClientIdentity<'_>,
    ) -> Result<BTreeSet<EqMappingName>, ResolveError> {
        let keymaps = self.effective_keymaps(identity)?;
        let mut candidates = known_default_mapping_names();
        if let Some(keymaps) = &keymaps {
            for key in keymaps.values.keys() {
                if let Some(mapping) = mapping_name_from_key(key) {
                    candidates.insert(mapping);
                }
            }
        }

        let mut mapped = BTreeSet::new();
        for mapping in candidates {
            match resolve_from_keymaps(&mapping, keymaps.as_ref()) {
                Ok(_) => {
                    mapped.insert(mapping);
                }
                Err(ResolveError::Unbound) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(mapped)
    }

    /// EQ stores character/persona `[KeyMaps]` sections as sparse overlays.
    /// Build the effective map from the shared file through the legacy
    /// character profile to the current class/persona profile, preserving an
    /// explicit zero while inheriting entries that a more specific file omits.
    fn effective_keymaps(
        &mut self,
        identity: ClientIdentity<'_>,
    ) -> Result<Option<KeymapFile>, ResolveError> {
        let mut effective = self.load_keymaps(&self.eq_dir.join("eqclient.ini"))?;
        for path in self.profile_candidates(identity).into_iter().rev() {
            let Some(profile) = self.load_keymaps(&path)? else {
                continue;
            };
            if let Some(effective) = &mut effective {
                effective.values.extend(profile.values);
            } else {
                effective = Some(profile);
            }
        }
        Ok(effective)
    }

    fn profile_candidates(&self, identity: ClientIdentity<'_>) -> Vec<PathBuf> {
        let (Some(character), Some(server)) = (identity.character, identity.server) else {
            return Vec::new();
        };
        if !valid_component(character) || !valid_component(server) {
            return Vec::new();
        }

        let mut names = Vec::with_capacity(2);
        if let Some(class_code) = identity.class_code.filter(|value| valid_component(value)) {
            names.push(format!(
                "{character}_{server}_{}.ini",
                profile_class_code(class_code)
            ));
        }
        names.push(format!("{character}_{server}.ini"));
        names
            .into_iter()
            .map(|name| self.find_case_insensitive(&name))
            .collect()
    }

    fn find_case_insensitive(&self, name: &str) -> PathBuf {
        let direct = self.eq_dir.join(name);
        if direct.exists() {
            return direct;
        }
        fs::read_dir(&self.eq_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .find_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .filter(|candidate| candidate.eq_ignore_ascii_case(name))
                    .map(|_| entry.path())
            })
            .unwrap_or(direct)
    }

    fn load_keymaps(&mut self, path: &Path) -> Result<Option<KeymapFile>, ResolveError> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.cache.remove(path);
                return Ok(None);
            }
            Err(error) => {
                return Err(ResolveError::Read(format!(
                    "failed to inspect {}: {error}",
                    path.display()
                )))
            }
        };
        let stamp = FileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        };
        if let Some(cached) = self.cache.get(path).filter(|cached| cached.stamp == stamp) {
            return Ok(cached.keymaps.clone());
        }

        let contents = fs::read(path).map_err(|error| {
            ResolveError::Read(format!("failed to read {}: {error}", path.display()))
        })?;
        let contents = String::from_utf8_lossy(&contents);
        let keymaps = parse_keymaps(&contents);
        self.cache.insert(
            path.to_path_buf(),
            CachedFile {
                stamp,
                keymaps: keymaps.clone(),
            },
        );
        Ok(keymaps)
    }
}

fn profile_class_code(class_code: &str) -> &str {
    if class_code.eq_ignore_ascii_case("SHK") {
        "SHD"
    } else {
        class_code
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-'))
}

fn parse_keymaps(contents: &str) -> Option<KeymapFile> {
    let mut in_keymaps = false;
    let mut found = false;
    let mut values = HashMap::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') {
            in_keymaps = line[1..line.len() - 1].eq_ignore_ascii_case("KeyMaps");
            found |= in_keymaps;
            continue;
        }
        if !in_keymaps || line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        values.insert(key.trim().to_ascii_uppercase(), value.trim().to_owned());
    }

    found.then_some(KeymapFile { values })
}

fn parse_mapping(value: &str, name: &str) -> Result<u32, ResolveError> {
    value
        .parse::<u32>()
        .map_err(|_| ResolveError::Malformed(format!("{name} is not a valid 32-bit key mapping")))
}

fn decode_mapping(encoded: u32, name: &str) -> Result<ResolvedBinding, ResolveError> {
    if encoded & !KNOWN_MAPPING_MASK != 0 {
        return Err(ResolveError::Malformed(format!(
            "{name} contains unsupported modifier flags"
        )));
    }
    let raw_scan = encoded & 0xffff;
    let scan = u8::try_from(raw_scan)
        .ok()
        .filter(|scan| (1..=254).contains(scan));
    let Some(scan) = scan else {
        return Err(ResolveError::Malformed(format!(
            "{name} contains an invalid DirectInput scan code"
        )));
    };

    let mut scans = Vec::with_capacity(4);
    if encoded & CONTROL_FLAG != 0 {
        scans.push(LEFT_CONTROL_SCAN);
    }
    if encoded & SHIFT_FLAG != 0 {
        scans.push(LEFT_SHIFT_SCAN);
    }
    if encoded & ALT_FLAG != 0 {
        scans.push(LEFT_ALT_SCAN);
    }
    if !scans.contains(&scan) {
        scans.push(scan);
    }
    Ok(ResolvedBinding { scans })
}

fn resolve_from_keymaps(
    mapping: &EqMappingName,
    keymaps: Option<&KeymapFile>,
) -> Result<ResolvedBinding, ResolveError> {
    for binding in 1..=2 {
        let name = format!("KEYMAPPING_{}_{binding}", mapping.as_str());
        let encoded = match keymaps.and_then(|keymaps| keymaps.values.get(&name)) {
            Some(value) => parse_mapping(value, &name)?,
            None => default_mapping(mapping, binding).unwrap_or(0),
        };
        if encoded != 0 {
            return decode_mapping(encoded, &name);
        }
    }
    Err(ResolveError::Unbound)
}

fn mapping_name_from_key(key: &str) -> Option<EqMappingName> {
    let rest = key.strip_prefix("KEYMAPPING_")?;
    let (mapping, binding) = rest.rsplit_once('_')?;
    if !matches!(binding, "1" | "2") {
        return None;
    }
    EqMappingName::new(mapping).ok()
}

fn known_default_mapping_names() -> BTreeSet<EqMappingName> {
    let mut mappings = BTreeSet::new();
    for name in ["USE", "INVITE_FOLLOW"] {
        mappings.insert(EqMappingName::new(name).expect("literal mapping name"));
    }
    for button in 1..=MAX_HOTBAR_BUTTONS {
        mappings
            .insert(EqMappingName::new(format!("HOT1_{button}")).expect("generated mapping name"));
    }
    for gem in 1..=MAX_SPELL_GEMS {
        mappings.insert(EqMappingName::new(format!("CAST{gem}")).expect("generated mapping name"));
    }
    mappings
}

fn default_mapping(mapping: &EqMappingName, binding: u8) -> Option<u32> {
    if binding != 1 {
        return None;
    }
    match mapping.as_str() {
        "USE" => Some(0x16),                          // U
        "INVITE_FOLLOW" => Some(CONTROL_FLAG | 0x17), // Ctrl+I
        value if value.starts_with("HOT1_") => value
            .strip_prefix("HOT1_")
            .and_then(|button| button.parse::<u8>().ok())
            .and_then(number_row_scan)
            .map(u32::from),
        value if value.starts_with("CAST") => match value
            .strip_prefix("CAST")
            .and_then(|gem| gem.parse::<u8>().ok())
        {
            Some(gem @ 1..=12) => number_row_scan(gem).map(|scan| ALT_FLAG | u32::from(scan)),
            Some(13) => Some(ALT_FLAG | 0x16), // Alt+U
            Some(14) => Some(ALT_FLAG | 0x17), // Alt+I
            _ => None,
        },
        _ => None,
    }
}

fn number_row_scan(position: u8) -> Option<u8> {
    match position {
        1..=9 => Some(position + 1),
        10 => Some(0x0b), // 0
        11 => Some(0x0c), // -
        12 => Some(0x0d), // =
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "stonemite-keymaps-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, contents: &str) {
            fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolves_every_documented_default() {
        let dir = TestDir::new();
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        let mut defaults = vec![
            (EqAction::UseCenterScreen, vec![0x16]),
            (EqAction::InviteFollow, vec![LEFT_CONTROL_SCAN, 0x17]),
        ];
        let number_row_scans = [
            0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
        ];
        for (index, scan) in number_row_scans.into_iter().enumerate() {
            let position = u8::try_from(index + 1).unwrap();
            defaults.push((EqAction::hotbar(1, position).unwrap(), vec![scan]));
            defaults.push((
                EqAction::spell_gem(position).unwrap(),
                vec![LEFT_ALT_SCAN, scan],
            ));
        }
        defaults.push((EqAction::spell_gem(13).unwrap(), vec![LEFT_ALT_SCAN, 0x16]));
        defaults.push((EqAction::spell_gem(14).unwrap(), vec![LEFT_ALT_SCAN, 0x17]));

        for (action, scans) in defaults {
            assert_eq!(
                resolver.resolve(&action, ClientIdentity::default()),
                Ok(ResolvedBinding { scans }),
                "default for {action:?}"
            );
        }
        assert_eq!(
            resolver.resolve(
                &EqAction::hotbar(11, 12).unwrap(),
                ClientIdentity::default()
            ),
            Err(ResolveError::Unbound)
        );
    }

    #[test]
    fn global_overrides_decode_modifiers_and_alternates() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[KeyMaps]\nKEYMAPPING_USE_1=0\nKEYMAPPING_USE_2=1610612795\n",
        );
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ClientIdentity::default()),
            Ok(ResolvedBinding {
                scans: vec![LEFT_CONTROL_SCAN, LEFT_SHIFT_SCAN, 0x3b]
            })
        );
    }

    #[test]
    fn decodes_all_modifiers_and_rejects_invalid_raw_values() {
        assert_eq!(
            decode_mapping(ALT_FLAG | 0x1e, "ALT_ONLY"),
            Ok(ResolvedBinding {
                scans: vec![LEFT_ALT_SCAN, 0x1e]
            })
        );
        assert_eq!(
            decode_mapping(CONTROL_FLAG | SHIFT_FLAG | ALT_FLAG | 0x1e, "ALL_MODIFIERS"),
            Ok(ResolvedBinding {
                scans: vec![LEFT_CONTROL_SCAN, LEFT_SHIFT_SCAN, LEFT_ALT_SCAN, 0x1e]
            })
        );
        for encoded in [0, 255, 256, 0x8000_001e] {
            assert!(
                matches!(
                    decode_mapping(encoded, "INVALID"),
                    Err(ResolveError::Malformed(_))
                ),
                "encoded value {encoded:#x}"
            );
        }
    }

    #[test]
    fn accepts_a_utf8_bom_before_the_keymaps_section() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "\u{feff}[KeyMaps]\nKEYMAPPING_USE_1=18\n");
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ClientIdentity::default()),
            Ok(ResolvedBinding { scans: vec![18] })
        );
    }

    #[test]
    fn persona_profile_is_selected_case_insensitively_per_client() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_USE_1=18\n");
        dir.write("Laika_xegony_RNG.ini", "[KeyMaps]\nKEYMAPPING_USE_1=19\n");
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        let ranger = ClientIdentity {
            character: Some("laika"),
            server: Some("XEGONY"),
            class_code: Some("rng"),
        };
        let shadowknight = ClientIdentity {
            class_code: Some("SHD"),
            ..ranger
        };
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ranger),
            Ok(ResolvedBinding { scans: vec![19] })
        );
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, shadowknight),
            Ok(ResolvedBinding { scans: vec![18] })
        );
    }

    #[test]
    fn shadow_knight_ui_code_resolves_the_shd_profile_suffix() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_USE_1=18\n");
        dir.write("Laika_xegony_SHD.ini", "[KeyMaps]\nKEYMAPPING_USE_1=21\n");
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(
                &EqAction::UseCenterScreen,
                ClientIdentity {
                    character: Some("Laika"),
                    server: Some("xegony"),
                    class_code: Some("SHK"),
                },
            ),
            Ok(ResolvedBinding { scans: vec![21] })
        );
    }

    #[test]
    fn legacy_character_profile_is_supported() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_USE_1=18\n");
        dir.write("Laika_xegony.ini", "[KeyMaps]\nKEYMAPPING_USE_1=20\n");
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(
                &EqAction::UseCenterScreen,
                ClientIdentity {
                    character: Some("Laika"),
                    server: Some("xegony"),
                    class_code: None,
                },
            ),
            Ok(ResolvedBinding { scans: vec![20] })
        );
    }

    #[test]
    fn sparse_persona_profile_inherits_global_binding() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_HOT2_2_1=536870915\n");
        dir.write(
            "Mudkip_frostreaver_BRD.ini",
            "[KeyMaps]\nKEYMAPPING_MOUSELOOK_1=0\nKEYMAPPING_STRAFE_RIGHT_1=0\n",
        );
        let identity = ClientIdentity {
            character: Some("Mudkip"),
            server: Some("frostreaver"),
            class_code: Some("BRD"),
        };
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::hotbar(2, 2).unwrap(), identity),
            Ok(ResolvedBinding {
                scans: vec![LEFT_CONTROL_SCAN, 0x03]
            })
        );
        assert!(resolver
            .mapped_actions(identity)
            .unwrap()
            .contains(&EqMappingName::new("HOT2_2").unwrap()));
    }

    #[test]
    fn sparse_persona_profile_can_explicitly_unbind_a_global_binding() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_HOT2_2_1=536870915\n");
        dir.write(
            "Mudkip_frostreaver_BRD.ini",
            "[KeyMaps]\nKEYMAPPING_HOT2_2_1=0\n",
        );
        let identity = ClientIdentity {
            character: Some("Mudkip"),
            server: Some("frostreaver"),
            class_code: Some("BRD"),
        };
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::hotbar(2, 2).unwrap(), identity),
            Err(ResolveError::Unbound)
        );
        assert!(!resolver
            .mapped_actions(identity)
            .unwrap()
            .contains(&EqMappingName::new("HOT2_2").unwrap()));
    }

    #[test]
    fn persona_overrides_legacy_and_global_entries_independently() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[KeyMaps]\nKEYMAPPING_USE_1=18\nKEYMAPPING_DUCK_1=44\n",
        );
        dir.write(
            "Laika_xegony.ini",
            "[KeyMaps]\nKEYMAPPING_USE_1=20\nKEYMAPPING_SIT_STAND_1=45\n",
        );
        dir.write("Laika_xegony_RNG.ini", "[KeyMaps]\nKEYMAPPING_USE_1=21\n");
        let identity = ClientIdentity {
            character: Some("Laika"),
            server: Some("xegony"),
            class_code: Some("RNG"),
        };
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        for (mapping, scan) in [("USE", 21), ("SIT_STAND", 45), ("DUCK", 44)] {
            assert_eq!(
                resolver.resolve(&EqAction::keymap(mapping).unwrap(), identity),
                Ok(ResolvedBinding { scans: vec![scan] })
            );
        }
    }

    #[test]
    fn explicit_zero_can_leave_an_action_unbound() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[KeyMaps]\nKEYMAPPING_USE_1=0\nKEYMAPPING_USE_2=0\n",
        );
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ClientIdentity::default()),
            Err(ResolveError::Unbound)
        );
    }

    #[test]
    fn resolves_every_supported_hotbar_and_spell_gem_identifier() {
        let dir = TestDir::new();
        let mut contents = String::from("[KeyMaps]\n");
        for bar in 1..=trushar::control::MAX_HOTBARS {
            for button in 1..=trushar::control::MAX_HOTBAR_BUTTONS {
                contents.push_str(&format!("KEYMAPPING_HOT{bar}_{button}_1=30\n"));
            }
        }
        for gem in 1..=trushar::control::MAX_SPELL_GEMS {
            contents.push_str(&format!("KEYMAPPING_CAST{gem}_1=31\n"));
        }
        dir.write("eqclient.ini", &contents);
        let mut resolver = EqKeymapResolver::new(dir.0.clone());

        for bar in 1..=trushar::control::MAX_HOTBARS {
            for button in 1..=trushar::control::MAX_HOTBAR_BUTTONS {
                assert_eq!(
                    resolver.resolve(
                        &EqAction::hotbar(bar, button).unwrap(),
                        ClientIdentity::default()
                    ),
                    Ok(ResolvedBinding { scans: vec![30] })
                );
            }
        }
        for gem in 1..=trushar::control::MAX_SPELL_GEMS {
            assert_eq!(
                resolver.resolve(
                    &EqAction::spell_gem(gem).unwrap(),
                    ClientIdentity::default()
                ),
                Ok(ResolvedBinding { scans: vec![31] })
            );
        }
    }

    #[test]
    fn refreshes_cached_keymaps_when_the_file_changes() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_USE_1=18\n");
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ClientIdentity::default()),
            Ok(ResolvedBinding { scans: vec![18] })
        );

        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_USE_1=536870931\n");
        assert_eq!(
            resolver.resolve(&EqAction::UseCenterScreen, ClientIdentity::default()),
            Ok(ResolvedBinding {
                scans: vec![LEFT_CONTROL_SCAN, 19]
            })
        );
    }

    #[test]
    fn discovers_and_resolves_arbitrary_explicit_mappings() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[KeyMaps]\nKEYMAPPING_DUCK_1=44\nKEYMAPPING_SIT_STAND_1=0\nKEYMAPPING_CMD_TOGGLE_ADVANCED_LOOT_WIN_2=36\nKEYMAPPING_USE_1=0\nKEYMAPPING_USE_2=0\n",
        );
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        let mapped = resolver.mapped_actions(ClientIdentity::default()).unwrap();

        assert!(mapped.contains(&EqMappingName::new("DUCK").unwrap()));
        assert!(mapped.contains(&EqMappingName::new("CMD_TOGGLE_ADVANCED_LOOT_WIN").unwrap()));
        assert!(!mapped.contains(&EqMappingName::new("SIT_STAND").unwrap()));
        assert!(!mapped.contains(&EqMappingName::new("USE").unwrap()));
        assert!(mapped.contains(&EqMappingName::new("CAST14").unwrap()));
        assert_eq!(
            resolver.resolve(
                &EqAction::keymap("duck").unwrap(),
                ClientIdentity::default()
            ),
            Ok(ResolvedBinding { scans: vec![44] })
        );
        assert_eq!(
            resolver.resolve(
                &EqAction::keymap("cmd_toggle_advanced_loot_win").unwrap(),
                ClientIdentity::default()
            ),
            Ok(ResolvedBinding { scans: vec![36] })
        );
    }

    #[test]
    fn generic_mapping_discovery_uses_the_effective_persona_profile() {
        let dir = TestDir::new();
        dir.write("eqclient.ini", "[KeyMaps]\nKEYMAPPING_DUCK_1=44\n");
        dir.write(
            "Laika_xegony_RNG.ini",
            "[KeyMaps]\nKEYMAPPING_SIT_STAND_1=45\n",
        );
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        let mapped = resolver
            .mapped_actions(ClientIdentity {
                character: Some("Laika"),
                server: Some("xegony"),
                class_code: Some("RNG"),
            })
            .unwrap();
        assert!(mapped.contains(&EqMappingName::new("SIT_STAND").unwrap()));
        assert!(mapped.contains(&EqMappingName::new("DUCK").unwrap()));
    }

    #[test]
    fn rejects_malformed_relevant_mapping() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[KeyMaps]\nKEYMAPPING_CAST1_1=not-a-number\n",
        );
        let mut resolver = EqKeymapResolver::new(dir.0.clone());
        assert!(matches!(
            resolver.resolve(&EqAction::spell_gem(1).unwrap(), ClientIdentity::default()),
            Err(ResolveError::Malformed(_))
        ));
        assert!(matches!(
            resolver.mapped_actions(ClientIdentity::default()),
            Err(ResolveError::Malformed(_))
        ));
    }
}
