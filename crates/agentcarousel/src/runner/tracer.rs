use agentcarousel_core::{CaseResult, ExecutionTrace, TraceStep};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

// Compiled once; SecretScrubber itself is a ZST (zero allocation per case).
static SCRUBBER_PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

/// Secret shapes removed from traces and case errors before a run is printed,
/// persisted to history, exported, or uploaded. Each entry is a pattern and its
/// replacement (`$1` keeps a captured prefix such as `key=`).
fn scrubber_patterns() -> &'static [(Regex, &'static str)] {
    SCRUBBER_PATTERNS.get_or_init(|| {
        [
            // OpenAI (sk-..., sk-proj-...), Anthropic (sk-ant-...), OpenRouter (sk-or-...).
            (r"\bsk-[A-Za-z0-9][A-Za-z0-9_\-]{15,}", "[REDACTED]"),
            // Google API keys (Gemini).
            (r"\bAIza[0-9A-Za-z_\-]{35}", "[REDACTED]"),
            // GitHub tokens: classic (ghp_/gho_/ghu_/ghs_/ghr_) and fine-grained.
            (r"\bgh[pousr]_[A-Za-z0-9]{16,}", "[REDACTED]"),
            (r"\bgithub_pat_[A-Za-z0-9_]{20,}", "[REDACTED]"),
            // AWS access key ids.
            (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "[REDACTED]"),
            // Authorization headers.
            (r"Bearer\s+[A-Za-z0-9\-_\.~+/]+=*", "Bearer [REDACTED]"),
            // API keys passed as URL query parameters (`?key=...`, `&api_key=...`).
            (
                r"(?i)([?&](?:key|api_key|apikey|access_token|token)=)[^&\s)\]]+",
                "${1}[REDACTED]",
            ),
        ]
        .into_iter()
        .map(|(pattern, replacement)| {
            (
                Regex::new(pattern).expect("scrubber pattern must compile"),
                replacement,
            )
        })
        .collect()
    })
}

#[derive(Debug, Clone, Copy)]
pub struct SecretScrubber;

impl SecretScrubber {
    pub fn scrub_string(&self, value: &str) -> (String, bool) {
        let mut redacted = false;
        let mut output = value.to_string();
        for (pattern, replacement) in scrubber_patterns() {
            if pattern.is_match(&output) {
                redacted = true;
                output = pattern.replace_all(&output, *replacement).to_string();
            }
        }
        (output, redacted)
    }

    pub fn scrub_value(&self, value: &Value) -> (Value, bool) {
        match value {
            Value::String(value) => {
                let (scrubbed, redacted) = self.scrub_string(value);
                (Value::String(scrubbed), redacted)
            }
            Value::Object(map) => {
                let mut redacted = false;
                let mut next = serde_json::Map::new();
                for (key, value) in map {
                    let (scrubbed, scrubbed_redacted) = self.scrub_value(value);
                    if scrubbed_redacted {
                        redacted = true;
                    }
                    next.insert(key.clone(), scrubbed);
                }
                (Value::Object(next), redacted)
            }
            Value::Array(values) => {
                let mut redacted = false;
                let mut next = Vec::new();
                for value in values {
                    let (scrubbed, scrubbed_redacted) = self.scrub_value(value);
                    if scrubbed_redacted {
                        redacted = true;
                    }
                    next.push(scrubbed);
                }
                (Value::Array(next), redacted)
            }
            other => (other.clone(), false),
        }
    }
}

pub struct Tracer {
    scrubber: SecretScrubber,
}

impl Tracer {
    pub fn new(scrubber: SecretScrubber) -> Self {
        Self { scrubber }
    }

    pub fn scrub_trace(&mut self, trace: &mut ExecutionTrace) {
        let mut redacted = trace.redacted;
        if let Some(output) = trace.final_output.clone() {
            let (scrubbed, scrubbed_redacted) = self.scrubber.scrub_string(&output);
            trace.final_output = Some(scrubbed);
            redacted |= scrubbed_redacted;
        }
        for step in &mut trace.steps {
            redacted |= scrub_step(step, &self.scrubber);
        }
        trace.redacted = redacted;
    }
}

/// Scrub secrets from case traces and free-text case errors.
///
/// Provider and transport errors can echo request details (URLs, headers), and
/// `CaseResult.error` is printed, persisted to history, exported in evidence
/// packs, and uploaded with runs. Called when a run is assembled and again when
/// a stored run is read back, so runs recorded by older versions (or before a
/// pattern was added) are cleaned before they are shown, exported, or uploaded.
pub fn scrub_case_results(results: &mut [CaseResult]) {
    let scrubber = SecretScrubber;
    let mut tracer = Tracer::new(scrubber);
    for result in results {
        tracer.scrub_trace(&mut result.trace);
        if let Some(error) = result.error.as_deref() {
            let (scrubbed, redacted) = scrubber.scrub_string(error);
            if redacted {
                result.error = Some(scrubbed);
                result.trace.redacted = true;
            }
        }
    }
}

fn scrub_step(step: &mut TraceStep, scrubber: &SecretScrubber) -> bool {
    let mut redacted = false;
    if let Some(args) = step.args.clone() {
        let (scrubbed, scrubbed_redacted) = scrubber.scrub_value(&args);
        step.args = Some(scrubbed);
        redacted |= scrubbed_redacted;
    }
    if let Some(result) = step.result.clone() {
        let (scrubbed, scrubbed_redacted) = scrubber.scrub_value(&result);
        step.result = Some(scrubbed);
        redacted |= scrubbed_redacted;
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrub(value: &str) -> String {
        SecretScrubber.scrub_string(value).0
    }

    #[test]
    fn provider_keys_are_redacted() {
        let secrets = [
            "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "sk-or-v1-0123456789abcdef0123456789abcdef",
            "sk-AbCdEfGhIjKlMnOpQrSt",
            "AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q",
            "ghp_AbCdEfGhIjKlMnOpQrStUvWxYz012345",
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
        ];
        for secret in secrets {
            let out = scrub(&format!("request failed: {secret} (retrying)"));
            assert!(!out.contains(secret), "{secret} survived: {out}");
            assert!(out.contains("[REDACTED]"), "{out}");
        }
    }

    #[test]
    fn bearer_and_query_keys_keep_their_prefix() {
        assert_eq!(
            scrub("Authorization: Bearer abc.def-ghi_jkl"),
            "Authorization: Bearer [REDACTED]"
        );
        assert_eq!(
            scrub("error sending request for url (https://x.test/v1:gen?key=AIzaSECRET&alt=json)"),
            "error sending request for url (https://x.test/v1:gen?key=[REDACTED]&alt=json)"
        );
        assert_eq!(
            scrub("GET /v1/data?api_key=abc123"),
            "GET /v1/data?api_key=[REDACTED]"
        );
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        for text in [
            "risk-assessment-framework-for-agents",
            "task-1234567890abcdefghij",
            "see https://example.com/docs?page=2&monkey=1",
            "effectiveness 0.72 below threshold 0.80",
        ] {
            let (out, redacted) = SecretScrubber.scrub_string(text);
            assert_eq!(out, text);
            assert!(!redacted, "{text}");
        }
    }

    #[test]
    fn case_errors_are_scrubbed_and_marked() {
        let mut results = vec![crate::runner::executor::timeout_result(
            agentcarousel_core::CaseId("demo/case".to_string()),
            Vec::new(),
            Vec::new(),
        )];
        results[0].error = Some(
            "live generation failed: error sending request for url \
             (https://generativelanguage.googleapis.com/v1beta/models/m:generateContent?key=AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q)"
                .to_string(),
        );
        scrub_case_results(&mut results);
        let error = results[0].error.as_deref().unwrap();
        assert!(!error.contains("AIzaSy"), "{error}");
        assert!(results[0].trace.redacted);
    }

    #[test]
    fn reqwest_errors_without_url_drop_query_keys() {
        // Port 1 on localhost refuses the connection; the error must not echo the URL.
        let err = reqwest::blocking::Client::new()
            .get("http://127.0.0.1:1/v1?key=AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q")
            .send()
            .unwrap_err();
        assert!(err.to_string().contains("key="), "precondition: {err}");
        let message = err.without_url().to_string();
        assert!(!message.contains("key="), "{message}");
    }
}
