use agentcarousel_core::FixtureFile;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub enum FixtureSource {
    Yaml,
    Toml,
}

#[derive(Debug, Error)]
pub enum FixtureLoadError {
    #[error("path traversal detected: {0}")]
    PathTraversal(PathBuf),
    #[error("unsupported fixture extension: {0}")]
    UnsupportedExtension(PathBuf),
    #[error("failed to read fixture: {0}")]
    ReadError(PathBuf),
    #[error("failed to parse fixture: {0}")]
    ParseError(String),
}

pub fn load_fixture(path: &Path) -> Result<FixtureFile, FixtureLoadError> {
    load_typed(path)
}

pub fn load_fixture_value(path: &Path) -> Result<Value, FixtureLoadError> {
    load_typed(path)
}

fn load_typed<T: DeserializeOwned>(path: &Path) -> Result<T, FixtureLoadError> {
    guard_path(path)?;
    let contents =
        fs::read_to_string(path).map_err(|_| FixtureLoadError::ReadError(path.to_path_buf()))?;

    match detect_source(path)? {
        FixtureSource::Yaml => serde_yaml::from_str(&contents)
            .map_err(|err| FixtureLoadError::ParseError(err.to_string())),
        FixtureSource::Toml => {
            toml::from_str(&contents).map_err(|err| FixtureLoadError::ParseError(err.to_string()))
        }
    }
}

fn detect_source(path: &Path) -> Result<FixtureSource, FixtureLoadError> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    match extension {
        "yaml" | "yml" => Ok(FixtureSource::Yaml),
        "toml" => Ok(FixtureSource::Toml),
        _ => Err(FixtureLoadError::UnsupportedExtension(path.to_path_buf())),
    }
}

fn guard_path(path: &Path) -> Result<(), FixtureLoadError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(FixtureLoadError::PathTraversal(path.to_path_buf()));
    }
    Ok(())
}
