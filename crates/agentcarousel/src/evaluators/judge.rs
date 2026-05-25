use agentcarousel_core::{
    compute_backoff_ms, is_retryable_status, judge_key_candidates, judge_provider_from_model,
    retry_policy, AuditFinding, Case, CaseResult, CaseStatus, EvalScores, JudgeProvider,
    PromptAudit, PromptAuditFailureMode, RubricScore,
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
    /// Base URL for a custom/Ollama judge endpoint. Required when `model` is `custom/*` or `ollama/*`.
    pub endpoint: Option<String>,
}

impl JudgeEvaluator {
    pub fn from_case(
        case: &Case,
        judge_model: Option<&str>,
        judge_max_tokens: Option<u32>,
        judge_endpoint: Option<&str>,
    ) -> Result<Self, EvaluatorError> {
        let prompt = case
            .evaluator_config
            .as_ref()
            .and_then(|config| config.judge_prompt.clone());
        Ok(Self {
            prompt,
            model: judge_model.unwrap_or("gemini-2.5-flash").to_string(),
            max_tokens: judge_max_tokens,
            endpoint: judge_endpoint.map(|s| s.to_string()),
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
        // Custom provider needs no API key — resolve_judge_key would fail with empty candidates.
        let judge_key = if matches!(provider, JudgeProvider::Custom) {
            String::new()
        } else {
            resolve_judge_key(provider)?
        };
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
            JudgeProvider::Custom => match self.endpoint.as_deref() {
                Some(ep) => call_custom_judge_blocking(ep, &self.model, max_tok, sp, up),
                None => Err(EvaluatorError::MissingConfig(
                    "--judge-endpoint is required when judge model is 'custom' or 'ollama/<name>'",
                )),
            },
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

        let msg = format!(
            "gemini judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        );
        return Err(if retryable {
            EvaluatorError::JudgeFailed(msg)
        } else {
            EvaluatorError::JudgeUnavailable(msg)
        });
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
        let msg = format!(
            "openai judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        );
        return Err(if retryable {
            EvaluatorError::JudgeFailed(msg)
        } else {
            EvaluatorError::JudgeUnavailable(msg)
        });
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
        return Err(EvaluatorError::JudgeUnavailable(
            "anthropic judge requires max_tokens — set judge.max_tokens in agentcarousel.toml"
                .to_string(),
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
        let msg = format!(
            "anthropic judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        );
        return Err(if retryable {
            EvaluatorError::JudgeFailed(msg)
        } else {
            EvaluatorError::JudgeUnavailable(msg)
        });
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
        let msg = format!(
            "openrouter judge request failed ({}): {}",
            status,
            redact_api_key(body.trim())
        );
        return Err(if retryable {
            EvaluatorError::JudgeFailed(msg)
        } else {
            EvaluatorError::JudgeUnavailable(msg)
        });
    }
    Err(EvaluatorError::JudgeFailed(
        "openrouter judge request failed after retries".to_string(),
    ))
}

fn call_custom_judge_blocking(
    endpoint: &str,
    model: &str,
    max_tokens: Option<u32>,
    system_prompt: String,
    user_prompt: String,
) -> Result<JudgeCallOutput, EvaluatorError> {
    let model_name = model
        .strip_prefix("ollama/")
        .or_else(|| model.strip_prefix("custom/"))
        .filter(|s| !s.is_empty())
        .unwrap_or(model);

    let body = if endpoint.contains("/api/generate") {
        let combined = format!("{system_prompt}\n\n{user_prompt}");
        let mut b = serde_json::json!({
            "model": model_name,
            "prompt": combined,
            "stream": false,
        });
        if let Some(n) = max_tokens {
            b["options"] = serde_json::json!({"num_predict": n});
        }
        b
    } else {
        let mut b = serde_json::json!({
            "model": model_name,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
        });
        if let Some(n) = max_tokens {
            b["max_tokens"] = serde_json::json!(n);
        }
        b
    };

    let client = shared_blocking_client();
    let response =
        client.post(endpoint).json(&body).send().map_err(|e| {
            EvaluatorError::JudgeFailed(format!("custom judge request failed: {e}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().unwrap_or_default();
        return Err(EvaluatorError::JudgeUnavailable(format!(
            "custom judge returned {status}: {body_text}"
        )));
    }

    let json: serde_json::Value = response
        .json()
        .map_err(|e| EvaluatorError::InvalidOutput(format!("custom judge parse failed: {e}")))?;

    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| json["response"].as_str())
        .or_else(|| json["output"].as_str())
        .ok_or_else(|| {
            EvaluatorError::InvalidOutput(
                "custom judge response missing 'choices[0].message.content', 'response', or 'output'"
                    .to_string(),
            )
        })?
        .to_string();

    Ok(JudgeCallOutput {
        text,
        tokens_in: None,
        tokens_out: None,
    })
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

fn looks_truncated_audit_json(value: &str) -> bool {
    let trimmed = value.trim();
    let has_json_start = trimmed.starts_with('{') || trimmed.starts_with("```");
    has_json_start
        && (trimmed.contains("\"failure_mode\"")
            || trimmed.contains("\"findings\"")
            || trimmed.contains("\"suggested_fixes\""))
        && !trimmed.ends_with('}')
}

// ── Prompt audit ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PromptAuditResponse {
    failure_mode: String,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    findings: Vec<RawAuditFinding>,
    #[serde(default)]
    suggested_fixes: Vec<String>,
    /// Actual prompt text to paste for each fix — parallel to suggested_fixes.
    #[serde(default)]
    suggested_implementations: Vec<String>,
    #[serde(default)]
    overall_rationale: String,
}

#[derive(Debug, Deserialize)]
struct RawAuditFinding {
    pattern: String,
    #[serde(default)]
    affected_case_count: u32,
    #[serde(default)]
    root_cause: String,
}

/// Run the second-pass prompt-audit judge after all cases are scored.
///
/// Takes the fixture's `prompt.md` text and the completed case results. Calls the same
/// judge model to classify the root cause of failures (prompt design, model capability,
/// fixture miscalibration, or mixed) and produce actionable fix suggestions.
///
/// Only runs when there are failed/errored cases; returns `None` silently when there is
/// nothing to diagnose.
pub fn run_prompt_audit(
    prompt_text: &str,
    results: &[CaseResult],
    model: &str,
    max_tokens: Option<u32>,
    endpoint: Option<&str>,
) -> Result<PromptAudit, EvaluatorError> {
    let provider = judge_provider_from_model(model);
    let judge_key = if matches!(provider, JudgeProvider::Custom) {
        String::new()
    } else {
        resolve_judge_key(provider)?
    };

    let system_prompt = build_prompt_audit_system_prompt();
    let user_prompt = build_prompt_audit_user_prompt(prompt_text, results);

    let call_judge = |sp: String, up: String, max_tok: Option<u32>| match provider {
        JudgeProvider::Gemini => call_gemini_text(&judge_key, model, max_tok, sp, up),
        JudgeProvider::OpenAi => call_openai_text(&judge_key, model, max_tok, sp, up),
        JudgeProvider::Anthropic => call_anthropic_text(&judge_key, model, max_tok, sp, up),
        JudgeProvider::OpenRouter => call_openrouter_text(&judge_key, model, max_tok, sp, up),
        JudgeProvider::Custom => match endpoint {
            Some(ep) => call_custom_judge_blocking(ep, model, max_tok, sp, up),
            None => Err(EvaluatorError::MissingConfig(
                "--judge-endpoint is required when judge model is 'custom' or 'ollama/<name>'",
            )),
        },
    };

    let first_out = call_judge(system_prompt.clone(), user_prompt.clone(), max_tokens)?;
    let mut tokens_in = first_out.tokens_in;
    let mut tokens_out = first_out.tokens_out;

    let parsed = match parse_prompt_audit_response(&first_out.text) {
        Ok(p) => p,
        Err(first_err) => {
            if !looks_truncated_audit_json(&first_out.text) {
                return Err(first_err);
            }
            // The response was cut off mid-JSON — retry with a larger budget and tighter
            // field-length constraints so the model fits the whole object in the window.
            let retry_tokens = Some(max_tokens.unwrap_or(2048).saturating_mul(4).min(8192));
            let retry_system = format!(
                "{}\nReturn minified JSON only. Keep finding.pattern <=40 chars, each suggested_fix <=60 chars, overall_rationale <=80 chars.",
                system_prompt
            );
            let retry_out = call_judge(retry_system, user_prompt, retry_tokens)?;
            tokens_in = add_opt(tokens_in, retry_out.tokens_in);
            tokens_out = add_opt(tokens_out, retry_out.tokens_out);
            parse_prompt_audit_response(&retry_out.text)?
        }
    };

    let failure_mode = match parsed.failure_mode.to_lowercase().as_str() {
        "model" => PromptAuditFailureMode::Model,
        "fixture" => PromptAuditFailureMode::Fixture,
        "mixed" => PromptAuditFailureMode::Mixed,
        _ => PromptAuditFailureMode::Prompt,
    };

    Ok(PromptAudit {
        failure_mode,
        confidence: parsed.confidence.clamp(0.0, 1.0),
        findings: parsed
            .findings
            .into_iter()
            .map(|f| AuditFinding {
                pattern: f.pattern,
                affected_case_count: f.affected_case_count,
                root_cause: f.root_cause,
            })
            .collect(),
        suggested_fixes: parsed.suggested_fixes,
        suggested_implementations: parsed
            .suggested_implementations
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect(),
        overall_rationale: parsed.overall_rationale,
        judge_tokens_in: tokens_in,
        judge_tokens_out: tokens_out,
    })
}

fn build_prompt_audit_system_prompt() -> String {
    r#"You are a prompt-audit judge. You have just seen the results of an agentcarousel eval run.
Your job is to diagnose WHY cases are failing and WHERE the fix should be applied.

Classify the primary failure mode as exactly one of:
- "prompt"  — the agent prompt is underspecified, ambiguous, or missing worked examples;
              fixing the prompt is likely to fix the failures without a model upgrade.
- "model"   — the model lacks the capability to follow these instructions regardless of
              how the prompt is worded; a stronger model is needed.
- "fixture" — the rubric thresholds or test expectations are miscalibrated; the model
              output is actually reasonable but the evaluator is scoring it too harshly.
- "mixed"   — two or more of the above are materially contributing.

Evidence patterns for "prompt":
- The same structural element (citation format, section header, required block) is absent
  across ALL or nearly ALL cases → model never saw a concrete example of what to produce.
- Failures concentrate on format/structure requirements rather than factual accuracy.
- The model's output is clinically/factually reasonable but ignores the specified format.

Evidence patterns for "model":
- The model attempts the required format but produces garbled or partially correct output.
- Failures involve semantic reasoning (e.g. identifying an escalation trigger), not structure.
- Prompt already contains worked examples and the model still fails.

Evidence patterns for "fixture":
- The model output looks reasonable to a domain expert but is being scored 0.
- Rubric items are overly specific (exact string matches on phrasing that can vary).
- Effectiveness thresholds are set higher than the task's inherent ambiguity warrants.

Return JSON only:
{
  "failure_mode": "prompt" | "model" | "fixture" | "mixed",
  "confidence": <0.0–1.0>,
  "findings": [
    {
      "pattern": "<what failed, how many cases, e.g. '7/7 cases missing [T####] citations'>",
      "affected_case_count": <integer>,
      "root_cause": "<why it failed — one concrete sentence>"
    }
  ],
  "suggested_fixes": ["<one-line title for fix 1>", "<one-line title for fix 2>"],
  "suggested_implementations": [
    "<complete markdown block to paste into prompt.md for fix 1 — full worked example, restructured section, or new rule, ready to use as-is>",
    "<complete markdown block to paste into prompt.md for fix 2>"
  ],
  "overall_rationale": "<2–3 sentence synthesis>"
}

Rules for suggested_implementations:
- One element per fix, parallel to suggested_fixes (same index).
- Write the actual prompt text the author should add or replace — not a description of what to do.
- Include a worked input/output example when the fix is about adding an example.
- Include the restructured section text when the fix is about reorganising a section.
- Keep each implementation under 800 characters; use \n for line breaks inside the JSON string.
- If a fix applies only to model or fixture issues (not prompt text), write an empty string "".
Keep each finding.pattern under 80 chars. Keep each suggested_fix title under 100 chars. JSON only — no prose outside the JSON object."#.to_string()
}

fn build_prompt_audit_user_prompt(prompt_text: &str, results: &[CaseResult]) -> String {
    let mut out = String::new();

    out.push_str("## Agent prompt (prompt.md)\n\n```\n");
    out.push_str(prompt_text.trim());
    out.push_str("\n```\n\n");

    out.push_str("## Case results\n\n");
    for r in results {
        let status = match r.status {
            CaseStatus::Passed => "PASS",
            CaseStatus::Failed => "FAIL",
            CaseStatus::Error => "ERROR",
            CaseStatus::TimedOut => "TIMEOUT",
            CaseStatus::Skipped => "SKIP",
            CaseStatus::Flaky => "FLAKY",
        };
        out.push_str(&format!("### {} [{}]\n", r.case_id.0, status));

        if let Some(scores) = &r.eval_scores {
            if let Some(rationale) = &scores.judge_rationale {
                out.push_str(&format!("Judge: {}\n", rationale.trim()));
            }
            for rs in &scores.rubric_scores {
                if rs.score < 1.0 {
                    if let Some(rat) = &rs.rationale {
                        out.push_str(&format!(
                            "  · {} ({:.2}): {}\n",
                            rs.rubric_id,
                            rs.score,
                            rat.trim()
                        ));
                    }
                }
            }
        }

        if let Some(output) = &r.trace.final_output {
            let preview: String = output.chars().take(300).collect();
            out.push_str(&format!("Output preview: {}\n", preview.trim()));
            if output.chars().count() > 300 {
                out.push_str("...[truncated]\n");
            }
        }
        out.push('\n');
    }

    out
}

fn parse_prompt_audit_response(raw_text: &str) -> Result<PromptAuditResponse, EvaluatorError> {
    let trimmed = raw_text.trim();
    if let Ok(parsed) = serde_json::from_str::<PromptAuditResponse>(trimmed) {
        return Ok(parsed);
    }
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if let Ok(parsed) = serde_json::from_str::<PromptAuditResponse>(&fenced) {
            return Ok(parsed);
        }
    }
    let start = raw_text.find('{');
    let end = raw_text.rfind('}');
    if let (Some(s), Some(e)) = (start, end) {
        return serde_json::from_str::<PromptAuditResponse>(&raw_text[s..=e])
            .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()));
    }
    Err(EvaluatorError::InvalidOutput(
        "prompt audit response was not valid JSON".to_string(),
    ))
}
