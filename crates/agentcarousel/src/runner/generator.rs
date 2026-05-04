use agentcarousel_core::{compute_backoff_ms, is_retryable_status, retry_policy, Case, Role};
use openrouter_rs::{
    api::chat::{ChatCompletionRequest, Message as OpenRouterMessage},
    types::Role as OpenRouterRole,
    OpenRouterClient,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

use super::RunnerConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorProvider {
    Gemini,
    OpenAi,
    Anthropic,
    OpenRouter,
}

impl GeneratorProvider {
    pub fn from_model(model: &str) -> Self {
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

    fn key_candidates(self) -> &'static [&'static str] {
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
    let key = resolve_generator_key(provider)?;
    let prompt = build_generation_prompt(case);
    let max_tokens = config.generator_max_tokens;

    match provider {
        GeneratorProvider::Gemini => generate_with_gemini(&key, model, &prompt, max_tokens).await,
        GeneratorProvider::OpenAi => generate_with_openai(&key, model, &prompt, max_tokens).await,
        GeneratorProvider::Anthropic => {
            generate_with_anthropic(&key, model, &prompt, max_tokens).await
        }
        GeneratorProvider::OpenRouter => {
            generate_with_openrouter(&key, model, &prompt, max_tokens).await
        }
    }
}

fn resolve_generator_key(provider: GeneratorProvider) -> Result<String, String> {
    provider
        .key_candidates()
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .ok_or_else(|| {
            format!(
                "missing generator API key; set one of {}",
                provider.key_candidates().join(", ")
            )
        })
}

fn build_generation_prompt(case: &Case) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are generating the agent response for this evaluation case.\n");
    prompt.push_str("Respond with the best final answer only.\n\n");
    prompt.push_str("Conversation:\n");
    for message in &case.input.messages {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
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

#[derive(Debug, Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: GeminiGenerationConfig,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiContent {
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(Debug, Serialize)]
struct GeminiGenerationConfig {
    temperature: f32,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsage {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u64>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u64>,
}

async fn generate_with_gemini(
    key: &str,
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, key
    );
    let request = GeminiRequest {
        contents: vec![GeminiContent {
            role: Some("user".to_string()),
            parts: vec![GeminiPart {
                text: prompt.to_string(),
            }],
        }],
        generation_config: GeminiGenerationConfig {
            temperature: 0.2,
            max_output_tokens: max_tokens,
        },
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;
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

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

async fn generate_with_openai(
    key: &str,
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let request = OpenAiRequest {
        model: model.to_string(),
        messages: vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: "You are generating the best final answer for this evaluation case."
                    .to_string(),
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            },
        ],
        temperature: 0.2,
        max_tokens,
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;
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
                .first()
                .map(|choice| choice.message.content.trim().to_string())
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

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    temperature: f32,
    system: String,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

async fn generate_with_anthropic(
    key: &str,
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let max_tokens =
        max_tokens.ok_or_else(|| "max_tokens is required for Anthropic generation".to_string())?;
    let request = AnthropicRequest {
        model: model.to_string(),
        max_tokens,
        temperature: 0.2,
        system: "You are generating the best final answer for this evaluation case.".to_string(),
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| err.to_string())?;
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
            let output = body
                .content
                .iter()
                .find(|block| block.block_type == "text")
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
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<GenerationResult, String> {
    let openrouter_model = model.strip_prefix("openrouter/").unwrap_or(model);
    let client = OpenRouterClient::builder()
        .api_key(key.to_string())
        .x_title("agentcarousel")
        .build()
        .map_err(|err| err.to_string())?;
    let candidates = openrouter_model_candidates(openrouter_model);
    let mut last_error = None;
    for candidate in candidates {
        let request = build_openrouter_request(candidate, prompt, max_tokens)?;
        match client.chat().create(&request).await {
            Ok(response) => {
                let output = response
                    .choices
                    .first()
                    .and_then(|choice| choice.content())
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| "openrouter returned empty generation output".to_string())?;
                return Ok(GenerationResult {
                    output,
                    tokens_in: response
                        .usage
                        .as_ref()
                        .map(|usage| usage.prompt_tokens as u64),
                    tokens_out: response
                        .usage
                        .as_ref()
                        .map(|usage| usage.completion_tokens as u64),
                });
            }
            Err(err) => {
                let err_text = err.to_string();
                // For missing OpenRouter routes, try known model suffix variants.
                let retryable_model_miss = err_text.contains("No endpoints found")
                    || (err_text.contains("api_code=404") && err_text.contains(candidate));
                last_error = Some(err_text);
                if retryable_model_miss {
                    continue;
                }
                break;
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "openrouter generation failed".to_string()))
}

fn build_openrouter_request(
    model: &str,
    prompt: &str,
    max_tokens: Option<u32>,
) -> Result<ChatCompletionRequest, String> {
    let mut builder = ChatCompletionRequest::builder();
    builder
        .model(model.to_string())
        .messages(vec![OpenRouterMessage::new(
            OpenRouterRole::User,
            prompt.to_string(),
        )])
        .temperature(0.2);
    if let Some(max_tokens) = max_tokens {
        builder.max_tokens(max_tokens);
    }
    builder.build().map_err(|err| err.to_string())
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
