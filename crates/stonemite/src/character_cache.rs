use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Serialize, Deserialize, Clone)]
struct CharacterEntry {
    server: String,
    name: String,
    /// True only after this identity was assigned to a live local EQ process.
    #[serde(default, skip_serializing_if = "is_false")]
    owned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pet: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    #[serde(default)]
    characters: Vec<CharacterEntry>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub struct CharacterCache {
    entries: Vec<CharacterEntry>,
    /// (server_lower, name_lower) → index in entries.
    by_key: HashMap<(String, String), usize>,
    /// (server_lower, pet_lower) → owner name (original case).
    pet_to_owner: HashMap<(String, String), String>,
    dirty: bool,
}

impl CharacterCache {
    pub fn load() -> Self {
        let entries = Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str::<CacheFile>(&s).ok())
            .map(|f| f.characters)
            .unwrap_or_default();

        let mut cache = Self {
            entries,
            by_key: HashMap::new(),
            pet_to_owner: HashMap::new(),
            dirty: false,
        };
        cache.rebuild_indexes();
        cache
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = Self::path() else { return };
        let file = CacheFile {
            characters: self.entries.clone(),
        };
        if let Ok(contents) = toml::to_string_pretty(&file) {
            let _ = std::fs::write(path, contents);
        }
        self.dirty = false;
    }

    /// Identities confirmed as characters logged into a local EQ process.
    /// Metadata learned about other players (for example from /who) is excluded.
    pub fn identities(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .filter(|entry| entry.owned)
            .map(|entry| (entry.server.as_str(), entry.name.as_str()))
    }

    pub fn remember(&mut self, server: &str, name: &str) {
        let idx = self.upsert(server, name);
        if !self.entries[idx].owned {
            self.entries[idx].owned = true;
            self.dirty = true;
        }
    }

    pub fn get_class(&self, server: &str, name: &str) -> Option<&str> {
        let key = (server.to_ascii_lowercase(), name.to_ascii_lowercase());
        let idx = *self.by_key.get(&key)?;
        self.entries[idx].class.as_deref()
    }

    pub fn set_class(&mut self, server: &str, name: &str, class: &str) {
        let idx = self.upsert(server, name);
        let entry = &mut self.entries[idx];
        if entry.class.as_deref() != Some(class) {
            entry.class = Some(class.to_string());
            self.dirty = true;
        }
    }

    pub fn set_pet(&mut self, server: &str, owner: &str, pet: &str) {
        let idx = self.upsert(server, owner);
        let entry = &mut self.entries[idx];
        if entry.pet.as_deref() != Some(pet) {
            // Remove old pet→owner mapping if pet name changed.
            if let Some(old_pet) = &entry.pet {
                let old_key = (server.to_ascii_lowercase(), old_pet.to_ascii_lowercase());
                self.pet_to_owner.remove(&old_key);
            }
            entry.pet = Some(pet.to_string());
            self.pet_to_owner.insert(
                (server.to_ascii_lowercase(), pet.to_ascii_lowercase()),
                owner.to_string(),
            );
            self.dirty = true;
        }
    }

    fn upsert(&mut self, server: &str, name: &str) -> usize {
        let key = (server.to_ascii_lowercase(), name.to_ascii_lowercase());
        if let Some(&idx) = self.by_key.get(&key) {
            return idx;
        }
        let idx = self.entries.len();
        self.entries.push(CharacterEntry {
            server: server.to_string(),
            name: name.to_string(),
            owned: false,
            class: None,
            pet: None,
        });
        self.by_key.insert(key, idx);
        self.dirty = true;
        idx
    }

    fn rebuild_indexes(&mut self) {
        self.by_key.clear();
        self.pet_to_owner.clear();
        for (i, entry) in self.entries.iter().enumerate() {
            let key = (
                entry.server.to_ascii_lowercase(),
                entry.name.to_ascii_lowercase(),
            );
            self.by_key.insert(key, i);
            if let Some(pet) = &entry.pet {
                self.pet_to_owner.insert(
                    (entry.server.to_ascii_lowercase(), pet.to_ascii_lowercase()),
                    entry.name.clone(),
                );
            }
        }
    }

    fn path() -> Option<PathBuf> {
        Config::dir().map(|d| d.join("characters.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cache() -> CharacterCache {
        CharacterCache {
            entries: Vec::new(),
            by_key: HashMap::new(),
            pet_to_owner: HashMap::new(),
            dirty: false,
        }
    }

    #[test]
    fn observed_metadata_does_not_make_an_identity_owned() {
        let mut cache = empty_cache();
        cache.set_class("xegony", "SomeoneElse", "CLR");

        assert!(cache.identities().next().is_none());

        cache.remember("Xegony", "someoneelse");
        assert_eq!(
            cache.identities().collect::<Vec<_>>(),
            vec![("xegony", "SomeoneElse")]
        );
    }

    #[test]
    fn legacy_entries_default_to_not_owned() {
        let file: CacheFile = toml::from_str(
            r#"
[[characters]]
server = "xegony"
name = "WhoResult"
class = "WAR"
"#,
        )
        .unwrap();

        assert!(!file.characters[0].owned);
    }
}
