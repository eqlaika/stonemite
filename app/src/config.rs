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

    /// Parse the hide_hotkey config string into a Windows virtual-key code.
    pub fn hide_hotkey_vk(&self) -> Option<u32> {
        parse_vk_name(&self.hide_hotkey)
    }
}

/// Map a key name (case-insensitive) to a Windows virtual-key code.
fn parse_vk_name(name: &str) -> Option<u32> {
    match name.trim().to_uppercase().as_str() {
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
        "PAUSE" => Some(0x13),
        "SCROLLLOCK" | "SCROLL_LOCK" => Some(0x91),
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" | "PAGE_UP" => Some(0x21),
        "PAGEDOWN" | "PAGE_DOWN" => Some(0x22),
        _ => None,
    }
}
