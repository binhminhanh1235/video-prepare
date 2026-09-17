use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{StockProviderError, TaskState};

pub const RUN_LOG_SCHEMA_VERSION: u32 = 1;
pub const RUN_LOG_RELATIVE_PATH: &str = "logs/run.ndjson";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiagnosticCategory {
    Configuration,
    Validation,
    Storage,
    Network,
    Authentication,
    RateLimit,
    RemoteUnknown,
    RemoteFailure,
    Integrity,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub category: DiagnosticCategory,
    pub retryable: bool,
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        code: impl Into<String>,
        category: DiagnosticCategory,
        retryable: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            retryable,
            message: message.into(),
        }
    }

    pub fn configuration(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, DiagnosticCategory::Configuration, false, message)
    }

    pub fn storage(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, DiagnosticCategory::Storage, true, message)
    }

    pub fn integrity(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, DiagnosticCategory::Integrity, false, message)
    }

    pub fn remote_unknown(message: impl Into<String>) -> Self {
        Self::new(
            "AUDIO_REMOTE_UNKNOWN",
            DiagnosticCategory::RemoteUnknown,
            true,
            message,
        )
    }

    pub fn remote_failure(message: impl Into<String>) -> Self {
        Self::new(
            "AUDIO_REMOTE_FAILED",
            DiagnosticCategory::RemoteFailure,
            true,
            message,
        )
    }

    pub fn from_stock_provider(error: &StockProviderError) -> Self {
        match error {
            StockProviderError::InvalidRequest(message) => Self::new(
                "PROVIDER_INVALID_REQUEST",
                DiagnosticCategory::Validation,
                false,
                message,
            ),
            StockProviderError::InvalidApiKey => Self::configuration(
                "CONFIG_PEXELS_KEY_INVALID",
                "Pexels API key is empty or has an invalid header format",
            ),
            StockProviderError::Authentication { status } => Self::new(
                "PROVIDER_AUTH",
                DiagnosticCategory::Authentication,
                false,
                format!("stock-provider authentication failed with HTTP {status}"),
            ),
            StockProviderError::RateLimited {
                retry_after_seconds,
            } => Self::new(
                "PROVIDER_RATE_LIMIT",
                DiagnosticCategory::RateLimit,
                true,
                match retry_after_seconds {
                    Some(seconds) => format!("stock-provider rate limited; retry after {seconds}s"),
                    None => "stock-provider rate limited".to_owned(),
                },
            ),
            StockProviderError::HttpStatus { status } => Self::new(
                "PROVIDER_HTTP_STATUS",
                DiagnosticCategory::Network,
                *status >= 500,
                format!("stock-provider returned HTTP {status}"),
            ),
            StockProviderError::Transport { message } => Self::new(
                "PROVIDER_NETWORK",
                DiagnosticCategory::Network,
                true,
                message,
            ),
            StockProviderError::Decode { message } => Self::new(
                "PROVIDER_DECODE",
                DiagnosticCategory::Internal,
                true,
                message,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLogEvent {
    pub schema_version: u32,
    pub unix_ms: u64,
    pub project_id: String,
    pub operation: String,
    pub outcome: String,
    pub state: Option<TaskState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl RunLogEvent {
    pub fn new(
        project_id: impl Into<String>,
        operation: impl Into<String>,
        outcome: impl Into<String>,
        state: Option<TaskState>,
    ) -> Self {
        Self {
            schema_version: RUN_LOG_SCHEMA_VERSION,
            unix_ms: unix_ms(),
            project_id: project_id.into(),
            operation: operation.into(),
            outcome: outcome.into(),
            state,
            diagnostic: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostic = Some(diagnostic);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    fn redacted(mut self, secrets: &[&str]) -> Self {
        self.project_id = redact(&self.project_id, secrets);
        self.operation = redact(&self.operation, secrets);
        self.outcome = redact(&self.outcome, secrets);
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.code = redact(&diagnostic.code, secrets);
            diagnostic.message = redact(&diagnostic.message, secrets);
        }
        for value in self.metadata.values_mut() {
            *value = redact(value, secrets);
        }
        self
    }
}

#[derive(Debug, Error)]
pub enum RunLogError {
    #[error("run log I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("run log JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn run_log_path(project_root: impl AsRef<Path>) -> PathBuf {
    project_root.as_ref().join(RUN_LOG_RELATIVE_PATH)
}

pub fn append_run_log_event(
    project_root: impl AsRef<Path>,
    event: RunLogEvent,
    secrets: &[&str],
) -> Result<(), RunLogError> {
    let path = run_log_path(project_root);
    let parent = path.parent().expect("run log path has a parent");
    fs::create_dir_all(parent).map_err(|source| RunLogError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|source| RunLogError::Io {
            path: path.clone(),
            source,
        })?;

    let event = event.redacted(secrets);
    serde_json::to_writer(&mut file, &event)?;
    file.write_all(b"\n").map_err(|source| RunLogError::Io {
        path: path.clone(),
        source,
    })?;
    file.flush().map_err(|source| RunLogError::Io {
        path: path.clone(),
        source,
    })?;
    file.sync_data()
        .map_err(|source| RunLogError::Io { path, source })?;
    Ok(())
}

pub fn read_run_log(project_root: impl AsRef<Path>) -> Result<Vec<RunLogEvent>, RunLogError> {
    let path = run_log_path(project_root);
    let file = std::fs::File::open(&path).map_err(|source| RunLogError::Io {
        path: path.clone(),
        source,
    })?;
    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|source| RunLogError::Io {
            path: path.clone(),
            source,
        })?;
        if !line.trim().is_empty() {
            events.push(serde_json::from_str(&line)?);
        }
    }
    Ok(events)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn redact(value: &str, secrets: &[&str]) -> String {
    let mut redacted = value.to_owned();
    let mut secrets: Vec<&str> = secrets
        .iter()
        .copied()
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    for secret in secrets {
        redacted = redacted.replace(secret, "[REDACTED]");
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_provider_errors_have_stable_categories_and_retryability() {
        let auth =
            Diagnostic::from_stock_provider(&StockProviderError::Authentication { status: 401 });
        assert_eq!(auth.code, "PROVIDER_AUTH");
        assert_eq!(auth.category, DiagnosticCategory::Authentication);
        assert!(!auth.retryable);

        let rate = Diagnostic::from_stock_provider(&StockProviderError::RateLimited {
            retry_after_seconds: Some(12),
        });
        assert_eq!(rate.code, "PROVIDER_RATE_LIMIT");
        assert_eq!(rate.category, DiagnosticCategory::RateLimit);
        assert!(rate.retryable);
    }

    #[test]
    fn remote_unknown_is_distinct_from_confirmed_remote_failure() {
        let unknown = Diagnostic::remote_unknown("timeout while checking job");
        let failed = Diagnostic::remote_failure("remote job failed");
        assert_eq!(unknown.category, DiagnosticCategory::RemoteUnknown);
        assert_eq!(failed.category, DiagnosticCategory::RemoteFailure);
        assert_ne!(unknown.code, failed.code);
    }

    #[test]
    fn ndjson_appends_parseable_events_without_truncating_prior_lines() {
        let temp = tempfile::tempdir().unwrap();
        append_run_log_event(
            temp.path(),
            RunLogEvent::new("demo", "run", "started", Some(TaskState::Running)),
            &[],
        )
        .unwrap();
        append_run_log_event(
            temp.path(),
            RunLogEvent::new("demo", "run", "completed", Some(TaskState::Completed))
                .with_metadata("flow", "visual"),
            &[],
        )
        .unwrap();

        let raw = std::fs::read_to_string(run_log_path(temp.path())).unwrap();
        assert_eq!(raw.lines().count(), 2);
        for line in raw.lines() {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }

        let parsed = read_run_log(temp.path()).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].outcome, "started");
        assert_eq!(parsed[1].outcome, "completed");
    }

    #[test]
    fn writer_redacts_secrets_from_diagnostic_and_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let api_key = "pexels-secret-123";
        let token = "omnivoice-token-456";
        let event = RunLogEvent::new("demo", "run", "failed", Some(TaskState::Failed))
            .with_diagnostic(Diagnostic::new(
                "TEST_FAILURE",
                DiagnosticCategory::Internal,
                true,
                format!("provider said key={api_key} token={token}"),
            ))
            .with_metadata("debug", format!("{api_key}:{token}"));
        append_run_log_event(temp.path(), event, &[api_key, token]).unwrap();

        let raw = std::fs::read_to_string(run_log_path(temp.path())).unwrap();
        assert!(!raw.contains(api_key));
        assert!(!raw.contains(token));
        assert!(raw.contains("[REDACTED]"));
    }

    #[test]
    fn empty_redaction_values_do_not_modify_log_content() {
        let temp = tempfile::tempdir().unwrap();
        append_run_log_event(
            temp.path(),
            RunLogEvent::new("demo", "run", "ok", None).with_metadata("message", "visible"),
            &["", "   "],
        )
        .unwrap();
        let raw = std::fs::read_to_string(run_log_path(temp.path())).unwrap();
        assert!(raw.contains("visible"));
    }
}
