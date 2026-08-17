use crate::control::{
    validate_key_strokes, validate_text_input, ActivationStatus, ClientId, ClientTarget,
    CommandOutcome, ControlError, InputKind, KeyCode, KeyStroke, StateSnapshot,
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
    SetBroadcast {
        version: u16,
        request_id: String,
        enabled: bool,
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
}

impl ClientMessage {
    pub fn version(&self) -> u16 {
        match self {
            Self::GetState { version, .. }
            | Self::Activate { version, .. }
            | Self::SetBroadcast { version, .. }
            | Self::SendText { version, .. }
            | Self::SendKeys { version, .. } => *version,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::GetState { request_id, .. }
            | Self::Activate { request_id, .. }
            | Self::SetBroadcast { request_id, .. }
            | Self::SendText { request_id, .. }
            | Self::SendKeys { request_id, .. } => request_id,
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
        if let Self::Activate { target, .. } = self {
            target.validate()?;
        }
        match self {
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
    BroadcastSet {
        enabled: bool,
    },
    InputDelivered {
        input: WireInputKind,
        strokes: usize,
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
            CommandOutcome::BroadcastSet { enabled } => Self::BroadcastSet { enabled },
            CommandOutcome::InputDelivered { kind, strokes } => Self::InputDelivered {
                input: kind.into(),
                strokes,
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
            capabilities: WireCapabilities {
                activate: value.capabilities.activate,
                set_broadcast: value.capabilities.set_broadcast,
                send_text: value.capabilities.send_text,
                send_keys: value.capabilities.send_keys,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireCapabilities {
    pub activate: bool,
    pub set_broadcast: bool,
    pub send_text: bool,
    pub send_keys: bool,
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
            ClientMessage::SetBroadcast {
                version: 1,
                request_id: "five".into(),
                enabled: true,
            },
            ClientMessage::SendText {
                version: 1,
                request_id: "six".into(),
                client_id: "client-1".into(),
                text: "/who".into(),
                submit: true,
            },
            ClientMessage::SendKeys {
                version: 1,
                request_id: "seven".into(),
                client_id: "client-1".into(),
                strokes: vec![WireKeyStroke {
                    keys: vec!["left_control".into(), "1".into()],
                    hold_ms: 50,
                    pause_ms: 50,
                }],
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
            capabilities: Capabilities {
                activate: true,
                set_broadcast: true,
                send_text: true,
                send_keys: true,
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
                Success::BroadcastSet { enabled: false },
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
            ServerMessage::error(
                Some("four".into()),
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
    fn older_wire_clients_default_to_not_input_ready() {
        let client: WireClient = serde_json::from_str(
            r#"{"id":"client-1","window_number":1,"active":true,"activatable":true}"#,
        )
        .unwrap();
        assert!(!client.input_ready);
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
