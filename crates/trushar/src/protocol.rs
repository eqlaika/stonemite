use crate::control::{
    validate_key_strokes, validate_text_input, ActivationStatus, ClientId, ClientTarget,
    CommandOutcome, ControlError, EqAction, EqActionTargets, EqMappingName, InputKind, KeyCode,
    KeyStroke, MouseClutchAvailability, MouseClutchPhase, MouseClutchState, StateSnapshot,
    DEFAULT_KEY_HOLD_MS, DEFAULT_KEY_PAUSE_MS,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_TEXT_MESSAGE_SIZE: usize = 16 * 1024;
pub const MAX_REQUEST_ID_SIZE: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    GetState {
        version: u16,
        request_id: String,
    },
    Activate {
        version: u16,
        request_id: String,
        target: Target,
    },
    SwapWindowNumbers {
        version: u16,
        request_id: String,
        target: Target,
    },
    SetBroadcast {
        version: u16,
        request_id: String,
        enabled: bool,
    },
    BeginMouseClutch {
        version: u16,
        request_id: String,
        hold_id: String,
    },
    RenewMouseClutch {
        version: u16,
        request_id: String,
        hold_id: String,
    },
    EndMouseClutch {
        version: u16,
        request_id: String,
        hold_id: String,
    },
    SendText {
        version: u16,
        request_id: String,
        client_id: String,
        text: String,
        #[serde(default)]
        submit: bool,
    },
    SendKeys {
        version: u16,
        request_id: String,
        client_id: String,
        strokes: Vec<WireKeyStroke>,
    },
    SendEqAction {
        version: u16,
        request_id: String,
        client_id: String,
        action: WireEqAction,
    },
    ListEqKeymapActions {
        version: u16,
        request_id: String,
        targets: WireEqActionTargets,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
    SendEqActionBatch {
        version: u16,
        request_id: String,
        targets: WireEqActionTargets,
        action: WireEqAction,
    },
}

impl ClientMessage {
    pub fn version(&self) -> u16 {
        match self {
            Self::GetState { version, .. }
            | Self::Activate { version, .. }
            | Self::SwapWindowNumbers { version, .. }
            | Self::SetBroadcast { version, .. }
            | Self::BeginMouseClutch { version, .. }
            | Self::RenewMouseClutch { version, .. }
            | Self::EndMouseClutch { version, .. }
            | Self::SendText { version, .. }
            | Self::SendKeys { version, .. }
            | Self::SendEqAction { version, .. }
            | Self::ListEqKeymapActions { version, .. }
            | Self::SendEqActionBatch { version, .. } => *version,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::GetState { request_id, .. }
            | Self::Activate { request_id, .. }
            | Self::SwapWindowNumbers { request_id, .. }
            | Self::SetBroadcast { request_id, .. }
            | Self::BeginMouseClutch { request_id, .. }
            | Self::RenewMouseClutch { request_id, .. }
            | Self::EndMouseClutch { request_id, .. }
            | Self::SendText { request_id, .. }
            | Self::SendKeys { request_id, .. }
            | Self::SendEqAction { request_id, .. }
            | Self::ListEqKeymapActions { request_id, .. }
            | Self::SendEqActionBatch { request_id, .. } => request_id,
        }
    }

    pub fn validate(&self) -> Result<(), ControlError> {
        if self.version() != PROTOCOL_VERSION {
            return Err(ControlError::new(
                crate::control::ErrorCode::UnsupportedProtocolVersion,
                format!(
                    "protocol version {} is unsupported; expected {}",
                    self.version(),
                    PROTOCOL_VERSION
                ),
            ));
        }
        let request_id = self.request_id();
        if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_SIZE {
            return Err(ControlError::new(
                crate::control::ErrorCode::InvalidArgument,
                "request_id must contain 1 to 128 bytes",
            ));
        }
        if let Self::Activate { target, .. } | Self::SwapWindowNumbers { target, .. } = self {
            target.validate()?;
        }
        match self {
            Self::BeginMouseClutch { hold_id, .. }
            | Self::RenewMouseClutch { hold_id, .. }
            | Self::EndMouseClutch { hold_id, .. } => {
                if hold_id.is_empty() || hold_id.len() > MAX_REQUEST_ID_SIZE {
                    return Err(ControlError::new(
                        crate::control::ErrorCode::InvalidArgument,
                        "hold_id must contain 1 to 128 bytes",
                    ));
                }
            }
            Self::SendText {
                client_id, text, ..
            } => {
                ClientId::new(client_id.clone())?;
                validate_text_input(text)?;
            }
            Self::SendKeys {
                client_id, strokes, ..
            } => {
                ClientId::new(client_id.clone())?;
                let strokes = strokes
                    .iter()
                    .cloned()
                    .map(KeyStroke::try_from)
                    .collect::<Result<Vec<_>, _>>()?;
                validate_key_strokes(&strokes)?;
            }
            Self::SendEqAction {
                client_id, action, ..
            } => {
                ClientId::new(client_id.clone())?;
                EqAction::try_from(action.clone())?;
            }
            Self::ListEqKeymapActions { targets, after, .. } => {
                EqActionTargets::try_from(targets.clone())?;
                if let Some(after) = after {
                    EqMappingName::new(after.clone())?;
                }
            }
            Self::SendEqActionBatch {
                targets, action, ..
            } => {
                EqActionTargets::try_from(targets.clone())?;
                EqAction::try_from(action.clone())?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PairingRequest {
    Pair { version: u16, code: String },
}

impl PairingRequest {
    fn validate(&self) -> Result<(), ControlError> {
        let Self::Pair { version, code } = self;
        if *version != PROTOCOL_VERSION {
            return Err(ControlError::new(
                crate::control::ErrorCode::UnsupportedProtocolVersion,
                format!("protocol version {version} is unsupported; expected {PROTOCOL_VERSION}"),
            ));
        }
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ControlError::new(
                crate::control::ErrorCode::InvalidArgument,
                "pairing code must contain exactly six digits",
            ));
        }
        Ok(())
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Pair { code, .. } => code,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireKeyStroke {
    pub keys: Vec<String>,
    #[serde(default = "default_key_hold_ms")]
    pub hold_ms: u16,
    #[serde(default = "default_key_pause_ms")]
    pub pause_ms: u16,
}

fn default_key_hold_ms() -> u16 {
    DEFAULT_KEY_HOLD_MS
}

fn default_key_pause_ms() -> u16 {
    DEFAULT_KEY_PAUSE_MS
}

impl TryFrom<WireKeyStroke> for KeyStroke {
    type Error = ControlError;

    fn try_from(value: WireKeyStroke) -> Result<Self, Self::Error> {
        let keys = value
            .keys
            .into_iter()
            .map(KeyCode::new)
            .collect::<Result<Vec<_>, _>>()?;
        KeyStroke::new(keys, value.hold_ms, value.pause_ms)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireEqAction {
    UseCenterScreen,
    InviteFollow,
    Hotbar { bar: u8, button: u8 },
    SpellGem { gem: u8 },
    Keymap { mapping: String },
}

impl TryFrom<WireEqAction> for EqAction {
    type Error = ControlError;

    fn try_from(value: WireEqAction) -> Result<Self, Self::Error> {
        match value {
            WireEqAction::UseCenterScreen => Ok(Self::UseCenterScreen),
            WireEqAction::InviteFollow => Ok(Self::InviteFollow),
            WireEqAction::Hotbar { bar, button } => Self::hotbar(bar, button),
            WireEqAction::SpellGem { gem } => Self::spell_gem(gem),
            WireEqAction::Keymap { mapping } => Self::keymap(mapping),
        }
    }
}

impl From<EqAction> for WireEqAction {
    fn from(value: EqAction) -> Self {
        match value {
            EqAction::UseCenterScreen => Self::UseCenterScreen,
            EqAction::InviteFollow => Self::InviteFollow,
            EqAction::Hotbar { bar, button } => Self::Hotbar { bar, button },
            EqAction::SpellGem { gem } => Self::SpellGem { gem },
            EqAction::Keymap { mapping } => Self::Keymap {
                mapping: mapping.as_str().to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireEqActionTargets {
    AllLoaded,
    Active,
    BackgroundLoaded,
    WindowNumbers { window_numbers: Vec<usize> },
}

impl TryFrom<WireEqActionTargets> for EqActionTargets {
    type Error = ControlError;

    fn try_from(value: WireEqActionTargets) -> Result<Self, Self::Error> {
        match value {
            WireEqActionTargets::AllLoaded => Ok(Self::AllLoaded),
            WireEqActionTargets::Active => Ok(Self::Active),
            WireEqActionTargets::BackgroundLoaded => Ok(Self::BackgroundLoaded),
            WireEqActionTargets::WindowNumbers { window_numbers } => {
                Self::window_numbers(window_numbers)
            }
        }
    }
}

impl From<EqActionTargets> for WireEqActionTargets {
    fn from(value: EqActionTargets) -> Self {
        match value {
            EqActionTargets::AllLoaded => Self::AllLoaded,
            EqActionTargets::Active => Self::Active,
            EqActionTargets::BackgroundLoaded => Self::BackgroundLoaded,
            EqActionTargets::WindowNumbers(window_numbers) => {
                Self::WindowNumbers { window_numbers }
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    ClientId {
        client_id: String,
    },
    WindowNumber {
        window_number: usize,
    },
    Identity {
        character: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server: Option<String>,
    },
}

impl Target {
    fn validate(&self) -> Result<(), ControlError> {
        match self {
            Self::ClientId { client_id } => {
                ClientId::new(client_id.clone())?;
            }
            Self::WindowNumber { window_number } if *window_number == 0 => {
                return Err(ControlError::new(
                    crate::control::ErrorCode::InvalidArgument,
                    "window_number is one-based and must be greater than zero",
                ));
            }
            Self::Identity { character, server } => {
                if character.trim().is_empty() || character.len() > 128 {
                    return Err(ControlError::new(
                        crate::control::ErrorCode::InvalidArgument,
                        "character must contain 1 to 128 bytes",
                    ));
                }
                if server
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty() || value.len() > 128)
                {
                    return Err(ControlError::new(
                        crate::control::ErrorCode::InvalidArgument,
                        "server must contain 1 to 128 bytes when supplied",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl TryFrom<Target> for ClientTarget {
    type Error = ControlError;

    fn try_from(value: Target) -> Result<Self, Self::Error> {
        value.validate()?;
        Ok(match value {
            Target::ClientId { client_id } => Self::Id(ClientId::new(client_id)?),
            Target::WindowNumber { window_number } => Self::WindowNumber(window_number),
            Target::Identity { character, server } => Self::Identity { character, server },
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    State {
        version: u16,
        state: WireState,
    },
    Result {
        version: u16,
        request_id: String,
        result: Success,
        state: WireState,
    },
    Error {
        version: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        error: WireError,
    },
    Paired {
        version: u16,
        auth_token: String,
    },
}

impl ServerMessage {
    pub fn state(snapshot: &StateSnapshot) -> Self {
        Self::State {
            version: PROTOCOL_VERSION,
            state: snapshot.into(),
        }
    }

    pub fn success(request_id: String, outcome: Success, snapshot: &StateSnapshot) -> Self {
        Self::Result {
            version: PROTOCOL_VERSION,
            request_id,
            result: outcome,
            state: snapshot.into(),
        }
    }

    pub fn error(request_id: Option<String>, error: ControlError) -> Self {
        Self::Error {
            version: PROTOCOL_VERSION,
            request_id,
            error: error.into(),
        }
    }

    pub fn paired(auth_token: String) -> Self {
        Self::Paired {
            version: PROTOCOL_VERSION,
            auth_token,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Success {
    State,
    Activated {
        status: WireActivationStatus,
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
        input: WireInputKind,
        strokes: usize,
    },
    EqActionDelivered {
        action: WireEqAction,
    },
    EqKeymapActionsListed {
        mappings: Vec<String>,
        window_numbers: Vec<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        next_after: Option<String>,
    },
    EqActionBatchDelivered {
        action: WireEqAction,
        window_numbers: Vec<usize>,
    },
}

impl From<CommandOutcome> for Success {
    fn from(value: CommandOutcome) -> Self {
        match value {
            CommandOutcome::Activated {
                status,
                foreground_confirmed,
            } => Self::Activated {
                status: status.into(),
                foreground_confirmed,
            },
            CommandOutcome::WindowNumbersSwapped {
                active_previous_number,
                selected_previous_number,
            } => Self::WindowNumbersSwapped {
                active_previous_number,
                selected_previous_number,
            },
            CommandOutcome::BroadcastSet { enabled } => Self::BroadcastSet { enabled },
            CommandOutcome::MouseClutchHoldUpdated { held } => {
                Self::MouseClutchHoldUpdated { held }
            }
            CommandOutcome::InputDelivered { kind, strokes } => Self::InputDelivered {
                input: kind.into(),
                strokes,
            },
            CommandOutcome::EqActionDelivered { action } => Self::EqActionDelivered {
                action: action.into(),
            },
            CommandOutcome::EqKeymapActionsListed {
                mappings,
                window_numbers,
                next_after,
            } => Self::EqKeymapActionsListed {
                mappings: mappings
                    .into_iter()
                    .map(|mapping| mapping.as_str().to_owned())
                    .collect(),
                window_numbers,
                next_after: next_after.map(|mapping| mapping.as_str().to_owned()),
            },
            CommandOutcome::EqActionBatchDelivered {
                action,
                window_numbers,
            } => Self::EqActionBatchDelivered {
                action: action.into(),
                window_numbers,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireInputKind {
    Text,
    Keys,
}

impl From<InputKind> for WireInputKind {
    fn from(value: InputKind) -> Self {
        match value {
            InputKind::Text => Self::Text,
            InputKind::Keys => Self::Keys,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireActivationStatus {
    Activated,
    AlreadyActive,
}

impl From<ActivationStatus> for WireActivationStatus {
    fn from(value: ActivationStatus) -> Self {
        match value {
            ActivationStatus::Activated => Self::Activated,
            ActivationStatus::AlreadyActive => Self::AlreadyActive,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireState {
    pub revision: u64,
    pub clients: Vec<WireClient>,
    pub active_client_id: Option<String>,
    pub broadcast: WireBroadcast,
    #[serde(default)]
    pub mouse_clutch: WireMouseClutch,
    pub capabilities: WireCapabilities,
}

impl From<&StateSnapshot> for WireState {
    fn from(value: &StateSnapshot) -> Self {
        Self {
            revision: value.revision,
            active_client_id: value
                .clients
                .iter()
                .find(|client| client.active)
                .map(|client| client.id.as_str().to_owned()),
            clients: value.clients.iter().map(WireClient::from).collect(),
            broadcast: WireBroadcast {
                available: value.broadcast.available,
                enabled: value.broadcast.enabled,
            },
            mouse_clutch: value.mouse_clutch.into(),
            capabilities: WireCapabilities {
                activate: value.capabilities.activate,
                swap_window_numbers: value.capabilities.swap_window_numbers,
                set_broadcast: value.capabilities.set_broadcast,
                set_mouse_clutch: value.capabilities.set_mouse_clutch,
                send_text: value.capabilities.send_text,
                send_keys: value.capabilities.send_keys,
                eq_actions: WireEqActionCapabilities {
                    use_center_screen: value.capabilities.eq_actions.use_center_screen,
                    invite_follow: value.capabilities.eq_actions.invite_follow,
                    hotbars: value.capabilities.eq_actions.hotbars,
                    hotbar_buttons: value.capabilities.eq_actions.hotbar_buttons,
                    spell_gems: value.capabilities.eq_actions.spell_gems,
                    keymap_actions: value.capabilities.eq_actions.keymap_actions,
                },
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireClient {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_code: Option<String>,
    pub window_number: usize,
    pub active: bool,
    pub activatable: bool,
    #[serde(default)]
    pub input_ready: bool,
}

impl From<&crate::control::ClientState> for WireClient {
    fn from(value: &crate::control::ClientState) -> Self {
        Self {
            id: value.id.as_str().to_owned(),
            character: value.character.clone(),
            server: value.server.clone(),
            class_code: value.class_code.clone(),
            window_number: value.window_number,
            active: value.active,
            activatable: value.activatable,
            input_ready: value.input_ready,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireBroadcast {
    pub available: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireMouseClutch {
    pub phase: WireMouseClutchPhase,
    pub availability: WireMouseClutchAvailability,
}

impl From<MouseClutchState> for WireMouseClutch {
    fn from(value: MouseClutchState) -> Self {
        Self {
            phase: value.phase.into(),
            availability: value.availability.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireMouseClutchPhase {
    #[default]
    Inactive,
    Active,
    Releasing,
}

impl From<MouseClutchPhase> for WireMouseClutchPhase {
    fn from(value: MouseClutchPhase) -> Self {
        match value {
            MouseClutchPhase::Inactive => Self::Inactive,
            MouseClutchPhase::Active => Self::Active,
            MouseClutchPhase::Releasing => Self::Releasing,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireMouseClutchAvailability {
    Ready,
    NoActiveClient,
    NoCompatibleTargets,
    #[default]
    InputUnavailable,
}

impl From<MouseClutchAvailability> for WireMouseClutchAvailability {
    fn from(value: MouseClutchAvailability) -> Self {
        match value {
            MouseClutchAvailability::Ready => Self::Ready,
            MouseClutchAvailability::NoActiveClient => Self::NoActiveClient,
            MouseClutchAvailability::NoCompatibleTargets => Self::NoCompatibleTargets,
            MouseClutchAvailability::InputUnavailable => Self::InputUnavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireEqActionCapabilities {
    #[serde(default)]
    pub use_center_screen: bool,
    #[serde(default)]
    pub invite_follow: bool,
    #[serde(default)]
    pub hotbars: u8,
    #[serde(default)]
    pub hotbar_buttons: u8,
    #[serde(default)]
    pub spell_gems: u8,
    #[serde(default)]
    pub keymap_actions: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireCapabilities {
    pub activate: bool,
    #[serde(default)]
    pub swap_window_numbers: bool,
    pub set_broadcast: bool,
    #[serde(default)]
    pub set_mouse_clutch: bool,
    pub send_text: bool,
    pub send_keys: bool,
    #[serde(default)]
    pub eq_actions: WireEqActionCapabilities,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

impl From<ControlError> for WireError {
    fn from(value: ControlError) -> Self {
        Self {
            code: value.code.as_str().to_owned(),
            message: value.message,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct DecodeError {
    pub request_id: Option<String>,
    pub error: ControlError,
}

pub fn decode_pairing_request(text: &str) -> Result<PairingRequest, ControlError> {
    if text.len() > MAX_TEXT_MESSAGE_SIZE {
        return Err(ControlError::new(
            crate::control::ErrorCode::MalformedRequest,
            "text message exceeds the 16384-byte limit",
        ));
    }
    let request: PairingRequest = serde_json::from_str(text).map_err(|_| {
        ControlError::new(
            crate::control::ErrorCode::MalformedRequest,
            "pairing request has an unknown type or invalid fields",
        )
    })?;
    request.validate()?;
    Ok(request)
}

pub fn decode_client_message(text: &str) -> Result<ClientMessage, DecodeError> {
    if text.len() > MAX_TEXT_MESSAGE_SIZE {
        return Err(DecodeError {
            request_id: None,
            error: ControlError::new(
                crate::control::ErrorCode::MalformedRequest,
                "text message exceeds the 16384-byte limit",
            ),
        });
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| DecodeError {
        request_id: None,
        error: ControlError::new(
            crate::control::ErrorCode::MalformedRequest,
            "request is not valid JSON",
        ),
    })?;
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let message: ClientMessage = serde_json::from_value(value).map_err(|_| DecodeError {
        request_id: request_id.clone(),
        error: ControlError::new(
            crate::control::ErrorCode::MalformedRequest,
            "request has an unknown type or invalid fields",
        ),
    })?;
    message
        .validate()
        .map_err(|error| DecodeError { request_id, error })?;
    Ok(message)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataFramePolicy {
    ParseText,
    RejectBinary,
    IgnoreControl,
}

pub fn data_frame_policy(is_text: bool, is_binary: bool) -> DataFramePolicy {
    if is_text {
        DataFramePolicy::ParseText
    } else if is_binary {
        DataFramePolicy::RejectBinary
    } else {
        DataFramePolicy::IgnoreControl
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{BroadcastState, Capabilities, ClientState, ErrorCode};

    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + Eq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).unwrap();
        assert_eq!(*value, serde_json::from_str(&json).unwrap());
    }

    #[test]
    fn pairing_requests_validate_six_digit_codes() {
        let request = PairingRequest::Pair {
            version: 1,
            code: "004271".into(),
        };
        round_trip(&request);
        assert_eq!(
            decode_pairing_request(r#"{"type":"pair","version":1,"code":"004271"}"#).unwrap(),
            request
        );
        assert_eq!(
            decode_pairing_request(r#"{"type":"pair","version":1,"code":"4271"}"#)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn every_client_message_round_trips() {
        let messages = [
            ClientMessage::GetState {
                version: 1,
                request_id: "one".into(),
            },
            ClientMessage::Activate {
                version: 1,
                request_id: "two".into(),
                target: Target::ClientId {
                    client_id: "client-1".into(),
                },
            },
            ClientMessage::Activate {
                version: 1,
                request_id: "three".into(),
                target: Target::WindowNumber { window_number: 2 },
            },
            ClientMessage::Activate {
                version: 1,
                request_id: "four".into(),
                target: Target::Identity {
                    character: "Laika".into(),
                    server: Some("Xegony".into()),
                },
            },
            ClientMessage::SwapWindowNumbers {
                version: 1,
                request_id: "five".into(),
                target: Target::ClientId {
                    client_id: "client-2".into(),
                },
            },
            ClientMessage::SetBroadcast {
                version: 1,
                request_id: "six".into(),
                enabled: true,
            },
            ClientMessage::BeginMouseClutch {
                version: 1,
                request_id: "clutch-down".into(),
                hold_id: "hold-1".into(),
            },
            ClientMessage::RenewMouseClutch {
                version: 1,
                request_id: "clutch-renew".into(),
                hold_id: "hold-1".into(),
            },
            ClientMessage::EndMouseClutch {
                version: 1,
                request_id: "clutch-up".into(),
                hold_id: "hold-1".into(),
            },
            ClientMessage::SendText {
                version: 1,
                request_id: "seven".into(),
                client_id: "client-1".into(),
                text: "/who".into(),
                submit: true,
            },
            ClientMessage::SendKeys {
                version: 1,
                request_id: "eight".into(),
                client_id: "client-1".into(),
                strokes: vec![WireKeyStroke {
                    keys: vec!["left_control".into(), "1".into()],
                    hold_ms: 50,
                    pause_ms: 50,
                }],
            },
            ClientMessage::SendEqAction {
                version: 1,
                request_id: "nine".into(),
                client_id: "client-1".into(),
                action: WireEqAction::Hotbar {
                    bar: 11,
                    button: 12,
                },
            },
            ClientMessage::ListEqKeymapActions {
                version: 1,
                request_id: "ten".into(),
                targets: WireEqActionTargets::WindowNumbers {
                    window_numbers: vec![1, 3],
                },
                after: Some("DUCK".into()),
            },
            ClientMessage::SendEqActionBatch {
                version: 1,
                request_id: "eleven".into(),
                targets: WireEqActionTargets::AllLoaded,
                action: WireEqAction::Keymap {
                    mapping: "SIT_STAND".into(),
                },
            },
            ClientMessage::SendEqActionBatch {
                version: 1,
                request_id: "twelve".into(),
                targets: WireEqActionTargets::Active,
                action: WireEqAction::Keymap {
                    mapping: "HOT1_1".into(),
                },
            },
            ClientMessage::SendEqActionBatch {
                version: 1,
                request_id: "thirteen".into(),
                targets: WireEqActionTargets::BackgroundLoaded,
                action: WireEqAction::Keymap {
                    mapping: "HOT1_2".into(),
                },
            },
        ];
        for message in messages {
            round_trip(&message);
        }
    }

    #[test]
    fn every_server_message_round_trips_with_optional_identity() {
        let snapshot = StateSnapshot {
            revision: 9,
            clients: vec![ClientState {
                id: ClientId::new("client-1").unwrap(),
                character: None,
                server: None,
                class_code: Some("SHK".into()),
                window_number: 1,
                active: true,
                activatable: true,
                input_ready: true,
            }],
            broadcast: BroadcastState {
                available: true,
                enabled: false,
            },
            mouse_clutch: MouseClutchState {
                phase: MouseClutchPhase::Active,
                availability: MouseClutchAvailability::Ready,
            },
            capabilities: Capabilities {
                activate: true,
                swap_window_numbers: true,
                set_broadcast: true,
                set_mouse_clutch: true,
                send_text: true,
                send_keys: true,
                eq_actions: crate::control::EqActionCapabilities::available(true),
            },
        };
        let messages = [
            ServerMessage::state(&snapshot),
            ServerMessage::success("one".into(), Success::State, &snapshot),
            ServerMessage::success(
                "two".into(),
                Success::Activated {
                    status: WireActivationStatus::AlreadyActive,
                    foreground_confirmed: true,
                },
                &snapshot,
            ),
            ServerMessage::success(
                "three".into(),
                Success::WindowNumbersSwapped {
                    active_previous_number: 1,
                    selected_previous_number: 2,
                },
                &snapshot,
            ),
            ServerMessage::success(
                "four".into(),
                Success::BroadcastSet { enabled: false },
                &snapshot,
            ),
            ServerMessage::success(
                "clutch".into(),
                Success::MouseClutchHoldUpdated { held: true },
                &snapshot,
            ),
            ServerMessage::success(
                "input".into(),
                Success::InputDelivered {
                    input: WireInputKind::Text,
                    strokes: 5,
                },
                &snapshot,
            ),
            ServerMessage::success(
                "action".into(),
                Success::EqActionDelivered {
                    action: WireEqAction::SpellGem { gem: 14 },
                },
                &snapshot,
            ),
            ServerMessage::success(
                "mapped".into(),
                Success::EqKeymapActionsListed {
                    mappings: vec!["DUCK".into(), "SIT_STAND".into()],
                    window_numbers: vec![1, 2],
                    next_after: Some("SIT_STAND".into()),
                },
                &snapshot,
            ),
            ServerMessage::success(
                "batch".into(),
                Success::EqActionBatchDelivered {
                    action: WireEqAction::Keymap {
                        mapping: "DUCK".into(),
                    },
                    window_numbers: vec![1, 2],
                },
                &snapshot,
            ),
            ServerMessage::error(
                Some("five".into()),
                ControlError::new(ErrorCode::ClientNotFound, "not found"),
            ),
            ServerMessage::paired("secret-token".into()),
        ];
        for message in messages {
            round_trip(&message);
        }
        let json = serde_json::to_value(ServerMessage::state(&snapshot)).unwrap();
        let client = &json["state"]["clients"][0];
        assert!(client.get("character").is_none());
        assert!(client.get("server").is_none());
        assert_eq!(client["class_code"], "SHK");
        assert_eq!(client["input_ready"], true);
    }

    #[test]
    fn older_wire_state_defaults_new_capabilities_and_client_readiness() {
        let client: WireClient = serde_json::from_str(
            r#"{"id":"client-1","window_number":1,"active":true,"activatable":true}"#,
        )
        .unwrap();
        assert!(!client.input_ready);

        let capabilities: WireCapabilities = serde_json::from_str(
            r#"{"activate":true,"set_broadcast":true,"send_text":true,"send_keys":true}"#,
        )
        .unwrap();
        assert!(!capabilities.swap_window_numbers);
        assert!(!capabilities.set_mouse_clutch);
        assert_eq!(capabilities.eq_actions, WireEqActionCapabilities::default());
        assert!(!capabilities.eq_actions.keymap_actions);
    }

    #[test]
    fn rejects_unknown_type_and_malformed_json_without_panicking() {
        let unknown =
            decode_client_message(r#"{"type":"future","version":1,"request_id":"x"}"#).unwrap_err();
        assert_eq!(unknown.request_id.as_deref(), Some("x"));
        assert_eq!(unknown.error.code, ErrorCode::MalformedRequest);

        let malformed = decode_client_message("{").unwrap_err();
        assert_eq!(malformed.request_id, None);
        assert_eq!(malformed.error.code, ErrorCode::MalformedRequest);
    }

    #[test]
    fn validates_version_request_id_and_arguments() {
        let unsupported =
            decode_client_message(r#"{"type":"get_state","version":2,"request_id":"x"}"#)
                .unwrap_err();
        assert_eq!(
            unsupported.error.code,
            ErrorCode::UnsupportedProtocolVersion
        );

        let missing_id =
            decode_client_message(r#"{"type":"get_state","version":1,"request_id":""}"#)
                .unwrap_err();
        assert_eq!(missing_id.error.code, ErrorCode::InvalidArgument);

        let zero = decode_client_message(
            r#"{"type":"activate","version":1,"request_id":"x","target":{"type":"window_number","window_number":0}}"#,
        )
        .unwrap_err();
        assert_eq!(zero.error.code, ErrorCode::InvalidArgument);

        let empty_hold = decode_client_message(
            r#"{"type":"begin_mouse_clutch","version":1,"request_id":"x","hold_id":""}"#,
        )
        .unwrap_err();
        assert_eq!(empty_hold.error.code, ErrorCode::InvalidArgument);

        let control_text = decode_client_message(
            "{\"type\":\"send_text\",\"version\":1,\"request_id\":\"x\",\"client_id\":\"client-1\",\"text\":\"bad\\ntext\"}",
        )
        .unwrap_err();
        assert_eq!(control_text.error.code, ErrorCode::InvalidArgument);

        let unknown_key = decode_client_message(
            r#"{"type":"send_keys","version":1,"request_id":"x","client_id":"client-1","strokes":[{"keys":["launch_missiles"]}]}"#,
        )
        .unwrap_err();
        assert_eq!(unknown_key.error.code, ErrorCode::InvalidArgument);

        for invalid_action in [
            r#"{"type":"hotbar","bar":0,"button":1}"#,
            r#"{"type":"hotbar","bar":1,"button":13}"#,
            r#"{"type":"spell_gem","gem":15}"#,
            r#"{"type":"keymap","mapping":"../DUCK"}"#,
        ] {
            let request = format!(
                r#"{{"type":"send_eq_action","version":1,"request_id":"x","client_id":"client-1","action":{invalid_action}}}"#
            );
            let error = decode_client_message(&request).unwrap_err();
            assert_eq!(error.error.code, ErrorCode::InvalidArgument);
        }

        for invalid_targets in [
            r#"{"type":"window_numbers","window_numbers":[]}"#,
            r#"{"type":"window_numbers","window_numbers":[1,1]}"#,
            r#"{"type":"window_numbers","window_numbers":[7]}"#,
        ] {
            let request = format!(
                r#"{{"type":"send_eq_action_batch","version":1,"request_id":"x","targets":{invalid_targets},"action":{{"type":"keymap","mapping":"DUCK"}}}}"#
            );
            let error = decode_client_message(&request).unwrap_err();
            assert_eq!(error.error.code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn binary_and_oversized_policies_are_explicit() {
        assert_eq!(data_frame_policy(true, false), DataFramePolicy::ParseText);
        assert_eq!(
            data_frame_policy(false, true),
            DataFramePolicy::RejectBinary
        );
        assert_eq!(
            data_frame_policy(false, false),
            DataFramePolicy::IgnoreControl
        );
        let oversized = "x".repeat(MAX_TEXT_MESSAGE_SIZE + 1);
        assert_eq!(
            decode_client_message(&oversized).unwrap_err().error.code,
            ErrorCode::MalformedRequest
        );
    }
}
