use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockStub {
    pub tool: String,
    #[serde(default)]
    pub args_match: Option<Value>,
    pub response: Value,
}

#[derive(Debug, Error)]
pub enum MockError {
    #[error("failed to read mock file: {0}")]
    ReadError(PathBuf),
    #[error("failed to parse mock file: {0}")]
    ParseError(String),
}

#[derive(Debug, Default, Clone)]
pub struct MockEngine {
    stubs: Vec<MockStub>,
}

impl MockEngine {
    pub fn load_dir(path: &Path) -> Result<Self, MockError> {
        let mut stubs = Vec::new();
        if !path.exists() {
            return Ok(Self { stubs });
        }

        for entry in fs::read_dir(path).map_err(|_| MockError::ReadError(path.to_path_buf()))? {
            let entry = entry.map_err(|_| MockError::ReadError(path.to_path_buf()))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let contents =
                fs::read_to_string(&path).map_err(|_| MockError::ReadError(path.clone()))?;
            let mut file_stubs: Vec<MockStub> = serde_json::from_str(&contents)
                .map_err(|err| MockError::ParseError(err.to_string()))?;
            stubs.append(&mut file_stubs);
        }

        Ok(Self { stubs })
    }

    pub fn match_response(&self, tool: &str, args: &Value) -> Option<Value> {
        self.stubs
            .iter()
            .find(|stub| stub.tool == tool && matches_args(stub.args_match.as_ref(), args))
            .map(|stub| stub.response.clone())
    }

    pub fn describe_miss(&self, tool: &str, args: &Value) -> String {
        let candidates: Vec<&MockStub> =
            self.stubs.iter().filter(|stub| stub.tool == tool).collect();
        if candidates.is_empty() {
            return format!("no mocks registered for tool '{}'", tool);
        }

        let mut details = Vec::new();
        for stub in candidates {
            let args_match = stub
                .args_match
                .as_ref()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<any>".to_string());
            details.push(args_match);
        }

        format!(
            "no mock matched for tool '{}' with args {}. candidates: {}",
            tool,
            args,
            details.join(", ")
        )
    }
}

fn matches_args(expected: Option<&Value>, actual: &Value) -> bool {
    match expected {
        None => true,
        Some(expected) => is_subset(expected, actual),
    }
}

fn is_subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            expected_map.iter().all(|(key, value)| {
                actual_map
                    .get(key)
                    .map(|actual_value| is_subset(value, actual_value))
                    .unwrap_or(false)
            })
        }
        (Value::Array(expected_arr), Value::Array(actual_arr)) => expected_arr == actual_arr,
        _ => expected == actual,
    }
}
