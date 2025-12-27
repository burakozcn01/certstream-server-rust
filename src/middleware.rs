use axum::{
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use dashmap::DashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::config::{AuthConfig, ConnectionLimitConfig};

pub struct ConnectionLimiter {
    config: ConnectionLimitConfig,
    total_connections: AtomicU32,
    per_ip_connections: DashMap<IpAddr, u32>,
}

impl ConnectionLimiter {
    pub fn new(config: ConnectionLimitConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            total_connections: AtomicU32::new(0),
            per_ip_connections: DashMap::new(),
        })
    }

    pub fn try_acquire(&self, ip: IpAddr) -> bool {
        if !self.config.enabled {
            return true;
        }

        loop {
            let current_total = self.total_connections.load(Ordering::SeqCst);
            if current_total >= self.config.max_connections {
                metrics::counter!("certstream_connection_limit_rejected").increment(1);
                return false;
            }

            if self
                .total_connections
                .compare_exchange(current_total, current_total + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }

        if let Some(per_ip_limit) = self.config.per_ip_limit {
            let mut should_release = false;
            {
                let mut entry = self.per_ip_connections.entry(ip).or_insert(0);
                if *entry >= per_ip_limit {
                    should_release = true;
                } else {
                    *entry += 1;
                }
            }
            if should_release {
                self.total_connections.fetch_sub(1, Ordering::SeqCst);
                metrics::counter!("certstream_per_ip_limit_rejected").increment(1);
                return false;
            }
        } else {
            self.per_ip_connections
                .entry(ip)
                .and_modify(|v| *v += 1)
                .or_insert(1);
        }

        true
    }

    pub fn release(&self, ip: IpAddr) {
        if !self.config.enabled {
            return;
        }

        self.total_connections.fetch_sub(1, Ordering::SeqCst);

        if let Some(mut entry) = self.per_ip_connections.get_mut(&ip) {
            *entry = entry.saturating_sub(1);
            if *entry == 0 {
                drop(entry);
                self.per_ip_connections.remove(&ip);
            }
        }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware {
    enabled: bool,
    tokens: Vec<Vec<u8>>,
    header_name: String,
}

impl AuthMiddleware {
    pub fn new(config: &AuthConfig) -> Self {
        Self {
            enabled: config.enabled,
            tokens: config.tokens.iter().map(|t| t.as_bytes().to_vec()).collect(),
            header_name: config.header_name.clone(),
        }
    }

    pub fn validate(&self, token: Option<&str>) -> bool {
        if !self.enabled {
            return true;
        }

        match token {
            Some(t) => {
                let token_value = t.strip_prefix("Bearer ").unwrap_or(t);
                let token_bytes = token_value.as_bytes();
                self.tokens.iter().any(|stored| {
                    stored.len() == token_bytes.len()
                        && stored.ct_eq(token_bytes).into()
                })
            }
            None => false,
        }
    }
}

pub async fn auth_middleware(
    State(auth): State<Arc<AuthMiddleware>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !auth.enabled {
        return next.run(request).await;
    }

    let token = request
        .headers()
        .get(&auth.header_name)
        .and_then(|v| v.to_str().ok());

    if auth.validate(token) {
        next.run(request).await
    } else {
        metrics::counter!("certstream_auth_rejected").increment(1);
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

pub async fn connection_limit_middleware(
    State(limiter): State<Arc<ConnectionLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !limiter.try_acquire(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Connection limit exceeded",
        )
            .into_response();
    }

    let response = next.run(request).await;

    limiter.release(addr.ip());

    response
}

