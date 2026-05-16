use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const EMBEDDED_SCHEMA: &str = include_str!("../../../../schemas/skill-definition.schema.json");

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
            .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))?,
        SchemaLocation::Path(path) => load_schema(&path)?,
    };

    let compiled = jsonschema::validator_for(&schema)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))?;

    let issues = compiled
        .iter_errors(value)
        .map(|err| SchemaValidationIssue::ValidationError(err.to_string()))
        .collect();

    Ok(issues)
}

fn load_schema(path: &Path) -> Result<Value, SchemaValidationIssue> {
    let contents = fs::read_to_string(path)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))?;
    serde_json::from_str(&contents)
        .map_err(|err| SchemaValidationIssue::SchemaError(err.to_string()))
}
