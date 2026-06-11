//! Batch generation infrastructure — foundational types and trait for batch API dispatchers.
//!
//! This module defines the scaffolding that Anthropic and OpenAI batch dispatchers
//! (implemented in downstream tasks agc-bhgd and agc-lvjv) will plug into.
//!
//! Nothing in this module calls an external API — it is purely types, a trait, and
//! a simple JSON-backed state store for resumability.

use std::{collections::HashMap, fmt};

// ── sanitize_custom_id ────────────────────────────────────────────────────────

/// Map a case ID to a valid Anthropic/OpenAI batch `custom_id`.
///
/// Both APIs require `^[a-zA-Z0-9_-]{1,64}$`. Case IDs often contain `/`
/// (e.g. `skill-id/scenario-name`) which violates that pattern. We replace
/// every illegal character with `_` and truncate to 64 bytes. Because
/// sanitization can be lossy, callers must build a reverse map
/// (`sanitized → original CaseId`) before dispatching and use it when
/// reconstructing results.
fn sanitize_custom_id(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Truncate to 64 bytes (all chars are ASCII here so bytes == chars).
    sanitized[..sanitized.len().min(64)].to_string()
}

// ── CaseBatchItem ─────────────────────────────────────────────────────────────

/// One unit of work submitted to a batch dispatcher.
pub struct CaseBatchItem {
    pub case_id: agentcarousel_core::CaseId,
    pub system: String,
    /// Conversation turns from the fixture (system turns are carried in `system`).
    pub messages: Vec<agentcarousel_core::Message>,
    /// Optional structured context from the fixture's `input.context`.
    pub context: Option<serde_json::Value>,
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
    /// Original fixture file paths — used by `agc batch fetch` to re-read fixtures.
    #[serde(default)]
    pub fixture_paths: Vec<String>,
    /// Generator model used when the batch was submitted.
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Judge model to use when fetching results (None = no judge scoring).
    #[serde(default)]
    pub judge_model: Option<String>,
}

fn default_max_tokens() -> u32 {
    2048
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

use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Concrete [`BatchDispatcher`] that submits cases to the Anthropic Messages Batch API.
///
/// Chunks items into slices of at most 50,000 (the Anthropic per-batch limit), creates
/// one batch per chunk, polls for completion with a progress bar, and collects results.
/// State is persisted to `.agc/batch_state/` after each batch creation so that a
/// crashed run can be resumed.
///
/// Uses raw `reqwest`. `anthropic-sdk-rust 0.1.1`'s `BatchesResource` is broken — its
/// HTTP helpers pass relative paths straight to `reqwest::Client::post`, which fails
/// with a builder error before the request is sent. Hand-rolling the three calls
/// (`POST /v1/messages/batches`, `GET /v1/messages/batches/{id}`, `GET {results_url}`)
/// is cheaper than vendoring the SDK.
pub struct AnthropicBatch {
    api_key: String,
}

impl AnthropicBatch {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl AnthropicBatch {
    /// Submit `items` to the Anthropic batch API and return the batch ID.
    /// Saves a minimal [`BatchStateRecord`] for resumability; callers that need
    /// richer metadata (fixture paths, judge config) should update the record
    /// after calling this.
    pub async fn submit_only(
        &self,
        items: &[CaseBatchItem],
        client: &reqwest::Client,
    ) -> Result<(String, Vec<String>), BatchError> {
        let id_map: HashMap<String, agentcarousel_core::CaseId> = items
            .iter()
            .map(|item| (sanitize_custom_id(&item.case_id.0), item.case_id.clone()))
            .collect();

        let requests: Vec<serde_json::Value> = items
            .iter()
            .map(|item| {
                let messages: Vec<serde_json::Value> =
                    super::generator::messages_to_turns(&item.messages, item.context.as_ref())
                        .into_iter()
                        .map(
                            |(role, content)| serde_json::json!({"role": role, "content": content}),
                        )
                        .collect();
                let mut params = serde_json::json!({
                    "model": item.model,
                    "max_tokens": item.max_tokens,
                    "temperature": 0.2,
                    "messages": messages,
                });
                if !item.system.is_empty() {
                    params["system"] = serde_json::Value::String(item.system.clone());
                }
                serde_json::json!({
                    "custom_id": sanitize_custom_id(&item.case_id.0),
                    "params": params,
                })
            })
            .collect();

        let create_resp = client
            .post("https://api.anthropic.com/v1/messages/batches")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await
            .map_err(|e| BatchError::Fatal(format!("batch create request failed: {e}")))?;

        let create_status = create_resp.status();
        let create_json: serde_json::Value = create_resp
            .json()
            .await
            .map_err(|e| BatchError::Fatal(format!("batch create response parse failed: {e}")))?;

        if !create_status.is_success() {
            return Err(BatchError::Fatal(format!(
                "batch create HTTP {create_status}: {}",
                create_json
            )));
        }

        let batch_id = create_json["id"]
            .as_str()
            .ok_or_else(|| BatchError::Fatal("batch create response missing 'id'".to_string()))?
            .to_string();

        // Persist minimal state — callers may enrich it with fixture_paths etc.
        let case_ids: Vec<String> = items.iter().map(|i| i.case_id.0.clone()).collect();
        let state = BatchStateRecord {
            batch_id: batch_id.clone(),
            provider: "anthropic".to_string(),
            status: "in_progress".to_string(),
            created_at: chrono::Utc::now(),
            case_ids: case_ids.clone(),
            fixture_paths: Vec::new(),
            model: items.first().map(|i| i.model.clone()).unwrap_or_default(),
            max_tokens: items.first().map(|i| i.max_tokens).unwrap_or(2048),
            judge_model: None,
        };
        let _ = BatchStateStore::save(&state, &std::path::PathBuf::from(".agc/batch_state"));

        // Return (batch_id, sanitized-ids in order) so the caller can reconstruct the id_map.
        let sanitized_ids: Vec<String> = id_map.keys().cloned().collect();
        drop(sanitized_ids); // not needed — case_ids is what matters
        Ok((batch_id, case_ids))
    }

    /// Poll until the batch ends, then download and parse the JSONL results.
    ///
    /// `case_ids` must be the original (un-sanitized) case IDs in submission order.
    pub async fn collect_batch(
        &self,
        batch_id: &str,
        case_ids: &[String],
        client: &reqwest::Client,
        total: usize,
    ) -> Result<Vec<BatchCaseResult>, BatchError> {
        // Rebuild id_map from stored original IDs.
        let id_map: HashMap<String, agentcarousel_core::CaseId> = case_ids
            .iter()
            .map(|id| {
                (
                    sanitize_custom_id(id),
                    agentcarousel_core::CaseId(id.clone()),
                )
            })
            .collect();

        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} batch cases",
            )
            .expect("progress template")
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.enable_steady_tick(Duration::from_millis(120));

        let backoff_sequence: [u64; 5] = [5, 10, 20, 40, 60];
        let mut backoff_idx = 0usize;

        let results_url = loop {
            tokio::time::sleep(Duration::from_secs(backoff_sequence[backoff_idx])).await;
            if backoff_idx < backoff_sequence.len() - 1 {
                backoff_idx += 1;
            }

            let poll_resp = client
                .get(format!(
                    "https://api.anthropic.com/v1/messages/batches/{batch_id}"
                ))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_API_VERSION)
                .send()
                .await
                .map_err(|e| BatchError::Transient(format!("batch poll request failed: {e}")))?;

            let poll_status = poll_resp.status();
            let poll_json: serde_json::Value = poll_resp
                .json()
                .await
                .map_err(|e| BatchError::Transient(format!("batch poll parse failed: {e}")))?;

            if !poll_status.is_success() {
                return Err(BatchError::Transient(format!(
                    "batch poll HTTP {poll_status}: {}",
                    poll_json
                )));
            }

            let processing_status = poll_json["processing_status"].as_str().unwrap_or("unknown");
            let done = poll_json["request_counts"]["succeeded"]
                .as_u64()
                .unwrap_or(0)
                + poll_json["request_counts"]["errored"].as_u64().unwrap_or(0)
                + poll_json["request_counts"]["canceled"]
                    .as_u64()
                    .unwrap_or(0)
                + poll_json["request_counts"]["expired"].as_u64().unwrap_or(0);
            pb.set_position(done);

            if processing_status == "ended" {
                let url = poll_json["results_url"]
                    .as_str()
                    .ok_or_else(|| {
                        BatchError::Transient("ended batch missing 'results_url'".to_string())
                    })?
                    .to_string();
                break url;
            }
        };

        pb.finish_and_clear();

        let results_resp = client
            .get(&results_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .send()
            .await
            .map_err(|e| BatchError::Transient(format!("results download failed: {e}")))?;

        if !results_resp.status().is_success() {
            return Err(BatchError::Transient(format!(
                "results download HTTP {}",
                results_resp.status()
            )));
        }

        let results_text = results_resp
            .text()
            .await
            .map_err(|e| BatchError::Transient(format!("results body read failed: {e}")))?;

        let mut all_results: Vec<BatchCaseResult> = Vec::with_capacity(total);
        for raw_line in results_text.lines() {
            let raw_line = raw_line.trim();
            if raw_line.is_empty() {
                continue;
            }

            let line: serde_json::Value = serde_json::from_str(raw_line)
                .map_err(|e| BatchError::Transient(format!("results JSONL parse error: {e}")))?;

            let custom_id = line["custom_id"].as_str().unwrap_or("").to_string();
            let case_id = id_map
                .get(&custom_id)
                .cloned()
                .unwrap_or(agentcarousel_core::CaseId(custom_id));
            let result = &line["result"];
            let result_type = result["type"].as_str().unwrap_or("");

            let batch_case_result = match result_type {
                "succeeded" => {
                    let message = &result["message"];
                    let output = message["content"]
                        .as_array()
                        .and_then(|blocks| {
                            blocks.iter().find_map(|block| {
                                if block["type"].as_str() == Some("text") {
                                    block["text"].as_str().map(|s| s.trim().to_string())
                                } else {
                                    None
                                }
                            })
                        })
                        .filter(|s| !s.is_empty());
                    BatchCaseResult {
                        case_id,
                        output,
                        tokens_in: message["usage"]["input_tokens"].as_u64(),
                        tokens_out: message["usage"]["output_tokens"].as_u64(),
                        error: None,
                    }
                }
                _ => {
                    let err_msg = result["error"]["message"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!("batch result type='{result_type}' had no error message")
                        });
                    BatchCaseResult {
                        case_id,
                        output: None,
                        tokens_in: None,
                        tokens_out: None,
                        error: Some(err_msg),
                    }
                }
            };
            all_results.push(batch_case_result);
        }

        Ok(all_results)
    }
}

impl BatchDispatcher for AnthropicBatch {
    fn dispatch(
        &self,
        items: Vec<CaseBatchItem>,
    ) -> impl std::future::Future<Output = Result<Vec<BatchCaseResult>, BatchError>> + Send {
        let api_key = self.api_key.clone();
        async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| BatchError::Fatal(format!("reqwest client build failed: {e}")))?;

            const CHUNK_SIZE: usize = 50_000;
            let mut all_results: Vec<BatchCaseResult> = Vec::with_capacity(items.len());
            let dispatcher = AnthropicBatch { api_key };

            for chunk in items.chunks(CHUNK_SIZE) {
                let (batch_id, case_ids) = dispatcher.submit_only(chunk, &client).await?;
                let chunk_results = dispatcher
                    .collect_batch(&batch_id, &case_ids, &client, chunk.len())
                    .await?;
                all_results.extend(chunk_results);
            }

            Ok(all_results)
        }
    }
}

// ── OpenAiBatch ───────────────────────────────────────────────────────────────

/// Concrete [`BatchDispatcher`] that submits cases to the OpenAI Batch API.
///
/// Three-phase flow:
/// 1. Serialize all items as JSONL and upload to the Files API (`purpose=batch`).
/// 2. Create a batch job referencing the uploaded file; persist a [`BatchStateRecord`].
/// 3. Poll the batch status with exponential back-off (indicatif spinner), download
///    and parse the output JSONL when the batch completes.
///
/// Uses raw `reqwest` — no OpenAI Rust SDK.
pub struct OpenAiBatch {
    api_key: String,
}

impl OpenAiBatch {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl BatchDispatcher for OpenAiBatch {
    fn dispatch(
        &self,
        items: Vec<CaseBatchItem>,
    ) -> impl std::future::Future<Output = Result<Vec<BatchCaseResult>, BatchError>> + Send {
        let api_key = self.api_key.clone();
        async move {
            // Build a dedicated reqwest client with a 60-second timeout.
            // We intentionally do NOT reuse the shared ASYNC_CLIENT from generator.rs
            // because it has a 30-second timeout which is far too short for batch polling.
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| BatchError::Fatal(format!("reqwest client build failed: {e}")))?;

            // ── Phase 1: Serialize items as JSONL and upload to Files API ─────

            // Build sanitized-id → original-CaseId map for result recovery.
            let id_map: HashMap<String, agentcarousel_core::CaseId> = items
                .iter()
                .map(|item| (sanitize_custom_id(&item.case_id.0), item.case_id.clone()))
                .collect();

            let mut jsonl = String::new();
            for item in &items {
                let mut messages =
                    vec![serde_json::json!({"role": "system", "content": item.system})];
                messages.extend(
                    super::generator::messages_to_turns(&item.messages, item.context.as_ref())
                        .into_iter()
                        .map(
                            |(role, content)| serde_json::json!({"role": role, "content": content}),
                        ),
                );
                let line = serde_json::json!({
                    "custom_id": sanitize_custom_id(&item.case_id.0),
                    "method": "POST",
                    "url": "/v1/chat/completions",
                    "body": {
                        "model": item.model,
                        "messages": messages,
                        "temperature": 0.2,
                        "max_tokens": item.max_tokens
                    }
                });
                jsonl.push_str(&line.to_string());
                jsonl.push('\n');
            }
            let jsonl_bytes = jsonl.into_bytes();

            let form = reqwest::multipart::Form::new()
                .part("purpose", reqwest::multipart::Part::text("batch"))
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(jsonl_bytes)
                        .file_name("batch.jsonl")
                        .mime_str("application/jsonl")
                        .unwrap(),
                );

            let upload_resp = client
                .post("https://api.openai.com/v1/files")
                .bearer_auth(&api_key)
                .multipart(form)
                .send()
                .await
                .map_err(|e| BatchError::Fatal(format!("file upload request failed: {e}")))?;

            let upload_status = upload_resp.status();
            let upload_body: serde_json::Value = upload_resp.json().await.map_err(|e| {
                BatchError::Fatal(format!("file upload response parse failed: {e}"))
            })?;

            if !upload_status.is_success() {
                return Err(BatchError::Fatal(format!(
                    "file upload HTTP {upload_status}: {}",
                    upload_body
                )));
            }

            let file_id = upload_body["id"]
                .as_str()
                .ok_or_else(|| BatchError::Fatal("file upload response missing 'id'".to_string()))?
                .to_string();

            // ── Phase 2: Create the batch job ─────────────────────────────────

            let create_body = serde_json::json!({
                "input_file_id": file_id,
                "endpoint": "/v1/chat/completions",
                "completion_window": "24h"
            });

            let create_resp = client
                .post("https://api.openai.com/v1/batches")
                .bearer_auth(&api_key)
                .json(&create_body)
                .send()
                .await
                .map_err(|e| BatchError::Fatal(format!("batch create request failed: {e}")))?;

            let create_status = create_resp.status();
            let create_json: serde_json::Value = create_resp.json().await.map_err(|e| {
                BatchError::Fatal(format!("batch create response parse failed: {e}"))
            })?;

            if !create_status.is_success() {
                return Err(BatchError::Fatal(format!(
                    "batch create HTTP {create_status}: {}",
                    create_json
                )));
            }

            let batch_id = create_json["id"]
                .as_str()
                .ok_or_else(|| BatchError::Fatal("batch create response missing 'id'".to_string()))?
                .to_string();

            // Persist state for resumability.
            let state = BatchStateRecord {
                batch_id: batch_id.clone(),
                provider: "openai".to_string(),
                status: "in_progress".to_string(),
                created_at: chrono::Utc::now(),
                case_ids: items.iter().map(|i| i.case_id.0.clone()).collect(),
                fixture_paths: Vec::new(),
                model: items.first().map(|i| i.model.clone()).unwrap_or_default(),
                max_tokens: items.first().map(|i| i.max_tokens).unwrap_or(2048),
                judge_model: None,
            };
            let state_dir = std::path::PathBuf::from(".agc/batch_state");
            let _ = BatchStateStore::save(&state, &state_dir);

            // ── Phase 3: Poll until complete, then download results ────────────

            // Indicatif spinner — we don't know total until the batch completes.
            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {msg}")
                    .expect("spinner template")
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );
            spinner.enable_steady_tick(Duration::from_millis(120));
            spinner.set_message(format!("OpenAI batch {batch_id}: waiting…"));

            // Exponential back-off: 5s → 10s → 20s → 40s → 60s (cap).
            let backoff_sequence: [u64; 5] = [5, 10, 20, 40, 60];
            let mut backoff_idx = 0usize;

            let output_file_id = loop {
                tokio::time::sleep(Duration::from_secs(backoff_sequence[backoff_idx])).await;
                if backoff_idx < backoff_sequence.len() - 1 {
                    backoff_idx += 1;
                }

                let poll_resp = client
                    .get(format!("https://api.openai.com/v1/batches/{batch_id}"))
                    .bearer_auth(&api_key)
                    .send()
                    .await
                    .map_err(|e| {
                        BatchError::Transient(format!("batch poll request failed: {e}"))
                    })?;

                let poll_status = poll_resp.status();
                let poll_json: serde_json::Value = poll_resp.json().await.map_err(|e| {
                    BatchError::Transient(format!("batch poll response parse failed: {e}"))
                })?;

                if !poll_status.is_success() {
                    return Err(BatchError::Transient(format!(
                        "batch poll HTTP {poll_status}: {}",
                        poll_json
                    )));
                }

                let status = poll_json["status"].as_str().unwrap_or("unknown");
                spinner.set_message(format!("OpenAI batch {batch_id}: {status}"));

                match status {
                    "completed" => {
                        let ofid = poll_json["output_file_id"]
                            .as_str()
                            .ok_or_else(|| {
                                BatchError::Transient(
                                    "completed batch missing 'output_file_id'".to_string(),
                                )
                            })?
                            .to_string();
                        break ofid;
                    }
                    "failed" | "expired" | "cancelled" => {
                        spinner.finish_and_clear();
                        return Err(BatchError::Fatal(format!(
                            "OpenAI batch {batch_id} ended with status '{status}'"
                        )));
                    }
                    // "validating" | "in_progress" | "finalizing" | "cancelling" | "queued"
                    _ => {}
                }
            };

            spinner.finish_and_clear();

            // Download the output file.
            let output_resp = client
                .get(format!(
                    "https://api.openai.com/v1/files/{output_file_id}/content"
                ))
                .bearer_auth(&api_key)
                .send()
                .await
                .map_err(|e| {
                    BatchError::Transient(format!("output file download request failed: {e}"))
                })?;

            let output_status = output_resp.status();
            if !output_status.is_success() {
                return Err(BatchError::Transient(format!(
                    "output file download HTTP {output_status}"
                )));
            }

            let output_text = output_resp
                .text()
                .await
                .map_err(|e| BatchError::Transient(format!("output file read failed: {e}")))?;

            // Parse JSONL output. Each line is either a success or an error result.
            // Partial failures are handled gracefully — we collect all lines.
            let mut all_results: Vec<BatchCaseResult> = Vec::with_capacity(items.len());

            for raw_line in output_text.lines() {
                let raw_line = raw_line.trim();
                if raw_line.is_empty() {
                    continue;
                }

                let line: serde_json::Value = serde_json::from_str(raw_line)
                    .map_err(|e| BatchError::Transient(format!("output JSONL parse error: {e}")))?;

                let custom_id = line["custom_id"].as_str().unwrap_or("").to_string();
                // Recover original CaseId via the id_map.
                let case_id = id_map
                    .get(&custom_id)
                    .cloned()
                    .unwrap_or(agentcarousel_core::CaseId(custom_id));

                // Check for a top-level error field first.
                if let Some(err_msg) = line["error"]["message"].as_str() {
                    all_results.push(BatchCaseResult {
                        case_id,
                        output: None,
                        tokens_in: None,
                        tokens_out: None,
                        error: Some(err_msg.to_string()),
                    });
                    continue;
                }

                // Success path.
                let content = line["response"]["body"]["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());

                let tokens_in = line["response"]["body"]["usage"]["prompt_tokens"].as_u64();
                let tokens_out = line["response"]["body"]["usage"]["completion_tokens"].as_u64();

                // If there's no content and no error field, treat as an error.
                let (output, error) = if content.is_some() {
                    (content, None)
                } else {
                    let status_code = line["response"]["status_code"].as_u64().unwrap_or(0);
                    (
                        None,
                        Some(format!(
                            "no content in response (status_code={status_code})"
                        )),
                    )
                };

                all_results.push(BatchCaseResult {
                    case_id,
                    output,
                    tokens_in,
                    tokens_out,
                    error,
                });
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
            fixture_paths: vec![],
            model: "claude-3-5-sonnet-latest".to_string(),
            max_tokens: 2048,
            judge_model: None,
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
            fixture_paths: vec![],
            model: String::new(),
            max_tokens: 2048,
            judge_model: None,
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
