use thiserror::Error;

/// Errors surfaced by core parsing or validation helpers.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
}
