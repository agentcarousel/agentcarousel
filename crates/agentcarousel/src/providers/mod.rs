// Shared serde types for LLM provider HTTP requests and responses.
// Used by both evaluators/judge.rs (blocking client) and runner/generator.rs (async client).
// Keep provider-specific HTTP logic in each caller; only serialization shapes live here.

use serde::{Deserialize, Serialize};

// ── Gemini ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct GeminiRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiSystemInstruction>,
    pub contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    pub generation_config: GeminiGenerationConfig,
}

#[derive(Debug, Serialize)]
pub struct GeminiSystemInstruction {
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiContent {
    pub role: Option<String>,
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiPart {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct GeminiGenerationConfig {
    pub temperature: f32,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    // Used by judge (JSON mode); set to None for plain text generation.
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
    #[serde(rename = "usageMetadata")]
    pub usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiCandidate {
    pub content: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiUsage {
    #[serde(rename = "promptTokenCount")]
    pub prompt_token_count: Option<u64>,
    #[serde(rename = "candidatesTokenCount")]
    pub candidates_token_count: Option<u64>,
}

// ── OpenAI / OpenRouter ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OpenAiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    // Used by judge (JSON mode); set to None for plain text generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<OpenAiResponseFormat>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiResponse {
    pub choices: Option<Vec<OpenAiChoice>>,
    pub usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiChoice {
    pub message: Option<OpenAiChoiceMessage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiChoiceMessage {
    pub content: Option<String>,
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub cache_type: &'static str,
}

/// A single block in the structured `system` array.
/// Pass `cache_control` on the last block to enable Anthropic prompt caching.
#[derive(Debug, Serialize)]
pub struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

impl AnthropicSystemBlock {
    pub fn cached(text: String) -> Self {
        Self {
            block_type: "text",
            text,
            cache_control: Some(AnthropicCacheControl {
                cache_type: "ephemeral",
            }),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct AnthropicRequest {
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Structured system blocks; omitted from JSON when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub system: Vec<AnthropicSystemBlock>,
    pub messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicResponse {
    pub content: Option<Vec<AnthropicContent>>,
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicContent {
    // Present in all API responses; used by generator to filter for text blocks.
    #[serde(rename = "type")]
    pub block_type: Option<String>,
    pub text: Option<String>,
}
