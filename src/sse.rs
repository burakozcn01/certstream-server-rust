use axum::{
    extract::{ConnectInfo, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures_util::StreamExt;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tracing::info;

use crate::middleware::ConnectionLimiter;
use crate::models::PreSerializedMessage;

static SSE_CONNECTION_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize, Default)]
pub struct SseQueryParams {
    #[serde(default)]
    pub stream: Option<String>,
}

pub async fn handle_sse_stream(
    Query(params): Query<SseQueryParams>,
    State(state): State<Arc<crate::websocket::AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let ip = addr.ip();

    if !state.limiter.try_acquire(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, "Connection limit exceeded").into_response();
    }

    let rx = state.tx.subscribe();
    let stream_type = params.stream.as_deref().unwrap_or("lite").to_string();

    SSE_CONNECTION_COUNT.fetch_add(1, Ordering::Relaxed);
    update_sse_metrics();

    info!(
        stream = stream_type.as_str(),
        total = SSE_CONNECTION_COUNT.load(Ordering::Relaxed),
        ip = %ip,
        "SSE client connected"
    );

    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        let stream_type = stream_type.clone();
        std::future::ready(match result {
            Ok(msg) => process_message(msg, &stream_type),
            Err(_) => None,
        })
    });

    let stream = SseStreamWrapper {
        inner: Box::pin(stream),
        limiter: state.limiter.clone(),
        client_ip: ip,
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ).into_response()
}

fn process_message(
    msg: Arc<PreSerializedMessage>,
    stream_type: &str,
) -> Option<Result<Event, std::convert::Infallible>> {
    let bytes = match stream_type {
        "full" => &msg.full,
        "domains" | "domains-only" => &msg.domains_only,
        _ => &msg.lite,
    };

    std::str::from_utf8(bytes)
        .ok()
        .map(|json_str| Ok(Event::default().data(json_str.to_owned())))
}

struct SseStreamWrapper<S> {
    inner: std::pin::Pin<Box<S>>,
    limiter: Arc<ConnectionLimiter>,
    client_ip: IpAddr,
}

impl<S> Drop for SseStreamWrapper<S> {
    fn drop(&mut self) {
        self.limiter.release(self.client_ip);
        SSE_CONNECTION_COUNT.fetch_sub(1, Ordering::Relaxed);
        update_sse_metrics();
        info!(
            total = SSE_CONNECTION_COUNT.load(Ordering::Relaxed),
            ip = %self.client_ip,
            "SSE client disconnected"
        );
    }
}

impl<S> futures_util::Stream for SseStreamWrapper<S>
where
    S: futures_util::Stream<Item = Result<Event, std::convert::Infallible>>,
{
    type Item = Result<Event, std::convert::Infallible>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

fn update_sse_metrics() {
    metrics::gauge!("certstream_sse_connections")
        .set(SSE_CONNECTION_COUNT.load(Ordering::Relaxed) as f64);
}
