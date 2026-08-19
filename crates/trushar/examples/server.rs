//! Disposable generic server for protocol/LAN validation without EQ or UI hardware.

use std::sync::Arc;
use std::time::Duration;
use trushar::control::{BroadcastState, InMemoryController};
use trushar::server::{ServerConfig, ServerHandle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:19720".to_owned())
        .parse()?;
    let lifetime_seconds: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "300".to_owned())
        .parse()?;
    let token = std::env::var("TRUSHAR_TOKEN").ok();
    let controller = InMemoryController::new(BroadcastState {
        available: true,
        enabled: false,
    });
    controller.add_client(
        1,
        Some("Example"),
        Some("TestServer"),
        Some("TST"),
        true,
        true,
    );
    let server = ServerHandle::start(
        ServerConfig {
            bind,
            auth_token: token,
        },
        Arc::new(controller),
    )?;
    println!(
        "trushar validation server listening on {}",
        server.local_addr()
    );
    std::thread::sleep(Duration::from_secs(lifetime_seconds));
    server.shutdown();
    Ok(())
}
