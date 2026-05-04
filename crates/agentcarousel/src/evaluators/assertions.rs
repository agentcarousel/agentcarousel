use agentcarousel_core::{AssertionKind, ExecutionTrace, OutputAssertion};
use regex::Regex;
use serde_json::Value;

pub fn check_output(assertion: &OutputAssertion, trace: &ExecutionTrace) -> Result<(), String> {
    let output = trace.final_output.clone().unwrap_or_default();
    let target = if assertion.field.is_some() || assertion.kind == AssertionKind::JsonPath {
        output_field(&output, assertion.field.as_deref()).unwrap_or_default()
    } else {
        output.clone()
    };

    match assertion.kind {
        AssertionKind::Contains => {
            if target.contains(&assertion.value) {
                Ok(())
            } else {
                Err(format!("expected output to contain '{}'", assertion.value))
            }
        }
        AssertionKind::NotContains => {
            if target.contains(&assertion.value) {
                Err(format!(
                    "expected output to not contain '{}'",
                    assertion.value
                ))
            } else {
                Ok(())
            }
        }
        AssertionKind::Equals => {
            if target == assertion.value {
                Ok(())
            } else {
                Err(format!("expected output to equal '{}'", assertion.value))
            }
        }
        AssertionKind::Regex => {
            let regex = Regex::new(&assertion.value)
                .map_err(|err| format!("invalid regex {}: {err}", assertion.value))?;
            if regex.is_match(&target) {
                Ok(())
            } else {
                Err(format!(
                    "expected output to match regex '{}'",
                    assertion.value
                ))
            }
        }
        AssertionKind::JsonPath => {
            if target.is_empty() {
                Err("json_path assertion had no matching field".to_string())
            } else if target.contains(&assertion.value) {
                Ok(())
            } else {
                Err(format!(
                    "expected json_path output to contain '{}'",
                    assertion.value
                ))
            }
        }
        AssertionKind::GoldenDiff => Err("golden diff evaluator required".to_string()),
    }
}

fn output_field(output: &str, field: Option<&str>) -> Option<String> {
    let parsed: Value = serde_json::from_str(output).ok()?;
    let field = field?;
    if field.starts_with('/') {
        parsed.pointer(field).map(json_to_string)
    } else {
        parsed.get(field).map(json_to_string)
    }
}

fn json_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
