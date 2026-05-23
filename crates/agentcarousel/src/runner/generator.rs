use agentcarousel_core::{compute_backoff_ms, is_retryable_status, retry_policy, Case, Role};
use serde_json::json;
use std::sync::OnceLock;
use std::time::Duration;

use super::RunnerConfig;
use crate::providers::{
    AnthropicMessage, AnthropicRequest, AnthropicResponse, GeminiContent, GeminiGenerationConfig,
    GeminiPart, GeminiRequest, GeminiResponse, OpenAiMessage, OpenAiRequest, OpenAiResponse,
};

static ASYNC_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn shared_client() -> &'static reqwest::Client {
    ASYNC_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest async client")
    })
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
        if model == "custom" {
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
) -> Result<GenerationResult, String> {
    let model = config
        .generator_model
        .as_ref()
        .ok_or_else(|| "generator model is not configured".to_string())?;
    let provider = GeneratorProvider::from_model(model);
    let max_tokens = config.generator_max_tokens;

    if let GeneratorProvider::Custom = provider {
        let endpoint = config.generator_endpoint.as_deref().ok_or_else(|| {
            "--generator-endpoint <URL> is required when --generator-model is 'custom'".to_string()
        })?;
        return call_custom_endpoint(endpoint, case, config.timeout_secs, max_tokens).await;
    }

    let key = resolve_generator_key(provider)?;
    let system = resolve_system_prompt(case);
    let user_prompt = build_user_prompt(case);
    match provider {
        GeneratorProvider::Gemini => {
            generate_with_gemini(&key, model, &system, &user_prompt, max_tokens).await
        }
        GeneratorProvider::OpenAi => {
            generate_with_openai(&key, model, &system, &user_prompt, max_tokens).await
        }
        GeneratorProvider::Anthropic => {
            generate_with_anthropic(&key, model, &system, &user_prompt, max_tokens).await
        }
        GeneratorProvider::OpenRouter => {
            generate_with_openrouter(&key, model, &system, &user_prompt, max_tokens).await
        }
        GeneratorProvider::Custom => unreachable!(),
    }
}

pub async fn call_custom_endpoint(
    endpoint: &str,
    case: &Case,
    timeout_secs: u64,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
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
    let body = serde_json::json!({
        "messages": messages,
        "max_tokens": max_tokens.unwrap_or(2048),
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("custom endpoint request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(format!("custom endpoint returned {status}: {body_text}"));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("custom endpoint response parse failed: {e}"))?;
    let output = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["output"].as_str())
        .ok_or_else(|| {
            "custom endpoint response missing 'choices[0].message.content' or 'output'".to_string()
        })?
        .to_string();
    Ok(GenerationResult {
        output,
        tokens_in: None,
        tokens_out: None,
    })
}

fn resolve_generator_key(provider: GeneratorProvider) -> Result<String, String> {
    let key = provider
        .key_candidates()
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .ok_or_else(|| {
            format!(
                "missing generator API key; set one of {}",
                provider.key_candidates().join(", ")
            )
        })?;
    reqwest::header::HeaderValue::from_str(&key)
        .map_err(|_| "generator API key contains invalid header characters".to_string())?;
    Ok(key)
}

/// Resolve the system prompt for a case.
///
/// Priority:
///   1. An explicit `role: system` message in the fixture's input.messages.
///   2. `fixtures/<skill>/prompt.md` where skill is the prefix of the case ID before `/`.
///   3. A minimal generic fallback so generation still works for fixture-less cases.
fn resolve_system_prompt(case: &Case) -> String {
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
fn build_user_prompt(case: &Case) -> String {
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

async fn generate_with_gemini(
    key: &str,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
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
        contents: vec![GeminiContent {
            role: Some("user".to_string()),
            parts: vec![GeminiPart {
                text: prompt.to_string(),
            }],
        }],
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
            .map_err(|err| err.to_string())?;
        let status = response.status();
        if status.is_success() {
            let body: GeminiResponse = response.json().await.map_err(|err| err.to_string())?;
            let output = body
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.content.as_ref())
                .and_then(|content| content.parts.first())
                .map(|part| part.text.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| "gemini returned empty generation output".to_string())?;
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
        return Err(format!("gemini generation failed ({status}): {body}"));
    }

    Err("gemini generation failed after retries".to_string())
}

async fn generate_with_openai(
    key: &str,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let request = OpenAiRequest {
        model: model.to_string(),
        messages: vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: system.to_string(),
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
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
            .map_err(|err| err.to_string())?;
        let status = response.status();
        if status.is_success() {
            let body: OpenAiResponse = response.json().await.map_err(|err| err.to_string())?;
            let output = body
                .choices
                .as_ref()
                .and_then(|v| v.first())
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.content.as_deref())
                .map(|s| s.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| "openai returned empty generation output".to_string())?;
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
        return Err(format!("openai generation failed ({status}): {body}"));
    }
    Err("openai generation failed after retries".to_string())
}

async fn generate_with_anthropic(
    key: &str,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let max_tokens =
        max_tokens.ok_or_else(|| "max_tokens is required for Anthropic generation".to_string())?;
    let request = AnthropicRequest {
        model: model.to_string(),
        max_tokens,
        temperature: 0.2,
        system: system.to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
    };
    let client = shared_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let status = response.status();
        if status.is_success() {
            let body: AnthropicResponse = response.json().await.map_err(|err| err.to_string())?;
            let empty: Vec<_> = Vec::new();
            let output = body
                .content
                .as_deref()
                .unwrap_or(&empty)
                .iter()
                .find(|block| block.block_type.as_deref() == Some("text"))
                .and_then(|block| block.text.as_ref())
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| "anthropic returned empty generation output".to_string())?;
            return Ok(GenerationResult {
                output,
                tokens_in: body.usage.as_ref().and_then(|usage| usage.input_tokens),
                tokens_out: body.usage.as_ref().and_then(|usage| usage.output_tokens),
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
        return Err(format!("anthropic generation failed ({status}): {body}"));
    }
    Err("anthropic generation failed after retries".to_string())
}

async fn generate_with_openrouter(
    key: &str,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let openrouter_model = model.strip_prefix("openrouter/").unwrap_or(model);
    let client = shared_client();
    let candidates = openrouter_model_candidates(openrouter_model);
    let mut last_error = None;
    for candidate in candidates {
        let mut messages = Vec::new();
        if !system.is_empty() {
            messages.push(OpenAiMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }
        messages.push(OpenAiMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        });
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
            let body: OpenAiResponse = response.json().await.map_err(|err| err.to_string())?;
            let output = body
                .choices
                .as_ref()
                .and_then(|v| v.first())
                .and_then(|c| c.message.as_ref())
                .and_then(|m| m.content.as_deref())
                .map(|s| s.trim().to_string())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| "openrouter returned empty generation output".to_string())?;
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

    Err(last_error.unwrap_or_else(|| "openrouter generation failed".to_string()))
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
/// resolution as `generate_case_output`. Intended for `agc generate` fixture synthesis.
pub async fn call_llm(
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let provider = GeneratorProvider::from_model(model);
    if let GeneratorProvider::Custom = provider {
        return Err(
            "custom provider requires --generator-endpoint; not supported in agc generate"
                .to_string(),
        );
    }
    let key = resolve_generator_key(provider)?;
    match provider {
        GeneratorProvider::Gemini => {
            generate_with_gemini(&key, model, "", prompt, max_tokens).await
        }
        GeneratorProvider::OpenAi => {
            generate_with_openai(&key, model, "", prompt, max_tokens).await
        }
        GeneratorProvider::Anthropic => {
            generate_with_anthropic(&key, model, "", prompt, max_tokens).await
        }
        GeneratorProvider::OpenRouter => {
            generate_with_openrouter(&key, model, "", prompt, max_tokens).await
        }
        GeneratorProvider::Custom => unreachable!(),
    }
}
