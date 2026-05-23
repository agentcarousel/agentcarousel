use agentcarousel_core::{Case, CaseResult, EvalScores};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvaluatorError {
    #[error("missing evaluator config: {0}")]
    MissingConfig(&'static str),
    #[error("missing output for evaluation")]
    MissingOutput,
    #[error("failed to read golden file {path}: {source}")]
    GoldenRead {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("process evaluator failed: {0}")]
    ProcessFailed(String),
    #[error("judge evaluator failed: {0}")]
    JudgeFailed(String),
    /// Permanent, non-retryable judge failure (e.g. 401, 403, 404 from wrong model name).
    /// Unlike `JudgeFailed`, this error will affect every subsequent case in the run.
    #[error("judge unavailable: {0}")]
    JudgeUnavailable(String),
    #[error("invalid evaluator output: {0}")]
    InvalidOutput(String),
    #[error("unknown evaluator: {0}")]
    UnknownEvaluator(String),
}

impl EvaluatorError {
    /// Returns `true` when the error is guaranteed to repeat on every subsequent case,
    /// meaning there is no point running the generator for the rest of the run.
    pub fn is_fatal_for_run(&self) -> bool {
        matches!(self, Self::MissingConfig(_) | Self::JudgeUnavailable(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluatorKind {
    Rules,
    Golden,
    Process,
    Judge,
}

impl EvaluatorKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "rules" => Some(Self::Rules),
            "golden" => Some(Self::Golden),
            "process" => Some(Self::Process),
            "judge" => Some(Self::Judge),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Golden => "golden",
            Self::Process => "process",
            Self::Judge => "judge",
        }
    }
}

pub trait Evaluator {
    fn id(&self) -> &'static str;
    fn evaluate(&self, case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError>;
}
