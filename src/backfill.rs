//! Re-read a fixed index range from one CT log and write it out as JSONL.
//!
//! Deliberately not the live watcher: the range is fixed before the first
//! request, so the job is finite and repeatable, and it never touches the
//! live state file — a backfill must not move the position the streaming
//! server is holding.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::io::{AsyncWriteExt, BufWriter};
use tracing::{info, warn};

use crate::config::CtLogConfig;
use crate::ct::{parse_leaf_input_with_options, CtLog, LogType};
use crate::ct::static_ct::{decompress_tile, parse_tile_leaves, tile_url};
use crate::models::{
    CertificateData, CertificateMessage, Source, Verification, VerificationState,
};

/// Entries per static-CT data tile, fixed by the spec.
const TILE_WIDTH: u64 = 256;

pub struct BackfillRequest {
    pub start: u64,
    /// Inclusive.
    pub end: u64,
    /// `None` writes to stdout.
    pub out: Option<PathBuf>,
}

pub struct BackfillReport {
    pub requested: u64,
    pub written: u64,
    pub unparseable: u64,
    pub fetch_failures: u64,
}

/// Pick the one log the request names. Ambiguity is an error rather than a
/// guess: replaying the wrong log's index range produces plausible, wrong
/// output.
pub fn resolve_target(logs: &[CtLog], want: &str) -> Result<CtLog, String> {
    let needle = want.trim().trim_end_matches('/').to_lowercase();

    let matches: Vec<&CtLog> = logs
        .iter()
        .filter(|l| {
            l.normalized_url().to_lowercase().trim_end_matches('/') == needle
                || l.log_id.as_deref().is_some_and(|id| id == want)
                || l.description.to_lowercase().contains(&needle)
        })
        .collect();

    match matches.as_slice() {
        [] => Err(format!("no CT log matches `{want}`")),
        [one] => Ok((*one).clone()),
        many => Err(format!(
            "`{want}` matches {} logs: {}",
            many.len(),
            many.iter()
                .map(|l| l.description.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub async fn run(
    req: BackfillRequest,
    log: CtLog,
    config: &CtLogConfig,
    client: &reqwest::Client,
) -> Result<BackfillReport, String> {
    if req.end < req.start {
        return Err(format!(
            "end ({}) is before start ({})",
            req.end, req.start
        ));
    }

    let base_url = log.normalized_url();
    let source = Arc::new(Source {
        name: Arc::from(log.description.as_str()),
        url: Arc::from(base_url.as_str()),
        log_id: log.log_id.as_deref().map(Arc::from),
        operator: Arc::from(log.operator_name()),
        log_type: match log.log_type {
            LogType::Rfc6962 => "rfc6962",
            LogType::StaticCt => "static_ct",
        },
    });

    let mut out: Box<dyn AsyncWrite> = match &req.out {
        Some(path) => Box::new(BufWriter::new(
            tokio::fs::File::create(path)
                .await
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?,
        )),
        None => Box::new(BufWriter::new(tokio::io::stdout())),
    };

    // The batch job honours the same per-operator floor as the watchers. A
    // backfill is not a reason to spend someone else's request budget faster
    // than the streaming server would.
    let operator = crate::ct::normalize_operator(&log.operator);
    let pace = Duration::from_millis(
        config
            .operator_rate_limits
            .get(&operator)
            .copied()
            .unwrap_or(config.default_operator_rate_limit_ms),
    );
    let timeout = Duration::from_secs(config.request_timeout_secs);

    info!(
        log = %log.description,
        start = req.start,
        end = req.end,
        "backfill starting"
    );

    let mut report = BackfillReport {
        requested: req.end - req.start + 1,
        written: 0,
        unparseable: 0,
        fetch_failures: 0,
    };

    match log.log_type {
        LogType::Rfc6962 => {
            backfill_rfc6962(
                &req, &base_url, &source, config, client, timeout, pace, &mut out, &mut report,
            )
            .await?
        }
        LogType::StaticCt => {
            backfill_static_ct(
                &req, &base_url, &source, config, client, timeout, pace, &mut out, &mut report,
            )
            .await?
        }
    }

    out.flush_all().await.map_err(|e| e.to_string())?;
    Ok(report)
}

/// Minimal object-safe writer so the two protocol paths share one sink.
trait AsyncWrite: Send {
    fn write_line<'a>(
        &'a mut self,
        line: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>>;
    fn flush_all(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + '_>>;
}

impl<W: tokio::io::AsyncWrite + Unpin + Send> AsyncWrite for BufWriter<W> {
    fn write_line<'a>(
        &'a mut self,
        line: &'a [u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.write_all(line).await?;
            self.write_all(b"\n").await
        })
    }

    fn flush_all(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + '_>> {
        Box::pin(async move { self.flush().await })
    }
}

/// Every backfilled record carries the same honest verification state: this
/// tool reads entries back, it does not prove them.
fn backfill_verification(log_type: LogType) -> Verification {
    Verification {
        checkpoint_signature: match log_type {
            LogType::Rfc6962 => VerificationState::NotApplicable,
            LogType::StaticCt => VerificationState::Unverified,
        },
        inclusion: VerificationState::Unverified,
    }
}

#[allow(clippy::too_many_arguments)]
async fn backfill_rfc6962(
    req: &BackfillRequest,
    base_url: &str,
    source: &Arc<Source>,
    config: &CtLogConfig,
    client: &reqwest::Client,
    timeout: Duration,
    pace: Duration,
    out: &mut Box<dyn AsyncWrite>,
    report: &mut BackfillReport,
) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        leaf_input: String,
        extra_data: String,
    }
    #[derive(serde::Deserialize)]
    struct Entries {
        entries: Vec<Entry>,
    }

    let verification = backfill_verification(LogType::Rfc6962);
    let batch = config.batch_size.max(1);
    let mut index = req.start;

    while index <= req.end {
        let last = (index + batch - 1).min(req.end);
        let url = format!("{base_url}/ct/v1/get-entries?start={index}&end={last}");

        let body = match client.get(&url).timeout(timeout).send().await {
            Ok(resp) if resp.status().is_success() => resp.text().await.map_err(|e| e.to_string()),
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(e) => Err(e.to_string()),
        };
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                warn!(url = %url, error = %e, "get-entries failed; skipping window");
                report.fetch_failures += 1;
                index = last + 1;
                tokio::time::sleep(pace).await;
                continue;
            }
        };

        let entries: Entries = serde_json::from_str(&body)
            .map_err(|e| format!("malformed get-entries response at {index}: {e}"))?;
        if entries.entries.is_empty() {
            return Err(format!("log returned no entries at index {index}"));
        }

        for (offset, entry) in entries.entries.iter().enumerate() {
            let cert_index = index + offset as u64;
            if cert_index > req.end {
                break;
            }
            match parse_leaf_input_with_options(
                &entry.leaf_input,
                &entry.extra_data,
                Default::default(),
            ) {
                Some(parsed) => {
                    let chain = parsed.parse_chain();
                    let msg = CertificateMessage {
                        message_type: std::borrow::Cow::Borrowed("certificate_update"),
                        data: CertificateData {
                            update_type: parsed.update_type,
                            leaf_cert: Arc::new(parsed.leaf_cert),
                            chain: Some(chain),
                            cert_index,
                            cert_link: format!(
                                "{base_url}/ct/v1/get-entries?start={cert_index}&end={cert_index}"
                            ),
                            seen: now_secs(),
                            submission_timestamp: parsed.submission_timestamp,
                            source: Arc::clone(source),
                            verification,
                        },
                    };
                    write_record(out, &msg, report).await?;
                }
                None => report.unparseable += 1,
            }
        }

        index += entries.entries.len() as u64;
        tokio::time::sleep(pace).await;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn backfill_static_ct(
    req: &BackfillRequest,
    base_url: &str,
    source: &Arc<Source>,
    _config: &CtLogConfig,
    client: &reqwest::Client,
    timeout: Duration,
    pace: Duration,
    out: &mut Box<dyn AsyncWrite>,
    report: &mut BackfillReport,
) -> Result<(), String> {
    let verification = backfill_verification(LogType::StaticCt);
    let mut tile_index = req.start / TILE_WIDTH;
    let last_tile = req.end / TILE_WIDTH;

    while tile_index <= last_tile {
        let url = tile_url(base_url, 0, tile_index, 0);
        let body: Result<Bytes, String> = match client.get(&url).timeout(timeout).send().await {
            Ok(resp) if resp.status().is_success() => resp.bytes().await.map_err(|e| e.to_string()),
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(e) => Err(e.to_string()),
        };
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                warn!(url = %url, error = %e, "tile fetch failed; skipping tile");
                report.fetch_failures += 1;
                tile_index += 1;
                tokio::time::sleep(pace).await;
                continue;
            }
        };

        let decompressed = Bytes::from(decompress_tile(&body).into_owned());
        let tile_start = tile_index * TILE_WIDTH;
        for (offset, leaf) in parse_tile_leaves(decompressed).into_iter().enumerate() {
            let cert_index = tile_start + offset as u64;
            if cert_index < req.start {
                continue;
            }
            if cert_index > req.end {
                break;
            }
            match crate::ct::parse_certificate_with_options(&leaf.cert_der, Default::default()) {
                Some(parsed) => {
                    let msg = CertificateMessage {
                        message_type: std::borrow::Cow::Borrowed("certificate_update"),
                        data: CertificateData {
                            update_type: std::borrow::Cow::Borrowed(if leaf.is_precert {
                                "PrecertLogEntry"
                            } else {
                                "X509LogEntry"
                            }),
                            leaf_cert: Arc::new(parsed),
                            // Issuers live behind a second endpoint; a range
                            // replay does not chase them.
                            chain: None,
                            cert_index,
                            cert_link: url.clone(),
                            seen: now_secs(),
                            submission_timestamp: leaf.submission_timestamp as f64 / 1000.0,
                            source: Arc::clone(source),
                            verification,
                        },
                    };
                    write_record(out, &msg, report).await?;
                }
                None => report.unparseable += 1,
            }
        }

        tile_index += 1;
        tokio::time::sleep(pace).await;
    }
    Ok(())
}

async fn write_record(
    out: &mut Box<dyn AsyncWrite>,
    msg: &CertificateMessage,
    report: &mut BackfillReport,
) -> Result<(), String> {
    let line = msg.to_v2_json().map_err(|e| e.to_string())?;
    out.write_line(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    report.written += 1;
    Ok(())
}

fn now_secs() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomCtLog;

    fn log(name: &str, url: &str) -> CtLog {
        CtLog::from(CustomCtLog {
            name: name.to_string(),
            url: url.to_string(),
            expected_log_id: None,
            batch_size: None,
            poll_interval_ms: None,
        })
    }

    #[test]
    fn a_target_resolves_by_url_or_name() {
        let logs = vec![
            log("Argon 2026h1", "https://ct.example.com/argon2026h1/"),
            log("Nimbus 2026", "https://ct.other.com/nimbus2026/"),
        ];

        assert_eq!(
            resolve_target(&logs, "https://ct.example.com/argon2026h1")
                .unwrap()
                .description,
            "Argon 2026h1"
        );
        assert_eq!(
            resolve_target(&logs, "nimbus").unwrap().description,
            "Nimbus 2026"
        );
    }

    /// Replaying the wrong log's index range yields plausible, wrong output,
    /// so an ambiguous name has to stop the job rather than pick one.
    #[test]
    fn an_ambiguous_target_is_an_error() {
        let logs = vec![
            log("Argon 2026h1", "https://ct.example.com/argon2026h1/"),
            log("Argon 2026h2", "https://ct.example.com/argon2026h2/"),
        ];
        let err = resolve_target(&logs, "argon").unwrap_err();
        assert!(err.contains("matches 2 logs"), "{err}");
        assert!(resolve_target(&logs, "xenon").is_err());
    }

    #[tokio::test]
    async fn an_inverted_range_is_rejected_before_any_request() {
        let target = log("Argon", "https://ct.example.com/argon/");
        let Err(err) = run(
            BackfillRequest {
                start: 100,
                end: 50,
                out: None,
            },
            target,
            &CtLogConfig::default(),
            &reqwest::Client::new(),
        )
        .await
        else {
            panic!("an inverted range must be rejected");
        };
        assert!(err.contains("before start"), "{err}");
    }
}
