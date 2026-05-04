//! Shared domain types for fixtures, runs, traces, and metrics, plus [`CoreError`] and
//! [`judge_provider`] helpers for LLM-backed judges.

pub mod error;
pub mod ids;
pub mod judge_provider;
pub mod models;
pub mod retry;

pub use error::CoreError;
pub use ids::new_run_id;
pub use judge_provider::{judge_key_candidates, judge_provider_from_model, JudgeProvider};
pub use models::*;
pub use retry::{compute_backoff_ms, is_retryable_status, retry_policy, RetryPolicy};
