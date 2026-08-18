use crate::control::{ControlError, Controller, ErrorCode};
use crate::protocol::{
    data_frame_policy, decode_client_message, decode_pairing_request, ClientMessage,
    DataFramePolicy, ServerMessage, Success, MAX_TEXT_MESSAGE_SIZE,
};
use futures_util::stream::FuturesUnordered;
use futures_util::{SinkExt, StreamExt};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{header, StatusCode};
use tokio_tungstenite::tungstenite::protocol::{
    frame::coding::CloseCode, CloseFrame, WebSocketConfig,
};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::{accept_hdr_async_with_config, WebSocketStream};

pub const DEFAULT_BIND: &str = "127.0.0.1:19720";
pub const ENDPOINT_PATH: &str = "/trushar/v1";
pub const PAIRING_ENDPOINT_PATH: &str = "/trushar/v1/pair";
pub const PAIRING_CODE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PAIRING_ATTEMPTS: u8 = 5;
const START_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_IN_FLIGHT_COMMANDS: usize = 16;

#[cfg(windows)]
fn prevent_listener_socket_inheritance(listener: &TcpListener) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    prevent_raw_socket_inheritance(listener.as_raw_socket())
}

#[cfg(not(windows))]
fn prevent_listener_socket_inheritance(_listener: &TcpListener) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn prevent_stream_socket_inheritance(stream: &TcpStream) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    prevent_raw_socket_inheritance(stream.as_raw_socket())
}

#[cfg(not(windows))]
fn prevent_stream_socket_inheritance(_stream: &TcpStream) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn prevent_raw_socket_inheritance(socket: std::os::windows::io::RawSocket) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT};

    let handle = socket as usize as *mut std::ffi::c_void;
    let succeeded = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    /// Required for non-loopback binds. Never included in protocol messages or diagnostics.
    pub auth_token: Option<String>,
}

impl ServerConfig {
    pub fn loopback(bind: SocketAddr) -> Self {
        Self {
            bind,
            auth_token: None,
        }
    }

    pub fn validate(&self) -> Result<(), StartError> {
        if self
            .auth_token
            .as_ref()
            .is_some_and(|token| token.trim().is_empty())
        {
            return Err(StartError::InvalidConfiguration(
                "authentication token must not be empty".into(),
            ));
        }
        if !self.bind.ip().is_loopback() && self.auth_token.is_none() {
            return Err(StartError::InvalidConfiguration(
                "wildcard and non-loopback binds require an authentication token".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct PairingHandle {
    inner: Arc<Mutex<PairingState>>,
}

#[derive(Default)]
struct PairingState {
    next_session_id: u64,
    session: Option<PairingSession>,
}

struct PairingSession {
    id: u64,
    code: String,
    auth_token: String,
    expires_at: Instant,
    failed_attempts: u8,
}

enum PairingAttempt {
    Paired(String),
    Failed,
}

impl PairingHandle {
    pub fn begin(&self, code: u32, auth_token: String) -> bool {
        if code > 999_999 || auth_token.trim().is_empty() {
            return false;
        }
        self.begin_until(code, auth_token, Instant::now() + PAIRING_CODE_TTL);
        true
    }

    fn begin_until(&self, code: u32, auth_token: String, expires_at: Instant) {
        let mut state = self.inner.lock().expect("pairing state poisoned");
        state.next_session_id = state.next_session_id.wrapping_add(1).max(1);
        let id = state.next_session_id;
        state.session = Some(PairingSession {
            id,
            code: format!("{code:06}"),
            auth_token,
            expires_at,
            failed_attempts: 0,
        });
    }

    pub fn cancel(&self) {
        self.inner.lock().expect("pairing state poisoned").session = None;
    }

    pub fn is_open(&self) -> bool {
        self.session_id().is_some()
    }

    fn session_id(&self) -> Option<u64> {
        let mut state = self.inner.lock().expect("pairing state poisoned");
        if state
            .session
            .as_ref()
            .is_some_and(|session| Instant::now() >= session.expires_at)
        {
            state.session = None;
        }
        state.session.as_ref().map(|session| session.id)
    }

    fn attempt(&self, session_id: u64, supplied_code: &str) -> PairingAttempt {
        let mut state = self.inner.lock().expect("pairing state poisoned");
        let Some(session) = state.session.as_ref() else {
            return PairingAttempt::Failed;
        };
        if Instant::now() >= session.expires_at {
            state.session = None;
            return PairingAttempt::Failed;
        }
        if session.id != session_id {
            return PairingAttempt::Failed;
        }
        if !constant_time_eq(supplied_code, &session.code) {
            let exhausted = {
                let session = state.session.as_mut().expect("pairing session disappeared");
                session.failed_attempts = session.failed_attempts.saturating_add(1);
                session.failed_attempts >= MAX_PAIRING_ATTEMPTS
            };
            if exhausted {
                state.session = None;
            }
            return PairingAttempt::Failed;
        }
        let session = state.session.take().expect("pairing session disappeared");
        PairingAttempt::Paired(session.auth_token)
    }
}

#[derive(Debug)]
pub enum StartError {
    InvalidConfiguration(String),
    Runtime(String),
    Bind {
        address: SocketAddr,
        message: String,
    },
    StartupTimeout,
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) | Self::Runtime(message) => f.write_str(message),
            Self::Bind { address, message } => write!(f, "failed to bind {address}: {message}"),
            Self::StartupTimeout => f.write_str("server thread did not report startup in time"),
        }
    }
}

impl std::error::Error for StartError {}

pub struct ServerHandle {
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    pairing: PairingHandle,
    thread: Option<thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn start(
        config: ServerConfig,
        controller: Arc<dyn Controller>,
    ) -> Result<Self, StartError> {
        config.validate()?;
        let (shutdown, shutdown_rx) = watch::channel(false);
        let (startup_tx, startup_rx) = std_mpsc::sync_channel(1);
        let pairing = PairingHandle::default();
        let server_pairing = pairing.clone();
        let thread = thread::Builder::new()
            .name("trushar-server".into())
            .spawn(move || {
                server_thread(config, controller, server_pairing, shutdown_rx, startup_tx)
            })
            .map_err(|error| StartError::Runtime(error.to_string()))?;

        match startup_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(local_addr)) => Ok(Self {
                local_addr,
                shutdown,
                pairing,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = shutdown.send(true);
                let _ = thread.join();
                Err(StartError::StartupTimeout)
            }
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn pairing_handle(&self) -> PairingHandle {
        self.pairing.clone()
    }

    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

fn server_thread(
    config: ServerConfig,
    controller: Arc<dyn Controller>,
    pairing: PairingHandle,
    shutdown: watch::Receiver<bool>,
    startup: std_mpsc::SyncSender<Result<SocketAddr, StartError>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = startup.send(Err(StartError::Runtime(error.to_string())));
            return;
        }
    };
    runtime.block_on(run_server(config, controller, pairing, shutdown, startup));
}

async fn run_server(
    config: ServerConfig,
    controller: Arc<dyn Controller>,
    pairing: PairingHandle,
    mut shutdown: watch::Receiver<bool>,
    startup: std_mpsc::SyncSender<Result<SocketAddr, StartError>>,
) {
    let listener = match TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(error) => {
            let _ = startup.send(Err(StartError::Bind {
                address: config.bind,
                message: error.to_string(),
            }));
            return;
        }
    };
    if let Err(error) = prevent_listener_socket_inheritance(&listener) {
        let _ = startup.send(Err(StartError::Runtime(format!(
            "failed to protect the listener from child-process inheritance: {error}"
        ))));
        return;
    }
    let local_addr = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            let _ = startup.send(Err(StartError::Bind {
                address: config.bind,
                message: error.to_string(),
            }));
            return;
        }
    };
    if startup.send(Ok(local_addr)).is_err() {
        return;
    }

    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        if prevent_stream_socket_inheritance(&stream).is_err() {
                            continue;
                        }
                        let config = config.clone();
                        let controller = controller.clone();
                        let pairing = pairing.clone();
                        let shutdown = shutdown.clone();
                        connections.spawn(async move {
                            serve_connection(stream, peer, config, controller, pairing, shutdown).await;
                        });
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
            }
        }
    }

    let drain = async { while connections.join_next().await.is_some() {} };
    if tokio::time::timeout(CONNECTION_SHUTDOWN_TIMEOUT, drain)
        .await
        .is_err()
    {
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    }
}

#[allow(clippy::result_large_err)] // Tungstenite's callback requires its concrete ErrorResponse.
async fn serve_connection(
    stream: TcpStream,
    _peer: SocketAddr,
    config: ServerConfig,
    controller: Arc<dyn Controller>,
    pairing: PairingHandle,
    shutdown: watch::Receiver<bool>,
) {
    let handshake_config = config.clone();
    let handshake_pairing = pairing.clone();
    let pairing_session_id = Arc::new(AtomicU64::new(0));
    let callback_pairing_session_id = pairing_session_id.clone();
    let callback = move |request: &Request, response: Response| {
        authorize_upgrade(
            request,
            response,
            &handshake_config,
            &handshake_pairing,
            &callback_pairing_session_id,
        )
    };
    let websocket_config = WebSocketConfig::default()
        .read_buffer_size(4 * 1024)
        .write_buffer_size(0)
        .max_write_buffer_size(64 * 1024)
        .max_message_size(Some(MAX_TEXT_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_TEXT_MESSAGE_SIZE));
    let websocket = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        accept_hdr_async_with_config(stream, callback, Some(websocket_config)),
    )
    .await
    {
        Ok(Ok(websocket)) => websocket,
        _ => return,
    };
    let pairing_session_id = pairing_session_id.load(Ordering::SeqCst);
    if pairing_session_id != 0 {
        pairing_loop(websocket, pairing, pairing_session_id, shutdown).await;
    } else {
        connection_loop(websocket, controller, shutdown).await;
    }
}

#[allow(clippy::result_large_err)] // Signature is fixed by Tungstenite's Callback trait.
fn authorize_upgrade(
    request: &Request,
    response: Response,
    config: &ServerConfig,
    pairing: &PairingHandle,
    pairing_session_id: &AtomicU64,
) -> Result<Response, ErrorResponse> {
    if request.uri().path() == PAIRING_ENDPOINT_PATH {
        if request.headers().contains_key(header::ORIGIN) {
            return Err(handshake_error(
                StatusCode::FORBIDDEN,
                Some(ErrorCode::Unauthorized),
                "Browser-originated pairing is not allowed",
            ));
        }
        let Some(session_id) = pairing.session_id() else {
            return Err(handshake_error(
                StatusCode::FORBIDDEN,
                Some(ErrorCode::Unauthorized),
                "pairing is not currently available",
            ));
        };
        pairing_session_id.store(session_id, Ordering::SeqCst);
        return Ok(response);
    }
    if request.uri().path() != ENDPOINT_PATH {
        return Err(handshake_error(
            StatusCode::NOT_FOUND,
            None,
            "WebSocket endpoint not found",
        ));
    }
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authenticated = config
        .auth_token
        .as_deref()
        .is_some_and(|expected| supplied.is_some_and(|actual| constant_time_eq(actual, expected)));

    if config.auth_token.is_some() && !authenticated {
        return Err(handshake_error(
            StatusCode::UNAUTHORIZED,
            Some(ErrorCode::Unauthorized),
            "authentication failed",
        ));
    }
    if request.headers().contains_key(header::ORIGIN) && !authenticated {
        return Err(handshake_error(
            StatusCode::FORBIDDEN,
            Some(ErrorCode::Unauthorized),
            "Browser-originated connections require authentication",
        ));
    }
    Ok(response)
}

fn handshake_error(
    status: StatusCode,
    code: Option<ErrorCode>,
    message: &'static str,
) -> ErrorResponse {
    let mut response = tokio_tungstenite::tungstenite::http::Response::builder().status(status);
    let body = code.map_or_else(
        || message.to_owned(),
        |code| {
            serde_json::json!({
                "error": { "code": code.as_str(), "message": message }
            })
            .to_string()
        },
    );
    if code.is_some() {
        response = response.header(header::CONTENT_TYPE, "application/json");
    }
    response
        .body(Some(body))
        .expect("static handshake response is valid")
}

fn constant_time_eq(actual: &str, expected: &str) -> bool {
    let actual = actual.as_bytes();
    let expected = expected.as_bytes();
    let mut difference = actual.len() ^ expected.len();
    for index in 0..actual.len().max(expected.len()) {
        difference |= usize::from(
            actual.get(index).copied().unwrap_or(0) ^ expected.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

type CommandFuture = futures_util::future::BoxFuture<
    'static,
    (String, Result<crate::control::CommandOutcome, ControlError>),
>;

async fn pairing_loop(
    mut websocket: WebSocketStream<TcpStream>,
    pairing: PairingHandle,
    pairing_session_id: u64,
    mut shutdown: watch::Receiver<bool>,
) {
    let incoming = tokio::select! {
        changed = shutdown.changed() => {
            if changed.is_err() || *shutdown.borrow() {
                graceful_close(&mut websocket, CloseCode::Away, "server shutting down").await;
            }
            return;
        }
        incoming = tokio::time::timeout(HANDSHAKE_TIMEOUT, websocket.next()) => incoming,
    };
    let request = match incoming {
        Ok(Some(Ok(message))) if message.is_text() => message
            .to_text()
            .ok()
            .and_then(|text| decode_pairing_request(text).ok()),
        _ => None,
    };
    let response = match request {
        Some(request) => match pairing.attempt(pairing_session_id, request.code()) {
            PairingAttempt::Paired(auth_token) => ServerMessage::paired(auth_token),
            PairingAttempt::Failed => ServerMessage::error(
                None,
                ControlError::new(
                    ErrorCode::Unauthorized,
                    "pairing code is invalid, expired, or no longer available",
                ),
            ),
        },
        None => ServerMessage::error(
            None,
            ControlError::new(ErrorCode::MalformedRequest, "invalid pairing request"),
        ),
    };
    let paired = matches!(response, ServerMessage::Paired { .. });
    if send_message(&mut websocket, &response).await.is_ok() {
        graceful_close(
            &mut websocket,
            if paired {
                CloseCode::Normal
            } else {
                CloseCode::Policy
            },
            if paired {
                "pairing complete"
            } else {
                "pairing failed"
            },
        )
        .await;
    }
}

async fn connection_loop(
    mut websocket: WebSocketStream<TcpStream>,
    controller: Arc<dyn Controller>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut states = controller.subscribe();
    let initial = controller.snapshot();
    if send_message(&mut websocket, &ServerMessage::state(&initial))
        .await
        .is_err()
    {
        return;
    }
    let mut last_state_revision = initial.revision;
    let mut commands: FuturesUnordered<CommandFuture> = FuturesUnordered::new();

    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    graceful_close(&mut websocket, CloseCode::Away, "server shutting down").await;
                    return;
                }
            }
            Some((request_id, result)) = commands.next(), if !commands.is_empty() => {
                let message = match result {
                    Ok(outcome) => ServerMessage::success(
                        request_id,
                        Success::from(outcome),
                        &controller.snapshot(),
                    ),
                    Err(error) => ServerMessage::error(Some(request_id), error),
                };
                if send_message(&mut websocket, &message).await.is_err() {
                    return;
                }
            }
            changed = states.changed() => {
                if changed.is_err() {
                    graceful_close(&mut websocket, CloseCode::Error, "state source closed").await;
                    return;
                }
                let snapshot = states.borrow_and_update().clone();
                if snapshot.revision > last_state_revision {
                    last_state_revision = snapshot.revision;
                    if send_message(&mut websocket, &ServerMessage::state(&snapshot)).await.is_err() {
                        return;
                    }
                }
            }
            incoming = websocket.next() => {
                match incoming {
                    Some(Ok(message)) => {
                        if !handle_frame(&mut websocket, &controller, &mut commands, message).await {
                            return;
                        }
                    }
                    Some(Err(WebSocketError::Capacity(_))) => {
                        graceful_close(&mut websocket, CloseCode::Size, "message too large").await;
                        return;
                    }
                    Some(Err(_)) | None => return,
                }
            }
        }
    }
}

async fn handle_frame(
    websocket: &mut WebSocketStream<TcpStream>,
    controller: &Arc<dyn Controller>,
    commands: &mut FuturesUnordered<CommandFuture>,
    message: Message,
) -> bool {
    match data_frame_policy(message.is_text(), message.is_binary()) {
        DataFramePolicy::ParseText => {
            let text = match message.to_text() {
                Ok(text) => text,
                Err(_) => return false,
            };
            match decode_client_message(text) {
                Ok(ClientMessage::GetState { request_id, .. }) => {
                    let response =
                        ServerMessage::success(request_id, Success::State, &controller.snapshot());
                    send_message(websocket, &response).await.is_ok()
                }
                Ok(ClientMessage::Activate {
                    request_id, target, ..
                }) => {
                    if commands.len() >= MAX_IN_FLIGHT_COMMANDS {
                        let response = ServerMessage::error(
                            Some(request_id),
                            ControlError::new(
                                ErrorCode::InvalidArgument,
                                "too many commands are already in flight",
                            ),
                        );
                        return send_message(websocket, &response).await.is_ok();
                    }
                    let target = match crate::control::ClientTarget::try_from(target) {
                        Ok(target) => target,
                        Err(error) => {
                            let response = ServerMessage::error(Some(request_id), error);
                            return send_message(websocket, &response).await.is_ok();
                        }
                    };
                    let future = controller.activate(target);
                    commands.push(Box::pin(async move { (request_id, future.await) }));
                    true
                }
                Ok(ClientMessage::SwapWindowNumbers {
                    request_id, target, ..
                }) => {
                    if commands.len() >= MAX_IN_FLIGHT_COMMANDS {
                        let response = ServerMessage::error(
                            Some(request_id),
                            ControlError::new(
                                ErrorCode::InvalidArgument,
                                "too many commands are already in flight",
                            ),
                        );
                        return send_message(websocket, &response).await.is_ok();
                    }
                    let target = match crate::control::ClientTarget::try_from(target) {
                        Ok(target) => target,
                        Err(error) => {
                            let response = ServerMessage::error(Some(request_id), error);
                            return send_message(websocket, &response).await.is_ok();
                        }
                    };
                    let future = controller.swap_window_numbers(target);
                    commands.push(Box::pin(async move { (request_id, future.await) }));
                    true
                }
                Ok(ClientMessage::SetBroadcast {
                    request_id,
                    enabled,
                    ..
                }) => {
                    if commands.len() >= MAX_IN_FLIGHT_COMMANDS {
                        let response = ServerMessage::error(
                            Some(request_id),
                            ControlError::new(
                                ErrorCode::InvalidArgument,
                                "too many commands are already in flight",
                            ),
                        );
                        return send_message(websocket, &response).await.is_ok();
                    }
                    let future = controller.set_broadcast_enabled(enabled);
                    commands.push(Box::pin(async move { (request_id, future.await) }));
                    true
                }
                Ok(ClientMessage::SendText {
                    request_id,
                    client_id,
                    text,
                    submit,
                    ..
                }) => {
                    if commands.len() >= MAX_IN_FLIGHT_COMMANDS {
                        let response = ServerMessage::error(
                            Some(request_id),
                            ControlError::new(
                                ErrorCode::InvalidArgument,
                                "too many commands are already in flight",
                            ),
                        );
                        return send_message(websocket, &response).await.is_ok();
                    }
                    let client_id = match crate::control::ClientId::new(client_id) {
                        Ok(client_id) => client_id,
                        Err(error) => {
                            let response = ServerMessage::error(Some(request_id), error);
                            return send_message(websocket, &response).await.is_ok();
                        }
                    };
                    let future = controller.send_text(client_id, text, submit);
                    commands.push(Box::pin(async move { (request_id, future.await) }));
                    true
                }
                Ok(ClientMessage::SendKeys {
                    request_id,
                    client_id,
                    strokes,
                    ..
                }) => {
                    if commands.len() >= MAX_IN_FLIGHT_COMMANDS {
                        let response = ServerMessage::error(
                            Some(request_id),
                            ControlError::new(
                                ErrorCode::InvalidArgument,
                                "too many commands are already in flight",
                            ),
                        );
                        return send_message(websocket, &response).await.is_ok();
                    }
                    let client_id = match crate::control::ClientId::new(client_id) {
                        Ok(client_id) => client_id,
                        Err(error) => {
                            let response = ServerMessage::error(Some(request_id), error);
                            return send_message(websocket, &response).await.is_ok();
                        }
                    };
                    let strokes = match strokes
                        .into_iter()
                        .map(crate::control::KeyStroke::try_from)
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(strokes) => strokes,
                        Err(error) => {
                            let response = ServerMessage::error(Some(request_id), error);
                            return send_message(websocket, &response).await.is_ok();
                        }
                    };
                    let future = controller.send_keys(client_id, strokes);
                    commands.push(Box::pin(async move { (request_id, future.await) }));
                    true
                }
                Err(error) => {
                    let response = ServerMessage::error(error.request_id, error.error);
                    send_message(websocket, &response).await.is_ok()
                }
            }
        }
        DataFramePolicy::RejectBinary => {
            let response = ServerMessage::error(
                None,
                ControlError::new(
                    ErrorCode::MalformedRequest,
                    "binary messages are not supported; send UTF-8 JSON text",
                ),
            );
            send_message(websocket, &response).await.is_ok()
        }
        DataFramePolicy::IgnoreControl => {
            if message.is_close() {
                let _ = websocket.flush().await;
                false
            } else if message.is_ping() {
                // Tungstenite queues the matching Pong automatically.
                tokio::time::timeout(WRITE_TIMEOUT, websocket.flush())
                    .await
                    .is_ok_and(|result| result.is_ok())
            } else {
                true
            }
        }
    }
}

async fn send_message(
    websocket: &mut WebSocketStream<TcpStream>,
    message: &ServerMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(message).map_err(|_| ())?;
    tokio::time::timeout(WRITE_TIMEOUT, websocket.send(Message::text(text)))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn graceful_close(
    websocket: &mut WebSocketStream<TcpStream>,
    code: CloseCode,
    reason: &'static str,
) {
    let _ = tokio::time::timeout(
        WRITE_TIMEOUT,
        websocket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
    let _ = tokio::time::timeout(WRITE_TIMEOUT, websocket.next()).await;
}

pub fn is_loopback_address(address: SocketAddr) -> bool {
    address.ip().is_loopback()
}

pub fn is_non_loopback_or_wildcard(ip: IpAddr) -> bool {
    !ip.is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[cfg(windows)]
    #[test]
    fn listener_socket_handle_is_not_inheritable() {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            prevent_listener_socket_inheritance(&listener).unwrap();
            let handle = listener.as_raw_socket() as usize as *mut std::ffi::c_void;
            let mut flags = 0;
            assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
            assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
        });
    }

    #[test]
    fn validates_loopback_ipv4_and_ipv6_without_authentication() {
        for bind in [
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0),
            SocketAddr::new(Ipv6Addr::LOCALHOST.into(), 0),
        ] {
            assert!(ServerConfig::loopback(bind).validate().is_ok());
            assert!(is_loopback_address(bind));
        }
    }

    #[test]
    fn rejects_wildcard_and_non_loopback_without_authentication() {
        for bind in [
            "0.0.0.0:19720".parse().unwrap(),
            "[::]:19720".parse().unwrap(),
            "192.168.1.10:19720".parse().unwrap(),
        ] {
            let error = ServerConfig::loopback(bind).validate().unwrap_err();
            assert!(matches!(error, StartError::InvalidConfiguration(_)));
            assert!(is_non_loopback_or_wildcard(bind.ip()));
        }
    }

    #[test]
    fn permits_explicit_lan_binding_with_nonempty_authentication() {
        let config = ServerConfig {
            bind: "0.0.0.0:19720".parse().unwrap(),
            auth_token: Some("secret-placeholder".into()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn token_comparison_is_exact() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn pairing_codes_are_six_digit_single_use_and_attempt_limited() {
        let pairing = PairingHandle::default();
        assert!(pairing.begin(4_271, "long-secret".into()));
        assert!(pairing.is_open());
        let first_session = pairing.session_id().unwrap();
        for _ in 0..4 {
            assert!(matches!(
                pairing.attempt(first_session, "999999"),
                PairingAttempt::Failed
            ));
        }
        assert!(matches!(
            pairing.attempt(first_session, "004271"),
            PairingAttempt::Paired(ref token) if token == "long-secret"
        ));
        assert!(!pairing.is_open());
        assert!(matches!(
            pairing.attempt(first_session, "004271"),
            PairingAttempt::Failed
        ));

        assert!(pairing.begin(111_111, "old-secret".into()));
        let replaced_session = pairing.session_id().unwrap();
        assert!(pairing.begin(123_456, "replacement-secret".into()));
        let second_session = pairing.session_id().unwrap();
        assert_ne!(replaced_session, second_session);
        assert!(matches!(
            pairing.attempt(replaced_session, "123456"),
            PairingAttempt::Failed
        ));
        assert!(pairing.is_open());
        for _ in 0..MAX_PAIRING_ATTEMPTS {
            assert!(matches!(
                pairing.attempt(second_session, "000000"),
                PairingAttempt::Failed
            ));
        }
        assert!(!pairing.is_open());
    }

    #[test]
    fn expired_pairing_codes_close_the_gate() {
        let pairing = PairingHandle::default();
        pairing.begin_until(
            123_456,
            "secret".into(),
            Instant::now() - Duration::from_secs(1),
        );
        assert!(!pairing.is_open());
        assert!(matches!(
            pairing.attempt(1, "123456"),
            PairingAttempt::Failed
        ));
    }
}
