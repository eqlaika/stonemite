use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::{client_async, WebSocketStream};
use trushar::control::{BroadcastState, InMemoryController, RecordedInput};
use trushar::protocol::{
    ClientMessage, PairingRequest, ServerMessage, Success, Target, WireInputKind, WireKeyStroke,
};
use trushar::server::{ServerConfig, ServerHandle, ENDPOINT_PATH, PAIRING_ENDPOINT_PATH};

const IO_TIMEOUT: Duration = Duration::from_secs(3);
type Client = WebSocketStream<TcpStream>;

async fn connect(
    address: SocketAddr,
    token: Option<&str>,
    origin: Option<&str>,
) -> Result<Client, WebSocketError> {
    connect_path(address, ENDPOINT_PATH, token, origin).await
}

async fn connect_path(
    address: SocketAddr,
    path: &str,
    token: Option<&str>,
    origin: Option<&str>,
) -> Result<Client, WebSocketError> {
    let stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
    }
    client_async(request, stream)
        .await
        .map(|(socket, _)| socket)
}

async fn receive(client: &mut Client) -> ServerMessage {
    let message = tokio::time::timeout(IO_TIMEOUT, client.next())
        .await
        .expect("timed out waiting for WebSocket frame")
        .expect("connection ended")
        .expect("WebSocket error");
    let text = message.into_text().expect("expected text frame");
    serde_json::from_str(&text).expect("invalid server JSON")
}

async fn send(client: &mut Client, message: ClientMessage) {
    let text = serde_json::to_string(&message).unwrap();
    tokio::time::timeout(IO_TIMEOUT, client.send(Message::text(text)))
        .await
        .expect("timed out sending")
        .expect("send failed");
}

async fn receive_result(client: &mut Client, expected_id: &str) -> ServerMessage {
    for _ in 0..8 {
        let message = receive(client).await;
        if matches!(&message, ServerMessage::Result { request_id, .. } if request_id == expected_id)
            || matches!(&message, ServerMessage::Error { request_id: Some(request_id), .. } if request_id == expected_id)
        {
            return message;
        }
    }
    panic!("did not receive response for {expected_id}");
}

async fn receive_state_where(
    client: &mut Client,
    predicate: impl Fn(&trushar::protocol::WireState) -> bool,
) -> trushar::protocol::WireState {
    for _ in 0..12 {
        if let ServerMessage::State { state, .. } = receive(client).await {
            if predicate(&state) {
                return state;
            }
        }
    }
    panic!("did not receive expected state update");
}

#[tokio::test(flavor = "current_thread")]
async fn real_network_listener_upgrade_commands_fanout_reconnect_and_shutdown() {
    let control = InMemoryController::new(BroadcastState {
        available: true,
        enabled: false,
    });
    let first = control.add_client(1, Some("Laika"), Some("Xegony"), Some("SHK"), true, true);
    let second = control.add_client(2, None, None, None, false, true);
    let server = ServerHandle::start(
        ServerConfig::loopback("127.0.0.1:0".parse().unwrap()),
        Arc::new(control.clone()),
    )
    .unwrap();
    let address = server.local_addr();

    let mut one = connect(address, None, None).await.unwrap();
    let initial = receive(&mut one).await;
    assert!(matches!(
        initial,
        ServerMessage::State { ref state, .. }
            if state.clients.len() == 2 && state.active_client_id.as_deref() == Some(first.as_str())
    ));

    send(
        &mut one,
        ClientMessage::GetState {
            version: 1,
            request_id: "state-1".into(),
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "state-1").await,
        ServerMessage::Result {
            result: Success::State,
            ..
        }
    ));

    send(
        &mut one,
        ClientMessage::Activate {
            version: 1,
            request_id: "activate-1".into(),
            target: Target::ClientId {
                client_id: second.as_str().into(),
            },
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "activate-1").await,
        ServerMessage::Result { result: Success::Activated { .. }, ref state, .. }
            if state.active_client_id.as_deref() == Some(second.as_str())
    ));

    send(
        &mut one,
        ClientMessage::SwapWindowNumbers {
            version: 1,
            request_id: "swap-numbers-1".into(),
            target: Target::ClientId {
                client_id: first.as_str().into(),
            },
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "swap-numbers-1").await,
        ServerMessage::Result {
            result: Success::WindowNumbersSwapped {
                active_previous_number: 2,
                selected_previous_number: 1,
            },
            ref state,
            ..
        } if state.active_client_id.as_deref() == Some(second.as_str())
            && state.clients.iter().any(|client| client.id == second.as_str() && client.window_number == 1)
    ));

    send(
        &mut one,
        ClientMessage::SetBroadcast {
            version: 1,
            request_id: "broadcast-1".into(),
            enabled: true,
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "broadcast-1").await,
        ServerMessage::Result {
            result: Success::BroadcastSet { enabled: true },
            ..
        }
    ));

    send(
        &mut one,
        ClientMessage::SendText {
            version: 1,
            request_id: "text-1".into(),
            client_id: second.as_str().into(),
            text: "/who".into(),
            submit: true,
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "text-1").await,
        ServerMessage::Result {
            result: Success::InputDelivered {
                input: WireInputKind::Text,
                strokes: 5,
            },
            ..
        }
    ));

    send(
        &mut one,
        ClientMessage::SendKeys {
            version: 1,
            request_id: "keys-1".into(),
            client_id: second.as_str().into(),
            strokes: vec![WireKeyStroke {
                keys: vec!["left_control".into(), "1".into()],
                hold_ms: 40,
                pause_ms: 20,
            }],
        },
    )
    .await;
    assert!(matches!(
        receive_result(&mut one, "keys-1").await,
        ServerMessage::Result {
            result: Success::InputDelivered {
                input: WireInputKind::Keys,
                strokes: 1,
            },
            ..
        }
    ));
    assert!(matches!(
        control.recorded_inputs().as_slice(),
        [RecordedInput::Text { client_id, .. }, RecordedInput::Keys { .. }]
            if client_id == &second
    ));

    let mut two = connect(address, None, None).await.unwrap();
    assert!(matches!(
        receive(&mut two).await,
        ServerMessage::State { ref state, .. }
            if state.active_client_id.as_deref() == Some(second.as_str()) && state.broadcast.enabled
    ));

    let third = control.add_client(3, Some("Third"), None, None, false, true);
    for client in [&mut one, &mut two] {
        receive_state_where(client, |state| {
            state
                .clients
                .iter()
                .any(|candidate| candidate.id == third.as_str())
        })
        .await;
    }

    drop(one);
    control.remove_client(&first);
    receive_state_where(&mut two, |state| {
        !state
            .clients
            .iter()
            .any(|candidate| candidate.id == first.as_str())
    })
    .await;

    let mut reconnected = connect(address, None, None).await.unwrap();
    assert!(matches!(
        receive(&mut reconnected).await,
        ServerMessage::State { ref state, .. }
            if state.clients.len() == 2 && state.broadcast.enabled
    ));
    drop(two);
    drop(reconnected);

    let port = address.port();
    server.shutdown();
    let rebound = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    drop(rebound);
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_binary_and_disconnected_slow_client_do_not_affect_healthy_client() {
    let control = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let server = ServerHandle::start(
        ServerConfig::loopback("127.0.0.1:0".parse().unwrap()),
        Arc::new(control.clone()),
    )
    .unwrap();
    let address = server.local_addr();
    let mut healthy = connect(address, None, None).await.unwrap();
    let _ = receive(&mut healthy).await;
    let slow = connect(address, None, None).await.unwrap();

    healthy.send(Message::text("{".to_owned())).await.unwrap();
    assert!(matches!(
        receive(&mut healthy).await,
        ServerMessage::Error { .. }
    ));
    healthy.send(Message::binary(vec![1, 2, 3])).await.unwrap();
    assert!(matches!(
        receive(&mut healthy).await,
        ServerMessage::Error { .. }
    ));

    drop(slow);
    let id = control.add_client(1, None, None, None, true, true);
    assert!(matches!(
        receive(&mut healthy).await,
        ServerMessage::State { ref state, .. }
            if state.clients.iter().any(|client| client.id == id.as_str())
    ));
    drop(healthy);
    server.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn authentication_origin_and_secret_nonreflection_are_enforced() {
    let token = "test-token-that-must-not-be-reflected";
    let control = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let server = ServerHandle::start(
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            auth_token: Some(token.into()),
        },
        Arc::new(control),
    )
    .unwrap();
    let address = server.local_addr();

    for supplied in [None, Some("incorrect")] {
        let error = connect(address, supplied, None).await.unwrap_err();
        match error {
            WebSocketError::Http(response) => {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                let body = String::from_utf8_lossy(response.body().as_deref().unwrap_or_default());
                assert!(body.contains(r#""code":"unauthorized""#));
                assert!(!body.contains(token));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
    let mut authenticated = connect(address, Some(token), Some("https://example.invalid"))
        .await
        .unwrap();
    let initial = receive(&mut authenticated).await;
    assert!(matches!(initial, ServerMessage::State { .. }));
    assert!(!serde_json::to_string(&initial).unwrap().contains(token));
    drop(authenticated);
    server.shutdown();

    let unauthenticated = ServerHandle::start(
        ServerConfig::loopback("127.0.0.1:0".parse().unwrap()),
        Arc::new(InMemoryController::new(BroadcastState::UNAVAILABLE)),
    )
    .unwrap();
    let error = connect(
        unauthenticated.local_addr(),
        None,
        Some("https://attacker.invalid"),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        WebSocketError::Http(ref response) if response.status() == StatusCode::FORBIDDEN
    ));
    unauthenticated.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn six_digit_pairing_exchanges_a_single_use_code_for_the_long_token() {
    let token = "long-random-token-stored-by-ikkinz";
    let server = ServerHandle::start(
        ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            auth_token: Some(token.into()),
        },
        Arc::new(InMemoryController::new(BroadcastState::UNAVAILABLE)),
    )
    .unwrap();
    let pairing_handle = server.pairing_handle();
    assert!(pairing_handle.begin(111_111, token.into()));
    let address = server.local_addr();
    let mut stale_pairing = connect_path(address, PAIRING_ENDPOINT_PATH, None, None)
        .await
        .unwrap();
    assert!(pairing_handle.begin(482_731, token.into()));
    stale_pairing
        .send(Message::text(
            serde_json::to_string(&PairingRequest::Pair {
                version: 1,
                code: "482731".into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        receive(&mut stale_pairing).await,
        ServerMessage::Error { .. }
    ));
    assert!(pairing_handle.is_open());
    drop(stale_pairing);

    let browser = connect_path(
        address,
        PAIRING_ENDPOINT_PATH,
        None,
        Some("https://attacker.invalid"),
    )
    .await;
    assert!(matches!(
        browser,
        Err(WebSocketError::Http(ref response)) if response.status() == StatusCode::FORBIDDEN
    ));

    let mut pairing = connect_path(address, PAIRING_ENDPOINT_PATH, None, None)
        .await
        .unwrap();
    pairing
        .send(Message::text(
            serde_json::to_string(&PairingRequest::Pair {
                version: 1,
                code: "482731".into(),
            })
            .unwrap(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        receive(&mut pairing).await,
        ServerMessage::Paired { auth_token, .. } if auth_token == token
    ));
    drop(pairing);

    let second = connect_path(address, PAIRING_ENDPOINT_PATH, None, None).await;
    assert!(matches!(
        second,
        Err(WebSocketError::Http(ref response)) if response.status() == StatusCode::FORBIDDEN
    ));
    let mut authenticated = connect(address, Some(token), None).await.unwrap();
    assert!(matches!(
        receive(&mut authenticated).await,
        ServerMessage::State { .. }
    ));
    drop(authenticated);
    server.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn oversized_input_closes_only_that_connection_with_size_policy() {
    let control = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let server = ServerHandle::start(
        ServerConfig::loopback("127.0.0.1:0".parse().unwrap()),
        Arc::new(control),
    )
    .unwrap();
    let address = server.local_addr();
    let mut oversized = connect(address, None, None).await.unwrap();
    let _ = receive(&mut oversized).await;
    oversized
        .send(Message::text("x".repeat(16 * 1024 + 1)))
        .await
        .unwrap();
    let closed = tokio::time::timeout(IO_TIMEOUT, oversized.next())
        .await
        .expect("oversized client was not disconnected");
    assert!(matches!(
        closed,
        Some(Ok(Message::Close(_))) | Some(Err(_)) | None
    ));

    let mut healthy = connect(address, None, None).await.unwrap();
    assert!(matches!(
        receive(&mut healthy).await,
        ServerMessage::State { .. }
    ));
    drop(healthy);
    server.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn open_connection_receives_close_and_server_thread_joins() {
    let server = ServerHandle::start(
        ServerConfig::loopback("127.0.0.1:0".parse().unwrap()),
        Arc::new(InMemoryController::new(BroadcastState::UNAVAILABLE)),
    )
    .unwrap();
    let mut client = connect(server.local_addr(), None, None).await.unwrap();
    let _ = receive(&mut client).await;
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        server.shutdown();
        let _ = done_tx.send(());
    });

    let frame = tokio::time::timeout(IO_TIMEOUT, client.next())
        .await
        .expect("server did not initiate close")
        .expect("connection ended without close")
        .expect("WebSocket close read failed");
    assert!(matches!(frame, Message::Close(_)));
    let _ = client.flush().await;
    done_rx
        .recv_timeout(IO_TIMEOUT)
        .expect("server thread did not join after close");
}
