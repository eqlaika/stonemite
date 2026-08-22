use serde::{Deserialize, Serialize};

use crate::config::{Account, Config, PipEdge, TrusharConfig};
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
    "Luclin - Stromm",
    "Mangler",
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
    pub pip_edges: Vec<OptionItem>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDraft {
    pub general: GeneralSettings,
    pub accounts: AccountsSettings,
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
    pub label_height: u32,
    pub label_opacity: u32,
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
    pub resurrections: bool,
    pub deaths: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub swap_hotkeys: Vec<String>,
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
        Ok(Self {
            options: SettingsOptions::new(),
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
    fn new() -> Self {
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
            pip: PipSettings {
                edge: config.pip_edge,
                label_height: config.pip_label_height.unwrap_or(48),
                label_opacity: config.pip_label_opacity.unwrap_or(80),
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
                resurrections: config.notify_resurrections,
                deaths: config.notify_deaths,
            },
            hotkeys: HotkeySettings {
                swap_hotkeys: normalized_swap_hotkeys(&config.swap_hotkeys),
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
        let existing = Config::load();
        let (config, outcome) = self.into_config(existing)?;
        config
            .save()
            .map_err(|error| format!("Stonemite could not save its settings: {error}"))?;
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

        let config = Config {
            eq_dir: self.general.eq_directory,
            hide_hotkey: self.pip.hide_hotkey,
            pip_edge: self.pip.edge,
            pip_strip_width: existing.pip_strip_width,
            pip_positions: existing.pip_positions,
            snap_grid: existing.snap_grid,
            trusik: existing.trusik,
            swap_hotkeys: self.hotkeys.swap_hotkeys,
            settings_position: existing.settings_position,
            tell_visual_enabled: self.notifications.visual_enabled,
            tell_sound_enabled: self.notifications.sound_enabled,
            tell_sound: crate::sound::normalized_id(&self.notifications.sound).to_owned(),
            notify_tells: self.notifications.tells,
            notify_group_invites: self.notifications.group_invites,
            notify_raid_invites: self.notifications.raid_invites,
            notify_resurrections: self.notifications.resurrections,
            notify_deaths: self.notifications.deaths,
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
            auto_order: self.pip.auto_order,
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
        validate_range("PiP label height", self.pip.label_height, 24, 64)?;
        validate_range("PiP label opacity", self.pip.label_opacity, 10, 100)?;
        if self.hotkeys.swap_hotkeys.len() != 6 {
            return Err("Exactly six window hotkeys are required".to_owned());
        }
        crate::config::validate_mouse_clutch_binding(
            &self.broadcasting.mouse_clutch_key,
            &self.pip.hide_hotkey,
            &self.broadcasting.toggle_hotkey,
            &self.hotkeys.swap_hotkeys,
        )?;
        Ok(())
    }
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
        assert_eq!(draft.pip.edge, PipEdge::Right);
        assert_eq!(draft.general.toast.duration_seconds, 2.0);
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
    fn payload_option_values_match_serialized_enums() {
        let options = SettingsOptions::new();
        assert_eq!(options.pip_edges[0].value, "right");
        assert_eq!(options.filter_modes[0].value, "blacklist");
    }
}
