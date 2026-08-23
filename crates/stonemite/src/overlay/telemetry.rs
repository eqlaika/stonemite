use std::collections::HashMap;
use std::path::PathBuf;

use super::clients::{reconcile_identity, ClientRegistry};
use crate::{character_cache, config, eq_chat_colors, log_watcher, trusik_shm};

/// Passive identity and log-derived knowledge kept separately from UI state.
pub(super) struct TelemetryState {
    pub(super) trusik_enabled: bool,
    pub(super) log_telemetry: HashMap<(String, String), log_watcher::CharacterTelemetry>,
    pub(super) chat_colors: eq_chat_colors::EqChatColorResolver,
    pub(super) character_cache: character_cache::CharacterCache,
}

impl TelemetryState {
    pub(super) fn new(cfg: &config::Config) -> Self {
        Self {
            trusik_enabled: cfg.trusik,
            log_telemetry: HashMap::new(),
            chat_colors: eq_chat_colors::EqChatColorResolver::new(cfg.eq_directory()),
            character_cache: character_cache::CharacterCache::load(),
        }
    }

    /// Poll automatic identities and return whether visible client identity changed.
    pub(super) fn poll_characters(&mut self, clients: &mut ClientRegistry) -> bool {
        let mut changed = false;
        for window in &mut clients.windows {
            if let Some((character, server)) = trusik_shm::read_character(window.pid) {
                let class = self
                    .character_cache
                    .get_class(&server, &character)
                    .map(String::from);
                if reconcile_identity(
                    &mut clients.observed_identities,
                    window,
                    character.clone(),
                    server.clone(),
                    class,
                ) {
                    self.character_cache.remember(&server, &character);
                    changed = true;
                }
            }
        }
        if changed {
            self.character_cache.save();
        }
        changed
    }

    /// Apply one reducer telemetry change and report whether a visible class changed.
    pub(super) fn apply_change(
        &mut self,
        clients: &mut ClientRegistry,
        change: &log_watcher::TelemetryChange,
    ) -> bool {
        let server = change.character.server.as_ref();
        let character = change.character.character.as_ref();
        self.log_telemetry.insert(
            (server.to_ascii_lowercase(), character.to_ascii_lowercase()),
            change.telemetry.clone(),
        );

        let mut class_changed = false;
        if let Some(class_code) = &change.telemetry.class_code {
            self.character_cache
                .set_class(server, character, class_code.as_ref());
            for window in &mut clients.windows {
                if let (Some(name), Some(window_server)) = (&window.character, &window.server) {
                    if name.eq_ignore_ascii_case(character)
                        && window_server.eq_ignore_ascii_case(server)
                    {
                        let new_class = Some(class_code.to_string());
                        if window.class != new_class {
                            window.class = new_class;
                            class_changed = true;
                        }
                    }
                }
            }
        }
        if let Some(pet) = &change.telemetry.pet {
            self.character_cache
                .set_pet(server, character, pet.as_ref());
        }
        class_changed
    }

    pub(super) fn save(&mut self) {
        self.character_cache.save();
    }

    pub(super) fn set_eq_dir(&mut self, cfg: &config::Config) {
        self.chat_colors.set_eq_dir(cfg.eq_directory());
    }
}

pub(super) fn publish_log_sources(clients: &ClientRegistry, logs_dir: PathBuf) {
    let sources = clients
        .windows
        .iter()
        .filter_map(|window| {
            Some(log_watcher::LogSource::new(
                format!("pid:{}", window.pid),
                window.character.as_ref()?.as_str(),
                window.server.as_ref()?.as_str(),
            ))
        })
        .collect();
    log_watcher::replace_sources(logs_dir, sources);
}
