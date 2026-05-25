//! Batch generation infrastructure — foundational types and trait for batch API dispatchers.
//!
//! This module defines the scaffolding that Anthropic and OpenAI batch dispatchers
//! (implemented in downstream tasks agc-bhgd and agc-lvjv) will plug into.
//!
//! Nothing in this module calls an external API — it is purely types, a trait, and
//! a simple JSON-backed state store for resumability.

use std::fmt;

// ── CaseBatchItem ─────────────────────────────────────────────────────────────

/// One unit of work submitted to a batch dispatcher.
pub struct CaseBatchItem {
    pub case_id: agentcarousel_core::CaseId,
    pub system: String,
    pub user_prompt: String,
    pub model: String,
    pub max_tokens: u32,
    pub seed: Option<u64>,
}

// ── BatchCaseResult ───────────────────────────────────────────────────────────

/// The outcome of a single case returned by a batch dispatcher.
pub struct BatchCaseResult {
    pub case_id: agentcarousel_core::CaseId,
    pub output: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub error: Option<String>,
}

// ── BatchError ────────────────────────────────────────────────────────────────

/// Error type for batch dispatchers, mirroring [`super::generator::GeneratorError`].
///
/// `Fatal` errors (bad API key, wrong model name, missing config) should abort the
/// entire batch. `Transient` errors (rate limits, server errors) may not recur on
/// a subsequent attempt.
#[derive(Debug)]
pub enum BatchError {
    /// Permanent failure — will affect every subsequent dispatch.
    Fatal(String),
    /// Transient failure — may not recur on the next dispatch attempt.
    Transient(String),
}

impl BatchError {
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fatal(msg) | Self::Transient(msg) => write!(f, "{msg}"),
        }
    }
}

// ── BatchDispatcher ───────────────────────────────────────────────────────────

/// Trait that Anthropic and OpenAI batch dispatchers implement.
///
/// A dispatcher accepts a batch of [`CaseBatchItem`]s, submits them to the
/// provider's batch API, waits for completion, and returns one [`BatchCaseResult`]
/// per input item (in any order — callers match on `case_id`).
pub trait BatchDispatcher: Send + Sync {
    fn dispatch(
        &self,
        items: Vec<CaseBatchItem>,
    ) -> impl std::future::Future<Output = Result<Vec<BatchCaseResult>, BatchError>> + Send;
}

// ── BatchStateRecord ──────────────────────────────────────────────────────────

/// Serialisable record that tracks an in-flight or completed batch job.
///
/// Written to `.agc/batch_state/{batch_id}.json` so that a crashed run can be
/// resumed by re-reading the provider batch ID and polling for results.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct BatchStateRecord {
    pub batch_id: String,
    pub provider: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub case_ids: Vec<String>,
}

// ── BatchStateStore ───────────────────────────────────────────────────────────

/// Simple filesystem-backed store for [`BatchStateRecord`]s.
///
/// Files are written to `{dir}/{batch_id}.json`. The directory is created if it
/// does not already exist.
pub struct BatchStateStore;

impl BatchStateStore {
    /// Serialise `record` and write it to `{dir}/{batch_id}.json`.
    ///
    /// Creates `dir` (and all parents) if it does not exist.
    pub fn save(record: &BatchStateRecord, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", record.batch_id));
        let json = serde_json::to_string_pretty(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, json)
    }

    /// Read and deserialise `{dir}/{batch_id}.json`.
    pub fn load(batch_id: &str, dir: &std::path::Path) -> std::io::Result<BatchStateRecord> {
        let path = dir.join(format!("{batch_id}.json"));
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

// ── AnthropicBatch ────────────────────────────────────────────────────────────

use anthropic_sdk::{
    types::ContentBlock, Anthropic, BatchCreateParams, BatchRequest, BatchResponseBody,
};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// Concrete [`BatchDispatcher`] that submits cases to the Anthropic Messages Batch API.
///
/// Chunks items into slices of at most 50,000 (the Anthropic per-batch limit), creates
/// one batch per chunk, polls for completion with a progress bar, and collects results.
/// State is persisted to `.agc/batch_state/` after each batch creation so that a
/// crashed run can be resumed.
pub struct AnthropicBatch {
    api_key: String,
}

impl AnthropicBatch {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl BatchDispatcher for AnthropicBatch {
    fn dispatch(
        &self,
        items: Vec<CaseBatchItem>,
    ) -> impl std::future::Future<Output = Result<Vec<BatchCaseResult>, BatchError>> + Send {
        let api_key = self.api_key.clone();
        async move {
            let sdk = Anthropic::new(&api_key)
                .map_err(|e| BatchError::Fatal(format!("anthropic client init failed: {e}")))?;

            const CHUNK_SIZE: usize = 50_000;
            let mut all_results: Vec<BatchCaseResult> = Vec::with_capacity(items.len());

            for chunk in items.chunks(CHUNK_SIZE) {
                // Build one BatchRequest per CaseBatchItem.
                let requests: Vec<BatchRequest> = chunk
                    .iter()
                    .map(|item| {
                        let mut b = BatchRequest::new(
                            item.case_id.0.as_str(),
                            &item.model,
                            item.max_tokens,
                        )
                        .temperature(0.2)
                        .user(&item.user_prompt);
                        if !item.system.is_empty() {
                            b = b.system(&item.system);
                        }
                        b.build()
                    })
                    .collect();

                let params = BatchCreateParams::new(requests);

                // Create the batch.
                let batch = sdk
                    .batches()
                    .create(params)
                    .await
                    .map_err(|e| BatchError::Fatal(format!("batch create failed: {e}")))?;

                let batch_id = batch.id.clone();

                // Persist state for resumability.
                let state = BatchStateRecord {
                    batch_id: batch_id.clone(),
                    provider: "anthropic".to_string(),
                    status: "in_progress".to_string(),
                    created_at: chrono::Utc::now(),
                    case_ids: chunk.iter().map(|i| i.case_id.0.clone()).collect(),
                };
                let state_dir = std::path::PathBuf::from(".agc/batch_state");
                let _ = BatchStateStore::save(&state, &state_dir);

                // Progress bar for this chunk.
                let pb = ProgressBar::new(chunk.len() as u64);
                pb.set_style(
                    ProgressStyle::with_template(
                        "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} batch cases",
                    )
                    .expect("progress template")
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
                );
                pb.enable_steady_tick(Duration::from_millis(120));

                let pb_for_monitor = pb.clone();
                // monitor_progress callback: (percentage, completed, total)
                sdk.batches()
                    .monitor_progress(
                        &batch_id,
                        move |_pct, completed, _total| {
                            pb_for_monitor.set_position(u64::from(completed));
                        },
                        Some(Duration::from_secs(10)),
                    )
                    .await
                    .map_err(|e| BatchError::Transient(format!("batch monitor failed: {e}")))?;

                pb.finish_and_clear();

                // Collect results.
                let results =
                    sdk.batches().get_results(&batch_id).await.map_err(|e| {
                        BatchError::Transient(format!("batch get_results failed: {e}"))
                    })?;

                // Map BatchResult → BatchCaseResult.
                for result in results {
                    let case_id = agentcarousel_core::CaseId(result.custom_id.clone());
                    let batch_case_result = match result.response.body {
                        BatchResponseBody::Success(msg) => {
                            let output = msg.content.iter().find_map(|block| {
                                if let ContentBlock::Text { text } = block {
                                    let t = text.trim().to_string();
                                    if !t.is_empty() {
                                        Some(t)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            });
                            BatchCaseResult {
                                case_id,
                                output,
                                tokens_in: Some(msg.usage.input_tokens as u64),
                                tokens_out: Some(msg.usage.output_tokens as u64),
                                error: None,
                            }
                        }
                        BatchResponseBody::Error(e) => BatchCaseResult {
                            case_id,
                            output: None,
                            tokens_in: None,
                            tokens_out: None,
                            error: Some(e.message),
                        },
                    };
                    all_results.push(batch_case_result);
                }
            }

            Ok(all_results)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn batch_error_is_fatal() {
        assert!(BatchError::Fatal("oops".to_string()).is_fatal());
        assert!(!BatchError::Transient("retry".to_string()).is_fatal());
    }

    #[test]
    fn batch_error_display() {
        assert_eq!(
            BatchError::Fatal("bad key".to_string()).to_string(),
            "bad key"
        );
        assert_eq!(
            BatchError::Transient("rate limit".to_string()).to_string(),
            "rate limit"
        );
    }

    #[test]
    fn batch_state_store_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let record = BatchStateRecord {
            batch_id: "test-batch-001".to_string(),
            provider: "anthropic".to_string(),
            status: "in_progress".to_string(),
            created_at: Utc::now(),
            case_ids: vec!["skill/case-1".to_string(), "skill/case-2".to_string()],
        };
        BatchStateStore::save(&record, dir.path()).unwrap();
        let loaded = BatchStateStore::load("test-batch-001", dir.path()).unwrap();
        assert_eq!(loaded.batch_id, record.batch_id);
        assert_eq!(loaded.provider, record.provider);
        assert_eq!(loaded.status, record.status);
        assert_eq!(loaded.case_ids, record.case_ids);
    }

    #[test]
    fn batch_state_store_creates_dir() {
        let base = tempfile::tempdir().unwrap();
        let nested = base.path().join("a").join("b").join("batch_state");
        let record = BatchStateRecord {
            batch_id: "nested-test".to_string(),
            provider: "openai".to_string(),
            status: "completed".to_string(),
            created_at: Utc::now(),
            case_ids: vec![],
        };
        BatchStateStore::save(&record, &nested).unwrap();
        assert!(nested.join("nested-test.json").exists());
    }

    #[test]
    fn batch_state_store_load_missing_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = BatchStateStore::load("nonexistent", dir.path());
        assert!(result.is_err());
    }
}
