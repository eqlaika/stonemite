use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_EQ_DIR: &str =
    r"C:\Users\Public\Daybreak Game Company\Installed Games\EverQuest";

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

/// Top-level configuration persisted to %APPDATA%\Stonemite\config.toml.
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Path to the EverQuest installation directory.
    pub eq_dir: String,
    /// Key name for the hide-overlay hotkey (e.g. "F9", "F12"). Default: "F9".
    #[serde(default = "default_hide_hotkey")]
    pub hide_hotkey: String,
    /// Screen edge for the PiP strip: right, left, top, bottom.
    #[serde(default)]
    pub pip_edge: PipEdge,
    /// Custom PiP strip width in pixels. None = auto-size.
    #[serde(default)]
    pub pip_strip_width: Option<u32>,
    /// Per-pip custom positions. Empty = auto strip layout.
    #[serde(default)]
    pub pip_positions: Vec<PipPosition>,
    /// Snap grid size in pixels. 0 = no grid snap. Default: 16.
    #[serde(default = "default_snap_grid")]
    pub snap_grid: u32,
    /// PiP label height in pixels. None = default (48).
    #[serde(default)]
    pub pip_label_height: Option<u32>,
    /// PiP label opacity as a percentage (0–100). None = default (80).
    #[serde(default)]
    pub pip_label_opacity: Option<u32>,
    /// Enable trusik DLL proxy for character detection. Requires restart.
    #[serde(default)]
    pub trusik: bool,
    /// Hotkey for toggling key broadcasting. Default: "Pause".
    #[serde(default = "default_broadcast_hotkey")]
    pub broadcast_hotkey: String,
    /// Filter mode: "blacklist" or "whitelist". Default: "blacklist".
    #[serde(default = "default_broadcast_filter_mode")]
    pub broadcast_filter_mode: String,
    /// Key names to filter (e.g. "Enter", "Escape").
    #[serde(default)]
    pub broadcast_filter_keys: Vec<String>,
    /// Hotkeys for swapping to specific window slots (1–6). Default: Ctrl+F1..Ctrl+F6.
    #[serde(default = "default_swap_hotkeys")]
    pub swap_hotkeys: Vec<String>,
    /// Remembered settings window position [x, y].
    #[serde(default)]
    pub settings_position: Option<[f32; 2]>,
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
    /// Enable anonymous usage telemetry. Default: true.
    #[serde(default = "default_telemetry")]
    pub telemetry: bool,
    /// Anonymous user identifier (auto-generated UUID).
    #[serde(default)]
    pub telemetry_id: Option<String>,
}

fn default_hide_hotkey() -> String {
    "F9".to_string()
}

fn default_snap_grid() -> u32 {
    16
}

fn default_broadcast_hotkey() -> String {
    "Pause".to_string()
}

fn default_broadcast_filter_mode() -> String {
    "blacklist".to_string()
}

fn default_swap_hotkeys() -> Vec<String> {
    (1..=6).map(|i| format!("Ctrl+F{i}")).collect()
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

fn default_telemetry() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            eq_dir: DEFAULT_EQ_DIR.to_string(),
            hide_hotkey: default_hide_hotkey(),
            pip_edge: PipEdge::default(),
            pip_strip_width: None,
            pip_positions: Vec::new(),
            snap_grid: default_snap_grid(),
            pip_label_height: None,
            pip_label_opacity: None,
            trusik: false,
            swap_hotkeys: default_swap_hotkeys(),
            settings_position: None,
            broadcast_hotkey: default_broadcast_hotkey(),
            broadcast_filter_mode: default_broadcast_filter_mode(),
            broadcast_filter_keys: Vec::new(),
            toast_enabled: default_toast_enabled(),
            toast_height: None,
            toast_duration: None,
            auto_update_check: default_auto_update(),
            update_check_interval_days: default_update_interval(),
            last_update_check: None,
            telemetry: true,
            telemetry_id: None,
        }
    }
}

impl Config {
    /// Return the config directory: %APPDATA%\Stonemite\
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join("Stonemite"))
    }

    /// Return the config file path: %APPDATA%\Stonemite\config.toml
    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.toml"))
    }

    /// Load config from disk. Creates default config file if it doesn't exist.
    /// Auto-generates a telemetry_id if missing and telemetry is enabled.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let mut config = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
                Err(_) => Self::default(),
            }
        } else {
            Self::default()
        };
        // Generate a stable anonymous ID on first run.
        if config.telemetry && config.telemetry_id.is_none() {
            config.telemetry_id = Some(uuid::Uuid::new_v4().to_string());
        }
        if let Err(e) = config.save() {
            eprintln!("Failed to save config: {e}");
        }
        config
    }

    /// Save config to disk, creating the directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(dir) = Self::dir() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "APPDATA not set",
            ));
        };
        std::fs::create_dir_all(&dir)?;
        let contents =
            toml::to_string_pretty(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(dir.join("config.toml"), contents)
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
        "F1" => Some(0x70),  "F2" => Some(0x71),  "F3" => Some(0x72),  "F4" => Some(0x73),
        "F5" => Some(0x74),  "F6" => Some(0x75),  "F7" => Some(0x76),  "F8" => Some(0x77),
        "F9" => Some(0x78),  "F10" => Some(0x79), "F11" => Some(0x7A), "F12" => Some(0x7B),
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
        "A" => Some(0x41), "B" => Some(0x42), "C" => Some(0x43), "D" => Some(0x44),
        "E" => Some(0x45), "F" => Some(0x46), "G" => Some(0x47), "H" => Some(0x48),
        "I" => Some(0x49), "J" => Some(0x4A), "K" => Some(0x4B), "L" => Some(0x4C),
        "M" => Some(0x4D), "N" => Some(0x4E), "O" => Some(0x4F), "P" => Some(0x50),
        "Q" => Some(0x51), "R" => Some(0x52), "S" => Some(0x53), "T" => Some(0x54),
        "U" => Some(0x55), "V" => Some(0x56), "W" => Some(0x57), "X" => Some(0x58),
        "Y" => Some(0x59), "Z" => Some(0x5A),
        // Digits
        "0" => Some(0x30), "1" => Some(0x31), "2" => Some(0x32), "3" => Some(0x33),
        "4" => Some(0x34), "5" => Some(0x35), "6" => Some(0x36), "7" => Some(0x37),
        "8" => Some(0x38), "9" => Some(0x39),
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
