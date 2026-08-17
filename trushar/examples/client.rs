//! Minimal generic interactive client for hardware-free manual validation.

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://127.0.0.1:19720/trushar/v1".to_owned());
    let mut request = url.into_client_request()?;
    if let Ok(token) = std::env::var("TRUSHAR_TOKEN") {
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    }

    let (mut websocket, _) = tokio_tungstenite::connect_async(request).await?;
    eprintln!("connected; enter one JSON request per line (Ctrl-C to stop)");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            line = lines.next_line(), if stdin_open => {
                match line? {
                    Some(line) if !line.trim().is_empty() => {
                        websocket.send(Message::text(line)).await?;
                    }
                    Some(_) => {}
                    None => stdin_open = false,
                }
            }
            frame = websocket.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => println!("{text}"),
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        eprintln!("server closed connection: {frame:?}");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                }
            }
        }
    }
    Ok(())
}
