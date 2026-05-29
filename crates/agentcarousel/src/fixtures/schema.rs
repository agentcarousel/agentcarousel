use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const EMBEDDED_SCHEMA: &str = include_str!("../../schemas/skill-definition.schema.json");

#[derive(Debug, Clone)]
pub enum SchemaLocation {
    Default,
    Path(PathBuf),
}

#[derive(Debug, Error)]
pub enum SchemaValidationIssue {
    #[error("schema error: {0}")]
    SchemaError(String),
    #[error("validation error: {0}")]
    ValidationError(String),
}

pub fn validate_fixture_value(
    value: &Value,
    schema_location: SchemaLocation,
) -> Result<Vec<SchemaValidationIssue>, SchemaValidationIssue> {
    let schema: Value = match schema_location {
        SchemaLocation::Default => serde_json::from_str(EMBEDDED_SCHEMA)
            .map_err(|e| SchemaValidationIssue::SchemaError(e.to_string()))?,
        SchemaLocation::Path(path) => load_schema(&path)?,
    };

    let mut errors = Vec::new();
    validate_value(value, &schema, "", &mut errors);
    Ok(errors
        .into_iter()
        .map(SchemaValidationIssue::ValidationError)
        .collect())
}

/// Recursively validate `value` against `schema`, collecting errors into `out`.
/// `path` is a human-readable location string built up as we descend (e.g. `cases[0].input`).
fn validate_value(value: &Value, schema: &Value, path: &str, out: &mut Vec<String>) {
    let Some(schema_obj) = schema.as_object() else {
        return;
    };

    // type
    if let Some(expected_type) = schema_obj.get("type").and_then(|t| t.as_str()) {
        let ok = match expected_type {
            "string" => value.is_string(),
            "integer" | "number" => value.is_number(),
            "array" => value.is_array(),
            "object" => value.is_object(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => true,
        };
        if !ok {
            let loc = if path.is_empty() { "<root>" } else { path };
            out.push(format!(
                "{loc}: expected {expected_type}, got {}",
                type_name(value)
            ));
            // If the type is wrong, further checks on this node are meaningless.
            return;
        }
    }

    // const
    if let Some(const_val) = schema_obj.get("const") {
        if value != const_val {
            let loc = if path.is_empty() { "<root>" } else { path };
            out.push(format!("{loc}: must equal {const_val}"));
        }
    }

    // enum
    if let Some(allowed) = schema_obj.get("enum").and_then(|e| e.as_array()) {
        if !allowed.contains(value) {
            let loc = if path.is_empty() { "<root>" } else { path };
            let choices: Vec<_> = allowed.iter().map(|v| v.to_string()).collect();
            out.push(format!("{loc}: must be one of [{}]", choices.join(", ")));
        }
    }

    // minLength
    if let Some(min) = schema_obj.get("minLength").and_then(|n| n.as_u64()) {
        if let Some(s) = value.as_str() {
            if s.len() < min as usize {
                let loc = if path.is_empty() { "<root>" } else { path };
                out.push(format!(
                    "{loc}: string length {} < minLength {min}",
                    s.len()
                ));
            }
        }
    }

    // minimum (for integers/numbers)
    if let Some(min) = schema_obj.get("minimum").and_then(|n| n.as_f64()) {
        if let Some(n) = value.as_f64() {
            if n < min {
                let loc = if path.is_empty() { "<root>" } else { path };
                out.push(format!("{loc}: value {n} is less than minimum {min}"));
            }
        }
    }

    // Object keywords: required, properties
    if let Some(obj) = value.as_object() {
        // required
        if let Some(required) = schema_obj.get("required").and_then(|r| r.as_array()) {
            for req in required {
                if let Some(key) = req.as_str() {
                    if !obj.contains_key(key) {
                        let loc = if path.is_empty() { "<root>" } else { path };
                        out.push(format!("{loc}: missing required field `{key}`"));
                    }
                }
            }
        }

        // properties — validate each key that is present in the value
        if let Some(props) = schema_obj.get("properties").and_then(|p| p.as_object()) {
            for (key, prop_schema) in props {
                if let Some(child_val) = obj.get(key.as_str()) {
                    let child_path = child_path(path, key);
                    validate_value(child_val, prop_schema, &child_path, out);
                }
            }
        }
    }

    // Array keywords: minItems, items
    if let Some(arr) = value.as_array() {
        // minItems
        if let Some(min) = schema_obj.get("minItems").and_then(|n| n.as_u64()) {
            if (arr.len() as u64) < min {
                let loc = if path.is_empty() { "<root>" } else { path };
                out.push(format!(
                    "{loc}: array has {} item(s), need at least {min}",
                    arr.len()
                ));
            }
        }

        // items
        if let Some(item_schema) = schema_obj.get("items") {
            for (i, item) in arr.iter().enumerate() {
                let item_path = format!("{}[{i}]", if path.is_empty() { "<root>" } else { path });
                validate_value(item, item_schema, &item_path, out);
            }
        }
    }
}

fn child_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn load_schema(path: &Path) -> Result<Value, SchemaValidationIssue> {
    let contents =
        fs::read_to_string(path).map_err(|e| SchemaValidationIssue::SchemaError(e.to_string()))?;
    serde_json::from_str(&contents).map_err(|e| SchemaValidationIssue::SchemaError(e.to_string()))
}
