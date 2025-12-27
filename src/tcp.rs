use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::models::PreSerializedMessage;

static TCP_CONNECTION_COUNT: AtomicU64 = AtomicU64::new(0);
static NEWLINE: &[u8] = b"\n";

pub async fn run_tcp_server(
    addr: SocketAddr,
    tx: broadcast::Sender<Arc<PreSerializedMessage>>,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, "failed to bind TCP server");
            return;
        }
    };

    info!(address = %addr, "TCP server started");

    loop {
        match listener.accept().await {
            Ok((socket, peer_addr)) => {
                let rx = tx.subscribe();
                tokio::spawn(async move {
                    handle_tcp_client(socket, rx, peer_addr).await;
                });
            }
            Err(e) => {
                warn!(error = %e, "failed to accept TCP connection");
            }
        }
    }
}

async fn handle_tcp_client(
    mut socket: TcpStream,
    mut rx: broadcast::Receiver<Arc<PreSerializedMessage>>,
    peer_addr: SocketAddr,
) {
    TCP_CONNECTION_COUNT.fetch_add(1, Ordering::Relaxed);
    update_tcp_metrics();

    info!(
        peer = %peer_addr,
        total = TCP_CONNECTION_COUNT.load(Ordering::Relaxed),
        "TCP client connected"
    );

    let mut first_byte = [0u8; 1];
    let stream_type = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::io::AsyncReadExt::read(&mut socket, &mut first_byte),
    )
    .await
    {
        Ok(Ok(1)) => match first_byte[0] {
            b'f' | b'F' => StreamType::Full,
            b'd' | b'D' => StreamType::DomainsOnly,
            _ => StreamType::Lite,
        },
        _ => StreamType::Lite,
    };

    loop {
        match rx.recv().await {
            Ok(msg) => {
                let bytes = match stream_type {
                    StreamType::Full => &msg.full,
                    StreamType::Lite => &msg.lite,
                    StreamType::DomainsOnly => &msg.domains_only,
                };

                if socket.write_all(bytes).await.is_err() {
                    break;
                }
                if socket.write_all(NEWLINE).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!(peer = %peer_addr, lagged = n, "TCP client lagged");
                metrics::counter!("certstream_tcp_messages_lagged").increment(n);
            }
            Err(broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }

    TCP_CONNECTION_COUNT.fetch_sub(1, Ordering::Relaxed);
    update_tcp_metrics();

    info!(
        peer = %peer_addr,
        total = TCP_CONNECTION_COUNT.load(Ordering::Relaxed),
        "TCP client disconnected"
    );
}

#[derive(Clone, Copy)]
enum StreamType {
    Full,
    Lite,
    DomainsOnly,
}

fn update_tcp_metrics() {
    metrics::gauge!("certstream_tcp_connections").set(TCP_CONNECTION_COUNT.load(Ordering::Relaxed) as f64);
}

#[allow(dead_code)]
pub fn tcp_connection_count() -> u64 {
    TCP_CONNECTION_COUNT.load(Ordering::Relaxed)
}
