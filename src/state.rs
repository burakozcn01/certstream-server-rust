use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogState {
    pub current_index: u64,
    pub tree_size: u64,
    pub last_success: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StateFile {
    version: u32,
    logs: std::collections::HashMap<String, LogState>,
}

pub struct StateManager {
    file_path: Option<String>,
    states: DashMap<String, LogState>,
    dirty: Arc<RwLock<bool>>,
}

impl StateManager {
    pub fn new(file_path: Option<String>) -> Arc<Self> {
        let manager = Arc::new(Self {
            file_path: file_path.clone(),
            states: DashMap::new(),
            dirty: Arc::new(RwLock::new(false)),
        });

        if let Some(ref path) = file_path {
            manager.load_from_file(path);
        }

        manager
    }

    fn load_from_file(&self, path: &str) {
        if !Path::new(path).exists() {
            debug!(path = %path, "state file does not exist, starting fresh");
            return;
        }

        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<StateFile>(&content) {
                Ok(state_file) => {
                    for (log_url, state) in state_file.logs {
                        self.states.insert(log_url, state);
                    }
                    info!(
                        path = %path,
                        logs = self.states.len(),
                        "loaded state from file"
                    );
                }
                Err(e) => {
                    warn!(path = %path, error = %e, "failed to parse state file, starting fresh");
                }
            },
            Err(e) => {
                warn!(path = %path, error = %e, "failed to read state file, starting fresh");
            }
        }
    }

    pub fn get_index(&self, log_url: &str) -> Option<u64> {
        self.states.get(log_url).map(|s| s.current_index)
    }

    pub fn update_index(&self, log_url: &str, index: u64, tree_size: u64) {
        let now = chrono::Utc::now().timestamp();
        self.states.insert(
            log_url.to_string(),
            LogState {
                current_index: index,
                tree_size,
                last_success: now,
            },
        );
        if let Ok(mut dirty) = self.dirty.try_write() {
            *dirty = true;
        }
    }

    pub async fn save_if_dirty(&self) {
        let is_dirty = {
            let dirty = self.dirty.read().await;
            *dirty
        };

        if !is_dirty {
            return;
        }

        if let Some(ref path) = self.file_path {
            self.save_to_file(path).await;
        }
    }

    async fn save_to_file(&self, path: &str) {
        let mut logs = std::collections::HashMap::new();
        for entry in self.states.iter() {
            logs.insert(entry.key().clone(), entry.value().clone());
        }

        let state_file = StateFile { version: 1, logs };

        match serde_json::to_string_pretty(&state_file) {
            Ok(content) => {
                let tmp_path = format!("{}.tmp", path);
                match fs::write(&tmp_path, &content) {
                    Ok(_) => match fs::rename(&tmp_path, path) {
                        Ok(_) => {
                            let mut dirty = self.dirty.write().await;
                            *dirty = false;
                            debug!(path = %path, "saved state to file");
                        }
                        Err(e) => {
                            error!(path = %path, error = %e, "failed to rename state file");
                            let _ = fs::remove_file(&tmp_path);
                        }
                    },
                    Err(e) => {
                        error!(tmp_path = %tmp_path, error = %e, "failed to write temp state file");
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "failed to serialize state");
            }
        }
    }

    pub fn start_periodic_save(self: Arc<Self>, interval: Duration) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                manager.save_if_dirty().await;
            }
        });
    }
}
