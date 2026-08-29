use futures_util::future::BoxFuture;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ClientId(String);

impl ClientId {
    pub fn new(value: impl Into<String>) -> Result<Self, ControlError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "client id must contain 1 to 128 bytes",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XTargetSlot {
    pub slot: u8,
    pub label: String,
    pub bound: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsiderDifficulty {
    Green,
    LightBlue,
    Blue,
    White,
    Yellow,
    Red,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsiderResult {
    Target {
        target: String,
        difficulty: ConsiderDifficulty,
        level: Option<u16>,
    },
    NoTarget,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XTargetState {
    pub slots: Vec<XTargetSlot>,
    pub selected_slot: Option<u8>,
    pub consider_bound: bool,
    pub consider_pending: bool,
    pub consider: Option<ConsiderResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientState {
    pub id: ClientId,
    pub character: Option<String>,
    pub server: Option<String>,
    pub class_code: Option<String>,
    pub window_number: usize,
    pub active: bool,
    pub activatable: bool,
    pub input_ready: bool,
    pub xtarget: XTargetState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BroadcastState {
    pub available: bool,
    pub enabled: bool,
}

impl BroadcastState {
    pub const UNAVAILABLE: Self = Self {
        available: false,
        enabled: false,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MouseClutchPhase {
    #[default]
    Inactive,
    Active,
    Releasing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MouseClutchAvailability {
    Ready,
    NoActiveClient,
    NoCompatibleTargets,
    #[default]
    InputUnavailable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseClutchState {
    pub phase: MouseClutchPhase,
    pub availability: MouseClutchAvailability,
}

impl MouseClutchState {
    pub const UNAVAILABLE: Self = Self {
        phase: MouseClutchPhase::Inactive,
        availability: MouseClutchAvailability::InputUnavailable,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EqActionCapabilities {
    pub use_center_screen: bool,
    pub invite_follow: bool,
    pub hotbars: u8,
    pub hotbar_buttons: u8,
    pub spell_gems: u8,
    pub keymap_actions: bool,
}

impl EqActionCapabilities {
    pub const fn available(available: bool) -> Self {
        Self {
            use_center_screen: available,
            invite_follow: available,
            hotbars: if available { MAX_HOTBARS } else { 0 },
            hotbar_buttons: if available { MAX_HOTBAR_BUTTONS } else { 0 },
            spell_gems: if available { MAX_SPELL_GEMS } else { 0 },
            // Discovery is a protocol/server feature and remains usable while
            // no trusik input channel is currently ready.
            keymap_actions: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub activate: bool,
    pub swap_window_numbers: bool,
    pub set_broadcast: bool,
    pub set_mouse_clutch: bool,
    pub send_text: bool,
    pub send_keys: bool,
    pub eq_actions: EqActionCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateData {
    pub clients: Vec<ClientState>,
    pub broadcast: BroadcastState,
    pub mouse_clutch: MouseClutchState,
    pub capabilities: Capabilities,
}

impl Default for StateData {
    fn default() -> Self {
        Self {
            clients: Vec::new(),
            broadcast: BroadcastState::UNAVAILABLE,
            mouse_clutch: MouseClutchState::UNAVAILABLE,
            capabilities: Capabilities {
                activate: true,
                swap_window_numbers: true,
                set_broadcast: false,
                set_mouse_clutch: false,
                send_text: false,
                send_keys: false,
                eq_actions: EqActionCapabilities::available(false),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateSnapshot {
    pub revision: u64,
    pub clients: Vec<ClientState>,
    pub broadcast: BroadcastState,
    pub mouse_clutch: MouseClutchState,
    pub capabilities: Capabilities,
}

impl StateSnapshot {
    pub fn data(&self) -> StateData {
        StateData {
            clients: self.clients.clone(),
            broadcast: self.broadcast,
            mouse_clutch: self.mouse_clutch,
            capabilities: self.capabilities,
        }
    }
}

/// Latest-value publisher. Equal states are deduplicated and do not consume a revision.
pub struct StateHub {
    current: Mutex<StateSnapshot>,
    sender: watch::Sender<StateSnapshot>,
}

impl StateHub {
    pub fn new(initial: StateData) -> Self {
        let snapshot = StateSnapshot {
            revision: 0,
            clients: initial.clients,
            broadcast: initial.broadcast,
            mouse_clutch: initial.mouse_clutch,
            capabilities: initial.capabilities,
        };
        let (sender, _) = watch::channel(snapshot.clone());
        Self {
            current: Mutex::new(snapshot),
            sender,
        }
    }

    pub fn snapshot(&self) -> StateSnapshot {
        self.current.lock().expect("state hub poisoned").clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<StateSnapshot> {
        self.sender.subscribe()
    }

    pub fn publish(&self, data: StateData) -> StateSnapshot {
        let mut current = self.current.lock().expect("state hub poisoned");
        if current.data() == data {
            return current.clone();
        }
        current.revision = current.revision.saturating_add(1);
        current.clients = data.clients;
        current.broadcast = data.broadcast;
        current.mouse_clutch = data.mouse_clutch;
        current.capabilities = data.capabilities;
        let snapshot = current.clone();
        self.sender.send_replace(snapshot.clone());
        snapshot
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientTarget {
    Id(ClientId),
    WindowNumber(usize),
    Identity {
        character: String,
        server: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivationStatus {
    Activated,
    AlreadyActive,
}

pub const MAX_TEXT_CHARS: usize = 256;
pub const MAX_TEXT_BYTES: usize = 1024;
pub const MAX_KEY_STROKES: usize = 64;
pub const MAX_HOTBARS: u8 = 11;
pub const MAX_HOTBAR_BUTTONS: u8 = 12;
pub const MAX_SPELL_GEMS: u8 = 14;
pub const MAX_EQ_TARGET_WINDOWS: usize = 6;
pub const MAX_EQ_MAPPING_NAME_BYTES: usize = 128;
pub const EQ_KEYMAP_PAGE_SIZE: usize = 64;
pub const MAX_KEYS_PER_STROKE: usize = 8;
pub const MAX_INPUT_DURATION_MS: u64 = 15_000;
pub const DEFAULT_KEY_HOLD_MS: u16 = 75;
pub const DEFAULT_KEY_PAUSE_MS: u16 = 75;

/// A semantic keyboard key, independent of transport and Windows scan codes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyCode(String);

impl KeyCode {
    pub fn new(value: impl Into<String>) -> Result<Self, ControlError> {
        let value = value.into();
        if !is_supported_key_name(&value) {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "key is not a supported semantic key name",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_supported_key_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if matches!(bytes, [b'a'..=b'z'] | [b'0'..=b'9']) {
        return true;
    }
    if let Some(number) = value
        .strip_prefix('f')
        .and_then(|value| value.parse::<u8>().ok())
    {
        if (1..=12).contains(&number) {
            return true;
        }
    }
    if let Some(number) = value
        .strip_prefix("numpad_")
        .and_then(|value| value.parse::<u8>().ok())
    {
        if number <= 9 {
            return true;
        }
    }
    matches!(
        value,
        "escape"
            | "minus"
            | "equals"
            | "backspace"
            | "tab"
            | "left_bracket"
            | "right_bracket"
            | "enter"
            | "left_control"
            | "semicolon"
            | "apostrophe"
            | "grave"
            | "left_shift"
            | "backslash"
            | "comma"
            | "period"
            | "slash"
            | "right_shift"
            | "numpad_multiply"
            | "left_alt"
            | "space"
            | "caps_lock"
            | "num_lock"
            | "scroll_lock"
            | "numpad_subtract"
            | "numpad_add"
            | "numpad_decimal"
            | "numpad_divide"
            | "numpad_enter"
            | "right_control"
            | "right_alt"
            | "home"
            | "arrow_up"
            | "page_up"
            | "arrow_left"
            | "arrow_right"
            | "end"
            | "arrow_down"
            | "page_down"
            | "insert"
            | "delete"
            | "pause"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyStroke {
    pub keys: Vec<KeyCode>,
    pub hold_ms: u16,
    pub pause_ms: u16,
}

impl KeyStroke {
    pub fn new(keys: Vec<KeyCode>, hold_ms: u16, pause_ms: u16) -> Result<Self, ControlError> {
        if keys.is_empty() || keys.len() > MAX_KEYS_PER_STROKE {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "each key stroke must contain 1 to 8 keys",
            ));
        }
        if hold_ms == 0 || hold_ms > 1_000 || pause_ms > 1_000 {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "hold_ms must be 1 to 1000 and pause_ms must be at most 1000",
            ));
        }
        let mut unique = HashSet::with_capacity(keys.len());
        if !keys.iter().all(|key| unique.insert(key.clone())) {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "a key stroke cannot contain duplicate keys",
            ));
        }
        Ok(Self {
            keys,
            hold_ms,
            pause_ms,
        })
    }
}

pub fn validate_text_input(text: &str) -> Result<(), ControlError> {
    if text.is_empty()
        || text.len() > MAX_TEXT_BYTES
        || text.chars().count() > MAX_TEXT_CHARS
        || text.chars().any(char::is_control)
    {
        return Err(ControlError::new(
            ErrorCode::InvalidArgument,
            "text must contain 1 to 256 printable characters and at most 1024 bytes",
        ));
    }
    Ok(())
}

pub fn validate_key_strokes(strokes: &[KeyStroke]) -> Result<(), ControlError> {
    if strokes.is_empty() || strokes.len() > MAX_KEY_STROKES {
        return Err(ControlError::new(
            ErrorCode::InvalidArgument,
            "send_keys must contain 1 to 64 key strokes",
        ));
    }
    let duration: u64 = strokes
        .iter()
        .map(|stroke| u64::from(stroke.hold_ms) + u64::from(stroke.pause_ms))
        .sum();
    if duration > MAX_INPUT_DURATION_MS {
        return Err(ControlError::new(
            ErrorCode::InvalidArgument,
            "the key sequence duration must not exceed 15 seconds",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EqMappingName(String);

impl EqMappingName {
    pub fn new(value: impl Into<String>) -> Result<Self, ControlError> {
        let value = value.into().to_ascii_uppercase();
        if value.is_empty()
            || value.len() > MAX_EQ_MAPPING_NAME_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "EQ mapping names must contain 1 to 128 ASCII letters, numbers, or underscores",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqActionTargets {
    AllLoaded,
    Active,
    BackgroundLoaded,
    WindowNumbers(Vec<usize>),
}

impl EqActionTargets {
    pub fn window_numbers(window_numbers: Vec<usize>) -> Result<Self, ControlError> {
        let targets = Self::WindowNumbers(window_numbers);
        targets.validate()?;
        Ok(targets)
    }

    pub fn validate(&self) -> Result<(), ControlError> {
        if let Self::WindowNumbers(window_numbers) = self {
            let unique: HashSet<usize> = window_numbers.iter().copied().collect();
            if window_numbers.is_empty()
                || window_numbers.len() > MAX_EQ_TARGET_WINDOWS
                || unique.len() != window_numbers.len()
                || window_numbers
                    .iter()
                    .any(|number| !(1..=MAX_EQ_TARGET_WINDOWS).contains(number))
            {
                return Err(ControlError::new(
                    ErrorCode::InvalidArgument,
                    "EQ action targets require 1 to 6 unique Stonemite window numbers from 1 to 6",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EqAction {
    UseCenterScreen,
    InviteFollow,
    Hotbar { bar: u8, button: u8 },
    SpellGem { gem: u8 },
    Keymap { mapping: EqMappingName },
}

impl EqAction {
    pub fn hotbar(bar: u8, button: u8) -> Result<Self, ControlError> {
        let action = Self::Hotbar { bar, button };
        action.validate()?;
        Ok(action)
    }

    pub fn spell_gem(gem: u8) -> Result<Self, ControlError> {
        let action = Self::SpellGem { gem };
        action.validate()?;
        Ok(action)
    }

    pub fn keymap(mapping: impl Into<String>) -> Result<Self, ControlError> {
        Ok(Self::Keymap {
            mapping: EqMappingName::new(mapping)?,
        })
    }

    pub fn mapping_name(&self) -> EqMappingName {
        let value = match self {
            Self::UseCenterScreen => "USE".to_owned(),
            Self::InviteFollow => "INVITE_FOLLOW".to_owned(),
            Self::Hotbar { bar, button } => format!("HOT{bar}_{button}"),
            Self::SpellGem { gem } => format!("CAST{gem}"),
            Self::Keymap { mapping } => return mapping.clone(),
        };
        EqMappingName(value)
    }

    pub fn validate(&self) -> Result<(), ControlError> {
        match self {
            Self::Hotbar { bar, button }
                if !(1..=MAX_HOTBARS).contains(bar)
                    || !(1..=MAX_HOTBAR_BUTTONS).contains(button) =>
            {
                Err(ControlError::new(
                    ErrorCode::InvalidArgument,
                    format!(
                        "hotbar actions require bar 1 to {MAX_HOTBARS} and button 1 to {MAX_HOTBAR_BUTTONS}"
                    ),
                ))
            }
            Self::SpellGem { gem } if !(1..=MAX_SPELL_GEMS).contains(gem) => {
                Err(ControlError::new(
                    ErrorCode::InvalidArgument,
                    format!("spell gem actions require gem 1 to {MAX_SPELL_GEMS}"),
                ))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Text,
    Keys,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MouseClutchOwner {
    session_id: u64,
    hold_id: String,
}

impl MouseClutchOwner {
    pub fn new(session_id: u64, hold_id: impl Into<String>) -> Result<Self, ControlError> {
        let hold_id = hold_id.into();
        if hold_id.is_empty() || hold_id.len() > 128 {
            return Err(ControlError::new(
                ErrorCode::InvalidArgument,
                "hold_id must contain 1 to 128 bytes",
            ));
        }
        Ok(Self {
            session_id,
            hold_id,
        })
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    pub fn hold_id(&self) -> &str {
        &self.hold_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseClutchOperation {
    Begin,
    Renew,
    End,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    Activated {
        status: ActivationStatus,
        /// Whether the OS reported the target as foreground immediately.
        foreground_confirmed: bool,
    },
    WindowNumbersSwapped {
        active_previous_number: usize,
        selected_previous_number: usize,
    },
    BroadcastSet {
        enabled: bool,
    },
    MouseClutchHoldUpdated {
        held: bool,
    },
    InputDelivered {
        kind: InputKind,
        strokes: usize,
    },
    EqActionDelivered {
        action: EqAction,
    },
    EqKeymapActionsListed {
        mappings: Vec<EqMappingName>,
        window_numbers: Vec<usize>,
        next_after: Option<EqMappingName>,
    },
    EqActionBatchDelivered {
        action: EqAction,
        window_numbers: Vec<usize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    MalformedRequest,
    UnsupportedProtocolVersion,
    Unauthorized,
    InvalidArgument,
    ClientNotFound,
    AmbiguousTarget,
    TargetDisappeared,
    BroadcastUnavailable,
    ActivationFailed,
    WindowNumberSwapFailed,
    BroadcastOperationFailed,
    MouseClutchUnavailable,
    MouseClutchNotReady,
    MouseClutchHoldExpired,
    MouseClutchOperationFailed,
    InputUnavailable,
    InputOperationFailed,
    EqActionUnbound,
    CommandTimeout,
    InternalError,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedRequest => "malformed_request",
            Self::UnsupportedProtocolVersion => "unsupported_protocol_version",
            Self::Unauthorized => "unauthorized",
            Self::InvalidArgument => "invalid_argument",
            Self::ClientNotFound => "client_not_found",
            Self::AmbiguousTarget => "ambiguous_target",
            Self::TargetDisappeared => "target_disappeared",
            Self::BroadcastUnavailable => "broadcast_unavailable",
            Self::ActivationFailed => "activation_failed",
            Self::WindowNumberSwapFailed => "window_number_swap_failed",
            Self::BroadcastOperationFailed => "broadcast_operation_failed",
            Self::MouseClutchUnavailable => "mouse_clutch_unavailable",
            Self::MouseClutchNotReady => "mouse_clutch_not_ready",
            Self::MouseClutchHoldExpired => "mouse_clutch_hold_expired",
            Self::MouseClutchOperationFailed => "mouse_clutch_operation_failed",
            Self::InputUnavailable => "input_unavailable",
            Self::InputOperationFailed => "input_operation_failed",
            Self::EqActionUnbound => "eq_action_unbound",
            Self::CommandTimeout => "command_timeout",
            Self::InternalError => "internal_error",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlError {
    pub code: ErrorCode,
    pub message: String,
}

impl ControlError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub trait Controller: Send + Sync + 'static {
    fn snapshot(&self) -> StateSnapshot;
    fn subscribe(&self) -> watch::Receiver<StateSnapshot>;
    fn activate(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn swap_window_numbers(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn set_broadcast_enabled(
        &self,
        enabled: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn update_mouse_clutch_hold(
        &self,
        owner: MouseClutchOwner,
        operation: MouseClutchOperation,
        sequence: u64,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn end_mouse_clutch_session(
        &self,
        session_id: u64,
        sequence: u64,
    ) -> BoxFuture<'static, Result<(), ControlError>>;
    fn send_text(
        &self,
        client_id: ClientId,
        text: String,
        submit: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn send_keys(
        &self,
        client_id: ClientId,
        strokes: Vec<KeyStroke>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn send_eq_action(
        &self,
        client_id: ClientId,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn list_eq_keymap_actions(
        &self,
        targets: EqActionTargets,
        after: Option<EqMappingName>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
    fn send_eq_action_batch(
        &self,
        targets: EqActionTargets,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
}

/// Owner-thread input used by adapters to map private window/process keys into public state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceClient {
    pub private_key: u64,
    pub character: Option<String>,
    pub server: Option<String>,
    pub class_code: Option<String>,
    pub window_number: usize,
    pub active: bool,
    pub activatable: bool,
    pub input_ready: bool,
}

/// Maintains opaque identifiers for exactly the lifetime of each loaded source client.
#[derive(Default)]
pub struct SnapshotMapper {
    ids: HashMap<u64, ClientId>,
    retired_ids: VecDeque<ClientId>,
    next_id: u64,
}

impl SnapshotMapper {
    pub fn id_for_key(&self, private_key: u64) -> Option<&ClientId> {
        self.ids.get(&private_key)
    }

    pub fn is_retired(&self, id: &ClientId) -> bool {
        self.retired_ids.contains(id)
    }

    pub fn map(&mut self, sources: &[SourceClient], broadcast: BroadcastState) -> StateData {
        let live: HashSet<u64> = sources.iter().map(|source| source.private_key).collect();
        let removed: Vec<u64> = self
            .ids
            .keys()
            .filter(|key| !live.contains(key))
            .copied()
            .collect();
        for key in removed {
            if let Some(id) = self.ids.remove(&key) {
                self.retired_ids.push_back(id);
                if self.retired_ids.len() > 256 {
                    self.retired_ids.pop_front();
                }
            }
        }
        let mut clients = Vec::with_capacity(sources.len());
        for source in sources {
            let id = self.ids.entry(source.private_key).or_insert_with(|| {
                self.next_id = self.next_id.saturating_add(1);
                ClientId(format!("client-{:016x}", self.next_id))
            });
            clients.push(ClientState {
                id: id.clone(),
                character: source.character.clone(),
                server: source.server.clone(),
                class_code: source.class_code.clone(),
                window_number: source.window_number,
                active: source.active,
                activatable: source.activatable,
                input_ready: source.input_ready,
                xtarget: XTargetState::default(),
            });
        }
        clients.sort_by_key(|client| client.window_number);
        let input_available =
            broadcast.available && clients.iter().any(|client| client.input_ready);
        StateData {
            clients,
            broadcast,
            mouse_clutch: MouseClutchState::UNAVAILABLE,
            capabilities: Capabilities {
                activate: true,
                swap_window_numbers: true,
                set_broadcast: broadcast.available,
                set_mouse_clutch: false,
                send_text: input_available,
                send_keys: input_available,
                eq_actions: EqActionCapabilities::available(input_available),
            },
        }
    }
}

#[derive(Clone)]
pub struct InMemoryController {
    inner: Arc<InMemoryInner>,
}

struct InMemoryInner {
    state: Mutex<MemoryState>,
    hub: Arc<StateHub>,
}

#[derive(Default)]
struct MemoryState {
    clients: Vec<ClientState>,
    broadcast: BroadcastState,
    mouse_clutch_holds: HashSet<MouseClutchOwner>,
    mouse_clutch_sequences: HashMap<MouseClutchOwner, u64>,
    closed_mouse_clutch_sessions: HashMap<u64, u64>,
    closed_mouse_clutch_session_order: VecDeque<u64>,
    next_id: u64,
    activation_failure: Option<String>,
    broadcast_failure: Option<String>,
    input_failure: Option<String>,
    disappear_on_activate: HashSet<ClientId>,
    disappear_on_input: HashSet<ClientId>,
    retired_ids: HashSet<ClientId>,
    inputs: Vec<RecordedInput>,
    mapped_actions: HashMap<ClientId, HashSet<EqMappingName>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordedInput {
    Text {
        client_id: ClientId,
        text: String,
        submit: bool,
    },
    Keys {
        client_id: ClientId,
        strokes: Vec<KeyStroke>,
    },
    EqAction {
        client_id: ClientId,
        action: EqAction,
    },
}

impl Default for BroadcastState {
    fn default() -> Self {
        Self::UNAVAILABLE
    }
}

impl InMemoryController {
    pub fn new(broadcast: BroadcastState) -> Self {
        let state = MemoryState {
            broadcast,
            ..MemoryState::default()
        };
        let data = memory_data(&state);
        Self {
            inner: Arc::new(InMemoryInner {
                state: Mutex::new(state),
                hub: Arc::new(StateHub::new(data)),
            }),
        }
    }

    pub fn add_client(
        &self,
        window_number: usize,
        character: Option<&str>,
        server: Option<&str>,
        class_code: Option<&str>,
        active: bool,
        activatable: bool,
    ) -> ClientId {
        let (id, data) = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            state.next_id = state.next_id.saturating_add(1);
            let id = ClientId(format!("client-{:016x}", state.next_id));
            if active {
                for client in &mut state.clients {
                    client.active = false;
                }
            }
            let input_ready = state.broadcast.available;
            state.clients.push(ClientState {
                id: id.clone(),
                character: character.map(str::to_owned),
                server: server.map(str::to_owned),
                class_code: class_code.map(str::to_owned),
                window_number,
                active,
                activatable,
                input_ready,
                xtarget: XTargetState::default(),
            });
            state
                .mapped_actions
                .insert(id.clone(), memory_default_mappings());
            (id, memory_data(&state))
        };
        self.inner.hub.publish(data);
        id
    }

    pub fn remove_client(&self, id: &ClientId) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            if state.clients.iter().any(|client| &client.id == id) {
                state.retired_ids.insert(id.clone());
            }
            state.clients.retain(|client| &client.id != id);
            state.mapped_actions.remove(id);
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn enrich_client(
        &self,
        id: &ClientId,
        character: Option<&str>,
        server: Option<&str>,
        class_code: Option<&str>,
    ) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            if let Some(client) = state.clients.iter_mut().find(|client| &client.id == id) {
                client.character = character.map(str::to_owned);
                client.server = server.map(str::to_owned);
                client.class_code = class_code.map(str::to_owned);
            }
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn set_active_locally(&self, id: &ClientId) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            for client in &mut state.clients {
                client.active = &client.id == id;
            }
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn set_input_ready(&self, id: &ClientId, input_ready: bool) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            if let Some(client) = state.clients.iter_mut().find(|client| &client.id == id) {
                client.input_ready = input_ready;
            }
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn set_broadcast_availability(&self, available: bool) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            state.broadcast.available = available;
            if !available {
                state.broadcast.enabled = false;
            }
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn set_broadcast_locally(&self, enabled: bool) {
        let data = {
            let mut state = self.inner.state.lock().expect("memory controller poisoned");
            if state.broadcast.available {
                state.broadcast.enabled = enabled;
            }
            memory_data(&state)
        };
        self.inner.hub.publish(data);
    }

    pub fn fail_next_activation(&self, message: impl Into<String>) {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .activation_failure = Some(message.into());
    }

    pub fn fail_next_broadcast(&self, message: impl Into<String>) {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .broadcast_failure = Some(message.into());
    }

    pub fn fail_next_input(&self, message: impl Into<String>) {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .input_failure = Some(message.into());
    }

    pub fn recorded_inputs(&self) -> Vec<RecordedInput> {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .inputs
            .clone()
    }

    pub fn set_mapped_actions(
        &self,
        id: &ClientId,
        mappings: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<(), ControlError> {
        let mappings = mappings
            .into_iter()
            .map(EqMappingName::new)
            .collect::<Result<HashSet<_>, _>>()?;
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .mapped_actions
            .insert(id.clone(), mappings);
        Ok(())
    }

    pub fn disappear_on_next_activation(&self, id: ClientId) {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .disappear_on_activate
            .insert(id);
    }

    pub fn disappear_on_next_input(&self, id: ClientId) {
        self.inner
            .state
            .lock()
            .expect("memory controller poisoned")
            .disappear_on_input
            .insert(id);
    }
}

fn memory_data(state: &MemoryState) -> StateData {
    let mut clients = state.clients.clone();
    clients.sort_by_key(|client| client.window_number);
    let input_available =
        state.broadcast.available && clients.iter().any(|client| client.input_ready);
    let active_id = clients
        .iter()
        .find(|client| client.active)
        .map(|client| &client.id);
    let mouse_clutch_availability = if !state.broadcast.available {
        MouseClutchAvailability::InputUnavailable
    } else if active_id.is_none() {
        MouseClutchAvailability::NoActiveClient
    } else if clients
        .iter()
        .any(|client| Some(&client.id) != active_id && client.input_ready)
    {
        MouseClutchAvailability::Ready
    } else {
        MouseClutchAvailability::NoCompatibleTargets
    };
    StateData {
        clients,
        broadcast: state.broadcast,
        mouse_clutch: MouseClutchState {
            phase: if state.mouse_clutch_holds.is_empty() {
                MouseClutchPhase::Inactive
            } else {
                MouseClutchPhase::Active
            },
            availability: mouse_clutch_availability,
        },
        capabilities: Capabilities {
            activate: true,
            swap_window_numbers: true,
            set_broadcast: state.broadcast.available,
            set_mouse_clutch: state.broadcast.available,
            send_text: input_available,
            send_keys: input_available,
            eq_actions: EqActionCapabilities::available(input_available),
        },
    }
}

impl Controller for InMemoryController {
    fn snapshot(&self) -> StateSnapshot {
        self.inner.hub.snapshot()
    }

    fn subscribe(&self) -> watch::Receiver<StateSnapshot> {
        self.inner.hub.subscribe()
    }

    fn activate(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let (result, data) = {
                let mut state = inner.state.lock().expect("memory controller poisoned");
                let matches: Vec<usize> = state
                    .clients
                    .iter()
                    .enumerate()
                    .filter_map(|(index, client)| target_matches(client, &target).then_some(index))
                    .collect();
                if matches.is_empty() {
                    if matches!(&target, ClientTarget::Id(id) if state.retired_ids.contains(id)) {
                        return Err(ControlError::new(
                            ErrorCode::TargetDisappeared,
                            "the target is no longer loaded",
                        ));
                    }
                    return Err(ControlError::new(
                        ErrorCode::ClientNotFound,
                        "no loaded client matches the target",
                    ));
                }
                if matches.len() > 1 {
                    return Err(ControlError::new(
                        ErrorCode::AmbiguousTarget,
                        "more than one loaded client matches the target",
                    ));
                }
                let index = matches[0];
                let id = state.clients[index].id.clone();
                if state.disappear_on_activate.remove(&id) {
                    state.clients.remove(index);
                    state.retired_ids.insert(id);
                    let data = memory_data(&state);
                    drop(state);
                    inner.hub.publish(data);
                    return Err(ControlError::new(
                        ErrorCode::TargetDisappeared,
                        "the target disappeared before activation",
                    ));
                }
                if !state.clients[index].activatable {
                    return Err(ControlError::new(
                        ErrorCode::ActivationFailed,
                        "the loaded client is outside the supported activation set",
                    ));
                }
                if let Some(message) = state.activation_failure.take() {
                    return Err(ControlError::new(ErrorCode::ActivationFailed, message));
                }
                let already_active = state.clients[index].active;
                if !already_active {
                    for client in &mut state.clients {
                        client.active = client.id == id;
                    }
                }
                (
                    CommandOutcome::Activated {
                        status: if already_active {
                            ActivationStatus::AlreadyActive
                        } else {
                            ActivationStatus::Activated
                        },
                        foreground_confirmed: true,
                    },
                    memory_data(&state),
                )
            };
            inner.hub.publish(data);
            Ok(result)
        })
    }

    fn swap_window_numbers(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let (result, data) = {
                let mut state = inner.state.lock().expect("memory controller poisoned");
                let matches: Vec<usize> = state
                    .clients
                    .iter()
                    .enumerate()
                    .filter_map(|(index, client)| target_matches(client, &target).then_some(index))
                    .collect();
                if matches.is_empty() {
                    if matches!(&target, ClientTarget::Id(id) if state.retired_ids.contains(id)) {
                        return Err(ControlError::new(
                            ErrorCode::TargetDisappeared,
                            "the target is no longer loaded",
                        ));
                    }
                    return Err(ControlError::new(
                        ErrorCode::ClientNotFound,
                        "no loaded client matches the target",
                    ));
                }
                if matches.len() > 1 {
                    return Err(ControlError::new(
                        ErrorCode::AmbiguousTarget,
                        "more than one loaded client matches the target",
                    ));
                }
                let selected_index = matches[0];
                let Some(active_index) = state.clients.iter().position(|client| client.active)
                else {
                    return Err(ControlError::new(
                        ErrorCode::WindowNumberSwapFailed,
                        "there is no active client whose window number can be swapped",
                    ));
                };
                let active_previous_number = state.clients[active_index].window_number;
                let selected_previous_number = state.clients[selected_index].window_number;
                if active_index != selected_index {
                    state.clients[active_index].window_number = selected_previous_number;
                    state.clients[selected_index].window_number = active_previous_number;
                }
                (
                    CommandOutcome::WindowNumbersSwapped {
                        active_previous_number,
                        selected_previous_number,
                    },
                    memory_data(&state),
                )
            };
            inner.hub.publish(data);
            Ok(result)
        })
    }

    fn set_broadcast_enabled(
        &self,
        enabled: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let data = {
                let mut state = inner.state.lock().expect("memory controller poisoned");
                if !state.broadcast.available {
                    return Err(ControlError::new(
                        ErrorCode::BroadcastUnavailable,
                        "broadcasting is unavailable because trusik is disabled",
                    ));
                }
                if let Some(message) = state.broadcast_failure.take() {
                    return Err(ControlError::new(
                        ErrorCode::BroadcastOperationFailed,
                        message,
                    ));
                }
                state.broadcast.enabled = enabled;
                memory_data(&state)
            };
            inner.hub.publish(data);
            Ok(CommandOutcome::BroadcastSet { enabled })
        })
    }

    fn update_mouse_clutch_hold(
        &self,
        owner: MouseClutchOwner,
        operation: MouseClutchOperation,
        sequence: u64,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let (held, data) = {
                let mut state = inner.state.lock().expect("memory controller poisoned");
                let session_id = owner.session_id();
                if state.closed_mouse_clutch_sessions.contains_key(&session_id) {
                    return Err(ControlError::new(
                        ErrorCode::MouseClutchHoldExpired,
                        "the Mouse Clutch connection is no longer active",
                    ));
                }
                let previous_sequence = state
                    .mouse_clutch_sequences
                    .get(&owner)
                    .copied()
                    .unwrap_or(0);
                if sequence <= previous_sequence {
                    let held = state.mouse_clutch_holds.contains(&owner);
                    (held, memory_data(&state))
                } else {
                    state.mouse_clutch_sequences.insert(owner.clone(), sequence);
                    match operation {
                        MouseClutchOperation::Begin => {
                            if memory_data(&state).mouse_clutch.availability
                                != MouseClutchAvailability::Ready
                            {
                                return Err(ControlError::new(
                                    ErrorCode::MouseClutchNotReady,
                                    "Mouse Clutch needs a foreground client and a compatible ready background target",
                                ));
                            }
                            state.mouse_clutch_holds.insert(owner.clone());
                        }
                        MouseClutchOperation::Renew => {
                            if !state.mouse_clutch_holds.contains(&owner) {
                                return Err(ControlError::new(
                                    ErrorCode::MouseClutchHoldExpired,
                                    "the Mouse Clutch hold is no longer active",
                                ));
                            }
                        }
                        MouseClutchOperation::End => {
                            state.mouse_clutch_holds.remove(&owner);
                        }
                    }
                    let held = state.mouse_clutch_holds.contains(&owner);
                    (held, memory_data(&state))
                }
            };
            inner.hub.publish(data);
            Ok(CommandOutcome::MouseClutchHoldUpdated { held })
        })
    }

    fn end_mouse_clutch_session(
        &self,
        session_id: u64,
        sequence: u64,
    ) -> BoxFuture<'static, Result<(), ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            let data = {
                let mut state = inner.state.lock().expect("memory controller poisoned");
                if !state.closed_mouse_clutch_sessions.contains_key(&session_id) {
                    state
                        .closed_mouse_clutch_session_order
                        .push_back(session_id);
                }
                state
                    .closed_mouse_clutch_sessions
                    .insert(session_id, sequence);
                state
                    .mouse_clutch_holds
                    .retain(|owner| owner.session_id() != session_id);
                state
                    .mouse_clutch_sequences
                    .retain(|owner, _| owner.session_id() != session_id);
                while state.closed_mouse_clutch_session_order.len() > 256 {
                    if let Some(expired) = state.closed_mouse_clutch_session_order.pop_front() {
                        state.closed_mouse_clutch_sessions.remove(&expired);
                    }
                }
                memory_data(&state)
            };
            inner.hub.publish(data);
            Ok(())
        })
    }

    fn send_text(
        &self,
        client_id: ClientId,
        text: String,
        submit: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            validate_text_input(&text)?;
            let strokes = text.chars().count() + usize::from(submit);
            let mut state = inner.state.lock().expect("memory controller poisoned");
            prepare_memory_input(&inner, &mut state, &client_id)?;
            state.inputs.push(RecordedInput::Text {
                client_id,
                text,
                submit,
            });
            Ok(CommandOutcome::InputDelivered {
                kind: InputKind::Text,
                strokes,
            })
        })
    }

    fn send_keys(
        &self,
        client_id: ClientId,
        strokes: Vec<KeyStroke>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            validate_key_strokes(&strokes)?;
            let count = strokes.len();
            let mut state = inner.state.lock().expect("memory controller poisoned");
            prepare_memory_input(&inner, &mut state, &client_id)?;
            state
                .inputs
                .push(RecordedInput::Keys { client_id, strokes });
            Ok(CommandOutcome::InputDelivered {
                kind: InputKind::Keys,
                strokes: count,
            })
        })
    }

    fn send_eq_action(
        &self,
        client_id: ClientId,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            action.validate()?;
            let mut state = inner.state.lock().expect("memory controller poisoned");
            prepare_memory_input(&inner, &mut state, &client_id)?;
            state.inputs.push(RecordedInput::EqAction {
                client_id,
                action: action.clone(),
            });
            Ok(CommandOutcome::EqActionDelivered { action })
        })
    }

    fn list_eq_keymap_actions(
        &self,
        targets: EqActionTargets,
        after: Option<EqMappingName>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            targets.validate()?;
            let state = inner.state.lock().expect("memory controller poisoned");
            let clients = memory_clients_for_listing(&state, &targets)?;
            let mut window_numbers = clients
                .iter()
                .map(|client| client.window_number)
                .collect::<Vec<_>>();
            window_numbers.sort_unstable();
            let mut shared = clients
                .first()
                .and_then(|client| state.mapped_actions.get(&client.id).cloned())
                .unwrap_or_default();
            for client in clients.iter().skip(1) {
                let mappings = state
                    .mapped_actions
                    .get(&client.id)
                    .cloned()
                    .unwrap_or_default();
                shared.retain(|mapping| mappings.contains(mapping));
            }
            if clients.is_empty() {
                shared.clear();
            }
            let mut mappings = shared.into_iter().collect::<Vec<_>>();
            mappings.sort();
            if let Some(after) = &after {
                mappings.retain(|mapping| mapping > after);
            }
            let next_after = (mappings.len() > EQ_KEYMAP_PAGE_SIZE)
                .then(|| mappings[EQ_KEYMAP_PAGE_SIZE - 1].clone());
            mappings.truncate(EQ_KEYMAP_PAGE_SIZE);
            Ok(CommandOutcome::EqKeymapActionsListed {
                mappings,
                window_numbers,
                next_after,
            })
        })
    }

    fn send_eq_action_batch(
        &self,
        targets: EqActionTargets,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        let inner = self.inner.clone();
        Box::pin(async move {
            targets.validate()?;
            action.validate()?;
            let mut state = inner.state.lock().expect("memory controller poisoned");
            let client_ids = memory_clients_for_delivery(&state, &targets)?
                .into_iter()
                .map(|client| client.id.clone())
                .collect::<Vec<_>>();
            for client_id in &client_ids {
                let client = state
                    .clients
                    .iter()
                    .find(|client| &client.id == client_id)
                    .expect("selected memory client disappeared while locked");
                if !client.input_ready {
                    return Err(ControlError::new(
                        ErrorCode::InputUnavailable,
                        format!(
                            "targeted input is unavailable because Stonemite box {} is not ready",
                            client.window_number
                        ),
                    ));
                }
                if !memory_action_is_mapped(&state, client_id, &action) {
                    return Err(ControlError::new(
                        ErrorCode::EqActionUnbound,
                        format!(
                            "the selected EQ action is not mapped for Stonemite box {}",
                            client.window_number
                        ),
                    ));
                }
            }
            if !state.broadcast.available {
                return Err(ControlError::new(
                    ErrorCode::InputUnavailable,
                    "targeted input is unavailable because trusik is disabled",
                ));
            }
            if let Some(client_id) = client_ids
                .iter()
                .find(|client_id| state.disappear_on_input.contains(*client_id))
                .cloned()
            {
                state.disappear_on_input.remove(&client_id);
                if let Some(index) = state
                    .clients
                    .iter()
                    .position(|client| client.id == client_id)
                {
                    let id = state.clients.remove(index).id;
                    state.mapped_actions.remove(&id);
                    state.retired_ids.insert(id);
                    inner.hub.publish(memory_data(&state));
                }
                return Err(ControlError::new(
                    ErrorCode::TargetDisappeared,
                    "a target disappeared before batch input delivery",
                ));
            }
            if let Some(message) = state.input_failure.take() {
                return Err(ControlError::new(ErrorCode::InputOperationFailed, message));
            }
            let mut window_numbers = Vec::with_capacity(client_ids.len());
            for client_id in client_ids {
                let window_number = state
                    .clients
                    .iter()
                    .find(|client| client.id == client_id)
                    .map(|client| client.window_number)
                    .expect("selected memory client disappeared while locked");
                window_numbers.push(window_number);
                state.inputs.push(RecordedInput::EqAction {
                    client_id,
                    action: action.clone(),
                });
            }
            window_numbers.sort_unstable();
            Ok(CommandOutcome::EqActionBatchDelivered {
                action,
                window_numbers,
            })
        })
    }
}

fn memory_default_mappings() -> HashSet<EqMappingName> {
    let mut mappings = HashSet::new();
    for name in ["USE", "INVITE_FOLLOW"] {
        mappings.insert(EqMappingName(name.to_owned()));
    }
    for button in 1..=MAX_HOTBAR_BUTTONS {
        mappings.insert(EqMappingName(format!("HOT1_{button}")));
    }
    for gem in 1..=MAX_SPELL_GEMS {
        mappings.insert(EqMappingName(format!("CAST{gem}")));
    }
    mappings
}

fn memory_action_is_mapped(state: &MemoryState, client_id: &ClientId, action: &EqAction) -> bool {
    state
        .mapped_actions
        .get(client_id)
        .is_some_and(|mappings| mappings.contains(&action.mapping_name()))
}

fn memory_clients_for_listing<'a>(
    state: &'a MemoryState,
    targets: &EqActionTargets,
) -> Result<Vec<&'a ClientState>, ControlError> {
    let active_id = matches!(
        targets,
        EqActionTargets::Active | EqActionTargets::BackgroundLoaded
    )
    .then(|| {
        state
            .clients
            .iter()
            .find(|client| client.active)
            .map(|client| &client.id)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::ClientNotFound,
                    "no active client is available for the dynamic EQ action target",
                )
            })
    })
    .transpose()?;
    Ok(state
        .clients
        .iter()
        .filter(|client| match targets {
            EqActionTargets::AllLoaded => true,
            EqActionTargets::Active => active_id == Some(&client.id),
            EqActionTargets::BackgroundLoaded => active_id != Some(&client.id),
            EqActionTargets::WindowNumbers(numbers) => numbers.contains(&client.window_number),
        })
        .collect())
}

fn memory_clients_for_delivery<'a>(
    state: &'a MemoryState,
    targets: &EqActionTargets,
) -> Result<Vec<&'a ClientState>, ControlError> {
    let clients = memory_clients_for_listing(state, targets)?;
    match targets {
        EqActionTargets::AllLoaded if clients.is_empty() => Err(ControlError::new(
            ErrorCode::ClientNotFound,
            "no loaded clients match the all-boxes target",
        )),
        EqActionTargets::BackgroundLoaded if clients.is_empty() => Err(ControlError::new(
            ErrorCode::ClientNotFound,
            "no loaded clients match the background-boxes target",
        )),
        EqActionTargets::WindowNumbers(numbers) if clients.len() != numbers.len() => {
            let missing = numbers
                .iter()
                .find(|number| {
                    !clients
                        .iter()
                        .any(|client| client.window_number == **number)
                })
                .copied()
                .unwrap_or_default();
            Err(ControlError::new(
                ErrorCode::ClientNotFound,
                format!("Stonemite box {missing} is not loaded"),
            ))
        }
        _ => Ok(clients),
    }
}

fn prepare_memory_input(
    inner: &InMemoryInner,
    state: &mut MemoryState,
    client_id: &ClientId,
) -> Result<(), ControlError> {
    if !state.broadcast.available {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable because trusik is disabled",
        ));
    }
    let Some(index) = state
        .clients
        .iter()
        .position(|client| &client.id == client_id)
    else {
        let code = if state.retired_ids.contains(client_id) {
            ErrorCode::TargetDisappeared
        } else {
            ErrorCode::ClientNotFound
        };
        return Err(ControlError::new(code, "the target client is not loaded"));
    };
    if !state.clients[index].input_ready {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable because the selected client's trusik proxy is not ready",
        ));
    }
    if state.disappear_on_input.remove(client_id) {
        let id = state.clients.remove(index).id;
        state.mapped_actions.remove(&id);
        state.retired_ids.insert(id);
        let data = memory_data(state);
        inner.hub.publish(data);
        return Err(ControlError::new(
            ErrorCode::TargetDisappeared,
            "the target disappeared before input delivery",
        ));
    }
    if let Some(message) = state.input_failure.take() {
        return Err(ControlError::new(ErrorCode::InputOperationFailed, message));
    }
    Ok(())
}

fn target_matches(client: &ClientState, target: &ClientTarget) -> bool {
    match target {
        ClientTarget::Id(id) => client.id == *id,
        ClientTarget::WindowNumber(number) => client.window_number == *number,
        ClientTarget::Identity { character, server } => {
            client
                .character
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(character))
                && server.as_ref().is_none_or(|expected| {
                    client
                        .server
                        .as_deref()
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
                })
        }
    }
}
