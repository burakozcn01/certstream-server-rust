use serde::Deserialize;
use std::env;
use std::fs;
use std::net::IpAddr;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct CustomCtLog {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtocolConfig {
    #[serde(default = "default_true")]
    pub websocket: bool,
    #[serde(default)]
    pub sse: bool,
    #[serde(default)]
    pub tcp: bool,
    #[serde(default)]
    pub tcp_port: Option<u16>,
    #[serde(default = "default_true")]
    pub metrics: bool,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            websocket: true,
            sse: false,
            tcp: false,
            tcp_port: None,
            metrics: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CtLogConfig {
    #[serde(default = "default_retry_max_attempts")]
    pub retry_max_attempts: u32,
    #[serde(default = "default_retry_initial_delay_ms")]
    pub retry_initial_delay_ms: u64,
    #[serde(default = "default_retry_max_delay_ms")]
    pub retry_max_delay_ms: u64,
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_healthy_threshold")]
    pub healthy_threshold: u32,
    #[serde(default = "default_unhealthy_threshold")]
    pub unhealthy_threshold: u32,
    #[serde(default = "default_health_check_interval_secs")]
    pub health_check_interval_secs: u64,
    #[serde(default)]
    pub state_file: Option<String>,
    #[serde(default = "default_batch_size")]
    pub batch_size: u64,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl Default for CtLogConfig {
    fn default() -> Self {
        Self {
            retry_max_attempts: default_retry_max_attempts(),
            retry_initial_delay_ms: default_retry_initial_delay_ms(),
            retry_max_delay_ms: default_retry_max_delay_ms(),
            request_timeout_secs: default_request_timeout_secs(),
            healthy_threshold: default_healthy_threshold(),
            unhealthy_threshold: default_unhealthy_threshold(),
            health_check_interval_secs: default_health_check_interval_secs(),
            state_file: None,
            batch_size: default_batch_size(),
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

fn default_retry_max_attempts() -> u32 {
    3
}
fn default_retry_initial_delay_ms() -> u64 {
    1000
}
fn default_retry_max_delay_ms() -> u64 {
    30000
}
fn default_request_timeout_secs() -> u64 {
    30
}
fn default_healthy_threshold() -> u32 {
    2
}
fn default_unhealthy_threshold() -> u32 {
    5
}
fn default_health_check_interval_secs() -> u64 {
    60
}
fn default_batch_size() -> u64 {
    256
}
fn default_poll_interval_ms() -> u64 {
    1000
}

#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_per_second")]
    pub per_second: u64,
    #[serde(default = "default_burst_size")]
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            per_second: default_per_second(),
            burst_size: default_burst_size(),
        }
    }
}

fn default_per_second() -> u64 {
    10
}
fn default_burst_size() -> u32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default)]
    pub per_ip_limit: Option<u32>,
}

impl Default for ConnectionLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_connections: default_max_connections(),
            per_ip_limit: None,
        }
    }
}

fn default_max_connections() -> u32 {
    10000
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub tokens: Vec<String>,
    #[serde(default = "default_header_name")]
    pub header_name: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tokens: Vec::new(),
            header_name: default_header_name(),
        }
    }
}

fn default_header_name() -> String {
    "Authorization".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotReloadConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub watch_path: Option<String>,
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            watch_path: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone)]
pub struct Config {
    pub host: IpAddr,
    pub port: u16,
    pub log_level: String,
    pub buffer_size: usize,
    pub ct_logs_url: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub custom_logs: Vec<CustomCtLog>,
    pub protocols: ProtocolConfig,
    pub ct_log: CtLogConfig,
    pub rate_limit: RateLimitConfig,
    pub connection_limit: ConnectionLimitConfig,
    pub auth: AuthConfig,
    pub hot_reload: HotReloadConfig,
    pub config_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct YamlConfig {
    host: Option<String>,
    port: Option<u16>,
    log_level: Option<String>,
    buffer_size: Option<usize>,
    ct_logs_url: Option<String>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    #[serde(default)]
    custom_logs: Vec<CustomCtLog>,
    #[serde(default)]
    protocols: Option<ProtocolConfig>,
    #[serde(default)]
    ct_log: Option<CtLogConfig>,
    #[serde(default)]
    rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    connection_limit: Option<ConnectionLimitConfig>,
    #[serde(default)]
    auth: Option<AuthConfig>,
    #[serde(default)]
    hot_reload: Option<HotReloadConfig>,
}

struct YamlConfigWithPath {
    config: YamlConfig,
    path: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let yaml_result = Self::load_yaml();
        let yaml_config = yaml_result.config;
        let config_path = yaml_result.path;

        let host = env::var("CERTSTREAM_HOST")
            .ok()
            .or(yaml_config.host)
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| "0.0.0.0".parse().unwrap());

        let port = env::var("CERTSTREAM_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(yaml_config.port)
            .unwrap_or(8080);

        let log_level = env::var("CERTSTREAM_LOG_LEVEL")
            .ok()
            .or(yaml_config.log_level)
            .unwrap_or_else(|| "info".to_string());

        let buffer_size = env::var("CERTSTREAM_BUFFER_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(yaml_config.buffer_size)
            .unwrap_or(1000);

        let ct_logs_url = env::var("CERTSTREAM_CT_LOGS_URL")
            .ok()
            .or(yaml_config.ct_logs_url)
            .unwrap_or_else(|| {
                "https://www.gstatic.com/ct/log_list/v3/all_logs_list.json".to_string()
            });

        let tls_cert = env::var("CERTSTREAM_TLS_CERT").ok().or(yaml_config.tls_cert);
        let tls_key = env::var("CERTSTREAM_TLS_KEY").ok().or(yaml_config.tls_key);

        let protocols = yaml_config.protocols.unwrap_or_else(|| {
            let ws = env::var("CERTSTREAM_WS_ENABLED")
                .map(|v| v.parse().unwrap_or(true))
                .unwrap_or(true);
            let sse = env::var("CERTSTREAM_SSE_ENABLED")
                .map(|v| v.parse().unwrap_or(false))
                .unwrap_or(false);
            let tcp = env::var("CERTSTREAM_TCP_ENABLED")
                .map(|v| v.parse().unwrap_or(false))
                .unwrap_or(false);
            let tcp_port = env::var("CERTSTREAM_TCP_PORT")
                .ok()
                .and_then(|v| v.parse().ok());
            let metrics = env::var("CERTSTREAM_METRICS_ENABLED")
                .map(|v| v.parse().unwrap_or(true))
                .unwrap_or(true);

            ProtocolConfig {
                websocket: ws,
                sse,
                tcp,
                tcp_port,
                metrics,
            }
        });

        let ct_log = yaml_config.ct_log.unwrap_or_else(|| {
            let mut cfg = CtLogConfig::default();
            if let Ok(v) = env::var("CERTSTREAM_RETRY_MAX_ATTEMPTS") {
                cfg.retry_max_attempts = v.parse().unwrap_or(cfg.retry_max_attempts);
            }
            if let Ok(v) = env::var("CERTSTREAM_RETRY_INITIAL_DELAY_MS") {
                cfg.retry_initial_delay_ms = v.parse().unwrap_or(cfg.retry_initial_delay_ms);
            }
            if let Ok(v) = env::var("CERTSTREAM_RETRY_MAX_DELAY_MS") {
                cfg.retry_max_delay_ms = v.parse().unwrap_or(cfg.retry_max_delay_ms);
            }
            if let Ok(v) = env::var("CERTSTREAM_REQUEST_TIMEOUT_SECS") {
                cfg.request_timeout_secs = v.parse().unwrap_or(cfg.request_timeout_secs);
            }
            if let Ok(v) = env::var("CERTSTREAM_UNHEALTHY_THRESHOLD") {
                cfg.unhealthy_threshold = v.parse().unwrap_or(cfg.unhealthy_threshold);
            }
            if let Ok(v) = env::var("CERTSTREAM_HEALTHY_THRESHOLD") {
                cfg.healthy_threshold = v.parse().unwrap_or(cfg.healthy_threshold);
            }
            if let Ok(v) = env::var("CERTSTREAM_HEALTH_CHECK_INTERVAL_SECS") {
                cfg.health_check_interval_secs = v.parse().unwrap_or(cfg.health_check_interval_secs);
            }
            if let Ok(v) = env::var("CERTSTREAM_STATE_FILE") {
                cfg.state_file = Some(v);
            }
            if let Ok(v) = env::var("CERTSTREAM_BATCH_SIZE") {
                cfg.batch_size = v.parse().unwrap_or(cfg.batch_size);
            }
            if let Ok(v) = env::var("CERTSTREAM_POLL_INTERVAL_MS") {
                cfg.poll_interval_ms = v.parse().unwrap_or(cfg.poll_interval_ms);
            }
            cfg
        });

        let rate_limit = yaml_config.rate_limit.unwrap_or_else(|| {
            let mut cfg = RateLimitConfig::default();
            if let Ok(v) = env::var("CERTSTREAM_RATE_LIMIT_ENABLED") {
                cfg.enabled = v.parse().unwrap_or(false);
            }
            if let Ok(v) = env::var("CERTSTREAM_RATE_LIMIT_PER_SECOND") {
                cfg.per_second = v.parse().unwrap_or(cfg.per_second);
            }
            if let Ok(v) = env::var("CERTSTREAM_RATE_LIMIT_BURST_SIZE") {
                cfg.burst_size = v.parse().unwrap_or(cfg.burst_size);
            }
            cfg
        });

        let connection_limit = yaml_config.connection_limit.unwrap_or_else(|| {
            let mut cfg = ConnectionLimitConfig::default();
            if let Ok(v) = env::var("CERTSTREAM_CONNECTION_LIMIT_ENABLED") {
                cfg.enabled = v.parse().unwrap_or(false);
            }
            if let Ok(v) = env::var("CERTSTREAM_MAX_CONNECTIONS") {
                cfg.max_connections = v.parse().unwrap_or(cfg.max_connections);
            }
            if let Ok(v) = env::var("CERTSTREAM_PER_IP_LIMIT") {
                cfg.per_ip_limit = v.parse().ok();
            }
            cfg
        });

        let auth = yaml_config.auth.unwrap_or_else(|| {
            let mut cfg = AuthConfig::default();
            if let Ok(v) = env::var("CERTSTREAM_AUTH_ENABLED") {
                cfg.enabled = v.parse().unwrap_or(false);
            }
            if let Ok(v) = env::var("CERTSTREAM_AUTH_TOKENS") {
                cfg.tokens = v.split(',').map(|s| s.trim().to_string()).collect();
            }
            if let Ok(v) = env::var("CERTSTREAM_AUTH_HEADER") {
                cfg.header_name = v;
            }
            cfg
        });

        let hot_reload = yaml_config.hot_reload.unwrap_or_else(|| {
            let mut cfg = HotReloadConfig::default();
            if let Ok(v) = env::var("CERTSTREAM_HOT_RELOAD_ENABLED") {
                cfg.enabled = v.parse().unwrap_or(false);
            }
            cfg
        });

        Self {
            host,
            port,
            log_level,
            buffer_size,
            ct_logs_url,
            tls_cert,
            tls_key,
            custom_logs: yaml_config.custom_logs,
            protocols,
            ct_log,
            rate_limit,
            connection_limit,
            auth,
            hot_reload,
            config_path,
        }
    }

    fn load_yaml() -> YamlConfigWithPath {
        let config_paths = [
            env::var("CERTSTREAM_CONFIG").ok(),
            Some("config.yaml".to_string()),
            Some("config.yml".to_string()),
            Some("/etc/certstream/config.yaml".to_string()),
        ];

        for path in config_paths.into_iter().flatten() {
            if Path::new(&path).exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(config) = serde_yaml::from_str::<YamlConfig>(&content) {
                        return YamlConfigWithPath {
                            config,
                            path: Some(path),
                        };
                    }
                }
            }
        }

        YamlConfigWithPath {
            config: YamlConfig::default(),
            path: None,
        }
    }

    pub fn has_tls(&self) -> bool {
        self.tls_cert.is_some() && self.tls_key.is_some()
    }
}
