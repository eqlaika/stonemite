//! Resolve arbitrary EverQuest `[TextColors]` `User_N` RGB values.
//!
//! EQ stores global colors in `eqclient.ini` and may write per-character
//! overrides to `eqclient-<Character> (<server>).ini`. Components are merged
//! independently so a partial profile override still inherits the remaining
//! global RGB values.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChatColorId(u16);

impl ChatColorId {
    pub const fn new(id: u16) -> Option<Self> {
        if id == 0 {
            None
        } else {
            Some(Self(id))
        }
    }

    #[allow(dead_code)]
    pub const fn get(self) -> u16 {
        self.0
    }
}

pub const TELL_COLOR_ID: ChatColorId = ChatColorId(2);
pub const GROUP_COLOR_ID: ChatColorId = ChatColorId(3);
pub const RAID_COLOR_ID: ChatColorId = ChatColorId(72);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl RgbColor {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    /// Convert RGB to Win32 COLORREF (`0x00BBGGRR`).
    pub const fn colorref(self) -> u32 {
        self.red as u32 | ((self.green as u32) << 8) | ((self.blue as u32) << 16)
    }
}

/// Stock EQ incoming-Tell purple. A user's configured `User_2` wins.
pub const DEFAULT_TELL_COLOR: RgbColor = RgbColor::new(190, 40, 190);
/// Stonemite group green used when EQ has no effective `User_3` value.
pub const DEFAULT_GROUP_COLOR: RgbColor = RgbColor::new(106, 176, 96);
/// Stonemite raid amber used when EQ has no effective `User_72` value.
pub const DEFAULT_RAID_COLOR: RgbColor = RgbColor::new(224, 184, 72);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ColorComponents {
    red: Option<u8>,
    green: Option<u8>,
    blue: Option<u8>,
}

impl ColorComponents {
    fn overlay(&mut self, other: Self) {
        if other.red.is_some() {
            self.red = other.red;
        }
        if other.green.is_some() {
            self.green = other.green;
        }
        if other.blue.is_some() {
            self.blue = other.blue;
        }
    }

    fn complete(self) -> Option<RgbColor> {
        Some(RgbColor::new(self.red?, self.green?, self.blue?))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TextColorsFile {
    values: HashMap<ChatColorId, ColorComponents>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Clone, Debug)]
struct CachedFile {
    stamp: FileStamp,
    colors: Option<TextColorsFile>,
}

pub struct EqChatColorResolver {
    eq_dir: PathBuf,
    cache: HashMap<PathBuf, CachedFile>,
    profile_paths: HashMap<String, PathBuf>,
}

impl EqChatColorResolver {
    pub fn new(eq_dir: PathBuf) -> Self {
        Self {
            eq_dir,
            cache: HashMap::new(),
            profile_paths: HashMap::new(),
        }
    }

    pub fn set_eq_dir(&mut self, eq_dir: PathBuf) {
        if self.eq_dir != eq_dir {
            self.eq_dir = eq_dir;
            self.cache.clear();
            self.profile_paths.clear();
        }
    }

    /// Resolve any EQ `User_N` color for one character profile.
    pub fn resolve(
        &mut self,
        id: ChatColorId,
        character: &str,
        server: &str,
    ) -> Result<Option<RgbColor>, String> {
        let mut effective: HashMap<ChatColorId, ColorComponents> = HashMap::new();
        let global_path = self.eq_dir.join("eqclient.ini");
        if let Some(global) = self.load(&global_path)? {
            merge_colors(&mut effective, &global);
        }

        if let Some(profile_path) = self.profile_path(character, server) {
            if let Some(profile) = self.load(&profile_path)? {
                merge_colors(&mut effective, &profile);
            }
        }

        Ok(effective
            .get(&id)
            .copied()
            .and_then(ColorComponents::complete))
    }

    fn profile_path(&mut self, character: &str, server: &str) -> Option<PathBuf> {
        if !valid_component(character) || !valid_component(server) {
            return None;
        }
        let name = format!("eqclient-{character} ({server}).ini");
        let key = name.to_ascii_lowercase();
        if let Some(path) = self.profile_paths.get(&key) {
            return Some(path.clone());
        }
        let path = self.find_case_insensitive(&name);
        self.profile_paths.insert(key, path.clone());
        Some(path)
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

    fn load(&mut self, path: &Path) -> Result<Option<TextColorsFile>, String> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.cache.remove(path);
                return Ok(None);
            }
            Err(error) => {
                return Err(format!("failed to inspect {}: {error}", path.display()));
            }
        };
        let stamp = FileStamp {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        };
        if let Some(cached) = self.cache.get(path).filter(|cached| cached.stamp == stamp) {
            return Ok(cached.colors.clone());
        }

        let contents = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let colors = parse_text_colors(&String::from_utf8_lossy(&contents));
        self.cache.insert(
            path.to_path_buf(),
            CachedFile {
                stamp,
                colors: colors.clone(),
            },
        );
        Ok(colors)
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b' '))
}

fn merge_colors(effective: &mut HashMap<ChatColorId, ColorComponents>, file: &TextColorsFile) {
    for (id, components) in &file.values {
        effective.entry(*id).or_default().overlay(*components);
    }
}

fn parse_text_colors(contents: &str) -> Option<TextColorsFile> {
    let mut in_text_colors = false;
    let mut found = false;
    let mut values: HashMap<ChatColorId, ColorComponents> = HashMap::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') {
            in_text_colors = line[1..line.len() - 1].eq_ignore_ascii_case("TextColors");
            found |= in_text_colors;
            continue;
        }
        if !in_text_colors || line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let bytes = key.as_bytes();
        if bytes.len() < 6 || !bytes[..5].eq_ignore_ascii_case(b"User_") {
            continue;
        }
        let Some((raw_id, component)) = key[5..].rsplit_once('_') else {
            continue;
        };
        let Some(id) = raw_id.parse::<u16>().ok().and_then(ChatColorId::new) else {
            continue;
        };
        let Some(value) = raw_value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|value| *value <= u8::MAX as u16)
            .map(|value| value as u8)
        else {
            continue;
        };
        let entry = values.entry(id).or_default();
        if component.eq_ignore_ascii_case("Red") {
            entry.red = Some(value);
        } else if component.eq_ignore_ascii_case("Green") {
            entry.green = Some(value);
        } else if component.eq_ignore_ascii_case("Blue") {
            entry.blue = Some(value);
        }
    }

    found.then_some(TextColorsFile { values })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stonemite-chat-colors-{}-{id}", std::process::id()));
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
    fn parses_arbitrary_user_color_ids_and_ignores_malformed_values() {
        let parsed = parse_text_colors(
            "\u{feff}[TextColors]\n\
             User_2_Red=190\nUser_2_Green=40\nUser_2_Blue=190\n\
             User_152_Red=1\nUser_152_Green=2\nUser_152_Blue=3\n\
             User_3_Red=999\nUser_3_Green=nope\nUser_0_Blue=4\n",
        )
        .unwrap();

        assert_eq!(
            parsed.values[&TELL_COLOR_ID].complete(),
            Some(RgbColor::new(190, 40, 190))
        );
        assert_eq!(
            parsed.values[&ChatColorId::new(152).unwrap()].complete(),
            Some(RgbColor::new(1, 2, 3))
        );
        assert!(parsed
            .values
            .get(&ChatColorId::new(3).unwrap())
            .is_none_or(|color| color.complete().is_none()));
    }

    #[test]
    fn character_profile_overlays_global_components_case_insensitively() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[TextColors]\nUser_2_Red=190\nUser_2_Green=40\nUser_2_Blue=190\n",
        );
        dir.write(
            "EQCLIENT-LAIKA (XEGONY).INI",
            "[textcolors]\nuser_2_red=240\nuser_2_green=127\nuser_2_blue=0\n",
        );
        let mut resolver = EqChatColorResolver::new(dir.0.clone());

        assert_eq!(
            resolver.resolve(TELL_COLOR_ID, "Laika", "xegony").unwrap(),
            Some(RgbColor::new(240, 127, 0))
        );
    }

    #[test]
    fn partial_profile_color_inherits_missing_global_components() {
        let dir = TestDir::new();
        dir.write(
            "eqclient.ini",
            "[TextColors]\nUser_77_Red=10\nUser_77_Green=20\nUser_77_Blue=30\n",
        );
        dir.write(
            "eqclient-Bilka (teek).ini",
            "[TextColors]\nUser_77_Blue=99\n",
        );
        let mut resolver = EqChatColorResolver::new(dir.0.clone());

        assert_eq!(
            resolver
                .resolve(ChatColorId::new(77).unwrap(), "Bilka", "teek")
                .unwrap(),
            Some(RgbColor::new(10, 20, 99))
        );
    }

    #[test]
    fn missing_color_returns_none_for_the_consumer_to_fallback() {
        let dir = TestDir::new();
        let mut resolver = EqChatColorResolver::new(dir.0.clone());
        assert_eq!(
            resolver.resolve(TELL_COLOR_ID, "Bilka", "teek").unwrap(),
            None
        );
        assert_eq!(DEFAULT_TELL_COLOR.colorref(), 0x00BE28BE);
        assert_eq!(TELL_COLOR_ID.get(), 2);
    }
}
