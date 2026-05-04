use agentcarousel_core::{ExecutionTrace, TraceStep};
use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SecretScrubber {
    patterns: Vec<Regex>,
}

impl Default for SecretScrubber {
    fn default() -> Self {
        let patterns = vec![
            Regex::new(r"sk-[A-Za-z0-9]{16,}").unwrap(),
            Regex::new(r"ghp_[A-Za-z0-9]{16,}").unwrap(),
            Regex::new(r"Bearer\s+[A-Za-z0-9\-_\.]+").unwrap(),
        ];
        Self { patterns }
    }
}

impl SecretScrubber {
    pub fn scrub_string(&self, value: &str) -> (String, bool) {
        let mut redacted = false;
        let mut output = value.to_string();
        for pattern in &self.patterns {
            if pattern.is_match(&output) {
                redacted = true;
                output = pattern.replace_all(&output, "[REDACTED]").to_string();
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
