//! Read saved Extended Target roles from the active character/persona profile.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::eq_keymap::ClientIdentity;

pub const MAX_XTARGET_SLOTS: u8 = 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XTargetSlot {
    pub slot: u8,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

#[derive(Clone, Debug)]
struct CachedFile {
    stamp: FileStamp,
    slots: Option<Vec<XTargetSlot>>,
}

pub struct EqXTargetResolver {
    eq_dir: PathBuf,
    cache: HashMap<PathBuf, CachedFile>,
}

impl EqXTargetResolver {
    pub fn new(eq_dir: PathBuf) -> Self {
        Self {
            eq_dir,
            cache: HashMap::new(),
        }
    }

    pub fn slots(&mut self, identity: ClientIdentity<'_>) -> Result<Vec<XTargetSlot>, String> {
        for path in self.profile_candidates(identity) {
            if let Some(slots) = self.load_slots(&path)? {
                return Ok(slots);
            }
        }
        Ok(Vec::new())
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

    fn load_slots(&mut self, path: &Path) -> Result<Option<Vec<XTargetSlot>>, String> {
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
            return Ok(cached.slots.clone());
        }
        let contents = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let contents = String::from_utf8_lossy(&contents);
        let slots = parse_external_target_roles(&contents);
        self.cache.insert(
            path.to_path_buf(),
            CachedFile {
                stamp,
                slots: slots.clone(),
            },
        );
        Ok(slots)
    }
}

fn parse_external_target_roles(contents: &str) -> Option<Vec<XTargetSlot>> {
    let mut in_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line[1..line.len() - 1].eq_ignore_ascii_case("ExternalTargetRoles");
            continue;
        }
        if !in_section || line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Current") {
            return Some(parse_current_roles(value.trim()));
        }
    }
    None
}

fn parse_current_roles(value: &str) -> Vec<XTargetSlot> {
    let fields = value.split('^').collect::<Vec<_>>();
    let declared_slots = fields
        .get(2)
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(MAX_XTARGET_SLOTS)
        .min(MAX_XTARGET_SLOTS);
    let mut slots = Vec::new();
    let mut index = 3;
    while index + 1 < fields.len() {
        let Some(zero_based_slot) = fields[index].parse::<u8>().ok() else {
            break;
        };
        let Some(role) = fields[index + 1].parse::<u8>().ok() else {
            break;
        };
        index += 2;
        let name = if matches!(role, 2 | 3) {
            let name = fields.get(index).copied().filter(|value| !value.is_empty());
            if name.is_some() {
                index += 1;
            }
            name
        } else {
            None
        };
        if zero_based_slot >= declared_slots || role == 0 {
            continue;
        }
        slots.push(XTargetSlot {
            slot: zero_based_slot + 1,
            label: role_label(role, name),
        });
    }
    slots.sort_by_key(|slot| slot.slot);
    slots.dedup_by_key(|slot| slot.slot);
    slots
}

fn role_label(role: u8, name: Option<&str>) -> String {
    match role {
        1 => "Auto hater".to_owned(),
        2 => name.unwrap_or("Specific PC").to_owned(),
        3 => name.unwrap_or("Specific NPC").to_owned(),
        4 => "Target's target".to_owned(),
        5 => "Group tank".to_owned(),
        6 => "Tank's target".to_owned(),
        7 => "Group assist".to_owned(),
        8 => "Assist target".to_owned(),
        9 => "Group puller".to_owned(),
        10 => "Puller target".to_owned(),
        11..=13 => format!("Group mark {}", role - 10),
        14..=16 => format!("Raid assist {}", role - 13),
        17..=19 => format!("Raid assist {} target", role - 16),
        20..=22 => format!("Raid mark {}", role - 19),
        23 => "My pet".to_owned(),
        24 => "My pet's target".to_owned(),
        25 => "My mercenary".to_owned(),
        26 => "My mercenary's target".to_owned(),
        _ => format!("Role {role}"),
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
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_named_and_role_slots_and_skips_empty_slots() {
        let slots = parse_external_target_roles(
            "[ExternalTargetRoles]\nCurrent=1^Autosave^6^0^2^Laika^1^0^2^1^3^5^4^8^5^1\n",
        )
        .unwrap();
        assert_eq!(
            slots,
            vec![
                XTargetSlot {
                    slot: 1,
                    label: "Laika".into(),
                },
                XTargetSlot {
                    slot: 3,
                    label: "Auto hater".into(),
                },
                XTargetSlot {
                    slot: 4,
                    label: "Group tank".into(),
                },
                XTargetSlot {
                    slot: 5,
                    label: "Assist target".into(),
                },
                XTargetSlot {
                    slot: 6,
                    label: "Auto hater".into(),
                },
            ]
        );
    }

    #[test]
    fn uses_persona_profile_before_legacy_and_maps_shaman_filename_code() {
        let root = std::env::temp_dir().join(format!(
            "stonemite-xtarget-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("Bilka_xegony.ini"),
            "[ExternalTargetRoles]\nCurrent=1^Autosave^1^0^1\n",
        )
        .unwrap();
        fs::write(
            root.join("Bilka_xegony_SHD.ini"),
            "[ExternalTargetRoles]\nCurrent=1^Autosave^1^0^2^Laika\n",
        )
        .unwrap();
        let mut resolver = EqXTargetResolver::new(root.clone());
        assert_eq!(
            resolver
                .slots(ClientIdentity {
                    character: Some("Bilka"),
                    server: Some("xegony"),
                    class_code: Some("SHK"),
                })
                .unwrap()[0]
                .label,
            "Laika"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
