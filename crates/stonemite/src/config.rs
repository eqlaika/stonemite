use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateMutexW, ReleaseMutex, WaitForSingleObject, INFINITE,
};

pub const DEFAULT_EQ_DIR: &str = r"C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest";
pub const DEFAULT_PIP_OPACITY: u32 = 80;
pub const MIN_PIP_OPACITY: u32 = 10;
pub const MAX_PIP_OPACITY: u32 = 100;
pub const DEFAULT_PIP_LABEL_FONT_FAMILY: &str = "Segoe UI";
pub const DEFAULT_PIP_LABEL_FONT_SCALE: u32 = 100;
pub const MIN_PIP_LABEL_FONT_SCALE: u32 = 60;
pub const MAX_PIP_LABEL_FONT_SCALE: u32 = 120;
pub const MAX_PIP_LABEL_FONT_FAMILY_LEN: usize = 31;
pub const DEFAULT_DPS_OVERLAY_TOP_ROWS: u8 = 10;
pub const DPS_OVERLAY_TOP_ROW_OPTIONS: &[u8] = &[5, 10, 15];
pub const DEFAULT_DPS_OVERLAY_WIDTH_DIP: u32 = 440;
pub const MIN_DPS_OVERLAY_WIDTH_DIP: u32 = 360;
pub const DEFAULT_COMBAT_HIT_DURATION_SECONDS: f32 = 3.0;
pub const MIN_COMBAT_HIT_DURATION_SECONDS: f32 = 0.5;
pub const MAX_COMBAT_HIT_DURATION_SECONDS: f32 = 10.0;
pub const DEFAULT_AA_POINTS_PER_NOTIFICATION: u32 = 1;
pub const MIN_AA_POINTS_PER_NOTIFICATION: u32 = 1;
pub const MAX_AA_POINTS_PER_NOTIFICATION: u32 = 100;
pub const MAX_BOX_CYCLES: usize = 16;

/// Screen edge where the PiP strip is anchored.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PipEdge {
    Right,
    Left,
    Top,
    Bottom,
}

impl Default for PipEdge {
    fn default() -> Self {
        Self::Right
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LabelFontWeight {
    Regular,
    Semibold,
    Bold,
    Heavy,
}

impl Default for LabelFontWeight {
    fn default() -> Self {
        Self::Bold
    }
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawString {
        Value(String),
        Other(serde::de::IgnoredAny),
    }

    Ok(match RawString::deserialize(deserializer)? {
        RawString::Value(value) => Some(value),
        RawString::Other(_) => None,
    })
}

fn deserialize_optional_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum RawNumber {
        Number(u64),
        Text(String),
        Other(serde::de::IgnoredAny),
    }

    Ok(match RawNumber::deserialize(deserializer)? {
        RawNumber::Number(value) => u32::try_from(value).ok(),
        RawNumber::Text(value) => value.parse().ok(),
        RawNumber::Other(_) => None,
    })
}

impl<'de> Deserialize<'de> for LabelFontWeight {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawWeight {
            Name(String),
            Number(u16),
            Other(serde::de::IgnoredAny),
        }

        Ok(match RawWeight::deserialize(deserializer)? {
            RawWeight::Name(value) if value.eq_ignore_ascii_case("regular") => Self::Regular,
            RawWeight::Name(value) if value.eq_ignore_ascii_case("semibold") => Self::Semibold,
            RawWeight::Name(value) if value.eq_ignore_ascii_case("heavy") => Self::Heavy,
            RawWeight::Number(400) => Self::Regular,
            RawWeight::Number(600) => Self::Semibold,
            RawWeight::Number(900) => Self::Heavy,
            RawWeight::Name(_) | RawWeight::Number(_) | RawWeight::Other(_) => Self::Bold,
        })
    }
}

/// Per-pip custom position and size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipPosition {
    /// Index in pip_order.
    pub slot: usize,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Work-area-relative DPS overlay placement persisted in device-independent
/// pixels so one global preference can follow the active EQ monitor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DpsOverlayPlacement {
    pub x_dip: i32,
    pub y_dip: i32,
    pub width_dip: u32,
}

/// An EverQuest account for auto-login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    /// DPAPI-encrypted, base64-encoded password.
    pub password: String,
}

/// Generic WebSocket control/state API configuration.
#[derive(Clone, Serialize, Deserialize)]
pub struct TrusharConfig {
    /// Start the server with the tray application.
    #[serde(default = "default_trushar_enabled")]
    pub enabled: bool,
    /// Numeric socket address. Non-loopback/wildcard binds require auth_token.
    #[serde(default = "default_trushar_bind")]
    pub bind: String,
    /// Shared bearer token. Required for LAN exposure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
}

impl std::fmt::Debug for TrusharConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrusharConfig")
            .field("enabled", &self.enabled)
            .field("bind", &self.bind)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

impl Default for TrusharConfig {
    fn default() -> Self {
        Self {
            enabled: default_trushar_enabled(),
            bind: default_trushar_bind(),
            auth_token: None,
        }
    }
}

fn default_trushar_enabled() -> bool {
    true
}

fn default_trushar_bind() -> String {
    trushar::server::DEFAULT_BIND.to_owned()
}

/// A durable EverQuest character identity used for ordering and box cycles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoxIdentity {
    pub server: String,
    pub character: String,
}

impl BoxIdentity {
    pub fn matches(&self, server: &str, character: &str) -> bool {
        self.server.eq_ignore_ascii_case(server) && self.character.eq_ignore_ascii_case(character)
    }
}

/// A named ring of characters activated one physical hotkey press at a time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoxCycle {
    pub name: String,
    #[serde(default)]
    pub next_hotkey: String,
    #[serde(default)]
    pub previous_hotkey: String,
    #[serde(default)]
    pub members: Vec<BoxIdentity>,
}

impl BoxCycle {
    pub fn next_hotkey_vk(&self) -> Option<(u32, u32)> {
        parse_hotkey_combo(&self.next_hotkey)
    }

    pub fn previous_hotkey_vk(&self) -> Option<(u32, u32)> {
        parse_hotkey_combo(&self.previous_hotkey)
    }
}

/// Top-level configuration persisted to %APPDATA%\Stonemite\config.toml.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the EverQuest installation directory.
    pub eq_dir: String,
    /// Key name for the hide-overlay hotkey (e.g. "F9", "F12"). Default: "F9".
    #[serde(default = "default_hide_hotkey")]
    pub hide_hotkey: String,
    /// Screen edge for the PiP strip: right, left, top, bottom.
    #[serde(default)]
    pub pip_edge: PipEdge,
    /// Keep the in-game Stonemite button available over EverQuest.
    #[serde(default = "default_show_stonemite_button")]
    pub show_stonemite_button: bool,
    /// Work-area-relative top-left position saved by dragging the in-game logo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stonemite_button_position: Option<[f32; 2]>,
    /// Custom PiP strip width in pixels. None = auto-size.
    #[serde(default)]
    pub pip_strip_width: Option<u32>,
    /// Normal live EQ thumbnail opacity as a percentage. None = default (80).
    #[serde(default, deserialize_with = "deserialize_optional_u32")]
    pub pip_opacity: Option<u32>,
    /// Per-pip custom positions. Empty = auto strip layout.
    #[serde(default)]
    pub pip_positions: Vec<PipPosition>,
    /// Show the passive encounter DPS panel. Default: true.
    #[serde(default = "default_dps_overlay_enabled")]
    pub dps_overlay_enabled: bool,
    /// Global participant cutoff before omitted managed boxes are appended.
    #[serde(default = "default_dps_overlay_top_rows")]
    pub dps_overlay_top_rows: u8,
    /// Work-area-relative placement and width in DIPs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dps_overlay_placement: Option<DpsOverlayPlacement>,
    /// Snap grid size in pixels. 0 = no grid snap. Default: 16.
    #[serde(default = "default_snap_grid")]
    pub snap_grid: u32,
    /// PiP label height in pixels. None = default (48).
    #[serde(default)]
    pub pip_label_height: Option<u32>,
    /// PiP label opacity as a percentage (0–100). None = default (80).
    #[serde(default)]
    pub pip_label_opacity: Option<u32>,
    /// Installed Windows font family used for character names.
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    pub pip_label_font_family: Option<String>,
    /// Character-name font size relative to the current automatic size.
    #[serde(default, deserialize_with = "deserialize_optional_u32")]
    pub pip_label_font_scale: Option<u32>,
    /// Character-name font weight.
    #[serde(default)]
    pub pip_label_font_weight: Option<LabelFontWeight>,
    /// Automatically order PiP windows by slot number in auto layout mode.
    #[serde(default = "default_auto_order")]
    pub auto_order: bool,
    /// Preferred global box order, matched by full character/server identity.
    #[serde(default)]
    pub box_order: Vec<BoxIdentity>,
    /// Hide background EQ windows from Alt-Tab (active window stays visible).
    #[serde(default = "default_hide_from_alt_tab")]
    pub hide_from_alt_tab: bool,
    /// Enable trusik DLL proxy for character detection. Requires restart.
    #[serde(default)]
    pub trusik: bool,
    /// Hotkey for toggling key broadcasting. Default: "Pause".
    #[serde(default = "default_broadcast_hotkey")]
    pub broadcast_hotkey: String,
    /// Disable key broadcasting after the last EverQuest client exits. Default: true.
    #[serde(default = "default_disable_broadcast_when_clients_exit")]
    pub disable_broadcast_when_clients_exit: bool,
    /// Single hold-to-broadcast mouse key. Default: "F13". Empty means unbound.
    #[serde(default = "default_mouse_clutch_key")]
    pub mouse_clutch_key: String,
    /// Filter mode: "blacklist" or "whitelist". Default: "blacklist".
    #[serde(default = "default_broadcast_filter_mode")]
    pub broadcast_filter_mode: String,
    /// Key names to filter (e.g. "Enter", "Escape").
    #[serde(default)]
    pub broadcast_filter_keys: Vec<String>,
    /// Hotkeys for swapping to specific window slots (1–6). Default: Ctrl+F1..Ctrl+F6.
    #[serde(default = "default_swap_hotkeys")]
    pub swap_hotkeys: Vec<String>,
    /// Named character rings with independent next and previous hotkeys.
    #[serde(default)]
    pub box_cycles: Vec<BoxCycle>,
    /// Remembered settings window position [x, y].
    #[serde(default)]
    pub settings_position: Option<[f32; 2]>,
    /// Highlight a background PiP when it receives a tell. Default: true.
    #[serde(default = "default_tell_visual_enabled")]
    pub tell_visual_enabled: bool,
    /// Play a sound for every incoming tell, including on the active box. Default: true.
    #[serde(default = "default_tell_sound_enabled")]
    pub tell_sound_enabled: bool,
    /// Bundled EQ audio-trigger filename used for incoming notifications.
    #[serde(default = "default_tell_sound")]
    pub tell_sound: String,
    /// Notify for incoming tells.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_tells: bool,
    /// Notify for incoming group invitations.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_group_invites: bool,
    /// Notify for incoming raid invitations.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_raid_invites: bool,
    /// Notify when another player proposes a trade.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_trade_proposals: bool,
    /// Notify when a resurrection is offered.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_resurrections: bool,
    /// Notify when a character dies.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_deaths: bool,
    /// Notify when a character gains a level.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_level_gains: bool,
    /// Notify after a character earns the configured number of AA points.
    #[serde(default = "default_notification_event_enabled")]
    pub notify_aa_gains: bool,
    /// AA points earned per notification, counted independently for each box.
    #[serde(default = "default_aa_points_per_notification")]
    pub aa_points_per_notification: u32,
    /// Show log-derived combat activity and recoverable problems on background PiPs.
    #[serde(default = "default_combat_awareness_enabled")]
    pub combat_awareness_enabled: bool,
    /// Seconds that recent outgoing weapon damage keeps the red combat frame visible.
    #[serde(default = "default_combat_hit_duration_seconds")]
    pub combat_hit_duration_seconds: f32,
    /// Enable toast notifications. Default: true.
    #[serde(default = "default_toast_enabled")]
    pub toast_enabled: bool,
    /// Toast notification height in pixels. None = default (40).
    #[serde(default)]
    pub toast_height: Option<u32>,
    /// Toast notification duration in seconds. None = default (2.0).
    #[serde(default)]
    pub toast_duration: Option<f32>,
    /// Automatically check for updates on launch. Default: true.
    #[serde(default = "default_auto_update")]
    pub auto_update_check: bool,
    /// Days between automatic update checks. Default: 7.
    #[serde(default = "default_update_interval")]
    pub update_check_interval_days: u32,
    /// ISO 8601 timestamp of last automatic update check.
    #[serde(default)]
    pub last_update_check: Option<String>,
    /// EverQuest accounts for auto-login.
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// EverQuest server name for auto-login (written to eqlsPlayerData.ini).
    #[serde(default)]
    pub server: String,
    /// Generic WebSocket control/state API. Edited manually in config.toml.
    #[serde(default)]
    pub trushar: TrusharConfig,
}

fn default_hide_hotkey() -> String {
    "F9".to_string()
}

fn default_snap_grid() -> u32 {
    16
}

fn default_dps_overlay_enabled() -> bool {
    true
}

fn default_dps_overlay_top_rows() -> u8 {
    DEFAULT_DPS_OVERLAY_TOP_ROWS
}

fn default_show_stonemite_button() -> bool {
    true
}

fn default_broadcast_hotkey() -> String {
    "Pause".to_string()
}

fn default_disable_broadcast_when_clients_exit() -> bool {
    true
}

fn default_mouse_clutch_key() -> String {
    "F13".to_string()
}

fn default_broadcast_filter_mode() -> String {
    "blacklist".to_string()
}

fn default_swap_hotkeys() -> Vec<String> {
    (1..=6).map(|i| format!("Ctrl+F{i}")).collect()
}

fn default_tell_visual_enabled() -> bool {
    true
}

fn default_tell_sound_enabled() -> bool {
    true
}

fn default_tell_sound() -> String {
    crate::sound::DEFAULT_SOUND_ID.to_owned()
}

fn default_notification_event_enabled() -> bool {
    true
}

fn default_aa_points_per_notification() -> u32 {
    DEFAULT_AA_POINTS_PER_NOTIFICATION
}

fn default_combat_awareness_enabled() -> bool {
    true
}

fn default_combat_hit_duration_seconds() -> f32 {
    DEFAULT_COMBAT_HIT_DURATION_SECONDS
}

fn default_toast_enabled() -> bool {
    true
}

fn default_auto_update() -> bool {
    true
}

fn default_update_interval() -> u32 {
    7
}

fn default_auto_order() -> bool {
    true
}

fn default_hide_from_alt_tab() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            eq_dir: DEFAULT_EQ_DIR.to_string(),
            hide_hotkey: default_hide_hotkey(),
            pip_edge: PipEdge::default(),
            show_stonemite_button: default_show_stonemite_button(),
            stonemite_button_position: None,
            pip_strip_width: None,
            pip_opacity: None,
            pip_positions: Vec::new(),
            dps_overlay_enabled: default_dps_overlay_enabled(),
            dps_overlay_top_rows: default_dps_overlay_top_rows(),
            dps_overlay_placement: None,
            snap_grid: default_snap_grid(),
            pip_label_height: None,
            pip_label_opacity: None,
            pip_label_font_family: None,
            pip_label_font_scale: None,
            pip_label_font_weight: None,
            auto_order: default_auto_order(),
            box_order: Vec::new(),
            hide_from_alt_tab: default_hide_from_alt_tab(),
            trusik: false,
            swap_hotkeys: default_swap_hotkeys(),
            box_cycles: Vec::new(),
            settings_position: None,
            tell_visual_enabled: default_tell_visual_enabled(),
            tell_sound_enabled: default_tell_sound_enabled(),
            tell_sound: default_tell_sound(),
            notify_tells: default_notification_event_enabled(),
            notify_group_invites: default_notification_event_enabled(),
            notify_raid_invites: default_notification_event_enabled(),
            notify_trade_proposals: default_notification_event_enabled(),
            notify_resurrections: default_notification_event_enabled(),
            notify_deaths: default_notification_event_enabled(),
            notify_level_gains: default_notification_event_enabled(),
            notify_aa_gains: default_notification_event_enabled(),
            aa_points_per_notification: default_aa_points_per_notification(),
            combat_awareness_enabled: default_combat_awareness_enabled(),
            combat_hit_duration_seconds: default_combat_hit_duration_seconds(),
            broadcast_hotkey: default_broadcast_hotkey(),
            disable_broadcast_when_clients_exit: default_disable_broadcast_when_clients_exit(),
            mouse_clutch_key: default_mouse_clutch_key(),
            broadcast_filter_mode: default_broadcast_filter_mode(),
            broadcast_filter_keys: Vec::new(),
            toast_enabled: default_toast_enabled(),
            toast_height: None,
            toast_duration: None,
            auto_update_check: default_auto_update(),
            update_check_interval_days: default_update_interval(),
            last_update_check: None,
            accounts: Vec::new(),
            server: String::new(),
            trushar: TrusharConfig::default(),
        }
    }
}

pub(crate) struct ConfigLock(HANDLE);

impl ConfigLock {
    fn acquire() -> std::io::Result<Self> {
        let handle = unsafe { CreateMutexW(None, false, w!("Local\\Laikasoft.Stonemite.Config")) }
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Ok(Self(handle))
        } else {
            let error = std::io::Error::last_os_error();
            unsafe {
                let _ = CloseHandle(handle);
            }
            Err(error)
        }
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

impl Config {
    pub fn effective_dps_overlay_top_rows(&self) -> u8 {
        DPS_OVERLAY_TOP_ROW_OPTIONS
            .contains(&self.dps_overlay_top_rows)
            .then_some(self.dps_overlay_top_rows)
            .unwrap_or(DEFAULT_DPS_OVERLAY_TOP_ROWS)
    }

    pub fn effective_pip_opacity(&self) -> u32 {
        self.pip_opacity
            .unwrap_or(DEFAULT_PIP_OPACITY)
            .clamp(MIN_PIP_OPACITY, MAX_PIP_OPACITY)
    }

    pub fn effective_pip_label_font_family(&self) -> &str {
        self.pip_label_font_family
            .as_deref()
            .map(str::trim)
            .filter(|family| {
                !family.is_empty()
                    && family.encode_utf16().count() <= MAX_PIP_LABEL_FONT_FAMILY_LEN
                    && !family.chars().any(char::is_control)
            })
            .unwrap_or(DEFAULT_PIP_LABEL_FONT_FAMILY)
    }

    pub fn effective_pip_label_font_scale(&self) -> u32 {
        self.pip_label_font_scale
            .unwrap_or(DEFAULT_PIP_LABEL_FONT_SCALE)
            .clamp(MIN_PIP_LABEL_FONT_SCALE, MAX_PIP_LABEL_FONT_SCALE)
    }

    pub fn effective_pip_label_font_weight(&self) -> LabelFontWeight {
        self.pip_label_font_weight.unwrap_or_default()
    }

    pub fn effective_aa_points_per_notification(&self) -> u32 {
        self.aa_points_per_notification.clamp(
            MIN_AA_POINTS_PER_NOTIFICATION,
            MAX_AA_POINTS_PER_NOTIFICATION,
        )
    }

    pub fn effective_combat_hit_duration_seconds(&self) -> f32 {
        if self.combat_hit_duration_seconds.is_finite() {
            self.combat_hit_duration_seconds.clamp(
                MIN_COMBAT_HIT_DURATION_SECONDS,
                MAX_COMBAT_HIT_DURATION_SECONDS,
            )
        } else {
            DEFAULT_COMBAT_HIT_DURATION_SECONDS
        }
    }

    pub fn effective_combat_hit_duration_ms(&self) -> u64 {
        (self.effective_combat_hit_duration_seconds() * 1000.0).round() as u64
    }

    /// Return the config directory: %APPDATA%\Stonemite\
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join("Stonemite"))
    }

    /// Return the config file path: %APPDATA%\Stonemite\config.toml
    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.toml"))
    }

    pub(crate) fn lock() -> std::io::Result<ConfigLock> {
        ConfigLock::acquire()
    }

    fn load_unlocked() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        }
    }

    fn save_unlocked(&self) -> std::io::Result<()> {
        let Some(dir) = Self::dir() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "APPDATA not set",
            ));
        };
        std::fs::create_dir_all(&dir)?;
        let contents = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("config.toml"), contents)
    }

    /// Load config from disk. Creates default config file if it doesn't exist.
    pub fn load() -> Self {
        let _lock = match Self::lock() {
            Ok(lock) => lock,
            Err(error) => {
                eprintln!("Failed to lock config: {error}");
                return Self::default();
            }
        };
        let config = Self::load_unlocked();
        if let Err(error) = config.save_unlocked() {
            eprintln!("Failed to save config: {error}");
        }
        config
    }

    /// Save config to disk, creating the directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let _lock = Self::lock()?;
        self.save_unlocked()
    }

    /// Atomically serialize a read-modify-write sequence with other processes.
    pub(crate) fn update(mutator: impl FnOnce(&mut Self)) -> std::io::Result<()> {
        let _lock = Self::lock()?;
        let mut config = Self::load_unlocked();
        mutator(&mut config);
        config.save_unlocked()
    }

    /// Resolve the EQ directory from config.
    pub fn eq_directory(&self) -> PathBuf {
        PathBuf::from(&self.eq_dir)
    }

    /// Parse the hide_hotkey config string into (modifiers, virtual-key code).
    /// Supports combos like "Ctrl+Shift+F9".
    pub fn hide_hotkey_vk(&self) -> Option<(u32, u32)> {
        parse_hotkey_combo(&self.hide_hotkey)
    }

    /// Parse the broadcast_hotkey config string into (modifiers, virtual-key code).
    pub fn broadcast_hotkey_vk(&self) -> Option<(u32, u32)> {
        parse_hotkey_combo(&self.broadcast_hotkey)
    }

    /// Validate and parse the optional single Mouse Clutch key.
    pub fn mouse_clutch_vk(&self) -> Result<Option<u32>, String> {
        validate_mouse_clutch_binding(
            &self.mouse_clutch_key,
            &self.hide_hotkey,
            &self.broadcast_hotkey,
            &self.swap_hotkeys,
            &self.box_cycles,
        )
    }

    /// Read `LastServerName` from the main `eqlsPlayerData.ini` in the EQ directory.
    pub fn read_server_from_ini(&self) -> Option<String> {
        let path = self.eq_directory().join("eqlsPlayerData.ini");
        read_ini_value(&path, "MISC", "LastServerName")
    }

    /// Write `LastServerName` to all `eqlsPlayerData*.ini` files in the EQ directory.
    pub fn write_server_to_ini(&self) {
        if self.server.is_empty() {
            return;
        }
        let eq_dir = self.eq_directory();
        let entries = match std::fs::read_dir(&eq_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("eqlsPlayerData") && name_str.ends_with(".ini") {
                let path = entry.path();
                write_ini_value(&path, "MISC", "LastServerName", &self.server);
            }
        }
    }

    /// Parse swap hotkey at the given index (0-based) into (modifiers, virtual-key code).
    pub fn swap_hotkey_vk(&self, index: usize) -> Option<(u32, u32)> {
        self.swap_hotkeys
            .get(index)
            .and_then(|s| parse_hotkey_combo(s))
    }
}

/// Map a key name (case-insensitive) to a Windows virtual-key code.
pub fn parse_vk_name(name: &str) -> Option<u32> {
    match name.trim().to_uppercase().as_str() {
        // Function keys
        "F1" => Some(0x70),
        "F2" => Some(0x71),
        "F3" => Some(0x72),
        "F4" => Some(0x73),
        "F5" => Some(0x74),
        "F6" => Some(0x75),
        "F7" => Some(0x76),
        "F8" => Some(0x77),
        "F9" => Some(0x78),
        "F10" => Some(0x79),
        "F11" => Some(0x7A),
        "F12" => Some(0x7B),
        "F13" => Some(0x7C),
        "F14" => Some(0x7D),
        "F15" => Some(0x7E),
        "F16" => Some(0x7F),
        "F17" => Some(0x80),
        "F18" => Some(0x81),
        "F19" => Some(0x82),
        "F20" => Some(0x83),
        "F21" => Some(0x84),
        "F22" => Some(0x85),
        "F23" => Some(0x86),
        "F24" => Some(0x87),
        // Navigation
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PAGE_UP" => Some(0x21),
        "PAGEDOWN" | "PAGE_DOWN" => Some(0x22),
        // Toggle keys
        "PAUSE" => Some(0x13),
        "SCROLLLOCK" | "SCROLL_LOCK" => Some(0x91),
        // Letters
        "A" => Some(0x41),
        "B" => Some(0x42),
        "C" => Some(0x43),
        "D" => Some(0x44),
        "E" => Some(0x45),
        "F" => Some(0x46),
        "G" => Some(0x47),
        "H" => Some(0x48),
        "I" => Some(0x49),
        "J" => Some(0x4A),
        "K" => Some(0x4B),
        "L" => Some(0x4C),
        "M" => Some(0x4D),
        "N" => Some(0x4E),
        "O" => Some(0x4F),
        "P" => Some(0x50),
        "Q" => Some(0x51),
        "R" => Some(0x52),
        "S" => Some(0x53),
        "T" => Some(0x54),
        "U" => Some(0x55),
        "V" => Some(0x56),
        "W" => Some(0x57),
        "X" => Some(0x58),
        "Y" => Some(0x59),
        "Z" => Some(0x5A),
        // Digits
        "0" => Some(0x30),
        "1" => Some(0x31),
        "2" => Some(0x32),
        "3" => Some(0x33),
        "4" => Some(0x34),
        "5" => Some(0x35),
        "6" => Some(0x36),
        "7" => Some(0x37),
        "8" => Some(0x38),
        "9" => Some(0x39),
        // Other
        "SPACE" => Some(0x20),
        "TAB" => Some(0x09),
        "MINUS" => Some(0xBD),
        "PLUS" => Some(0xBB),
        "EQUALS" => Some(0xBB),
        "BACKTICK" => Some(0xC0),
        "OPENBRACKET" => Some(0xDB),
        "CLOSEBRACKET" => Some(0xDD),
        "BACKSLASH" => Some(0xDC),
        "SEMICOLON" => Some(0xBA),
        "QUOTE" => Some(0xDE),
        "COMMA" => Some(0xBC),
        "PERIOD" => Some(0xBE),
        "SLASH" => Some(0xBF),
        _ => None,
    }
}

fn configured_global_hotkeys<'a>(
    hide_hotkey: &'a str,
    broadcast_hotkey: &'a str,
    swap_hotkeys: &'a [String],
    box_cycles: &'a [BoxCycle],
) -> Vec<(String, &'a str)> {
    let mut bindings = Vec::with_capacity(2 + swap_hotkeys.len() + box_cycles.len() * 2);
    bindings.push(("Hide overlay".to_owned(), hide_hotkey));
    bindings.push(("Broadcast toggle".to_owned(), broadcast_hotkey));
    bindings.extend(
        swap_hotkeys
            .iter()
            .enumerate()
            .map(|(index, hotkey)| (format!("Window {}", index + 1), hotkey.as_str())),
    );
    for cycle in box_cycles {
        bindings.push((format!("{} next", cycle.name), cycle.next_hotkey.as_str()));
        bindings.push((
            format!("{} previous", cycle.name),
            cycle.previous_hotkey.as_str(),
        ));
    }
    bindings
}

/// Validate every RegisterHotKey binding and reject exact chord collisions.
pub fn validate_global_hotkey_bindings(
    hide_hotkey: &str,
    broadcast_hotkey: &str,
    swap_hotkeys: &[String],
    box_cycles: &[BoxCycle],
) -> Result<(), String> {
    let mut seen: HashMap<(u32, u32), (String, String)> = HashMap::new();
    for (label, binding) in
        configured_global_hotkeys(hide_hotkey, broadcast_hotkey, swap_hotkeys, box_cycles)
    {
        let binding = binding.trim();
        if binding.is_empty() {
            continue;
        }
        let Some(chord) = parse_hotkey_combo(binding) else {
            return Err(format!("{label} hotkey '{binding}' is not supported"));
        };
        if let Some((existing_label, existing_binding)) = seen.get(&chord) {
            return Err(format!(
                "{label} hotkey ({binding}) conflicts with {existing_label} ({existing_binding})"
            ));
        }
        seen.insert(chord, (label, binding.to_owned()));
    }
    Ok(())
}

/// Validate Mouse Clutch's single-key syntax and reject collisions with every
/// global Stonemite hotkey. Modifier differences still collide because the
/// low-level clutch hook sees and swallows the underlying key.
pub fn validate_mouse_clutch_binding(
    binding: &str,
    hide_hotkey: &str,
    broadcast_hotkey: &str,
    swap_hotkeys: &[String],
    box_cycles: &[BoxCycle],
) -> Result<Option<u32>, String> {
    let binding = binding.trim();
    if binding.is_empty() {
        return Ok(None);
    }
    if binding.contains('+') {
        return Err("Mouse Clutch uses one key without modifiers".to_owned());
    }
    let Some(vk) = parse_vk_name(binding) else {
        return Err(format!("Mouse Clutch key '{binding}' is not supported"));
    };

    for (label, hotkey) in
        configured_global_hotkeys(hide_hotkey, broadcast_hotkey, swap_hotkeys, box_cycles)
    {
        if parse_hotkey_combo(hotkey).is_some_and(|(_, existing_vk)| existing_vk == vk) {
            return Err(format!("Mouse Clutch conflicts with {label} ({hotkey})"));
        }
    }

    Ok(Some(vk))
}

/// Parse a hotkey combo string like "Ctrl+Shift+F9" into (MOD flags, VK code).
/// MOD flags: 0x1 = Alt, 0x2 = Ctrl, 0x4 = Shift.
fn parse_hotkey_combo(combo: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut key_part = None;

    for part in combo.split('+') {
        match part.trim().to_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= 0x2,
            "ALT" => mods |= 0x1,
            "SHIFT" => mods |= 0x4,
            _ => key_part = Some(part.trim().to_string()),
        }
    }

    let vk = parse_vk_name(key_part.as_deref().unwrap_or(""))?;
    Some((mods, vk))
}

/// Write a key=value pair under [section] in a Windows INI file.
/// Creates the section if missing, replaces the key if it exists.
fn write_ini_value(path: &Path, section: &str, key: &str, value: &str) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let section_header = format!("[{section}]");
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    // Find the section.
    let section_pos = lines
        .iter()
        .position(|l| l.trim().eq_ignore_ascii_case(&section_header));

    if let Some(sec_idx) = section_pos {
        // Find existing key in this section.
        let mut key_idx = None;
        for (i, line) in lines.iter().enumerate().skip(sec_idx + 1) {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                break;
            }
            if trimmed.starts_with(&format!("{key}=")) || trimmed.starts_with(&format!("{key} =")) {
                key_idx = Some(i);
                break;
            }
        }
        if let Some(ki) = key_idx {
            lines[ki] = format!("{key}={value}");
        } else {
            lines.insert(sec_idx + 1, format!("{key}={value}"));
        }
    } else {
        // Append new section.
        lines.push(String::new());
        lines.push(section_header);
        lines.push(format!("{key}={value}"));
    }

    let _ = std::fs::write(path, lines.join("\r\n"));
}

/// Read a value for key under [section] from a Windows INI file.
fn read_ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let section_header = format!("[{section}]");
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case(&section_header) {
            in_section = true;
            continue;
        }
        if trimmed.starts_with('[') {
            if in_section {
                break;
            }
            continue;
        }
        if in_section {
            if let Some(val) = trimmed.strip_prefix(key) {
                let val = val.trim_start();
                if let Some(val) = val.strip_prefix('=') {
                    return Some(val.trim().to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_enables_loopback_integrations_by_default() {
        let config: Config = toml::from_str("eq_dir = 'C:\\EverQuest'").unwrap();

        assert!(config.show_stonemite_button);
        assert!(config.dps_overlay_enabled);
        assert_eq!(config.effective_dps_overlay_top_rows(), 10);
        assert_eq!(config.dps_overlay_placement, None);
        assert_eq!(config.mouse_clutch_key, "F13");
        assert_eq!(config.mouse_clutch_vk(), Ok(Some(0x7c)));
        assert!(config.tell_visual_enabled);
        assert!(config.tell_sound_enabled);
        assert_eq!(config.tell_sound, "tell.wav");
        assert!(config.notify_tells);
        assert!(config.notify_group_invites);
        assert!(config.notify_raid_invites);
        assert!(config.notify_trade_proposals);
        assert!(config.notify_resurrections);
        assert!(config.notify_deaths);
        assert!(config.notify_level_gains);
        assert!(config.notify_aa_gains);
        assert_eq!(
            config.effective_aa_points_per_notification(),
            DEFAULT_AA_POINTS_PER_NOTIFICATION
        );
        assert!(config.combat_awareness_enabled);
        assert_eq!(
            config.effective_combat_hit_duration_seconds(),
            DEFAULT_COMBAT_HIT_DURATION_SECONDS
        );
        assert!(config.box_order.is_empty());
        assert!(config.box_cycles.is_empty());
        assert!(config.trushar.enabled);
        assert_eq!(config.trushar.bind, "127.0.0.1:19720");
        assert_eq!(config.trushar.auth_token, None);
    }

    #[test]
    fn box_order_round_trips_full_identities() {
        let config: Config = toml::from_str(
            r#"
box_order = [
    { server = "xegony", character = "Laika" },
    { server = "bristlebane", character = "Foo" },
]
"#,
        )
        .unwrap();

        assert_eq!(
            config.box_order,
            vec![
                BoxIdentity {
                    server: "xegony".into(),
                    character: "Laika".into(),
                },
                BoxIdentity {
                    server: "bristlebane".into(),
                    character: "Foo".into(),
                },
            ]
        );
        assert!(config.box_order[0].matches("XEGONY", "laika"));

        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.box_order, config.box_order);
    }

    #[test]
    fn box_cycles_round_trip_named_directional_rings() {
        let config: Config = toml::from_str(
            r#"
box_cycles = [
    { name = "Melee", next_hotkey = "F14", previous_hotkey = "F15", members = [
        { server = "xegony", character = "Tank" },
        { server = "xegony", character = "Rogue" },
    ] },
]
"#,
        )
        .unwrap();

        assert_eq!(config.box_cycles.len(), 1);
        let cycle = &config.box_cycles[0];
        assert_eq!(cycle.name, "Melee");
        assert_eq!(cycle.next_hotkey_vk(), Some((0, 0x7d)));
        assert_eq!(cycle.previous_hotkey_vk(), Some((0, 0x7e)));
        assert_eq!(cycle.members[1].character, "Rogue");

        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.box_cycles, config.box_cycles);
    }

    #[test]
    fn in_game_button_defaults_on_for_new_and_existing_configs() {
        assert!(Config::default().show_stonemite_button);
        assert_eq!(Config::default().stonemite_button_position, None);
        let upgraded: Config = toml::from_str("eq_dir = 'C:\\EverQuest'").unwrap();
        assert!(upgraded.show_stonemite_button);
        assert_eq!(upgraded.stonemite_button_position, None);
        let disabled: Config = toml::from_str("show_stonemite_button = false").unwrap();
        assert!(!disabled.show_stonemite_button);
    }

    #[test]
    fn broadcast_exit_shutoff_defaults_on_for_new_and_existing_configs() {
        assert!(Config::default().disable_broadcast_when_clients_exit);
        let upgraded: Config = toml::from_str("eq_dir = 'C:\\EverQuest'").unwrap();
        assert!(upgraded.disable_broadcast_when_clients_exit);
        let disabled: Config =
            toml::from_str("disable_broadcast_when_clients_exit = false").unwrap();
        assert!(!disabled.disable_broadcast_when_clients_exit);
    }

    #[test]
    fn in_game_button_position_round_trips() {
        let config: Config = toml::from_str("stonemite_button_position = [0.25, 0.75]").unwrap();
        assert_eq!(config.stonemite_button_position, Some([0.25, 0.75]));

        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.stonemite_button_position, Some([0.25, 0.75]));
    }

    #[test]
    fn dps_overlay_defaults_validates_rows_and_round_trips_dip_placement() {
        let upgraded: Config = toml::from_str("eq_dir = 'C:\\EverQuest'").unwrap();
        assert!(upgraded.dps_overlay_enabled);
        assert_eq!(upgraded.effective_dps_overlay_top_rows(), 10);

        let malformed = Config {
            dps_overlay_top_rows: 12,
            ..Config::default()
        };
        assert_eq!(malformed.effective_dps_overlay_top_rows(), 10);

        let config = Config {
            dps_overlay_top_rows: 15,
            dps_overlay_placement: Some(DpsOverlayPlacement {
                x_dip: 24,
                y_dip: 32,
                width_dip: 500,
            }),
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.effective_dps_overlay_top_rows(), 15);
        assert_eq!(reparsed.dps_overlay_placement, config.dps_overlay_placement);
    }

    #[test]
    fn minimal_trushar_config_uses_safe_defaults_for_other_fields() {
        let config: Config = toml::from_str("[trushar]\nenabled = true").unwrap();

        assert!(config.trushar.enabled);
        assert_eq!(config.eq_dir, DEFAULT_EQ_DIR);
        assert_eq!(config.trushar.bind, "127.0.0.1:19720");
        assert_eq!(config.trushar.auth_token, None);
    }

    #[test]
    fn pip_opacity_defaults_and_clamps_malformed_ranges() {
        assert_eq!(
            Config::default().effective_pip_opacity(),
            DEFAULT_PIP_OPACITY
        );
        let low = Config {
            pip_opacity: Some(0),
            ..Config::default()
        };
        let high = Config {
            pip_opacity: Some(500),
            ..Config::default()
        };
        assert_eq!(low.effective_pip_opacity(), MIN_PIP_OPACITY);
        assert_eq!(high.effective_pip_opacity(), MAX_PIP_OPACITY);
    }

    #[test]
    fn aa_notifications_default_on_and_clamp_the_interval() {
        let default = Config::default();
        assert!(default.notify_aa_gains);
        assert_eq!(default.effective_aa_points_per_notification(), 1);

        let low = Config {
            aa_points_per_notification: 0,
            ..Config::default()
        };
        let high = Config {
            aa_points_per_notification: 101,
            ..Config::default()
        };
        assert_eq!(low.effective_aa_points_per_notification(), 1);
        assert_eq!(high.effective_aa_points_per_notification(), 100);
    }

    #[test]
    fn combat_awareness_defaults_on_and_clamps_hit_duration() {
        let default = Config::default();
        assert!(default.combat_awareness_enabled);
        assert_eq!(default.effective_combat_hit_duration_ms(), 3_000);

        let low = Config {
            combat_hit_duration_seconds: 0.1,
            ..Config::default()
        };
        let high = Config {
            combat_hit_duration_seconds: 30.0,
            ..Config::default()
        };
        assert_eq!(low.effective_combat_hit_duration_seconds(), 0.5);
        assert_eq!(high.effective_combat_hit_duration_seconds(), 10.0);
    }

    #[test]
    fn malformed_label_typography_falls_back_without_discarding_config() {
        let config: Config = toml::from_str(
            "eq_dir = 'D:\\EverQuest'\npip_label_font_family = 42\npip_label_font_scale = 'huge'\npip_label_font_weight = 'medium'\n",
        )
        .unwrap();

        assert_eq!(config.eq_dir, r"D:\EverQuest");
        assert_eq!(
            config.effective_pip_label_font_family(),
            DEFAULT_PIP_LABEL_FONT_FAMILY
        );
        assert_eq!(
            config.effective_pip_label_font_scale(),
            DEFAULT_PIP_LABEL_FONT_SCALE
        );
        assert_eq!(
            config.effective_pip_label_font_weight(),
            LabelFontWeight::Bold
        );
    }

    #[test]
    fn legacy_telemetry_fields_are_discarded_when_resaving() {
        let config: Config =
            toml::from_str("telemetry = true\ntelemetry_id = 'legacy-id'\n").unwrap();

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(!serialized.contains("telemetry"));
        assert!(!serialized.contains("legacy-id"));
    }

    #[test]
    fn debug_output_redacts_the_integration_credential() {
        let config = TrusharConfig {
            enabled: true,
            bind: "0.0.0.0:19720".into(),
            auth_token: Some("do-not-print-this".into()),
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("do-not-print-this"));
    }

    #[test]
    fn mouse_clutch_parses_extended_function_keys() {
        assert_eq!(parse_vk_name("F13"), Some(0x7c));
        assert_eq!(parse_vk_name("f24"), Some(0x87));

        let config = Config {
            mouse_clutch_key: "F24".into(),
            ..Config::default()
        };
        assert_eq!(config.mouse_clutch_vk(), Ok(Some(0x87)));

        let serialized = toml::to_string_pretty(&config).unwrap();
        let reparsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.mouse_clutch_key, "F24");
    }

    #[test]
    fn mouse_clutch_rejects_modifiers_invalid_keys_and_hotkey_collisions() {
        let swaps = default_swap_hotkeys();
        assert!(validate_mouse_clutch_binding("Ctrl+F13", "F9", "Pause", &swaps, &[]).is_err());
        assert!(validate_mouse_clutch_binding("NoSuchKey", "F9", "Pause", &swaps, &[]).is_err());
        assert!(validate_mouse_clutch_binding("F9", "F9", "Pause", &swaps, &[]).is_err());
        assert!(validate_mouse_clutch_binding("Pause", "F9", "Pause", &swaps, &[]).is_err());
        assert!(validate_mouse_clutch_binding("F1", "F9", "Pause", &swaps, &[]).is_err());
        assert_eq!(
            validate_mouse_clutch_binding("F13", "F9", "Pause", &swaps, &[]),
            Ok(Some(0x7c))
        );
    }

    #[test]
    fn cycle_hotkeys_reject_exact_chord_and_mouse_clutch_collisions() {
        let swaps = default_swap_hotkeys();
        let cycle = BoxCycle {
            name: "Melee".into(),
            next_hotkey: "F14".into(),
            previous_hotkey: "Shift+F14".into(),
            members: Vec::new(),
        };
        assert_eq!(
            validate_global_hotkey_bindings("F9", "Pause", &swaps, &[cycle.clone()]),
            Ok(())
        );
        assert!(
            validate_mouse_clutch_binding("F14", "F9", "Pause", &swaps, &[cycle.clone()]).is_err()
        );

        let duplicate = BoxCycle {
            previous_hotkey: "F14".into(),
            ..cycle
        };
        assert!(validate_global_hotkey_bindings("F9", "Pause", &swaps, &[duplicate]).is_err());
    }
}
