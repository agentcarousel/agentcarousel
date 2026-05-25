//! Async **test** and **eval** execution: expand fixtures into [`Case`] rows, apply mocks or live
//! generation, optionally run evaluators (rules / golden / process / judge), and produce a [`Run`].
//!
//! [`Case`]: crate::Case
//! [`Run`]: crate::Run
//!
//! Entry points:
//! - [`run_fixtures`] — `test`-style runs (assertions + optional rules on each case).
//! - [`run_eval`] — `eval`-style runs with configurable evaluator and multi-run aggregation.
//!
//! Requires a Tokio runtime (multi-thread recommended for parallel cases).

mod aggregation;
mod batch;
mod executor;
mod generator;
mod git_revision;
mod orchestration;
mod sandbox;
mod tracer;

pub use batch::{
    AnthropicBatch, BatchCaseResult, BatchDispatcher, BatchError, BatchStateRecord,
    BatchStateStore, CaseBatchItem, OpenAiBatch,
};

use agentcarousel_core::{new_run_id, FixtureFile, Run};
use agentcarousel_fixtures::MockEngine;
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub use executor::run_case;
pub use generator::{call_llm, GeneratorProvider};
pub use sandbox::SandboxError;
pub use tracer::SecretScrubber;

/// How synthetic traces are produced for each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationMode {
    /// Use only [`crate::fixtures::MockEngine`] stubs (offline-friendly).
    MockOnly,
    /// Call the configured generator provider (live; may require API keys).
    Live,
    /// Submit all cases to the provider's async batch API (50% cost saving, async).
    Batch,
}

/// Tunables for [`run_fixtures`] (concurrency, timeouts, mocks directory, offline mode, etc.).
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub concurrency: usize,
    pub timeout_secs: u64,
    pub offline: bool,
    pub mock_dir: PathBuf,
    pub generation_mode: GenerationMode,
    pub generator_model: Option<String>,
    pub generator_max_tokens: Option<u32>,
    /// Base URL for `--generator-model custom` calls.
    pub generator_endpoint: Option<String>,
    pub fail_fast: bool,
    pub mock_strict: bool,
    pub command: String,
    pub agentcarousel_version: String,
    pub config_hash: String,
    pub run_id: Option<String>,
}

/// Extends [`RunnerConfig`] with evaluation-specific options (evaluator id, judge, thresholds,
/// multi-run seeding, optional progress bar).
#[derive(Debug, Clone)]
pub struct EvalConfig {
    pub runner: RunnerConfig,
    pub runs: u32,
    pub seed: u64,
    pub evaluator: String,
    pub judge: bool,
    pub judge_model: Option<String>,
    pub judge_max_tokens: Option<u32>,
    pub effectiveness_threshold: f32,
    /// Case-level progress bar on stderr (indicatif).
    pub progress: bool,
}

/// Execute all cases from the given fixtures using [`RunnerConfig`] and return a completed [`Run`].
pub async fn run_fixtures(fixtures: Vec<FixtureFile>, config: RunnerConfig) -> Run {
    let started_at = Utc::now();
    let (fixture_bundle_id, fixture_bundle_version) = orchestration::bundle_metadata(&fixtures);
    let run_id = config
        .run_id
        .as_ref()
        .map(|id| agentcarousel_core::RunId(id.clone()))
        .unwrap_or_else(new_run_id);
    let mock_engine = MockEngine::load_dir(&config.mock_dir).unwrap_or_default();
    let skill_or_agent = orchestration::skill_display_label(&fixtures);
    let cases = orchestration::flatten_cases(fixtures);

    let results = if config.fail_fast {
        orchestration::run_sequential(cases, &mock_engine, &config).await
    } else {
        orchestration::run_parallel(cases, &mock_engine, &config).await
    };

    let summary = aggregation::build_summary(&results);
    let git_sha = git_revision::resolve_git_sha();

    Run {
        id: run_id,
        schema_version: 1,
        started_at,
        finished_at: Some(Utc::now()),
        command: config.command,
        git_sha,
        agentcarousel_version: config.agentcarousel_version,
        config_hash: config.config_hash,
        cases: results,
        summary,
        fixture_bundle_id,
        fixture_bundle_version,
        skill_or_agent,
        runner_offline: config.offline,
        runner_mock_strict: config.mock_strict,
        runner_mock_only: config.generation_mode == GenerationMode::MockOnly,
        prompt_audit: None,
    }
}

/// Like [`run_fixtures`], but runs the eval pipeline (repeated runs, effectiveness threshold,
/// selected [`crate::Evaluator`]) and attaches [`crate::EvalScores`] when applicable.
pub async fn run_eval(fixtures: Vec<FixtureFile>, config: EvalConfig) -> Run {
    let started_at = Utc::now();
    let (fixture_bundle_id, fixture_bundle_version) = orchestration::bundle_metadata(&fixtures);
    let run_id = config
        .runner
        .run_id
        .as_ref()
        .map(|id| agentcarousel_core::RunId(id.clone()))
        .unwrap_or_else(new_run_id);
    let mock_engine = MockEngine::load_dir(&config.runner.mock_dir).unwrap_or_default();
    let skill_or_agent = orchestration::skill_display_label(&fixtures);
    let cases = orchestration::flatten_cases(fixtures);
    let judge_cache = Arc::new(Mutex::new(orchestration::BoundedCache::new(1000)));

    let results =
        orchestration::run_eval_cases(cases, &mock_engine, &config, &run_id, judge_cache).await;
    let summary = aggregation::build_summary(&results);
    let git_sha = git_revision::resolve_git_sha();

    Run {
        id: run_id,
        schema_version: 1,
        started_at,
        finished_at: Some(Utc::now()),
        command: config.runner.command,
        git_sha,
        agentcarousel_version: config.runner.agentcarousel_version,
        config_hash: config.runner.config_hash,
        cases: results,
        summary,
        fixture_bundle_id,
        fixture_bundle_version,
        skill_or_agent,
        runner_offline: config.runner.offline,
        runner_mock_strict: config.runner.mock_strict,
        runner_mock_only: config.runner.generation_mode == GenerationMode::MockOnly,
        prompt_audit: None,
    }
}
