use crate::config::CustomCtLog;
use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LogListError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("No usable logs found")]
    NoLogs,
}

#[derive(Debug, Deserialize)]
struct LogListResponse {
    operators: Vec<Operator>,
}

#[derive(Debug, Deserialize)]
struct Operator {
    #[allow(dead_code)]
    name: String,
    logs: Vec<CtLog>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CtLog {
    pub description: String,
    pub url: String,
    #[serde(default)]
    state: Option<LogState>,
}

#[derive(Debug, Clone, Deserialize)]
struct LogState {
    #[serde(default)]
    usable: Option<StateInfo>,
    #[serde(default)]
    retired: Option<StateInfo>,
    #[serde(default)]
    #[allow(dead_code)]
    readonly: Option<StateInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct StateInfo {
    #[allow(dead_code)]
    timestamp: String,
}

impl CtLog {
    pub fn is_usable(&self) -> bool {
        match &self.state {
            Some(state) => state.usable.is_some() && state.retired.is_none(),
            None => true,
        }
    }

    pub fn normalized_url(&self) -> String {
        let url = self.url.trim_end_matches('/');
        if url.starts_with("https://") || url.starts_with("http://") {
            url.to_string()
        } else {
            format!("https://{}", url)
        }
    }
}

impl From<CustomCtLog> for CtLog {
    fn from(custom: CustomCtLog) -> Self {
        Self {
            description: custom.name,
            url: custom.url,
            state: None,
        }
    }
}

pub async fn fetch_log_list(
    client: &Client,
    url: &str,
    custom_logs: Vec<CustomCtLog>,
) -> Result<Vec<CtLog>, LogListError> {
    let response: LogListResponse = client.get(url).send().await?.json().await?;

    let mut logs: Vec<CtLog> = response
        .operators
        .into_iter()
        .flat_map(|op| op.logs)
        .filter(|log| log.is_usable())
        .collect();

    for custom_log in custom_logs {
        logs.push(CtLog::from(custom_log));
    }

    if logs.is_empty() {
        return Err(LogListError::NoLogs);
    }

    Ok(logs)
}
