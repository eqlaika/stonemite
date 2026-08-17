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
pub struct ClientState {
    pub id: ClientId,
    pub character: Option<String>,
    pub server: Option<String>,
    pub class_code: Option<String>,
    pub window_number: usize,
    pub active: bool,
    pub activatable: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub activate: bool,
    pub set_broadcast: bool,
    pub send_text: bool,
    pub send_keys: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateData {
    pub clients: Vec<ClientState>,
    pub broadcast: BroadcastState,
    pub capabilities: Capabilities,
}

impl Default for StateData {
    fn default() -> Self {
        Self {
            clients: Vec::new(),
            broadcast: BroadcastState::UNAVAILABLE,
            capabilities: Capabilities {
                activate: true,
                set_broadcast: false,
                send_text: false,
                send_keys: false,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateSnapshot {
    pub revision: u64,
    pub clients: Vec<ClientState>,
    pub broadcast: BroadcastState,
    pub capabilities: Capabilities,
}

impl StateSnapshot {
    pub fn data(&self) -> StateData {
        StateData {
            clients: self.clients.clone(),
            broadcast: self.broadcast,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputKind {
    Text,
    Keys,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    Activated {
        status: ActivationStatus,
        /// Whether the OS reported the target as foreground immediately.
        foreground_confirmed: bool,
    },
    BroadcastSet {
        enabled: bool,
    },
    InputDelivered {
        kind: InputKind,
        strokes: usize,
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
    BroadcastOperationFailed,
    InputUnavailable,
    InputOperationFailed,
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
            Self::BroadcastOperationFailed => "broadcast_operation_failed",
            Self::InputUnavailable => "input_unavailable",
            Self::InputOperationFailed => "input_operation_failed",
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
    fn set_broadcast_enabled(
        &self,
        enabled: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>>;
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
            });
        }
        clients.sort_by_key(|client| client.window_number);
        StateData {
            clients,
            broadcast,
            capabilities: Capabilities {
                activate: true,
                set_broadcast: broadcast.available,
                send_text: broadcast.available,
                send_keys: broadcast.available,
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
    next_id: u64,
    activation_failure: Option<String>,
    broadcast_failure: Option<String>,
    input_failure: Option<String>,
    disappear_on_activate: HashSet<ClientId>,
    disappear_on_input: HashSet<ClientId>,
    retired_ids: HashSet<ClientId>,
    inputs: Vec<RecordedInput>,
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
}

impl Default for BroadcastState {
    fn default() -> Self {
        Self::UNAVAILABLE
    }
}

impl InMemoryController {
    pub fn new(broadcast: BroadcastState) -> Self {
        let data = StateData {
            broadcast,
            capabilities: Capabilities {
                activate: true,
                set_broadcast: broadcast.available,
                send_text: broadcast.available,
                send_keys: broadcast.available,
            },
            ..StateData::default()
        };
        Self {
            inner: Arc::new(InMemoryInner {
                state: Mutex::new(MemoryState {
                    broadcast,
                    ..MemoryState::default()
                }),
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
            state.clients.push(ClientState {
                id: id.clone(),
                character: character.map(str::to_owned),
                server: server.map(str::to_owned),
                class_code: class_code.map(str::to_owned),
                window_number,
                active,
                activatable,
            });
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
    StateData {
        clients,
        broadcast: state.broadcast,
        capabilities: Capabilities {
            activate: true,
            set_broadcast: state.broadcast.available,
            send_text: state.broadcast.available,
            send_keys: state.broadcast.available,
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
    if state.disappear_on_input.remove(client_id) {
        let id = state.clients.remove(index).id;
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
