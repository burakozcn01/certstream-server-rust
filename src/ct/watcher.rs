use backon::{ExponentialBuilder, Retryable};
use reqwest::Client;
use serde::Deserialize;
use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::config::CtLogConfig;
use crate::ct::{parse_leaf_input, CtLog};
use crate::models::{CertificateData, CertificateMessage, PreSerializedMessage, Source};
use crate::state::StateManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

pub struct LogHealth {
    consecutive_failures: AtomicU32,
    consecutive_successes: AtomicU32,
    total_errors: AtomicU64,
    status: parking_lot::RwLock<HealthStatus>,
}

impl LogHealth {
    pub fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            consecutive_successes: AtomicU32::new(0),
            total_errors: AtomicU64::new(0),
            status: parking_lot::RwLock::new(HealthStatus::Healthy),
        }
    }

    pub fn record_success(&self, healthy_threshold: u32) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        let successes = self.consecutive_successes.fetch_add(1, Ordering::Relaxed) + 1;

        if successes >= healthy_threshold {
            let mut status = self.status.write();
            if *status != HealthStatus::Healthy {
                *status = HealthStatus::Healthy;
            }
        }
    }

    pub fn record_failure(&self, unhealthy_threshold: u32) {
        self.consecutive_successes.store(0, Ordering::Relaxed);
        self.total_errors.fetch_add(1, Ordering::Relaxed);
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;

        let mut status = self.status.write();
        if failures >= unhealthy_threshold {
            *status = HealthStatus::Unhealthy;
        } else if failures >= unhealthy_threshold / 2 {
            *status = HealthStatus::Degraded;
        }
    }

    #[allow(dead_code)]
    pub fn status(&self) -> HealthStatus {
        *self.status.read()
    }

    pub fn is_healthy(&self) -> bool {
        *self.status.read() != HealthStatus::Unhealthy
    }

    pub fn total_errors(&self) -> u64 {
        self.total_errors.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
struct SthResponse {
    tree_size: u64,
}

#[derive(Debug, Deserialize)]
struct EntriesResponse {
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    leaf_input: String,
    extra_data: String,
}

pub async fn run_watcher(
    client: Client,
    log: CtLog,
    tx: broadcast::Sender<Arc<PreSerializedMessage>>,
    config: Arc<CtLogConfig>,
    state_manager: Arc<StateManager>,
) {
    let base_url = log.normalized_url();
    let source = Arc::new(Source {
        name: Arc::from(log.description.as_str()),
        url: Arc::from(base_url.as_str()),
    });

    let health = Arc::new(LogHealth::new());
    let poll_interval = Duration::from_millis(config.poll_interval_ms);
    let error_backoff = Duration::from_secs(5);

    info!(log = %log.description, url = %base_url, "starting watcher");

    let mut current_index = if let Some(saved_index) = state_manager.get_index(&base_url) {
        info!(
            log = %log.description,
            saved_index = saved_index,
            "resuming from saved state"
        );
        saved_index
    } else {
        match get_tree_size_with_retry(&client, &base_url, &config, &health).await {
            Ok(size) => {
                let start = size.saturating_sub(1000);
                info!(
                    log = %log.description,
                    tree_size = size,
                    starting_at = start,
                    "starting fresh"
                );
                start
            }
            Err(e) => {
                error!(log = %log.description, error = %e, "failed to get initial tree size");
                0
            }
        }
    };

    loop {
        if !health.is_healthy() {
            warn!(
                log = %log.description,
                errors = health.total_errors(),
                "log is unhealthy, waiting for recovery check"
            );
            sleep(Duration::from_secs(config.health_check_interval_secs)).await;

            match get_tree_size_with_retry(&client, &base_url, &config, &health).await {
                Ok(_) => {
                    info!(log = %log.description, "health check passed, resuming");
                }
                Err(e) => {
                    warn!(
                        log = %log.description,
                        error = %e,
                        "health check failed, staying disabled"
                    );
                    metrics::counter!("certstream_log_health_checks_failed").increment(1);
                    continue;
                }
            }
        }

        match get_tree_size_with_retry(&client, &base_url, &config, &health).await {
            Ok(tree_size) => {
                if current_index >= tree_size {
                    sleep(poll_interval).await;
                    continue;
                }

                let end = (current_index + config.batch_size).min(tree_size - 1);

                match fetch_entries_with_retry(&client, &base_url, current_index, end, &config, &health).await {
                    Ok(entries) => {
                        let count = entries.len();
                        for (i, entry) in entries.into_iter().enumerate() {
                            if let Some(parsed) =
                                parse_leaf_input(&entry.leaf_input, &entry.extra_data)
                            {
                                let msg = CertificateMessage {
                                    message_type: Cow::Borrowed("certificate_update"),
                                    data: CertificateData {
                                        update_type: parsed.update_type,
                                        leaf_cert: parsed.leaf_cert,
                                        chain: Some(parsed.chain),
                                        cert_index: current_index + i as u64,
                                        seen: chrono::Utc::now().timestamp_millis() as f64
                                            / 1000.0,
                                        source: Arc::clone(&source),
                                    },
                                };

                                if let Some(serialized) = msg.pre_serialize() {
                                    let _ = tx.send(serialized);
                                    metrics::counter!("certstream_messages_sent").increment(1);
                                }
                            }
                        }

                        debug!(log = %log.description, count = count, "fetched entries");
                        current_index = end + 1;
                        state_manager.update_index(&base_url, current_index, tree_size);
                    }
                    Err(e) => {
                        warn!(log = %log.description, error = %e, "failed to fetch entries after retries");
                        metrics::counter!("certstream_fetch_errors").increment(1);
                        sleep(error_backoff).await;
                    }
                }
            }
            Err(e) => {
                warn!(log = %log.description, error = %e, "failed to get tree size after retries");
                metrics::counter!("certstream_tree_size_errors").increment(1);
                sleep(error_backoff).await;
            }
        }
    }
}

async fn get_tree_size_with_retry(
    client: &Client,
    base_url: &str,
    config: &CtLogConfig,
    health: &LogHealth,
) -> Result<u64, String> {
    let url = format!("{}/ct/v1/get-sth", base_url);
    let timeout = Duration::from_secs(config.request_timeout_secs);

    let backoff = ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(config.retry_initial_delay_ms))
        .with_max_delay(Duration::from_millis(config.retry_max_delay_ms))
        .with_max_times(config.retry_max_attempts as usize);

    let result = (|| async {
        let response: SthResponse = client
            .get(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(response.tree_size)
    })
    .retry(backoff)
    .sleep(tokio::time::sleep)
    .await;

    match &result {
        Ok(_) => health.record_success(config.healthy_threshold),
        Err(_) => health.record_failure(config.unhealthy_threshold),
    }

    result
}

async fn fetch_entries_with_retry(
    client: &Client,
    base_url: &str,
    start: u64,
    end: u64,
    config: &CtLogConfig,
    health: &LogHealth,
) -> Result<Vec<Entry>, String> {
    let url = format!("{}/ct/v1/get-entries?start={}&end={}", base_url, start, end);
    let timeout = Duration::from_secs(config.request_timeout_secs);

    let backoff = ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(config.retry_initial_delay_ms))
        .with_max_delay(Duration::from_millis(config.retry_max_delay_ms))
        .with_max_times(config.retry_max_attempts as usize);

    let result = (|| async {
        let response: EntriesResponse = client
            .get(&url)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        Ok(response.entries)
    })
    .retry(backoff)
    .sleep(tokio::time::sleep)
    .await;

    match &result {
        Ok(_) => health.record_success(config.healthy_threshold),
        Err(_) => health.record_failure(config.unhealthy_threshold),
    }

    result
}
