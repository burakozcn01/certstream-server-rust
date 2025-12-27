mod config;
mod ct;
mod hot_reload;
mod middleware;
mod models;
mod sse;
mod state;
mod tcp;
mod websocket;

use axum::{middleware as axum_middleware, routing::get, Json, Router};
use metrics_exporter_prometheus::PrometheusBuilder;
use reqwest::Client;
use smallvec::smallvec;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use config::Config;
use ct::{fetch_log_list, run_watcher};
use hot_reload::{ConfigWatcher, HotReloadableConfig};
use middleware::{auth_middleware, AuthMiddleware, ConnectionLimiter};
use models::{CertificateData, CertificateMessage, ChainCert, Extensions, LeafCert, PreSerializedMessage, Source, Subject};
use sse::handle_sse_stream;
use state::StateManager;
use tcp::run_tcp_server;
use websocket::{handle_domains_only, handle_full_stream, handle_lite_stream, AppState, ConnectionCounter};

async fn health() -> &'static str {
    "OK"
}

async fn example_json() -> Json<CertificateMessage> {
    let mut subject = Subject {
        cn: Some("example.com".to_string()),
        o: Some("Example Organization".to_string()),
        c: Some("US".to_string()),
        ..Default::default()
    };
    subject.build_aggregated();

    let mut issuer = Subject {
        cn: Some("Example CA".to_string()),
        o: Some("Example Certificate Authority".to_string()),
        c: Some("US".to_string()),
        ..Default::default()
    };
    issuer.build_aggregated();

    let mut chain_issuer = Subject {
        cn: Some("Root CA".to_string()),
        o: Some("Example Root Authority".to_string()),
        ..Default::default()
    };
    chain_issuer.build_aggregated();

    let extensions = Extensions {
        key_usage: Some("Digital Signature, Key Encipherment".to_string()),
        extended_key_usage: Some("serverAuth, clientAuth".to_string()),
        basic_constraints: Some("CA:FALSE".to_string()),
        subject_alt_name: Some("DNS:example.com, DNS:www.example.com, DNS:*.example.com".to_string()),
        ..Default::default()
    };

    let chain_extensions = Extensions {
        key_usage: Some("Certificate Signing, CRL Signing".to_string()),
        basic_constraints: Some("CA:TRUE".to_string()),
        ..Default::default()
    };

    let example = CertificateMessage {
        message_type: Cow::Borrowed("certificate_update"),
        data: CertificateData {
            update_type: Cow::Borrowed("X509LogEntry"),
            leaf_cert: LeafCert {
                subject: subject.clone(),
                issuer: issuer.clone(),
                serial_number: "0123456789ABCDEF".to_string(),
                not_before: 1704067200,
                not_after: 1735689600,
                fingerprint: "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01".to_string(),
                sha1: "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01".to_string(),
                sha256: "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89".to_string(),
                signature_algorithm: "sha256, rsa".to_string(),
                is_ca: false,
                all_domains: smallvec![
                    "example.com".to_string(),
                    "www.example.com".to_string(),
                    "*.example.com".to_string(),
                ],
                as_der: Some("BASE64_ENCODED_DER_DATA".to_string()),
                extensions,
            },
            chain: Some(vec![ChainCert {
                subject: issuer,
                issuer: chain_issuer,
                serial_number: "00112233445566".to_string(),
                not_before: 1672531200,
                not_after: 1767225600,
                fingerprint: "11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44".to_string(),
                sha1: "11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44".to_string(),
                sha256: "11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00".to_string(),
                signature_algorithm: "sha256, rsa".to_string(),
                is_ca: true,
                as_der: Some("BASE64_ENCODED_CA_DER".to_string()),
                extensions: chain_extensions,
            }]),
            cert_index: 123456789,
            seen: 1704067200.123,
            source: Arc::new(Source {
                name: Arc::from("Google 'Argon2024' log"),
                url: Arc::from("https://ct.googleapis.com/logs/argon2024"),
                operator: Arc::from("Google"),
            }),
        },
    };

    Json(example)
}

#[tokio::main]
async fn main() {
    let config = Config::load();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    info!("starting certstream-server-rust v1.0.5");

    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install prometheus recorder");

    let (tx, _rx) = broadcast::channel::<Arc<PreSerializedMessage>>(config.buffer_size);

    let client = Client::builder()
        .user_agent("certstream-server-rust/1.0.5")
        .pool_max_idle_per_host(20)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .build()
        .expect("failed to build http client");

    let state_manager = StateManager::new(config.ct_log.state_file.clone());
    if config.ct_log.state_file.is_some() {
        state_manager.clone().start_periodic_save(Duration::from_secs(30));
        info!("state persistence enabled");
    }

    if config.hot_reload.enabled {
        let initial_hot_config = HotReloadableConfig {
            connection_limit: config.connection_limit.clone(),
            auth: config.auth.clone(),
        };
        let config_watcher = ConfigWatcher::new(initial_hot_config);
        let watch_path = config.hot_reload.watch_path.clone().or(config.config_path.clone());
        config_watcher.start_watching(watch_path);
        info!("hot reload enabled");
    }

    let ct_log_config = Arc::new(config.ct_log.clone());

    info!(url = %config.ct_logs_url, "fetching CT log list");

    let custom_logs_count = config.custom_logs.len();
    if custom_logs_count > 0 {
        info!(count = custom_logs_count, "adding custom CT logs");
    }

    let host = config.host;
    let port = config.port;
    let has_tls = config.has_tls();
    let tls_cert = config.tls_cert.clone();
    let tls_key = config.tls_key.clone();
    let protocols = config.protocols.clone();

    match fetch_log_list(&client, &config.ct_logs_url, config.custom_logs.clone()).await {
        Ok(logs) => {
            info!(count = logs.len(), "found CT logs");
            metrics::gauge!("certstream_ct_logs_count").set(logs.len() as f64);

            for log in logs {
                let client = client.clone();
                let tx = tx.clone();
                let ct_config = ct_log_config.clone();
                let state_mgr = state_manager.clone();
                tokio::spawn(async move {
                    run_watcher(client, log, tx, ct_config, state_mgr).await;
                });
            }
        }
        Err(e) => {
            error!(error = %e, "failed to fetch CT log list");
            std::process::exit(1);
        }
    }

    let connection_limiter = ConnectionLimiter::new(config.connection_limit.clone());

    if protocols.tcp {
        let tcp_port = protocols.tcp_port.unwrap_or(port + 1);
        let tcp_addr = SocketAddr::from((host, tcp_port));
        let tcp_tx = tx.clone();
        let tcp_limiter = connection_limiter.clone();
        tokio::spawn(async move {
            run_tcp_server(tcp_addr, tcp_tx, tcp_limiter).await;
        });
        info!(port = tcp_port, "TCP protocol enabled");
    }

    let state = Arc::new(AppState {
        tx: tx.clone(),
        connections: ConnectionCounter::new(),
        limiter: connection_limiter.clone(),
    });
    let auth_middleware_state = Arc::new(AuthMiddleware::new(&config.auth));

    let mut app = Router::new();

    if protocols.health {
        app = app.route("/health", get(health));
    }

    if protocols.example_json {
        app = app.route("/example.json", get(example_json));
    }

    if protocols.metrics {
        app = app.route("/metrics", get(move || async move { prometheus_handle.render() }));
    }

    if protocols.websocket {
        app = app
            .route("/", get(handle_lite_stream))
            .route("/full-stream", get(handle_full_stream))
            .route("/domains-only", get(handle_domains_only));
        info!("WebSocket protocol enabled");
    }

    if protocols.sse {
        app = app.route("/sse", get(handle_sse_stream));
        info!("SSE protocol enabled");
    }

    let app = app.with_state(state);

    let app = if config.auth.enabled {
        info!("token authentication enabled");
        app.layer(axum_middleware::from_fn_with_state(
            auth_middleware_state,
            auth_middleware,
        ))
    } else {
        app
    };

    let app = app.layer(CorsLayer::permissive());

    if config.connection_limit.enabled {
        info!(
            max_connections = config.connection_limit.max_connections,
            per_ip_limit = ?config.connection_limit.per_ip_limit,
            "connection limiting enabled"
        );
    }

    let addr = SocketAddr::from((host, port));
    info!(address = %addr, "starting server");

    if has_tls {
        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            tls_cert.as_ref().unwrap(),
            tls_key.as_ref().unwrap(),
        )
        .await
        .expect("failed to load TLS config");

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .expect("server error");
    } else {
        let listener = tokio::net::TcpListener::bind(addr).await.expect("failed to bind");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("server error");
    }
}
