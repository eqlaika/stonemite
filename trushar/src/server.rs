use crate::control::{ControlError, Controller, ErrorCode};
use crate::protocol::{
    data_frame_policy, decode_client_message, ClientMessage, DataFramePolicy, ServerMessage,
    Success, MAX_TEXT_MESSAGE_SIZE,
};
use futures_util::stream::FuturesUnordered;
use futures_util::{SinkExt, StreamExt};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{mpsc as std_mpsc, Arc};
use std::thread;
use std::time::Duration;
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
const START_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECTION_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_IN_FLIGHT_COMMANDS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
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
        let thread = thread::Builder::new()
            .name("trushar-server".into())
            .spawn(move || server_thread(config, controller, shutdown_rx, startup_tx))
            .map_err(|error| StartError::Runtime(error.to_string()))?;

        match startup_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(local_addr)) => Ok(Self {
                local_addr,
                shutdown,
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
    runtime.block_on(run_server(config, controller, shutdown, startup));
}

async fn run_server(
    config: ServerConfig,
    controller: Arc<dyn Controller>,
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
                        let config = config.clone();
                        let controller = controller.clone();
                        let shutdown = shutdown.clone();
                        connections.spawn(async move {
                            serve_connection(stream, peer, config, controller, shutdown).await;
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
    shutdown: watch::Receiver<bool>,
) {
    let handshake_config = config.clone();
    let callback = move |request: &Request, response: Response| {
        authorize_upgrade(request, response, &handshake_config)
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
    connection_loop(websocket, controller, shutdown).await;
}

#[allow(clippy::result_large_err)] // Signature is fixed by Tungstenite's Callback trait.
fn authorize_upgrade(
    request: &Request,
    response: Response,
    config: &ServerConfig,
) -> Result<Response, ErrorResponse> {
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
}
