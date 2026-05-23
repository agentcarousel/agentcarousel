use agentcarousel_core::{
    compute_backoff_ms, is_retryable_status, judge_key_candidates, judge_provider_from_model,
    retry_policy, Case, CaseResult, EvalScores, JudgeProvider, RubricScore,
};
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;

use super::assertions::check_output;
use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};
use serde::Deserialize;

use crate::providers::{
    AnthropicMessage, AnthropicRequest, AnthropicResponse, GeminiContent, GeminiGenerationConfig,
    GeminiPart, GeminiRequest, GeminiResponse, GeminiSystemInstruction, OpenAiMessage,
    OpenAiRequest, OpenAiResponse, OpenAiResponseFormat,
};

static BLOCKING_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn shared_blocking_client() -> &'static reqwest::blocking::Client {
    BLOCKING_CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest blocking client")
    })
}

struct JudgeCallOutput {
    text: String,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct JudgeEvaluator {
    pub prompt: Option<String>,
    pub model: String,
    pub max_tokens: Option<u32>,
}

impl JudgeEvaluator {
    pub fn from_case(
        case: &Case,
        judge_model: Option<&str>,
        judge_max_tokens: Option<u32>,
    ) -> Result<Self, EvaluatorError> {
        let prompt = case
            .evaluator_config
            .as_ref()
            .and_then(|config| config.judge_prompt.clone());
        Ok(Self {
            prompt,
            model: judge_model.unwrap_or("gemini-2.5-flash").to_string(),
            max_tokens: judge_max_tokens,
        })
    }
}

#[derive(Debug, Deserialize)]
struct JudgeResponse {
    rubric: Vec<JudgeRubricScore>,
    overall_rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JudgeRubricScore {
    rubric_id: String,
    score: f32,
    rationale: Option<String>,
}

impl Evaluator for JudgeEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Judge.as_str()
    }

    fn evaluate(&self, case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let output = result.trace.final_output.clone().unwrap_or_default();
        if output.trim().is_empty() {
            return Err(EvaluatorError::MissingOutput);
        }

        let provider = judge_provider_from_model(&self.model);
        let judge_key = resolve_judge_key(provider)?;
        let system_prompt = build_system_prompt(case, self.prompt.as_deref());
        let user_prompt = build_user_prompt(case, &output);

        let call_judge = |sp: String, up: String, max_tok: Option<u32>| match provider {
            JudgeProvider::Gemini => call_gemini_text(&judge_key, &self.model, max_tok, sp, up),
            JudgeProvider::OpenAi => call_openai_text(&judge_key, &self.model, max_tok, sp, up),
            JudgeProvider::Anthropic => {
                call_anthropic_text(&judge_key, &self.model, max_tok, sp, up)
            }
            JudgeProvider::OpenRouter => {
                call_openrouter_text(&judge_key, &self.model, max_tok, sp, up)
            }
        };

        let first_out = call_judge(system_prompt.clone(), user_prompt.clone(), self.max_tokens)?;
        let mut judge_tokens_in: Option<u64> = first_out.tokens_in;
        let mut judge_tokens_out: Option<u64> = first_out.tokens_out;

        if first_out.text.trim().is_empty() {
            return Err(EvaluatorError::InvalidOutput(
                "judge returned empty response".to_string(),
            ));
        }

        let judge_response = match parse_judge_response(&first_out.text) {
            Ok(parsed) => parsed,
            Err(first_err) => {
                if !looks_truncated_json(&first_out.text) {
                    return Err(first_err);
                }
                // Retry once with a larger token budget and stricter brevity constraints.
                let retry_tokens =
                    Some(self.max_tokens.unwrap_or(1536).saturating_mul(4).min(4096));
                let retry_system_prompt = format!(
                    "{}\nKeep each rationale <= 12 words. Return minified JSON only.",
                    system_prompt
                );
                let retry_out = call_judge(retry_system_prompt, user_prompt, retry_tokens)?;
                judge_tokens_in = add_opt(judge_tokens_in, retry_out.tokens_in);
                judge_tokens_out = add_opt(judge_tokens_out, retry_out.tokens_out);
                parse_judge_response(&retry_out.text)?
            }
        };
        let mut judge_scores = std::collections::HashMap::new();
        for item in judge_response.rubric.into_iter() {
            judge_scores.insert(item.rubric_id.clone(), item);
        }

        let rubric_scores: Vec<RubricScore> = case
            .expected
            .rubric
            .as_ref()
            .map(|rubric| {
                rubric
                    .iter()
                    .map(|item| {
                        let mut rationale = None;
                        let score = if let Some(judge_score) = judge_scores.get(&item.id) {
                            rationale = judge_score.rationale.clone();
                            judge_score.score.clamp(0.0, 1.0)
                        } else if let Some(auto_check) = item.auto_check.as_ref() {
                            match check_output(auto_check, &result.trace) {
                                Ok(()) => 1.0,
                                Err(err) => {
                                    rationale = Some(err);
                                    0.0
                                }
                            }
                        } else {
                            rationale = Some("judge missing rubric score".to_string());
                            0.0
                        };

                        RubricScore {
                            rubric_id: item.id.clone(),
                            score,
                            weight: item.weight,
                            rationale,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let effectiveness_score = if rubric_scores.is_empty() {
            0.0
        } else {
            let total_weight: f32 = rubric_scores.iter().map(|score| score.weight).sum();
            if total_weight <= f32::EPSILON {
                0.0
            } else {
                rubric_scores
                    .iter()
                    .map(|score| score.score * score.weight)
                    .sum::<f32>()
                    / total_weight
            }
        };

        Ok(EvalScores {
            evaluator: self.id().to_string(),
            rubric_scores,
            effectiveness_score,
            passed: effectiveness_score >= 1.0,
            judge_rationale: judge_response
                .overall_rationale
                .or_else(|| Some("judge completed without rationale".to_string())),
            judge_tokens_in,
            judge_tokens_out,
        })
    }
}

fn resolve_judge_key(provider: JudgeProvider) -> Result<String, EvaluatorError> {
    let key = judge_key_candidates(provider)
        .iter()
        .find_map(|k| std::env::var(k).ok())
        .ok_or(EvaluatorError::MissingConfig(
            "missing judge API key (set AGENTCAROUSEL_JUDGE_KEY or provider key)",
        ))?;
    reqwest::header::HeaderValue::from_str(&key).map_err(|_| {
        EvaluatorError::MissingConfig("judge API key contains invalid header characters")
    })?;
    Ok(key)
}

fn build_system_prompt(case: &Case, custom_prompt: Option<&str>) -> String {
    let mut prompt = String::new();
    if let Some(custom_prompt) = custom_prompt {
        prompt.push_str(custom_prompt.trim());
        prompt.push('\n');
    }
    prompt.push_str("\nYou are an evaluation judge. Score each rubric item from 0.0 to 1.0.\n");
    prompt.push_str(
        "Return JSON only with keys: rubric (array of {rubric_id, score, rationale}) and overall_rationale.\n",
    );
    if let Some(rubric) = case.expected.rubric.as_ref() {
        prompt.push_str("\nRubric items:\n");
        for item in rubric {
            prompt.push_str("- ");
            prompt.push_str(&item.id);
            prompt.push_str(": ");
            prompt.push_str(item.description.trim());
            prompt.push('\n');
        }
    }
    prompt
}

fn build_user_prompt(case: &Case, output: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str("Case input messages:\n");
    for message in case.input.messages.iter() {
        prompt.push('[');
        prompt.push_str(&format!("{:?}", message.role).to_lowercase());
        prompt.push_str("] ");
        prompt.push_str(message.content.trim());
        prompt.push_str("\n\n");
    }
    prompt.push_str("Case output:\n");
    prompt.push_str(output.trim());
    prompt
}

fn call_gemini_text(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    call_gemini_blocking(judge_key, model, max_tokens, system_prompt, user_prompt)
}

fn call_gemini_blocking(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, judge_key
    );
    let request = GeminiRequest {
        system_instruction: Some(GeminiSystemInstruction {
            parts: vec![GeminiPart {
                text: system_prompt,
            }],
        }),
        contents: vec![GeminiContent {
            role: Some("user".to_string()),
            parts: vec![GeminiPart { text: user_prompt }],
        }],
        generation_config: GeminiGenerationConfig {
            temperature: 0.2,
            max_output_tokens: max_tokens,
            response_mime_type: Some("application/json".to_string()),
        },
    };

    let client = shared_blocking_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post(&url)
            .json(&request)
            .send()
            .map_err(|err| EvaluatorError::JudgeFailed(redact_api_key(&err.to_string())))?;

        let status = response.status();
        if status.is_success() {
            let parsed = response
                .json::<GeminiResponse>()
                .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
            let text = parsed
                .candidates
                .as_ref()
                .and_then(|candidates| candidates.first())
                .and_then(|candidate| candidate.content.as_ref())
                .and_then(|content| content.parts.first())
                .map(|part| part.text.clone())
                .unwrap_or_default();
            let tokens_in = parsed
                .usage_metadata
                .as_ref()
                .and_then(|u| u.prompt_token_count);
            let tokens_out = parsed
                .usage_metadata
                .as_ref()
                .and_then(|u| u.candidates_token_count);
            return Ok(JudgeCallOutput {
                text,
                tokens_in,
                tokens_out,
            });
        }

        let body = response.text().unwrap_or_default();
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            std::thread::sleep(Duration::from_millis(backoff_ms));
            continue;
        }

        return Err(EvaluatorError::JudgeFailed(format!(
            "gemini judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        )));
    }

    Err(EvaluatorError::JudgeFailed(
        "gemini judge request failed after retries".to_string(),
    ))
}

fn call_openai_text(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    call_openai_blocking(judge_key, model, max_tokens, system_prompt, user_prompt)
}

fn call_openai_blocking(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    let request = OpenAiRequest {
        model: model.to_string(),
        messages: vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: system_prompt,
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: user_prompt,
            },
        ],
        temperature: 0.2,
        max_tokens,
        response_format: Some(OpenAiResponseFormat {
            format_type: "json_object".to_string(),
        }),
    };
    let client = shared_blocking_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(judge_key)
            .json(&request)
            .send()
            .map_err(|err| EvaluatorError::JudgeFailed(redact_api_key(&err.to_string())))?;
        let status = response.status();
        if status.is_success() {
            let parsed = response
                .json::<OpenAiResponse>()
                .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
            let text = parsed
                .choices
                .as_ref()
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.message.as_ref())
                .and_then(|message| message.content.clone())
                .unwrap_or_default();
            let tokens_in = parsed.usage.as_ref().and_then(|u| u.prompt_tokens);
            let tokens_out = parsed.usage.as_ref().and_then(|u| u.completion_tokens);
            return Ok(JudgeCallOutput {
                text,
                tokens_in,
                tokens_out,
            });
        }
        let body = response.text().unwrap_or_default();
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            std::thread::sleep(Duration::from_millis(backoff_ms));
            continue;
        }
        return Err(EvaluatorError::JudgeFailed(format!(
            "openai judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        )));
    }
    Err(EvaluatorError::JudgeFailed(
        "openai judge request failed after retries".to_string(),
    ))
}

fn call_anthropic_text(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    call_anthropic_blocking(judge_key, model, max_tokens, system_prompt, user_prompt)
}

fn call_anthropic_blocking(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    let Some(max_tokens) = max_tokens else {
        return Err(EvaluatorError::JudgeFailed(
            "anthropic judge requires max_tokens".to_string(),
        ));
    };
    let request = AnthropicRequest {
        model: model.to_string(),
        max_tokens,
        system: system_prompt,
        messages: vec![AnthropicMessage {
            role: "user".to_string(),
            content: user_prompt,
        }],
        temperature: 0.2,
    };
    let client = shared_blocking_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", judge_key)
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .map_err(|err| EvaluatorError::JudgeFailed(redact_api_key(&err.to_string())))?;
        let status = response.status();
        if status.is_success() {
            let parsed = response
                .json::<AnthropicResponse>()
                .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
            let text = parsed
                .content
                .as_ref()
                .and_then(|items| items.first())
                .and_then(|item| item.text.clone())
                .unwrap_or_default();
            let tokens_in = parsed.usage.as_ref().and_then(|u| u.input_tokens);
            let tokens_out = parsed.usage.as_ref().and_then(|u| u.output_tokens);
            return Ok(JudgeCallOutput {
                text,
                tokens_in,
                tokens_out,
            });
        }
        let body = response.text().unwrap_or_default();
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            std::thread::sleep(Duration::from_millis(backoff_ms));
            continue;
        }
        return Err(EvaluatorError::JudgeFailed(format!(
            "anthropic judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        )));
    }
    Err(EvaluatorError::JudgeFailed(
        "anthropic judge request failed after retries".to_string(),
    ))
}

fn call_openrouter_text(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    call_openrouter_blocking(judge_key, model, max_tokens, system_prompt, user_prompt)
}

fn call_openrouter_blocking(
    judge_key: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    let request = OpenAiRequest {
        model: model.to_string(),
        messages: vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: system_prompt,
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: user_prompt,
            },
        ],
        temperature: 0.2,
        max_tokens,
        response_format: Some(OpenAiResponseFormat {
            format_type: "json_object".to_string(),
        }),
    };
    let client = shared_blocking_client();
    let retry = retry_policy();
    for attempt in 0..retry.max_attempts {
        let response = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .bearer_auth(judge_key)
            .header(
                "HTTP-Referer",
                "https://github.com/agentcarousel/agentcarousel",
            )
            .header("X-Title", "agentcarousel")
            .json(&request)
            .send()
            .map_err(|err| EvaluatorError::JudgeFailed(redact_api_key(&err.to_string())))?;
        let status = response.status();
        if status.is_success() {
            let parsed = response
                .json::<OpenAiResponse>()
                .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
            let text = parsed
                .choices
                .as_ref()
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.message.as_ref())
                .and_then(|message| message.content.clone())
                .unwrap_or_default();
            let tokens_in = parsed.usage.as_ref().and_then(|u| u.prompt_tokens);
            let tokens_out = parsed.usage.as_ref().and_then(|u| u.completion_tokens);
            return Ok(JudgeCallOutput {
                text,
                tokens_in,
                tokens_out,
            });
        }
        let body = response.text().unwrap_or_default();
        let retryable = is_retryable_status(status);
        if retryable && attempt + 1 < retry.max_attempts {
            let backoff_ms = compute_backoff_ms(attempt, &retry);
            std::thread::sleep(Duration::from_millis(backoff_ms));
            continue;
        }
        return Err(EvaluatorError::JudgeFailed(format!(
            "openrouter judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        )));
    }
    Err(EvaluatorError::JudgeFailed(
        "openrouter judge request failed after retries".to_string(),
    ))
}

fn parse_judge_response(raw_text: &str) -> Result<JudgeResponse, EvaluatorError> {
    let trimmed = raw_text.trim();
    if let Ok(parsed) = serde_json::from_str::<JudgeResponse>(trimmed) {
        return Ok(parsed);
    }
    if let Some(fenced_json) = extract_fenced_json(trimmed) {
        if let Ok(parsed) = serde_json::from_str::<JudgeResponse>(&fenced_json) {
            return Ok(parsed);
        }
    }
    let start = raw_text.find('{');
    let end = raw_text.rfind('}');
    if let (Some(start), Some(end)) = (start, end) {
        let candidate = &raw_text[start..=end];
        return serde_json::from_str::<JudgeResponse>(candidate)
            .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()));
    }
    if std::env::var("AGENTCAROUSEL_DEBUG_JUDGE").ok().as_deref() == Some("1") {
        return Err(EvaluatorError::InvalidOutput(format!(
            "judge response was not valid JSON; raw={}",
            truncate_for_debug(trimmed, 2000)
        )));
    }
    Err(EvaluatorError::InvalidOutput(
        "judge response was not valid JSON".to_string(),
    ))
}

fn add_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x + y),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

fn redact_api_key(message: &str) -> String {
    let key_param = Regex::new(r"(key=)[^&\s]+").expect("regex must compile");
    key_param.replace_all(message, "${1}REDACTED").into_owned()
}

fn extract_fenced_json(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return None;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    if lines.len() < 3 {
        return None;
    }
    let mut start_idx = 1;
    if lines
        .first()
        .is_some_and(|line| line.trim_start().starts_with("```json"))
    {
        start_idx = 1;
    }
    let mut end_idx = lines.len();
    for (idx, line) in lines.iter().enumerate().rev() {
        if line.trim_start().starts_with("```") {
            end_idx = idx;
            break;
        }
    }
    if end_idx <= start_idx {
        return None;
    }
    Some(lines[start_idx..end_idx].join("\n"))
}

fn truncate_for_debug(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value.chars().take(max_chars).collect::<String>();
    out.push_str("...[truncated]");
    out
}

fn looks_truncated_json(value: &str) -> bool {
    let trimmed = value.trim();
    let has_json_start = trimmed.starts_with('{') || trimmed.starts_with("```");
    has_json_start && trimmed.contains("\"rubric\"") && !trimmed.ends_with('}')
}
