use agentcarousel_core::{
    compute_backoff_ms, is_retryable_status, retry_policy, Case, Message, Role,
};
use serde_json::json;
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

use super::RunnerConfig;
use crate::providers::{
    AnthropicMessage, AnthropicRequest, AnthropicResponse, AnthropicSystemBlock, GeminiContent,
    GeminiGenerationConfig, GeminiPart, GeminiRequest, GeminiResponse, OpenAiMessage,
    OpenAiRequest, OpenAiResponse,
};

static ASYNC_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static CUSTOM_ASYNC_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn shared_client() -> &'static reqwest::Client {
    ASYNC_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest async client")
    })
}

fn shared_custom_client(timeout_secs: u64) -> reqwest::Client {
    // Return the cached client if the timeout matches the default; otherwise build a one-off.
    // The cached client covers the common case where all cases share the same RunnerConfig timeout.
    const DEFAULT_CUSTOM_TIMEOUT: u64 = 120;
    if timeout_secs == DEFAULT_CUSTOM_TIMEOUT {
        return CUSTOM_ASYNC_CLIENT
            .get_or_init(|| {
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(DEFAULT_CUSTOM_TIMEOUT))
                    .build()
                    .expect("reqwest custom async client")
            })
            .clone();
    }
    reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .expect("reqwest async client")
}

/// Generator error that distinguishes permanent failures from transient ones.
///
/// `Fatal` errors (bad API key, wrong model name, missing config) will repeat for every
/// subsequent case and trigger the generator circuit-breaker so remaining cases are skipped.
/// `Transient` errors (rate limits exhausted, server errors) may not recur.
#[derive(Debug)]
pub enum GeneratorError {
    /// Permanent failure — will affect every subsequent case.
    Fatal(String),
    /// Transient failure — may not recur on the next case.
    Transient(String),
}

impl GeneratorError {
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Fatal(_))
    }
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fatal(msg) | Self::Transient(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorProvider {
    Gemini,
    OpenAi,
    Anthropic,
    OpenRouter,
    /// Arbitrary HTTP endpoint; set `--generator-endpoint <URL>` to use.
    Custom,
}

impl GeneratorProvider {
    pub fn from_model(model: &str) -> Self {
        if model == "custom" || model.starts_with("custom/") || model.starts_with("ollama/") {
            return Self::Custom;
        }
        let normalized = model.to_ascii_lowercase();
        if normalized.starts_with("openrouter/") {
            return Self::OpenRouter;
        }
        // OpenRouter model IDs are typically provider-prefixed (e.g. anthropic/...)
        // and often include suffixes like :free.
        if normalized.contains(":free")
            || normalized.starts_with("anthropic/")
            || normalized.starts_with("google/")
            || normalized.starts_with("openai/")
        {
            return Self::OpenRouter;
        }
        if normalized.starts_with("claude") {
            return Self::Anthropic;
        }
        if normalized.starts_with("gpt")
            || normalized.starts_with("o1")
            || normalized.starts_with("o3")
            || normalized.starts_with("o4")
        {
            return Self::OpenAi;
        }
        Self::Gemini
    }

    pub fn key_candidates(self) -> &'static [&'static str] {
        match self {
            Self::Gemini => &[
                "AGENTCAROUSEL_GENERATOR_KEY",
                "agentcarousel_GENERATOR_KEY",
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "AGENTCAROUSEL_JUDGE_KEY",
                "agentcarousel_JUDGE_KEY",
            ],
            Self::OpenAi => &[
                "AGENTCAROUSEL_GENERATOR_KEY",
                "agentcarousel_GENERATOR_KEY",
                "OPENAI_API_KEY",
                "AGENTCAROUSEL_JUDGE_KEY",
                "agentcarousel_JUDGE_KEY",
            ],
            Self::Anthropic => &[
                "AGENTCAROUSEL_GENERATOR_KEY",
                "agentcarousel_GENERATOR_KEY",
                "ANTHROPIC_API_KEY",
                "AGENTCAROUSEL_JUDGE_KEY",
                "agentcarousel_JUDGE_KEY",
            ],
            Self::OpenRouter => &[
                "OPENROUTER_API_KEY",
                "AGENTCAROUSEL_GENERATOR_KEY",
                "agentcarousel_GENERATOR_KEY",
                "AGENTCAROUSEL_JUDGE_KEY",
                "agentcarousel_JUDGE_KEY",
            ],
            Self::Custom => &[],
        }
    }
}

#[derive(Debug)]
pub struct GenerationResult {
    pub output: String,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
}

pub async fn generate_case_output(
    case: &Case,
    config: &RunnerConfig,
) -> Result<GenerationResult, GeneratorError> {
    let model = config
        .generator_model
        .as_ref()
        .ok_or_else(|| GeneratorError::Fatal("generator model is not configured".to_string()))?;
    let provider = GeneratorProvider::from_model(model);
    let max_tokens = config.generator_max_tokens;

    if let GeneratorProvider::Custom = provider {
        let endpoint = config.generator_endpoint.as_deref().ok_or_else(|| {
            GeneratorError::Fatal(
                "--generator-endpoint <URL> is required when generator model is 'custom' or 'ollama/<name>'"
                    .to_string(),
            )
        })?;
        let model_name = model
            .strip_prefix("ollama/")
            .or_else(|| model.strip_prefix("custom/"))
            .filter(|s| !s.is_empty());
        return call_custom_endpoint(endpoint, model_name, case, config.timeout_secs, max_tokens)
            .await;
    }

    let key = resolve_generator_key(provider)?;
    let system = resolve_system_prompt(case);
    let turns = build_message_turns(case);
    match provider {
        GeneratorProvider::Gemini => {
            generate_with_gemini(&key, model, &system, &turns, max_tokens).await
        }
        GeneratorProvider::OpenAi => {
            generate_with_openai(&key, model, &system, &turns, max_tokens).await
        }
        GeneratorProvider::Anthropic => {
            generate_with_anthropic(&key, model, &system, &turns, max_tokens).await
        }
        GeneratorProvider::OpenRouter => {
            generate_with_openrouter(&key, model, &system, &turns, max_tokens).await
        }
        GeneratorProvider::Custom => unreachable!(),
    }
}

pub async fn call_custom_endpoint(
    endpoint: &str,
    model_name: Option<&str>,
    case: &Case,
    timeout_secs: u64,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    // Ollama's /api/generate takes a single prompt string; everything else gets messages.
    let is_ollama = endpoint.contains("/api/generate") || endpoint.contains("/api/chat");

    let body = if endpoint.contains("/api/generate") {
        let system = resolve_system_prompt(case);
        let user = build_user_prompt(case);
        let prompt = if system.is_empty() {
            user
        } else {
            format!("{system}\n\n{user}")
        };
        let mut b = serde_json::json!({
            "prompt": prompt,
            "stream": false,
            "think": false,
            "keep_alive": "10m",
        });
        if let Some(m) = model_name {
            b["model"] = serde_json::json!(m);
        }
        if let Some(n) = max_tokens {
            b["options"] = serde_json::json!({"num_predict": n});
        }
        b
    } else {
        let messages: Vec<serde_json::Value> = case
            .input
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "system",
                    Role::Tool => "tool",
                };
                serde_json::json!({"role": role, "content": m.content})
            })
            .collect();
        let mut b = serde_json::json!({"messages": messages});
        if is_ollama {
            b["keep_alive"] = serde_json::json!("10m");
        }
        if let Some(m) = model_name {
            b["model"] = serde_json::json!(m);
        }
        if let Some(n) = max_tokens {
            b["max_tokens"] = serde_json::json!(n);
        }
        b
    };

    let client = shared_custom_client(timeout_secs);
    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_connect() {
                format!("custom endpoint request failed: {e}. Ensure the model server at {endpoint} is running and reachable.")
            } else if e.is_timeout() {
                format!("custom endpoint request failed: {e}. The request timed out (timeout: {timeout_secs}s).")
            } else {
                format!("custom endpoint request failed: {e}")
            };
            GeneratorError::Transient(msg)
        })?;
    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(GeneratorError::Transient(format!(
            "custom endpoint returned {status}: {body_text}"
        )));
    }
    let json: serde_json::Value = response.json().await.map_err(|e| {
        GeneratorError::Transient(format!("custom endpoint response parse failed: {e}"))
    })?;
    // Accept OpenAI-compat, Ollama /api/generate ("response"), or generic ("output").
    let output = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["response"].as_str())
        .or_else(|| json["output"].as_str())
        .ok_or_else(|| {
            GeneratorError::Transient(
                "custom endpoint response missing 'choices[0].message.content', 'response', or 'output'"
                    .to_string(),
            )
        })?
        .to_string();
    Ok(GenerationResult {
        output,
        tokens_in: None,
        tokens_out: None,
    })
}

pub(super) fn resolve_generator_key(provider: GeneratorProvider) -> Result<String, GeneratorError> {
    let key = provider
        .key_candidates()
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .ok_or_else(|| {
            GeneratorError::Fatal(format!(
                "missing generator API key; set one of {}",
                provider.key_candidates().join(", ")
            ))
        })?;
    reqwest::header::HeaderValue::from_str(&key).map_err(|_| {
        GeneratorError::Fatal("generator API key contains invalid header characters".to_string())
    })?;
    Ok(key)
}

/// Resolve the system prompt for a case.
///
/// Priority:
///   1. An explicit `role: system` message in the fixture's input.messages.
///   2. `fixtures/<skill>/prompt.md` where skill is the prefix of the case ID before `/`.
///   3. A minimal generic fallback so generation still works for fixture-less cases.
pub(super) fn resolve_system_prompt(case: &Case) -> String {
    if let Some(msg) = case.input.messages.iter().find(|m| m.role == Role::System) {
        return msg.content.clone();
    }
    if let Some(text) = load_skill_prompt_for_case(case) {
        return text;
    }
    "You are an AI assistant. Respond with the best answer for the task.".to_string()
}

fn load_skill_prompt_for_case(case: &Case) -> Option<String> {
    let skill = case.id.0.split('/').next()?;
    let path = std::path::PathBuf::from("fixtures")
        .join(skill)
        .join("prompt.md");
    std::fs::read_to_string(&path)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Build the user-turn portion of the generation prompt (everything except the system message).
pub(super) fn build_user_prompt(case: &Case) -> String {
    let mut prompt = String::new();
    for message in &case.input.messages {
        if message.role == Role::System {
            continue; // consumed by resolve_system_prompt
        }
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => unreachable!(),
            Role::Tool => "tool",
        };
        prompt.push_str(&format!("[{role}] {}\n\n", message.content.trim()));
    }
    if let Some(context) = case.input.context.as_ref() {
        prompt.push_str("Context:\n");
        prompt.push_str(&context.to_string());
        prompt.push('\n');
    }
    prompt
}

/// Convert fixture messages into provider-ready `(role, content)` turns.
///
/// System messages are skipped (consumed by `resolve_system_prompt`) and tool
/// messages become user-side context — fixtures carry no `tool_call_id`, so
/// native tool-result formats are not representable. Consecutive same-role
/// turns are collapsed with a blank line because Anthropic requires strict
/// user/assistant alternation (harmless for the other providers). `context`
/// is appended to the last user turn, or becomes a synthetic user turn if
/// the conversation has none.
pub(super) fn messages_to_turns(
    messages: &[Message],
    context: Option<&serde_json::Value>,
) -> Vec<(String, String)> {
    let mut turns: Vec<(String, String)> = Vec::new();
    for message in messages {
        let role = match message.role {
            Role::System => continue, // consumed by resolve_system_prompt
            Role::User | Role::Tool => "user",
            Role::Assistant => "assistant",
        };
        let content = message.content.trim();
        if content.is_empty() {
            continue; // providers reject empty content blocks
        }
        match turns.last_mut() {
            Some((last_role, last_content)) if last_role == role => {
                last_content.push_str("\n\n");
                last_content.push_str(content);
            }
            _ => turns.push((role.to_string(), content.to_string())),
        }
    }
    if let Some(context) = context {
        let context_block = format!("Context:\n{context}");
        match turns.iter_mut().rev().find(|(role, _)| role == "user") {
            Some((_, content)) => {
                content.push_str("\n\n");
                content.push_str(&context_block);
            }
            None => turns.push(("user".to_string(), context_block)),
        }
    }
    turns
}

/// Build the multi-turn message array for a case (everything except the system message).
pub(super) fn build_message_turns(case: &Case) -> Vec<(String, String)> {
    messages_to_turns(&case.input.messages, case.input.context.as_ref())
}

async fn generate_with_gemini(
    key: &str,
    model: &str,
    system: &str,
    turns: &[(String, String)],
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, key
    );
    let request = GeminiRequest {
        system_instruction: if system.is_empty() {
            None
        } else {
            Some(crate::providers::GeminiSystemInstruction {
                parts: vec![GeminiPart {
                    text: system.to_string(),
                }],
            })
        },
        contents: turns
            .iter()
            .map(|(role, content)| GeminiContent {
                // Gemini calls the assistant role "model".
                role: Some(if role == "assistant" { "model" } else { "user" }.to_string()),
                parts: vec![GeminiPart {
                    text: content.clone(),
                }],
            })
            .collect(),
        generation_config: GeminiGenerationConfig {
            temperature: 0.2,
            max_output_tokens: max_tokens,
            response_mime_type: None,
        },
    };
    let client = shared_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|err| GeneratorError::Transient(err.to_string()))?;
        let status = response.status();
        if status.is_success() {
            let body: GeminiResponse = response
                .json()
                .await
                .map_err(|err| GeneratorError::Transient(err.to_string()))?;
            let output = body
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.content.as_ref())
                .and_then(|content| content.parts.first())
                .map(|part| part.text.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    GeneratorError::Transient("gemini returned empty generation output".to_string())
                })?;
            return Ok(GenerationResult {
                output,
                tokens_in: body
                    .usage_metadata
                    .as_ref()
                    .and_then(|usage| usage.prompt_token_count),
                tokens_out: body
                    .usage_metadata
                    .as_ref()
                    .and_then(|usage| usage.candidates_token_count),
            });
        }

        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }
        let msg = format!("gemini generation failed ({status}): {body}");
        return Err(if retryable {
            GeneratorError::Transient(msg)
        } else {
            GeneratorError::Fatal(msg)
        });
    }

    Err(GeneratorError::Transient(
        "gemini generation failed after retries".to_string(),
    ))
}

async fn generate_with_openai(
    key: &str,
    model: &str,
    system: &str,
    turns: &[(String, String)],
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    let mut messages = Vec::with_capacity(turns.len() + 1);
    messages.push(OpenAiMessage {
        role: "system".to_string(),
        content: system.to_string(),
    });
    messages.extend(turns.iter().map(|(role, content)| OpenAiMessage {
        role: role.clone(),
        content: content.clone(),
    }));
    let request = OpenAiRequest {
        model: model.to_string(),
        messages,
        temperature: 0.2,
        max_tokens,
        response_format: None,
    };
    let client = shared_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(key)
            .json(&request)
            .send()
            .await
            .map_err(|err| GeneratorError::Transient(err.to_string()))?;
        let status = response.status();
        if status.is_success() {
            let body: OpenAiResponse = response
                .json()
                .await
                .map_err(|err| GeneratorError::Transient(err.to_string()))?;
            let output = body
                .choices
                .as_ref()
                .and_then(|v| v.first())
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.content.as_deref())
                .map(|s| s.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    GeneratorError::Transient("openai returned empty generation output".to_string())
                })?;
            return Ok(GenerationResult {
                output,
                tokens_in: body.usage.as_ref().and_then(|usage| usage.prompt_tokens),
                tokens_out: body
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.completion_tokens),
            });
        }
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }
        let msg = format!("openai generation failed ({status}): {body}");
        return Err(if retryable {
            GeneratorError::Transient(msg)
        } else {
            GeneratorError::Fatal(msg)
        });
    }
    Err(GeneratorError::Transient(
        "openai generation failed after retries".to_string(),
    ))
}

async fn generate_with_anthropic(
    key: &str,
    model: &str,
    system: &str,
    turns: &[(String, String)],
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    let max_tokens = max_tokens.ok_or_else(|| {
        GeneratorError::Fatal("max_tokens is required for Anthropic generation".to_string())
    })?;
    let system_blocks = if system.is_empty() {
        vec![]
    } else {
        vec![AnthropicSystemBlock::cached(system.to_string())]
    };
    let request = AnthropicRequest {
        model: model.to_string(),
        max_tokens,
        temperature: 0.2,
        system: system_blocks,
        // Turns already alternate strictly (messages_to_turns collapses
        // consecutive same-role messages), as the API requires.
        messages: turns
            .iter()
            .map(|(role, content)| AnthropicMessage {
                role: role.clone(),
                content: content.clone(),
            })
            .collect(),
    };
    let client = shared_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .json(&request)
            .send()
            .await
            .map_err(|err| GeneratorError::Transient(err.to_string()))?;
        let status = response.status();
        if status.is_success() {
            let body: AnthropicResponse = response
                .json()
                .await
                .map_err(|err| GeneratorError::Transient(err.to_string()))?;
            let output = body
                .content
                .as_ref()
                .and_then(|blocks| {
                    blocks
                        .iter()
                        .find(|b| b.block_type.as_deref() == Some("text"))
                })
                .and_then(|b| b.text.as_deref())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    GeneratorError::Transient(
                        "anthropic returned empty generation output".to_string(),
                    )
                })?;
            return Ok(GenerationResult {
                output,
                tokens_in: body.usage.as_ref().and_then(|u| u.input_tokens),
                tokens_out: body.usage.as_ref().and_then(|u| u.output_tokens),
            });
        }
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }
        let msg = format!("anthropic generation failed ({status}): {body}");
        return Err(if retryable {
            GeneratorError::Transient(msg)
        } else {
            GeneratorError::Fatal(msg)
        });
    }
    Err(GeneratorError::Transient(
        "anthropic generation failed after retries".to_string(),
    ))
}

async fn generate_with_openrouter(
    key: &str,
    model: &str,
    system: &str,
    turns: &[(String, String)],
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    let openrouter_model = model.strip_prefix("openrouter/").unwrap_or(model);
    let client = shared_client();
    let candidates = openrouter_model_candidates(openrouter_model);
    let mut last_error = None;
    for candidate in candidates {
        let mut messages = Vec::with_capacity(turns.len() + 1);
        if !system.is_empty() {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }
        messages.extend(turns.iter().map(|(role, content)| OpenAiMessage {
            role: role.clone(),
            content: content.clone(),
        }));
        let request = OpenAiRequest {
            model: candidate.to_string(),
            messages,
            temperature: 0.2,
            max_tokens,
            response_format: None,
        };
        let send_result = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(key)
            .header(
                "HTTP-Referer",
                "https://github.com/agentcarousel/agentcarousel",
            )
            .header("X-Title", "agentcarousel")
            .json(&request)
            .send()
            .await;
        let response = match send_result {
            Ok(r) => r,
            Err(err) => {
                last_error = Some(err.to_string());
                break;
            }
        };
        let status = response.status();
        if status.is_success() {
            let body: OpenAiResponse = response
                .json()
                .await
                .map_err(|err| GeneratorError::Transient(err.to_string()))?;
            let output = body
                .choices
                .as_ref()
                .and_then(|v| v.first())
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.content.as_deref())
                .map(|s| s.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    GeneratorError::Transient(
                        "openrouter returned empty generation output".to_string(),
                    )
                })?;
            return Ok(GenerationResult {
                output,
                tokens_in: body.usage.as_ref().and_then(|u| u.prompt_tokens),
                tokens_out: body.usage.as_ref().and_then(|u| u.completion_tokens),
            });
        }
        let body_text = response
            .text()
            .await
            .unwrap_or_else(|_| "unable to read error body".to_string());
        // For missing OpenRouter routes, try known model suffix variants.
        let retryable_model_miss =
            status.as_u16() == 404 || body_text.contains("No endpoints found");
        last_error = Some(format!(
            "openrouter generation failed ({status}): {body_text}"
        ));
        if retryable_model_miss {
            continue;
        }
        break;
    }

    Err(GeneratorError::Transient(last_error.unwrap_or_else(|| {
        "openrouter generation failed".to_string()
    })))
}

fn openrouter_model_candidates(model: &str) -> Vec<&str> {
    // Some OpenRouter aliases map to tiered routes; keep the list centralized here.
    if model == "openrouter/free" {
        return vec!["openrouter/free"];
    }
    vec![model]
}

pub fn generation_step_result(provider: GeneratorProvider, model: &str) -> serde_json::Value {
    json!({
        "provider": format!("{provider:?}").to_ascii_lowercase(),
        "model": model
    })
}

/// Call an LLM with an arbitrary prompt. Uses the same provider detection and key
/// resolution as `generate_case_output`. Intended for `agc generate` fixture synthesis
/// and `agc optimize` failure analysis / prompt synthesis.
///
/// `endpoint` is required when `model` is `custom/<name>` or `ollama/<name>`.
pub async fn call_llm(
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
    endpoint: Option<&str>,
) -> Result<GenerationResult, String> {
    let provider = GeneratorProvider::from_model(model);
    if let GeneratorProvider::Custom = provider {
        let ep = endpoint.ok_or_else(|| {
            "custom provider requires --generator-endpoint; not supported in agc generate"
                .to_string()
        })?;
        let model_name = model
            .strip_prefix("ollama/")
            .or_else(|| model.strip_prefix("custom/"))
            .filter(|s| !s.is_empty())
            .unwrap_or(model);
        return call_llm_custom(ep, model_name, prompt, max_tokens)
            .await
            .map_err(|e| e.to_string());
    }
    let key = resolve_generator_key(provider).map_err(|e| e.to_string())?;
    let turns = vec![("user".to_string(), prompt.to_string())];
    match provider {
        GeneratorProvider::Gemini => generate_with_gemini(&key, model, "", &turns, max_tokens)
            .await
            .map_err(|e| e.to_string()),
        GeneratorProvider::OpenAi => generate_with_openai(&key, model, "", &turns, max_tokens)
            .await
            .map_err(|e| e.to_string()),
        GeneratorProvider::Anthropic => {
            generate_with_anthropic(&key, model, "", &turns, max_tokens)
                .await
                .map_err(|e| e.to_string())
        }
        GeneratorProvider::OpenRouter => {
            generate_with_openrouter(&key, model, "", &turns, max_tokens)
                .await
                .map_err(|e| e.to_string())
        }
        GeneratorProvider::Custom => unreachable!(),
    }
}

async fn call_llm_custom(
    endpoint: &str,
    model_name: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, GeneratorError> {
    const TIMEOUT_SECS: u64 = 120;

    let is_ollama = endpoint.contains("/api/generate") || endpoint.contains("/api/chat");

    let mut options = serde_json::json!({});
    if let Some(n) = max_tokens {
        options["num_predict"] = serde_json::json!(n);
    }

    let body = if endpoint.contains("/api/generate") {
        let mut b = serde_json::json!({
            "model": model_name,
            "prompt": prompt,
            "think": false,
            "stream": false,
            "keep_alive": "10m",
        });
        if max_tokens.is_some() {
            b["options"] = options;
        }
        b
    } else {
        let mut b = serde_json::json!({
            "model": model_name,
            "messages": [{"role": "user", "content": prompt}],
            "think": false,
            "stream": false,
        });
        if is_ollama {
            b["keep_alive"] = serde_json::json!("10m");
        }
        if max_tokens.is_some() {
            if endpoint.contains("/v1/chat/completions") {
                b["max_tokens"] = serde_json::json!(max_tokens);
            } else {
                b["options"] = options;
            }
        }
        b
    };

    let client = shared_custom_client(TIMEOUT_SECS);

    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = if e.is_connect() {
                format!("custom endpoint request failed: {e}. Ensure the model server at {endpoint} is running and reachable.")
            } else if e.is_timeout() {
                format!("custom endpoint request failed: {e}. The request timed out (timeout: {TIMEOUT_SECS}s).")
            } else {
                format!("custom endpoint request failed: {e}")
            };
            GeneratorError::Transient(msg)
        })?;

    let status = response.status();

    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(GeneratorError::Transient(format!(
            "custom endpoint returned {status}: {body_text}"
        )));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| {
        GeneratorError::Transient(format!("custom endpoint response parse failed: {e}"))
    })?;

    // Ollama's native /api/chat returns response["message"]["content"]
    let output = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["message"]["content"].as_str()) // Added native Ollama /api/chat fallback
        .or_else(|| json["response"].as_str()) // Native /api/generate
        .or_else(|| json["output"].as_str())
        .ok_or_else(|| {
            GeneratorError::Transient(
                "custom endpoint response missing expected content fields".to_string(),
            )
        })?
        .to_string();

    Ok(GenerationResult {
        output,
        tokens_in: None,
        tokens_out: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentcarousel_core::{CaseId, CaseInput, Expected};

    fn message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
        }
    }

    fn make_case(messages: Vec<Message>, context: Option<serde_json::Value>) -> Case {
        Case {
            id: CaseId("test/case".to_string()),
            description: None,
            tags: vec![],
            input: CaseInput {
                messages,
                context,
                env_overrides: None,
            },
            expected: Expected {
                tool_sequence: None,
                output: None,
                rubric: None,
            },
            evaluator_config: None,
            timeout_secs: None,
            seed: None,
        }
    }

    #[test]
    fn test_build_message_turns_single_user() {
        let case = make_case(vec![message(Role::User, "hello")], None);
        let turns = build_message_turns(&case);
        assert_eq!(turns, vec![("user".to_string(), "hello".to_string())]);
    }

    #[test]
    fn test_build_message_turns_multiturn() {
        let case = make_case(
            vec![
                message(Role::System, "system prompt"),
                message(Role::User, "first"),
                message(Role::Assistant, "reply"),
                message(Role::User, "second"),
            ],
            None,
        );
        let turns = build_message_turns(&case);
        assert_eq!(
            turns,
            vec![
                ("user".to_string(), "first".to_string()),
                ("assistant".to_string(), "reply".to_string()),
                ("user".to_string(), "second".to_string()),
            ]
        );
    }

    #[test]
    fn test_build_message_turns_context_appended() {
        let case = make_case(
            vec![
                message(Role::User, "first"),
                message(Role::Assistant, "reply"),
                message(Role::User, "second"),
            ],
            Some(serde_json::json!({"patient_id": 42})),
        );
        let turns = build_message_turns(&case);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[2].0, "user");
        assert!(turns[2].1.starts_with("second"));
        assert!(turns[2].1.contains("Context:\n{\"patient_id\":42}"));
    }

    #[test]
    fn test_build_message_turns_tool_maps_to_user() {
        let case = make_case(
            vec![
                message(Role::User, "run the lookup"),
                message(Role::Assistant, "looking it up"),
                message(Role::Tool, "lookup result: 7"),
            ],
            None,
        );
        let turns = build_message_turns(&case);
        assert_eq!(
            turns,
            vec![
                ("user".to_string(), "run the lookup".to_string()),
                ("assistant".to_string(), "looking it up".to_string()),
                ("user".to_string(), "lookup result: 7".to_string()),
            ]
        );
    }

    #[test]
    fn test_build_message_turns_consecutive_same_role_collapsed() {
        let case = make_case(
            vec![
                message(Role::User, "msg1"),
                message(Role::User, "msg2"),
                message(Role::Assistant, "reply"),
            ],
            None,
        );
        let turns = build_message_turns(&case);
        assert_eq!(
            turns,
            vec![
                ("user".to_string(), "msg1\n\nmsg2".to_string()),
                ("assistant".to_string(), "reply".to_string()),
            ]
        );
    }

    #[test]
    fn test_build_message_turns_context_no_user_turn() {
        let case = make_case(
            vec![message(Role::System, "system only")],
            Some(serde_json::json!({"key": "value"})),
        );
        let turns = build_message_turns(&case);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].0, "user");
        assert!(turns[0].1.starts_with("Context:\n"));
    }

    /// Every bundled fixture must contain at least one multi-turn case, and
    /// each multi-turn case must convert into a well-formed alternating turn
    /// array that starts with a user turn.
    #[test]
    fn test_repo_fixtures_contain_multiturn_cases() {
        // Canonicalize: the loader rejects paths containing `..` components.
        let fixtures_root =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"))
                .canonicalize()
                .expect("fixtures/ directory resolves");
        let bundles: Vec<std::path::PathBuf> = std::fs::read_dir(fixtures_root)
            .expect("fixtures/ directory exists")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.join("cases.yaml").is_file())
            .collect();
        assert!(!bundles.is_empty(), "no fixture bundles found");

        for bundle in bundles {
            let fixture = crate::fixtures::load_fixture(&bundle.join("cases.yaml"))
                .unwrap_or_else(|e| panic!("failed to load {}: {e}", bundle.display()));

            let multiturn: Vec<&Case> = fixture
                .cases
                .iter()
                .filter(|case| {
                    let non_system = case
                        .input
                        .messages
                        .iter()
                        .filter(|m| m.role != Role::System)
                        .count();
                    non_system > 1
                        && case
                            .input
                            .messages
                            .iter()
                            .any(|m| m.role == Role::Assistant)
                })
                .collect();
            assert!(
                !multiturn.is_empty(),
                "bundle {} has no multi-turn case (>1 non-system message incl. an assistant turn)",
                bundle.display()
            );

            for case in multiturn {
                let turns = build_message_turns(case);
                assert!(turns.len() > 1, "{}: expected multiple turns", case.id.0);
                assert_eq!(turns[0].0, "user", "{}: first turn must be user", case.id.0);
                for pair in turns.windows(2) {
                    assert_ne!(
                        pair[0].0, pair[1].0,
                        "{}: turns must alternate strictly",
                        case.id.0
                    );
                }
                for (role, content) in &turns {
                    assert!(
                        role == "user" || role == "assistant",
                        "{}: unexpected provider role '{role}'",
                        case.id.0
                    );
                    assert!(!content.is_empty(), "{}: empty turn content", case.id.0);
                }
                if let Some(context) = case.input.context.as_ref() {
                    let last_user = turns
                        .iter()
                        .rev()
                        .find(|(role, _)| role == "user")
                        .expect("multi-turn case has a user turn");
                    assert!(
                        last_user.1.contains(&format!("Context:\n{context}")),
                        "{}: context missing from last user turn",
                        case.id.0
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn test_custom_endpoint_connection_failure() {
        let endpoint = "http://127.0.0.1:65530/api/generate";
        let case = Case {
            id: CaseId("test/case".to_string()),
            description: None,
            tags: vec![],
            input: CaseInput {
                messages: vec![Message {
                    role: Role::User,
                    content: "hello".to_string(),
                }],
                context: None,
                env_overrides: None,
            },
            expected: Expected {
                tool_sequence: None,
                output: None,
                rubric: None,
            },
            evaluator_config: None,
            timeout_secs: None,
            seed: None,
        };

        let result = call_custom_endpoint(endpoint, Some("test-model"), &case, 5, None).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Ensure the model server at"));
        assert!(err_msg.contains(endpoint));
    }
}
