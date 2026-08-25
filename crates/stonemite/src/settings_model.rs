use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::config::{
    Account, BoxCycle, BoxIdentity, Config, LabelFontWeight, PipEdge, TrusharConfig,
    MAX_BOX_CYCLES, MAX_COMBAT_HIT_DURATION_SECONDS, MAX_PIP_LABEL_FONT_FAMILY_LEN,
    MAX_PIP_LABEL_FONT_SCALE, MAX_PIP_OPACITY, MIN_COMBAT_HIT_DURATION_SECONDS,
    MIN_PIP_LABEL_FONT_SCALE, MIN_PIP_OPACITY,
};
use crate::crypt;

const SERVER_OPTIONS: &[&str] = &[
    "",
    "Agnarr",
    "Antonius Bayle - Kane Bayle",
    "Aradune",
    "Bertoxxulous - Saryn",
    "Bristlebane - The Tribunal",
    "Cazic-Thule - Fennin Ro",
    "Drinal - Maelin Starpyre",
    "Erollisi Marr - The Nameless",
    "Fangbreaker",
    "Firiona Vie",
    "Frostreaver",
    "Lethar",
    "Luclin - Stromm",
    "Mischief",
    "Oakwynd",
    "Povar - Quellious",
    "Rizlona",
    "Teek",
    "The Rathe - Prexus",
    "Tormax",
    "Tunare - The Seventh Hammer",
    "Vaniki",
    "Vox",
    "Xegony - Druzzil Ro",
    "Yelinak",
    "Zek",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPayload {
    pub draft: SettingsDraft,
    pub options: SettingsOptions,
    pub runtime: SettingsRuntime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOptions {
    pub servers: Vec<OptionItem>,
    pub known_characters: Vec<BoxIdentity>,
    pub pip_edges: Vec<OptionItem>,
    pub label_font_families: Vec<String>,
    pub label_font_weights: Vec<OptionItem>,
    pub notification_sounds: Vec<OptionItem>,
    pub filter_modes: Vec<OptionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptionItem {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsRuntime {
    pub version: String,
    pub trusik_enabled: bool,
    pub integration_address: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunningCharacter {
    pub server: String,
    pub character: String,
    pub window_number: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDraft {
    pub general: GeneralSettings,
    pub accounts: AccountsSettings,
    pub box_order: Vec<BoxIdentity>,
    pub pip: PipSettings,
    pub notifications: NotificationSettings,
    pub hotkeys: HotkeySettings,
    pub broadcasting: BroadcastingSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub eq_directory: String,
    pub hide_from_alt_tab: bool,
    pub integrations: IntegrationSettings,
    pub toast: ToastSettings,
    pub updates: UpdateSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSettings {
    pub enabled: bool,
    pub lan_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToastSettings {
    pub enabled: bool,
    pub height: u32,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub automatic: bool,
    pub interval_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountsSettings {
    pub server: String,
    pub accounts: Vec<AccountDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountDraft {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PipSettings {
    pub edge: PipEdge,
    pub show_stonemite_button: bool,
    pub thumbnail_opacity: u32,
    pub label_height: u32,
    pub label_opacity: u32,
    pub font_family: String,
    pub font_scale: u32,
    pub font_weight: LabelFontWeight,
    pub auto_order: bool,
    pub hide_hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub visual_enabled: bool,
    pub sound_enabled: bool,
    pub sound: String,
    pub tells: bool,
    pub group_invites: bool,
    pub raid_invites: bool,
    pub trade_proposals: bool,
    pub resurrections: bool,
    pub deaths: bool,
    pub combat_awareness_enabled: bool,
    pub combat_hit_duration_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub swap_hotkeys: Vec<String>,
    pub box_cycles: Vec<BoxCycleSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BoxCycleSettings {
    pub name: String,
    pub next_hotkey: String,
    pub previous_hotkey: String,
    pub members: Vec<BoxIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastingSettings {
    pub toggle_hotkey: String,
    pub mouse_clutch_key: String,
    pub filter_mode: BroadcastFilterMode,
    pub filter_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BroadcastFilterMode {
    Blacklist,
    Whitelist,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOutcome {
    pub restart_required: bool,
}

impl SettingsPayload {
    pub fn load() -> Result<Self, String> {
        let config = Config::load();
        let draft = SettingsDraft::from_config(&config)?;
        let mut known_characters = config.box_order.clone();
        known_characters.extend(
            config
                .box_cycles
                .iter()
                .flat_map(|cycle| cycle.members.iter().cloned()),
        );
        known_characters.extend(
            crate::character_cache::CharacterCache::load()
                .identities()
                .map(|(server, character)| BoxIdentity {
                    server: server.to_owned(),
                    character: character.to_owned(),
                }),
        );
        normalize_known_characters(&mut known_characters);
        let mut label_font_families = crate::font_catalog::installed_font_families();
        if !label_font_families
            .iter()
            .any(|family| family.eq_ignore_ascii_case(&draft.pip.font_family))
        {
            label_font_families.push(draft.pip.font_family.clone());
            label_font_families.sort_by_key(|family| family.to_lowercase());
        }
        Ok(Self {
            options: SettingsOptions::new(known_characters, label_font_families),
            runtime: SettingsRuntime {
                version: crate::build_info::version().to_owned(),
                trusik_enabled: config.trusik,
                integration_address: integration_address(&config.trushar.bind),
            },
            draft,
        })
    }
}

impl SettingsOptions {
    fn new(known_characters: Vec<BoxIdentity>, label_font_families: Vec<String>) -> Self {
        Self {
            servers: SERVER_OPTIONS
                .iter()
                .map(|server| OptionItem {
                    value: (*server).to_owned(),
                    label: if server.is_empty() {
                        "None".to_owned()
                    } else {
                        (*server).to_owned()
                    },
                })
                .collect(),
            known_characters,
            pip_edges: [
                ("right", "Right"),
                ("left", "Left"),
                ("top", "Top"),
                ("bottom", "Bottom"),
            ]
            .into_iter()
            .map(|(value, label)| OptionItem {
                value: value.to_owned(),
                label: label.to_owned(),
            })
            .collect(),
            label_font_families,
            label_font_weights: [
                ("regular", "Regular"),
                ("semibold", "Semibold"),
                ("bold", "Bold"),
                ("heavy", "Heavy"),
            ]
            .into_iter()
            .map(|(value, label)| OptionItem {
                value: value.to_owned(),
                label: label.to_owned(),
            })
            .collect(),
            notification_sounds: crate::sound::BUILTIN_SOUNDS
                .iter()
                .map(|sound| OptionItem {
                    value: sound.id.to_owned(),
                    label: sound.label.to_owned(),
                })
                .collect(),
            filter_modes: [("blacklist", "Blacklist"), ("whitelist", "Whitelist")]
                .into_iter()
                .map(|(value, label)| OptionItem {
                    value: value.to_owned(),
                    label: label.to_owned(),
                })
                .collect(),
        }
    }
}

impl SettingsDraft {
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let server = config
            .read_server_from_ini()
            .filter(|server| !server.is_empty())
            .unwrap_or_else(|| config.server.clone());
        let accounts = config
            .accounts
            .iter()
            .enumerate()
            .map(|(index, account)| {
                crypt::decrypt(&account.password)
                    .map(|password| AccountDraft {
                        username: account.username.clone(),
                        password,
                    })
                    .map_err(|error| {
                        format!(
                            "Could not decrypt account {} ({}): {error}",
                            index + 1,
                            account.username
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            general: GeneralSettings {
                eq_directory: config.eq_dir.clone(),
                hide_from_alt_tab: config.hide_from_alt_tab,
                integrations: IntegrationSettings {
                    enabled: config.trushar.enabled,
                    lan_enabled: trushar_lan_enabled(&config.trushar.bind),
                },
                toast: ToastSettings {
                    enabled: config.toast_enabled,
                    height: config.toast_height.unwrap_or(64),
                    duration_seconds: config.toast_duration.unwrap_or(2.0),
                },
                updates: UpdateSettings {
                    automatic: config.auto_update_check,
                    interval_days: config.update_check_interval_days,
                },
            },
            accounts: AccountsSettings { server, accounts },
            box_order: config.box_order.clone(),
            pip: PipSettings {
                edge: config.pip_edge,
                show_stonemite_button: config.show_stonemite_button,
                thumbnail_opacity: config.effective_pip_opacity(),
                label_height: config.pip_label_height.unwrap_or(48),
                label_opacity: config.pip_label_opacity.unwrap_or(80),
                font_family: config.effective_pip_label_font_family().to_owned(),
                font_scale: config.effective_pip_label_font_scale(),
                font_weight: config.effective_pip_label_font_weight(),
                auto_order: config.auto_order,
                hide_hotkey: config.hide_hotkey.clone(),
            },
            notifications: NotificationSettings {
                visual_enabled: config.tell_visual_enabled,
                sound_enabled: config.tell_sound_enabled,
                sound: crate::sound::normalized_id(&config.tell_sound).to_owned(),
                tells: config.notify_tells,
                group_invites: config.notify_group_invites,
                raid_invites: config.notify_raid_invites,
                trade_proposals: config.notify_trade_proposals,
                resurrections: config.notify_resurrections,
                deaths: config.notify_deaths,
                combat_awareness_enabled: config.combat_awareness_enabled,
                combat_hit_duration_seconds: config.effective_combat_hit_duration_seconds(),
            },
            hotkeys: HotkeySettings {
                swap_hotkeys: normalized_swap_hotkeys(&config.swap_hotkeys),
                box_cycles: config
                    .box_cycles
                    .iter()
                    .map(|cycle| BoxCycleSettings {
                        name: cycle.name.clone(),
                        next_hotkey: cycle.next_hotkey.clone(),
                        previous_hotkey: cycle.previous_hotkey.clone(),
                        members: cycle.members.clone(),
                    })
                    .collect(),
            },
            broadcasting: BroadcastingSettings {
                toggle_hotkey: config.broadcast_hotkey.clone(),
                mouse_clutch_key: config.mouse_clutch_key.clone(),
                filter_mode: if config
                    .broadcast_filter_mode
                    .eq_ignore_ascii_case("whitelist")
                {
                    BroadcastFilterMode::Whitelist
                } else {
                    BroadcastFilterMode::Blacklist
                },
                filter_keys: config.broadcast_filter_keys.clone(),
            },
        })
    }

    pub fn save(self) -> Result<SaveOutcome, String> {
        let config_lock = Config::lock()
            .map_err(|error| format!("Stonemite could not lock its settings: {error}"))?;
        let existing = Config::load();
        let (config, outcome) = self.into_config(existing)?;
        config
            .save()
            .map_err(|error| format!("Stonemite could not save its settings: {error}"))?;
        drop(config_lock);
        config.write_server_to_ini();
        Ok(outcome)
    }

    fn into_config(self, existing: Config) -> Result<(Config, SaveOutcome), String> {
        self.validate()?;

        let previous_integrations = (
            existing.trushar.enabled,
            existing.trushar.bind.clone(),
            existing.trushar.auth_token.clone(),
        );
        let trushar = integration_config(
            existing.trushar,
            self.general.integrations.enabled,
            self.general.integrations.lan_enabled,
        );
        let restart_required = previous_integrations
            != (
                trushar.enabled,
                trushar.bind.clone(),
                trushar.auth_token.clone(),
            );

        let box_order = self
            .box_order
            .into_iter()
            .map(|identity| BoxIdentity {
                server: identity.server.trim().to_owned(),
                character: identity.character.trim().to_owned(),
            })
            .collect();

        let box_cycles = self
            .hotkeys
            .box_cycles
            .into_iter()
            .map(|cycle| BoxCycle {
                name: cycle.name.trim().to_owned(),
                next_hotkey: cycle.next_hotkey.trim().to_owned(),
                previous_hotkey: cycle.previous_hotkey.trim().to_owned(),
                members: cycle
                    .members
                    .into_iter()
                    .map(|identity| BoxIdentity {
                        server: identity.server.trim().to_owned(),
                        character: identity.character.trim().to_owned(),
                    })
                    .collect(),
            })
            .collect();

        let accounts = self
            .accounts
            .accounts
            .into_iter()
            .filter(|account| !account.username.trim().is_empty())
            .enumerate()
            .map(|(index, account)| {
                crypt::encrypt(&account.password)
                    .map(|password| Account {
                        username: account.username,
                        password,
                    })
                    .map_err(|error| format!("Could not encrypt account {}: {error}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let label_font_family = self.pip.font_family.trim().to_owned();
        let config = Config {
            eq_dir: self.general.eq_directory,
            hide_hotkey: self.pip.hide_hotkey,
            pip_edge: self.pip.edge,
            show_stonemite_button: self.pip.show_stonemite_button,
            stonemite_button_position: existing.stonemite_button_position,
            pip_strip_width: existing.pip_strip_width,
            pip_opacity: Some(self.pip.thumbnail_opacity),
            pip_positions: existing.pip_positions,
            snap_grid: existing.snap_grid,
            trusik: existing.trusik,
            swap_hotkeys: self.hotkeys.swap_hotkeys,
            box_cycles,
            settings_position: existing.settings_position,
            tell_visual_enabled: self.notifications.visual_enabled,
            tell_sound_enabled: self.notifications.sound_enabled,
            tell_sound: crate::sound::normalized_id(&self.notifications.sound).to_owned(),
            notify_tells: self.notifications.tells,
            notify_group_invites: self.notifications.group_invites,
            notify_raid_invites: self.notifications.raid_invites,
            notify_trade_proposals: self.notifications.trade_proposals,
            notify_resurrections: self.notifications.resurrections,
            notify_deaths: self.notifications.deaths,
            combat_awareness_enabled: self.notifications.combat_awareness_enabled,
            combat_hit_duration_seconds: self.notifications.combat_hit_duration_seconds,
            broadcast_hotkey: self.broadcasting.toggle_hotkey,
            mouse_clutch_key: self.broadcasting.mouse_clutch_key,
            broadcast_filter_mode: match self.broadcasting.filter_mode {
                BroadcastFilterMode::Blacklist => "blacklist",
                BroadcastFilterMode::Whitelist => "whitelist",
            }
            .to_owned(),
            broadcast_filter_keys: self.broadcasting.filter_keys,
            auto_update_check: self.general.updates.automatic,
            update_check_interval_days: self.general.updates.interval_days,
            last_update_check: existing.last_update_check,
            accounts,
            pip_label_height: Some(self.pip.label_height),
            pip_label_opacity: Some(self.pip.label_opacity),
            pip_label_font_family: Some(label_font_family),
            pip_label_font_scale: Some(self.pip.font_scale),
            pip_label_font_weight: Some(self.pip.font_weight),
            auto_order: self.pip.auto_order,
            box_order,
            hide_from_alt_tab: self.general.hide_from_alt_tab,
            toast_enabled: self.general.toast.enabled,
            toast_height: Some(self.general.toast.height),
            toast_duration: Some(self.general.toast.duration_seconds),
            server: self.accounts.server,
            trushar,
        };

        Ok((config, SaveOutcome { restart_required }))
    }

    fn validate(&self) -> Result<(), String> {
        validate_range("Toast height", self.general.toast.height, 24, 128)?;
        validate_float_range(
            "Toast duration",
            self.general.toast.duration_seconds,
            0.5,
            10.0,
        )?;
        validate_range("Update interval", self.general.updates.interval_days, 1, 30)?;
        validate_range(
            "PiP thumbnail opacity",
            self.pip.thumbnail_opacity,
            MIN_PIP_OPACITY,
            MAX_PIP_OPACITY,
        )?;
        validate_range("PiP label height", self.pip.label_height, 24, 64)?;
        validate_range("PiP label opacity", self.pip.label_opacity, 10, 100)?;
        validate_float_range(
            "Combat hit highlight duration",
            self.notifications.combat_hit_duration_seconds,
            MIN_COMBAT_HIT_DURATION_SECONDS,
            MAX_COMBAT_HIT_DURATION_SECONDS,
        )?;
        validate_range(
            "PiP label font scale",
            self.pip.font_scale,
            MIN_PIP_LABEL_FONT_SCALE,
            MAX_PIP_LABEL_FONT_SCALE,
        )?;
        let font_family = self.pip.font_family.trim();
        if font_family.is_empty() {
            return Err("PiP label font family is required".to_owned());
        }
        if font_family.encode_utf16().count() > MAX_PIP_LABEL_FONT_FAMILY_LEN {
            return Err(format!(
                "PiP label font family must be at most {MAX_PIP_LABEL_FONT_FAMILY_LEN} characters"
            ));
        }
        if font_family.chars().any(char::is_control) {
            return Err("PiP label font family cannot contain control characters".to_owned());
        }
        if self.hotkeys.swap_hotkeys.len() != 6 {
            return Err("Exactly six window hotkeys are required".to_owned());
        }
        if self.hotkeys.box_cycles.len() > MAX_BOX_CYCLES {
            return Err(format!("At most {MAX_BOX_CYCLES} box cycles are supported"));
        }
        let mut cycle_names = HashSet::with_capacity(self.hotkeys.box_cycles.len());
        let mut config_cycles = Vec::with_capacity(self.hotkeys.box_cycles.len());
        for (cycle_index, cycle) in self.hotkeys.box_cycles.iter().enumerate() {
            let name = cycle.name.trim();
            if name.is_empty() {
                return Err(format!("Box cycle {} requires a name", cycle_index + 1));
            }
            if name.len() > 64 || name.chars().any(char::is_control) {
                return Err(format!(
                    "Box cycle '{}' name must be at most 64 bytes without control characters",
                    name
                ));
            }
            if !cycle_names.insert(name.to_ascii_lowercase()) {
                return Err(format!("Box cycle name '{name}' is used more than once"));
            }
            if cycle.next_hotkey.trim().is_empty() && cycle.previous_hotkey.trim().is_empty() {
                return Err(format!(
                    "Box cycle '{name}' needs a Next or Previous hotkey"
                ));
            }
            if cycle.members.len() < 2 {
                return Err(format!("Box cycle '{name}' needs at least two characters"));
            }
            let mut members = HashSet::with_capacity(cycle.members.len());
            for (member_index, identity) in cycle.members.iter().enumerate() {
                let server = identity.server.trim();
                let character = identity.character.trim();
                validate_identity(
                    server,
                    character,
                    &format!("Box cycle '{name}' member {}", member_index + 1),
                )?;
                if !members.insert((server.to_ascii_lowercase(), character.to_ascii_lowercase())) {
                    return Err(format!(
                        "{character} on {server} appears more than once in box cycle '{name}'"
                    ));
                }
            }
            config_cycles.push(BoxCycle {
                name: name.to_owned(),
                next_hotkey: cycle.next_hotkey.trim().to_owned(),
                previous_hotkey: cycle.previous_hotkey.trim().to_owned(),
                members: cycle.members.clone(),
            });
        }
        crate::config::validate_global_hotkey_bindings(
            &self.pip.hide_hotkey,
            &self.broadcasting.toggle_hotkey,
            &self.hotkeys.swap_hotkeys,
            &config_cycles,
        )?;
        crate::config::validate_mouse_clutch_binding(
            &self.broadcasting.mouse_clutch_key,
            &self.pip.hide_hotkey,
            &self.broadcasting.toggle_hotkey,
            &self.hotkeys.swap_hotkeys,
            &config_cycles,
        )?;
        let mut identities = HashSet::with_capacity(self.box_order.len());
        for (index, identity) in self.box_order.iter().enumerate() {
            let server = identity.server.trim();
            let character = identity.character.trim();
            validate_identity(server, character, &format!("Box order entry {}", index + 1))?;
            let key = (server.to_ascii_lowercase(), character.to_ascii_lowercase());
            if !identities.insert(key) {
                return Err(format!(
                    "{} on {} appears more than once in box order",
                    character, server
                ));
            }
        }
        Ok(())
    }
}

fn validate_identity(server: &str, character: &str, label: &str) -> Result<(), String> {
    if server.is_empty() || character.is_empty() {
        return Err(format!("{label} requires both a character and server"));
    }
    if server.len() > 128 || character.len() > 128 {
        return Err(format!(
            "{label} character and server must be at most 128 bytes"
        ));
    }
    Ok(())
}

fn normalize_known_characters(characters: &mut Vec<BoxIdentity>) {
    characters.sort_by(|left, right| {
        left.server
            .to_ascii_lowercase()
            .cmp(&right.server.to_ascii_lowercase())
            .then_with(|| {
                left.character
                    .to_ascii_lowercase()
                    .cmp(&right.character.to_ascii_lowercase())
            })
    });
    characters.dedup_by(|right, left| left.matches(&right.server, &right.character));
}

fn normalized_swap_hotkeys(configured: &[String]) -> Vec<String> {
    let mut hotkeys: Vec<String> = (1..=6).map(|slot| format!("Ctrl+F{slot}")).collect();
    for (slot, binding) in configured.iter().enumerate().take(6) {
        hotkeys[slot] = binding.clone();
    }
    hotkeys
}

fn validate_range(label: &str, value: u32, minimum: u32, maximum: u32) -> Result<(), String> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(format!("{label} must be between {minimum} and {maximum}"))
    }
}

fn validate_float_range(label: &str, value: f32, minimum: f32, maximum: f32) -> Result<(), String> {
    if value.is_finite() && value >= minimum && value <= maximum {
        Ok(())
    } else {
        Err(format!(
            "{label} must be between {minimum:.1} and {maximum:.1}"
        ))
    }
}

fn trushar_lan_enabled(bind: &str) -> bool {
    bind.parse::<std::net::SocketAddr>()
        .is_ok_and(|address| !address.ip().is_loopback())
}

fn integration_port(bind: &str) -> u16 {
    bind.parse::<std::net::SocketAddr>()
        .map_or(19_720, |address| address.port())
}

fn integration_config(
    mut config: TrusharConfig,
    enabled: bool,
    lan_enabled: bool,
) -> TrusharConfig {
    let port = integration_port(&config.bind);
    config.enabled = enabled;
    if lan_enabled {
        config.bind = format!("0.0.0.0:{port}");
        if config
            .auth_token
            .as_ref()
            .is_none_or(|token| token.trim().is_empty())
        {
            config.auth_token = Some(generate_auth_token());
        }
    } else {
        config.bind = format!("127.0.0.1:{port}");
        config.auth_token = None;
    }
    config
}

fn generate_auth_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn integration_address(bind: &str) -> String {
    let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "this-pc".into());
    let hostname = hostname.to_ascii_lowercase();
    let hostname = if hostname.ends_with(".local") {
        hostname
    } else {
        format!("{hostname}.local")
    };
    format!("{hostname}:{}", integration_port(bind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_uses_stable_defaults_and_six_hotkeys() {
        let draft = SettingsDraft::from_config(&Config::default()).unwrap();
        assert_eq!(draft.hotkeys.swap_hotkeys.len(), 6);
        assert!(draft.hotkeys.box_cycles.is_empty());
        assert!(draft.box_order.is_empty());
        assert_eq!(draft.pip.edge, PipEdge::Right);
        assert!(draft.pip.show_stonemite_button);
        assert_eq!(
            draft.pip.thumbnail_opacity,
            crate::config::DEFAULT_PIP_OPACITY
        );
        assert_eq!(
            draft.pip.font_family,
            crate::config::DEFAULT_PIP_LABEL_FONT_FAMILY
        );
        assert_eq!(
            draft.pip.font_scale,
            crate::config::DEFAULT_PIP_LABEL_FONT_SCALE
        );
        assert_eq!(draft.pip.font_weight, LabelFontWeight::Bold);
        assert_eq!(draft.general.toast.duration_seconds, 2.0);
        assert!(draft.notifications.trade_proposals);
        assert!(draft.notifications.combat_awareness_enabled);
        assert_eq!(draft.notifications.combat_hit_duration_seconds, 3.0);
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.general.toast.height = 200;
        assert_eq!(
            draft.validate(),
            Err("Toast height must be between 24 and 128".to_owned())
        );
    }

    #[test]
    fn pip_appearance_is_validated_and_persisted() {
        let existing = Config {
            stonemite_button_position: Some([0.25, 0.75]),
            ..Config::default()
        };
        let mut draft = SettingsDraft::from_config(&existing).unwrap();
        draft.pip.show_stonemite_button = false;
        draft.pip.thumbnail_opacity = 65;
        draft.pip.label_height = 56;
        draft.pip.label_opacity = 72;
        let (config, _) = draft.into_config(existing).unwrap();
        assert!(!config.show_stonemite_button);
        assert_eq!(config.stonemite_button_position, Some([0.25, 0.75]));
        assert_eq!(config.pip_opacity, Some(65));
        assert_eq!(config.pip_label_height, Some(56));
        assert_eq!(config.pip_label_opacity, Some(72));

        let mut invalid = SettingsDraft::from_config(&Config::default()).unwrap();
        invalid.pip.thumbnail_opacity = 9;
        assert_eq!(
            invalid.validate(),
            Err("PiP thumbnail opacity must be between 10 and 100".to_owned())
        );
    }

    #[test]
    fn combat_awareness_is_validated_and_persisted() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.notifications.combat_awareness_enabled = false;
        draft.notifications.combat_hit_duration_seconds = 2.5;
        let (config, _) = draft.into_config(Config::default()).unwrap();
        assert!(!config.combat_awareness_enabled);
        assert_eq!(config.combat_hit_duration_seconds, 2.5);

        let mut invalid = SettingsDraft::from_config(&Config::default()).unwrap();
        invalid.notifications.combat_hit_duration_seconds = 10.5;
        assert_eq!(
            invalid.validate(),
            Err("Combat hit highlight duration must be between 0.5 and 10.0".to_owned())
        );
    }

    #[test]
    fn label_typography_is_validated_trimmed_and_persisted() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.pip.font_family = "  Tahoma  ".to_owned();
        draft.pip.font_scale = 115;
        draft.pip.font_weight = LabelFontWeight::Semibold;
        let (config, _) = draft.into_config(Config::default()).unwrap();
        assert_eq!(config.pip_label_font_family.as_deref(), Some("Tahoma"));
        assert_eq!(config.pip_label_font_scale, Some(115));
        assert_eq!(
            config.pip_label_font_weight,
            Some(LabelFontWeight::Semibold)
        );

        let mut invalid = SettingsDraft::from_config(&Config::default()).unwrap();
        invalid.pip.font_scale = 121;
        assert_eq!(
            invalid.validate(),
            Err("PiP label font scale must be between 60 and 120".to_owned())
        );
        invalid.pip.font_scale = 100;
        invalid.pip.font_family = " ".to_owned();
        assert_eq!(
            invalid.validate(),
            Err("PiP label font family is required".to_owned())
        );

        let malformed = Config {
            pip_label_font_family: Some(" ".to_owned()),
            pip_label_font_scale: Some(999),
            ..Config::default()
        };
        let normalized = SettingsDraft::from_config(&malformed).unwrap();
        assert_eq!(
            normalized.pip.font_family,
            crate::config::DEFAULT_PIP_LABEL_FONT_FAMILY
        );
        assert_eq!(normalized.pip.font_scale, MAX_PIP_LABEL_FONT_SCALE);
    }

    #[test]
    fn integration_access_preserves_port_and_authenticates_lan() {
        let config = TrusharConfig {
            enabled: true,
            bind: "127.0.0.1:20000".to_owned(),
            auth_token: None,
        };
        let lan = integration_config(config, true, true);
        assert_eq!(lan.bind, "0.0.0.0:20000");
        assert!(lan.auth_token.is_some());

        let local = integration_config(lan, true, false);
        assert_eq!(local.bind, "127.0.0.1:20000");
        assert_eq!(local.auth_token, None);
    }

    #[test]
    fn box_order_is_trimmed_and_duplicate_identities_are_rejected() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.box_order = vec![BoxIdentity {
            server: " Xegony ".into(),
            character: " Laika ".into(),
        }];
        let (config, _) = draft.into_config(Config::default()).unwrap();
        assert_eq!(
            config.box_order,
            vec![BoxIdentity {
                server: "Xegony".into(),
                character: "Laika".into(),
            }]
        );

        let mut duplicate = SettingsDraft::from_config(&Config::default()).unwrap();
        duplicate.box_order = vec![
            BoxIdentity {
                server: "xegony".into(),
                character: "Laika".into(),
            },
            BoxIdentity {
                server: "XEGONY".into(),
                character: "laika".into(),
            },
        ];
        assert_eq!(
            duplicate.validate(),
            Err("laika on XEGONY appears more than once in box order".into())
        );
    }

    #[test]
    fn box_cycles_are_validated_trimmed_and_persisted() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.hotkeys.box_cycles = vec![BoxCycleSettings {
            name: "  Melee  ".into(),
            next_hotkey: " F14 ".into(),
            previous_hotkey: " F15 ".into(),
            members: vec![
                BoxIdentity {
                    server: " Xegony ".into(),
                    character: " Tank ".into(),
                },
                BoxIdentity {
                    server: " Xegony ".into(),
                    character: " Rogue ".into(),
                },
            ],
        }];

        let (config, _) = draft.into_config(Config::default()).unwrap();
        assert_eq!(config.box_cycles[0].name, "Melee");
        assert_eq!(config.box_cycles[0].next_hotkey, "F14");
        assert_eq!(config.box_cycles[0].members[1].character, "Rogue");

        let restored = SettingsDraft::from_config(&config).unwrap();
        assert_eq!(restored.hotkeys.box_cycles[0].previous_hotkey, "F15");
    }

    #[test]
    fn box_cycles_reject_incomplete_rings_and_binding_collisions() {
        let mut draft = SettingsDraft::from_config(&Config::default()).unwrap();
        draft.hotkeys.box_cycles = vec![BoxCycleSettings {
            name: "Melee".into(),
            next_hotkey: "F14".into(),
            previous_hotkey: String::new(),
            members: vec![BoxIdentity {
                server: "Xegony".into(),
                character: "Tank".into(),
            }],
        }];
        assert_eq!(
            draft.validate(),
            Err("Box cycle 'Melee' needs at least two characters".into())
        );

        draft.hotkeys.box_cycles[0].members.push(BoxIdentity {
            server: "Xegony".into(),
            character: "Rogue".into(),
        });
        draft.hotkeys.box_cycles[0].previous_hotkey = "F14".into();
        assert!(draft
            .validate()
            .is_err_and(|error| error.contains("conflicts")));
    }

    #[test]
    fn known_characters_are_sorted_and_deduplicated_case_insensitively() {
        let mut characters = vec![
            BoxIdentity {
                server: "Xegony".into(),
                character: "Laika".into(),
            },
            BoxIdentity {
                server: "bristlebane".into(),
                character: "Foo".into(),
            },
            BoxIdentity {
                server: "xegony".into(),
                character: "laika".into(),
            },
        ];
        normalize_known_characters(&mut characters);
        assert_eq!(characters.len(), 2);
        assert_eq!(characters[0].character, "Foo");
        assert_eq!(characters[1].character, "Laika");
    }

    #[test]
    fn payload_option_values_match_serialized_enums() {
        let options = SettingsOptions::new(Vec::new(), vec!["Segoe UI".to_owned()]);
        assert_eq!(options.pip_edges[0].value, "right");
        assert_eq!(options.label_font_families, ["Segoe UI"]);
        assert_eq!(options.label_font_weights[2].value, "bold");
        assert_eq!(options.filter_modes[0].value, "blacklist");
    }
}
