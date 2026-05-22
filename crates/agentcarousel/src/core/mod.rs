//! Shared domain types for fixtures, runs, traces, and metrics, plus [`CoreError`] and
//! [`judge_provider`] helpers for LLM-backed judges.

pub(crate) mod hex_util;
pub mod judge_provider;
pub mod model_pricing;
pub mod models;
pub mod retry;

pub use judge_provider::{judge_key_candidates, judge_provider_from_model, JudgeProvider};
pub use model_pricing::{
    annotate_run_cost, compute_costs, fmt_cost, fmt_tokens, lookup_pricing, prefetch_pricing,
    ModelPricing,
};
pub use models::*;
pub use retry::{compute_backoff_ms, is_retryable_status, retry_policy, RetryPolicy};
