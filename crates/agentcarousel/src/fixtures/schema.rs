use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

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
    let schema_path = match schema_location {
        SchemaLocation::Default => PathBuf::from("fixtures/schemas/skill-definition.schema.json"),
        SchemaLocation::Path(path) => path,
    };

    let schema = load_schema(&schema_path)?;
    let compiled = jsonschema::JSONSchema::compile(&schema)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))?;

    let mut issues = Vec::new();
    if let Err(errors) = compiled.validate(value) {
        for error in errors {
            issues.push(SchemaValidationIssue::ValidationError(error.to_string()));
        }
    }

    Ok(issues)
}

fn load_schema(path: &Path) -> Result<Value, SchemaValidationIssue> {
    let contents = fs::read_to_string(path)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))?;
    serde_json::from_str(&contents)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))
}
