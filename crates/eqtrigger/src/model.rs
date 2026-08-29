//! Native trigger schema.
//!
//! The native model is a normalized superset of what Stonemite executes.
//! Fields that EQLP defines but Stonemite never executes (webhooks, chat
//! sending, clipboard sharing, …) are retained verbatim inside
//! [`Passthrough`] maps so a round-trip export reproduces them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Current native schema version written by [`crate::store`].
pub const SCHEMA_VERSION: u32 = 1;

/// Unknown source-format fields retained for re-export, keyed by their
/// original (EQLP PascalCase) field name.
pub type Passthrough = BTreeMap<String, Value>;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

id_type!(TriggerId);
id_type!(FolderId);
id_type!(ProfileId);
id_type!(OverlayId);

/// The complete trigger library persisted as `library.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TriggerLibrary {
    pub schema_version: u32,
    pub folders: Vec<Folder>,
    pub triggers: Vec<Trigger>,
    pub profiles: Vec<Profile>,
    pub text_overlays: Vec<TextOverlayPreset>,
    pub timer_overlays: Vec<TimerOverlayPreset>,
    pub assets: Vec<AssetRecord>,
}

impl TriggerLibrary {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ..Self::default()
        }
    }

    pub fn trigger(&self, id: TriggerId) -> Option<&Trigger> {
        self.triggers.iter().find(|trigger| trigger.id == id)
    }

    pub fn trigger_mut(&mut self, id: TriggerId) -> Option<&mut Trigger> {
        self.triggers.iter_mut().find(|trigger| trigger.id == id)
    }

    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// Ids of `folder` and every folder transitively under it.
    pub fn folder_subtree(&self, folder: FolderId) -> Vec<FolderId> {
        let mut result = vec![folder];
        let mut cursor = 0;
        while cursor < result.len() {
            let parent = result[cursor];
            cursor += 1;
            for child in &self.folders {
                if child.parent == Some(parent) && !result.contains(&child.id) {
                    result.push(child.id);
                }
            }
        }
        result
    }
}

/// Category-tree node. Folders exist purely to organize triggers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    pub parent: Option<FolderId>,
    pub index: u32,
    pub expanded: bool,
}

impl Default for Folder {
    fn default() -> Self {
        Self {
            id: FolderId::new(),
            name: String::new(),
            parent: None,
            index: 0,
            expanded: true,
        }
    }
}

/// Where a trigger's presentation is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresentationTarget {
    /// Follow the client whose log produced the activation.
    #[default]
    Source,
    /// Show on whichever client currently has focus.
    ActiveClient,
    /// Show on every managed client.
    AllClients,
    /// Show once, globally (not attached to any client).
    Global,
}

/// A match pattern: literal contains-text or a (.NET-compatible) regex.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pattern {
    pub text: String,
    pub use_regex: bool,
}

impl Pattern {
    pub fn literal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            use_regex: false,
        }
    }

    pub fn regex(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            use_regex: true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// EQLP timer types, mirrored one-to-one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimerKind {
    #[default]
    Countdown,
    FastCountdown,
    Progress,
    /// Restarts itself `times_to_loop` times after each natural end.
    Looping,
}

impl TimerKind {
    /// EQLP `TimerType` integer (1-4). 0 means "no timer" and is modeled by
    /// `Trigger::timer == None`.
    pub fn eqlp_code(self) -> i64 {
        match self {
            TimerKind::Countdown => 1,
            TimerKind::FastCountdown => 2,
            TimerKind::Progress => 3,
            TimerKind::Looping => 4,
        }
    }

    pub fn from_eqlp_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(TimerKind::Countdown),
            2 => Some(TimerKind::FastCountdown),
            3 => Some(TimerKind::Progress),
            4 => Some(TimerKind::Looping),
            _ => None,
        }
    }

    /// Dynamic `{TS}` durations only apply to these kinds (EQLP behavior).
    pub fn accepts_dynamic_duration(self) -> bool {
        matches!(self, TimerKind::Countdown | TimerKind::Progress)
    }
}

/// What happens when a trigger with a running timer fires again
/// (EQLP `TriggerAgainOption`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimerRestartMode {
    /// 0: always start an additional timer.
    #[default]
    StartNew,
    /// 1: stop every timer for this trigger, then start fresh.
    RestartAll,
    /// 2: restart only a timer with the same display name.
    RestartSameName,
    /// 3: do nothing while any timer for this trigger runs.
    IgnoreIfAnyRunning,
    /// 4: do nothing while a timer with the same display name runs.
    IgnoreIfSameNameRunning,
}

impl TimerRestartMode {
    pub fn eqlp_code(self) -> i64 {
        match self {
            TimerRestartMode::StartNew => 0,
            TimerRestartMode::RestartAll => 1,
            TimerRestartMode::RestartSameName => 2,
            TimerRestartMode::IgnoreIfAnyRunning => 3,
            TimerRestartMode::IgnoreIfSameNameRunning => 4,
        }
    }

    pub fn from_eqlp_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(TimerRestartMode::StartNew),
            1 => Some(TimerRestartMode::RestartAll),
            2 => Some(TimerRestartMode::RestartSameName),
            3 => Some(TimerRestartMode::IgnoreIfAnyRunning),
            4 => Some(TimerRestartMode::IgnoreIfSameNameRunning),
            _ => None,
        }
    }
}

/// Text/speech/sound emitted at one point in a timer's lifecycle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimerStageActions {
    pub display_text: Option<String>,
    pub speak_text: Option<String>,
    pub sound: Option<String>,
}

impl TimerStageActions {
    pub fn is_empty(&self) -> bool {
        self.display_text.is_none() && self.speak_text.is_none() && self.sound.is_none()
    }
}

/// Full timer lifecycle configuration for a trigger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimerBehavior {
    pub kind: TimerKind,
    /// Display-name template. Empty means "use the trigger name".
    pub timer_name: String,
    pub duration_seconds: f64,
    /// Cooldown/reset window shown by progress-style overlays.
    pub reset_duration_seconds: f64,
    /// For [`TimerKind::Looping`]: additional automatic restarts.
    pub times_to_loop: u32,
    pub restart_mode: TimerRestartMode,
    /// Seconds before natural end at which the warning stage fires.
    pub warning_seconds: u32,
    pub warning: TimerStageActions,
    pub end: TimerStageActions,
    pub early_end: TimerStageActions,
    /// Up to three patterns that end the timer early.
    pub end_early_patterns: Vec<Pattern>,
    /// End early once `{counter}`/`{repeated}` reaches this count (0 = off).
    pub end_early_repeated_count: u32,
    /// Variable names cleared when the timer ends.
    pub end_clear_variables: Vec<String>,
}

impl Default for TimerBehavior {
    fn default() -> Self {
        Self {
            kind: TimerKind::Countdown,
            timer_name: String::new(),
            duration_seconds: 30.0,
            reset_duration_seconds: 0.0,
            times_to_loop: 0,
            restart_mode: TimerRestartMode::StartNew,
            warning_seconds: 0,
            warning: TimerStageActions::default(),
            end: TimerStageActions::default(),
            early_end: TimerStageActions::default(),
            end_early_patterns: Vec::new(),
            end_early_repeated_count: 0,
            end_clear_variables: Vec::new(),
        }
    }
}

/// EQLP variable action kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableOp {
    #[default]
    SetValue,
    SetCounter,
    Clear,
}

/// One stateful variable mutation performed when a trigger fires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VariableAction {
    pub op: VariableOp,
    pub name: String,
    /// SetValue: capture reference, variable reference, or literal text.
    pub value: String,
    /// SetCounter increment.
    pub step: f64,
    /// SetCounter starting point when the counter does not exist yet.
    pub initial_value: f64,
    /// 0 = never expires.
    pub time_to_live_seconds: f64,
}

impl Default for VariableAction {
    fn default() -> Self {
        Self {
            op: VariableOp::SetValue,
            name: String::new(),
            value: String::new(),
            step: 1.0,
            initial_value: 0.0,
            time_to_live_seconds: 0.0,
        }
    }
}

/// Why a trigger was quarantined instead of activated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Quarantine {
    /// Machine-readable reason class (e.g. `unsupported-regex`).
    pub reason: String,
    /// Human-readable detail shown by the manager.
    pub detail: String,
}

/// A native trigger definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Trigger {
    pub id: TriggerId,
    pub name: String,
    pub folder: Option<FolderId>,
    pub index: u32,
    pub enabled: bool,
    pub comments: String,

    // Matching --------------------------------------------------------
    pub pattern: Pattern,
    /// Requirement against the immediately previous log line.
    pub previous_pattern: Option<Pattern>,
    /// Match-variable expression evaluated after the pattern matches.
    /// Empty = always passes; non-empty but unparsable = never fires.
    pub condition: String,
    /// Seconds after a firing during which the trigger stays inert.
    pub lockout_seconds: f64,
    /// Window for `{repeated}`/`{counter}` resets (EQLP documents 750 ms).
    pub repeated_reset_seconds: f64,

    // State -----------------------------------------------------------
    pub variable_actions: Vec<VariableAction>,

    // Initial actions --------------------------------------------------
    pub display_text: Option<String>,
    pub speak_text: Option<String>,
    pub sound: Option<String>,

    // Timer -------------------------------------------------------------
    pub timer: Option<TimerBehavior>,

    // Audio parameters ---------------------------------------------------
    /// Lower value = more important; interrupts queued higher values.
    pub priority: i64,
    /// 0 = character/system default; otherwise EQLP stores rate+1.
    pub voice_rate: i32,
    /// EQLP volume-increase code (4 = no change).
    pub volume: i32,

    // Presentation --------------------------------------------------------
    pub text_overlays: Vec<OverlayId>,
    pub timer_overlays: Vec<OverlayId>,
    pub font_color: Option<String>,
    pub active_color: Option<String>,
    pub idle_color: Option<String>,
    pub reset_color: Option<String>,
    pub target: PresentationTarget,

    // Compatibility ------------------------------------------------------
    /// Set when the import found constructs Stonemite cannot execute
    /// faithfully; a quarantined trigger never activates but round-trips.
    pub quarantine: Option<Quarantine>,
    /// Retained-but-unexecuted source fields (EQLP names).
    pub passthrough: Passthrough,
}

impl Default for Trigger {
    fn default() -> Self {
        Self {
            id: TriggerId::new(),
            name: String::new(),
            folder: None,
            index: 0,
            enabled: false,
            comments: String::new(),
            pattern: Pattern::default(),
            previous_pattern: None,
            condition: String::new(),
            lockout_seconds: 0.0,
            repeated_reset_seconds: 0.75,
            variable_actions: Vec::new(),
            display_text: None,
            speak_text: None,
            sound: None,
            timer: None,
            priority: 3,
            voice_rate: 0,
            volume: 4,
            text_overlays: Vec::new(),
            timer_overlays: Vec::new(),
            font_color: None,
            active_color: None,
            idle_color: None,
            reset_color: None,
            target: PresentationTarget::Source,
            quarantine: None,
            passthrough: Passthrough::new(),
        }
    }
}

/// Character selector used by profile assignment. Matching is
/// case-insensitive; an empty server matches every server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CharacterSelector {
    pub character: String,
    pub server: String,
}

impl CharacterSelector {
    pub fn matches(&self, character: &str, server: &str) -> bool {
        self.character.eq_ignore_ascii_case(character)
            && (self.server.is_empty() || self.server.eq_ignore_ascii_case(server))
    }
}

/// Whether a profile applies everywhere or to selected characters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProfileAssignment {
    #[default]
    Global,
    Characters {
        characters: Vec<CharacterSelector>,
    },
}

/// A named selection of enabled triggers plus per-profile voice defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub enabled: bool,
    pub assignment: ProfileAssignment,
    /// Triggers enabled by this profile (in addition to each trigger's
    /// own `enabled` flag being true).
    pub triggers: Vec<TriggerId>,
    /// Folders whose entire subtree is enabled by this profile.
    pub folders: Vec<FolderId>,
    pub voice: Option<String>,
    pub voice_rate: i32,
    pub volume: i32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: ProfileId::new(),
            name: String::new(),
            enabled: true,
            assignment: ProfileAssignment::Global,
            triggers: Vec::new(),
            folders: Vec::new(),
            voice: None,
            voice_rate: 0,
            volume: 100,
        }
    }
}

/// Reusable text-overlay preset (EQLP text overlay).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextOverlayPreset {
    pub id: OverlayId,
    pub name: String,
    pub is_default: bool,
    pub font_size: String,
    pub font_color: String,
    pub background_color: String,
    /// Seconds a text entry stays visible.
    pub fade_delay_seconds: u32,
    pub passthrough: Passthrough,
}

impl Default for TextOverlayPreset {
    fn default() -> Self {
        Self {
            id: OverlayId::new(),
            name: String::new(),
            is_default: false,
            font_size: "12pt".to_owned(),
            font_color: "#FFFFFFFF".to_owned(),
            background_color: "#5F000000".to_owned(),
            fade_delay_seconds: 10,
            passthrough: Passthrough::new(),
        }
    }
}

/// Timer overlay display mode (EQLP `TimerMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimerOverlayMode {
    #[default]
    Standard,
    Cooldown,
}

/// Reusable timer-overlay preset (EQLP timer overlay).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TimerOverlayPreset {
    pub id: OverlayId,
    pub name: String,
    pub is_default: bool,
    pub mode: TimerOverlayMode,
    /// 0 = trigger order, 1 = remaining time (EQLP `SortBy`).
    pub sort_by: i32,
    pub font_color: String,
    pub active_color: String,
    pub idle_color: String,
    pub reset_color: String,
    pub background_color: String,
    pub show_millis: bool,
    pub passthrough: Passthrough,
}

impl Default for TimerOverlayPreset {
    fn default() -> Self {
        Self {
            id: OverlayId::new(),
            name: String::new(),
            is_default: false,
            mode: TimerOverlayMode::Standard,
            sort_by: 0,
            font_color: "#FFFFFFFF".to_owned(),
            active_color: "#FF1D397E".to_owned(),
            idle_color: "#FF8F1515".to_owned(),
            reset_color: "#FF8F1515".to_owned(),
            background_color: "#5F000000".to_owned(),
            show_millis: false,
            passthrough: Passthrough::new(),
        }
    }
}

/// Managed media asset (WAV/MP3) stored under the library's `assets/`
/// directory, content-addressed to survive renames.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetRecord {
    /// Logical name triggers refer to (e.g. `raid-warning.mp3`).
    pub name: String,
    /// File name on disk under `assets/` (`<sha8>-<name>`).
    pub file_name: String,
    pub sha256: String,
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_round_trips_through_json() {
        let mut library = TriggerLibrary::new();
        let folder = Folder {
            name: "Raids".to_owned(),
            ..Folder::default()
        };
        let mut trigger = Trigger {
            name: "Complete Heal".to_owned(),
            folder: Some(folder.id),
            pattern: Pattern::regex(r"^{S1} begins to cast a spell\."),
            timer: Some(TimerBehavior::default()),
            ..Trigger::default()
        };
        trigger
            .passthrough
            .insert("ChatWebhook".to_owned(), Value::String("kept".to_owned()));
        library.folders.push(folder);
        library.triggers.push(trigger);

        let json = serde_json::to_string_pretty(&library).unwrap();
        let restored: TriggerLibrary = serde_json::from_str(&json).unwrap();
        assert_eq!(library, restored);
        assert_eq!(
            restored.triggers[0].passthrough.get("ChatWebhook"),
            Some(&Value::String("kept".to_owned()))
        );
    }

    #[test]
    fn folder_subtree_walks_nested_folders() {
        let mut library = TriggerLibrary::new();
        let root = Folder::default();
        let child = Folder {
            parent: Some(root.id),
            ..Folder::default()
        };
        let grandchild = Folder {
            parent: Some(child.id),
            ..Folder::default()
        };
        let unrelated = Folder::default();
        let ids = (root.id, child.id, grandchild.id);
        library.folders.extend([root, child, grandchild, unrelated]);

        let subtree = library.folder_subtree(ids.0);
        assert_eq!(subtree, vec![ids.0, ids.1, ids.2]);
    }

    #[test]
    fn unknown_native_fields_do_not_break_loading() {
        // Forward compatibility: newer Stonemite versions may add fields.
        let json = r#"{
            "schemaVersion": 1,
            "futureField": true,
            "triggers": [{ "name": "x", "someNewThing": 3 }]
        }"#;
        let library: TriggerLibrary = serde_json::from_str(json).unwrap();
        assert_eq!(library.triggers.len(), 1);
        assert_eq!(library.triggers[0].name, "x");
    }
}
