use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::config::{AuthConfig, ConnectionLimitConfig, RateLimitConfig};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct HotReloadableConfig {
    pub rate_limit: RateLimitConfig,
    pub connection_limit: ConnectionLimitConfig,
    pub auth: AuthConfig,
}

#[allow(dead_code)]
pub struct ConfigWatcher {
    tx: watch::Sender<HotReloadableConfig>,
    rx: watch::Receiver<HotReloadableConfig>,
}

#[allow(dead_code)]
impl ConfigWatcher {
    pub fn new(initial: HotReloadableConfig) -> Arc<Self> {
        let (tx, rx) = watch::channel(initial);
        Arc::new(Self { tx, rx })
    }

    pub fn subscribe(&self) -> watch::Receiver<HotReloadableConfig> {
        self.rx.clone()
    }

    pub fn current(&self) -> HotReloadableConfig {
        self.rx.borrow().clone()
    }

    pub fn start_watching(self: Arc<Self>, config_path: Option<String>) {
        let Some(path) = config_path else {
            info!("no config file specified, hot reload disabled");
            return;
        };

        let path_clone = path.clone();
        let watcher_self = self.clone();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build runtime for config watcher");

            rt.block_on(async move {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<notify::Result<Event>>(100);

                let mut watcher = match RecommendedWatcher::new(
                    move |res| {
                        let _ = tx.blocking_send(res);
                    },
                    NotifyConfig::default(),
                ) {
                    Ok(w) => w,
                    Err(e) => {
                        error!(error = %e, "failed to create file watcher");
                        return;
                    }
                };

                if let Err(e) = watcher.watch(Path::new(&path_clone), RecursiveMode::NonRecursive) {
                    error!(path = %path_clone, error = %e, "failed to watch config file");
                    return;
                }

                info!(path = %path_clone, "watching config file for changes");

                while let Some(event) = rx.recv().await {
                    match event {
                        Ok(event) => {
                            if event.kind.is_modify() || event.kind.is_create() {
                                info!("config file changed, reloading...");
                                if let Some(new_config) = load_hot_reloadable_config(&path_clone) {
                                    if watcher_self.tx.send(new_config).is_err() {
                                        error!("failed to send config update");
                                    } else {
                                        info!("config reloaded successfully");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "file watch error");
                        }
                    }
                }
            });
        });
    }
}

fn load_hot_reloadable_config(path: &str) -> Option<HotReloadableConfig> {
    use serde::Deserialize;

    #[derive(Deserialize, Default)]
    struct PartialConfig {
        #[serde(default)]
        rate_limit: Option<RateLimitConfig>,
        #[serde(default)]
        connection_limit: Option<ConnectionLimitConfig>,
        #[serde(default)]
        auth: Option<AuthConfig>,
    }

    match std::fs::read_to_string(path) {
        Ok(content) => match serde_yaml::from_str::<PartialConfig>(&content) {
            Ok(cfg) => Some(HotReloadableConfig {
                rate_limit: cfg.rate_limit.unwrap_or_default(),
                connection_limit: cfg.connection_limit.unwrap_or_default(),
                auth: cfg.auth.unwrap_or_default(),
            }),
            Err(e) => {
                error!(error = %e, "failed to parse config file");
                None
            }
        },
        Err(e) => {
            error!(error = %e, "failed to read config file");
            None
        }
    }
}
