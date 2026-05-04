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

mod executor;
mod generator;
mod git_revision;
mod sandbox;
mod tracer;

use agentcarousel_core::{
    new_run_id, Case, CaseResult, CaseStatus, EvalScores, FixtureFile, OverallStatus,
    ProviderErrorMetrics, RubricScore, Run, RunSummary,
};
use agentcarousel_evaluators::{
    Evaluator, EvaluatorError, EvaluatorKind, GoldenEvaluator, JudgeEvaluator, ProcessEvaluator,
    RulesEvaluator,
};
use agentcarousel_fixtures::MockEngine;
use chrono::Utc;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

pub use executor::run_case;
pub use generator::GeneratorProvider;
pub use sandbox::SandboxError;
pub use tracer::SecretScrubber;

/// How synthetic traces are produced for each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationMode {
    /// Use only [`crate::fixtures::MockEngine`] stubs (offline-friendly).
    MockOnly,
    /// Call the configured generator provider (live; may require API keys).
    Live,
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
    pub certification_context: Option<agentcarousel_core::CertificationContext>,
    pub carousel_iteration: Option<u32>,
    pub policy_version: Option<String>,
    /// Case-level progress bar on stderr (indicatif).
    pub progress: bool,
}

/// Execute all cases from the given fixtures using [`RunnerConfig`] and return a completed [`Run`].
pub async fn run_fixtures(fixtures: Vec<FixtureFile>, config: RunnerConfig) -> Run {
    let started_at = Utc::now();
    let (fixture_bundle_id, fixture_bundle_version) = bundle_metadata(&fixtures);
    let run_id = config
        .run_id
        .as_ref()
        .map(|id| agentcarousel_core::RunId(id.clone()))
        .unwrap_or_else(new_run_id);
    let mock_engine = MockEngine::load_dir(&config.mock_dir).unwrap_or_default();
    let cases = flatten_cases(fixtures);

    let results = if config.fail_fast {
        run_sequential(cases, &mock_engine, &config).await
    } else {
        run_parallel(cases, &mock_engine, &config).await
    };

    let summary = build_summary(&results);
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
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
    }
}

/// Like [`run_fixtures`], but runs the eval pipeline (repeated runs, effectiveness threshold,
/// selected [`crate::Evaluator`]) and attaches [`crate::EvalScores`] when applicable.
pub async fn run_eval(fixtures: Vec<FixtureFile>, config: EvalConfig) -> Run {
    let started_at = Utc::now();
    let (fixture_bundle_id, fixture_bundle_version) = bundle_metadata(&fixtures);
    let run_id = config
        .runner
        .run_id
        .as_ref()
        .map(|id| agentcarousel_core::RunId(id.clone()))
        .unwrap_or_else(new_run_id);
    let mock_engine = MockEngine::load_dir(&config.runner.mock_dir).unwrap_or_default();
    let cases = flatten_cases(fixtures);
    let judge_cache = Arc::new(Mutex::new(HashMap::new()));

    let results = run_eval_cases(cases, &mock_engine, &config, &run_id, judge_cache).await;
    let summary = build_summary(&results);
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
        carousel_iteration: config.carousel_iteration,
        certification_context: config.certification_context,
        policy_version: config.policy_version,
    }
}

fn bundle_metadata(fixtures: &[FixtureFile]) -> (Option<String>, Option<String>) {
    // Only carry bundle metadata when all fixtures agree on the same value.
    let mut bundle_ids = HashSet::new();
    let mut bundle_versions = HashSet::new();
    for fixture in fixtures {
        if let Some(bundle_id) = fixture.bundle_id.as_ref() {
            bundle_ids.insert(bundle_id.clone());
        }
        if let Some(bundle_version) = fixture.bundle_version.as_ref() {
            bundle_versions.insert(bundle_version.clone());
        }
    }
    let bundle_id = if bundle_ids.len() == 1 {
        bundle_ids.into_iter().next()
    } else {
        None
    };
    let bundle_version = if bundle_versions.len() == 1 {
        bundle_versions.into_iter().next()
    } else {
        None
    };
    (bundle_id, bundle_version)
}

async fn run_sequential(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &RunnerConfig,
) -> Vec<CaseResult> {
    let mut results = Vec::new();
    for case in cases {
        let case_id = case.id.clone();
        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(case.timeout_secs.unwrap_or(config.timeout_secs)),
            executor::run_case(case, mock_engine, config),
        )
        .await;
        let result = match timeout {
            Ok(result) => result,
            Err(_) => executor::timeout_result(case_id),
        };
        let should_stop = result.status != agentcarousel_core::CaseStatus::Passed;
        results.push(result);
        if config.fail_fast && should_stop {
            break;
        }
    }
    results
}

async fn run_parallel(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &RunnerConfig,
) -> Vec<CaseResult> {
    let concurrency = std::cmp::max(1, config.concurrency);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles: Vec<(agentcarousel_core::CaseId, _)> = Vec::new();

    for case in cases {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let mock_engine = mock_engine.clone();
        let config = config.clone();
        let case_id = case.id.clone();
        let case_id_for_tuple = case_id.clone();
        let handle = tokio::spawn(async move {
            let _permit = permit;
            let timeout = tokio::time::timeout(
                std::time::Duration::from_secs(case.timeout_secs.unwrap_or(config.timeout_secs)),
                executor::run_case(case, &mock_engine, &config),
            )
            .await;
            match timeout {
                Ok(result) => result,
                Err(_) => executor::timeout_result(case_id),
            }
        });
        handles.push((case_id_for_tuple, handle));
    }

    let mut results = Vec::new();
    for (case_id, handle) in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(err) => results.push(CaseResult {
                case_id,
                status: CaseStatus::Error,
                error: Some(format!("task panicked: {err}")),
                trace: agentcarousel_core::ExecutionTrace {
                    steps: Vec::new(),
                    final_output: None,
                    redacted: false,
                },
                metrics: agentcarousel_core::Metrics::default(),
                eval_scores: None,
            }),
        }
    }
    results
}

async fn run_eval_cases(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &EvalConfig,
    run_id: &agentcarousel_core::RunId,
    judge_cache: Arc<Mutex<HashMap<String, EvalScores>>>,
) -> Vec<CaseResult> {
    let progress_bar: Option<ProgressBar> = if config.progress && !cases.is_empty() {
        let pb = ProgressBar::new(cases.len() as u64);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} cases {msg}",
            )
            .expect("progress template")
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.set_message("");
        pb.enable_steady_tick(Duration::from_millis(120));
        Some(pb)
    } else {
        None
    };

    let concurrency = std::cmp::max(1, config.runner.concurrency);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for case in cases {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let mock_engine = mock_engine.clone();
        let config = config.clone();
        let run_id = run_id.clone();
        let judge_cache = judge_cache.clone();
        let pb = progress_bar.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let result = run_case_eval(case, &mock_engine, &config, &run_id, judge_cache).await;
            if let Some(pb) = pb {
                pb.inc(1);
            }
            result
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }
    if let Some(pb) = progress_bar {
        pb.finish_and_clear();
    }
    results
}

async fn run_case_eval(
    case: Case,
    mock_engine: &MockEngine,
    config: &EvalConfig,
    run_id: &agentcarousel_core::RunId,
    judge_cache: Arc<Mutex<HashMap<String, EvalScores>>>,
) -> CaseResult {
    let runs = std::cmp::max(1, config.runs);
    let mut per_run_results = Vec::new();
    let base_seed = case.seed.unwrap_or(config.seed);

    for run_index in 0..runs {
        let mut run_case = case.clone();
        run_case.seed = Some(base_seed.wrapping_add(run_index as u64));
        let mut result = executor::run_case_unscored(run_case, mock_engine, &config.runner).await;

        if result.status == CaseStatus::Passed {
            match evaluate_case_result(&case, &result, config, run_id, &judge_cache).await {
                Ok(scores) => {
                    result.eval_scores = Some(scores.clone());
                    if scores.effectiveness_score < config.effectiveness_threshold {
                        result.status = CaseStatus::Failed;
                        result.error = Some(format!(
                            "effectiveness {:.2} below threshold {:.2}",
                            scores.effectiveness_score, config.effectiveness_threshold
                        ));
                    }
                }
                Err(err) => {
                    result.status = CaseStatus::Error;
                    result.error = Some(err.to_string());
                }
            }
        }

        apply_provider_error_metrics(&mut result);
        per_run_results.push(result);
    }

    aggregate_case_results(
        &case,
        &per_run_results,
        runs,
        config.effectiveness_threshold,
    )
}

async fn evaluate_case_result(
    case: &Case,
    result: &CaseResult,
    config: &EvalConfig,
    run_id: &agentcarousel_core::RunId,
    judge_cache: &Arc<Mutex<HashMap<String, EvalScores>>>,
) -> Result<EvalScores, EvaluatorError> {
    let evaluator_id = resolve_evaluator_id(case, config);
    match evaluator_id.as_str() {
        "rules" => RulesEvaluator.evaluate(case, result),
        "golden" => GoldenEvaluator::from_case(case)?.evaluate(case, result),
        "process" => ProcessEvaluator::from_case(case)?.evaluate(case, result),
        "judge" => {
            if !config.judge {
                return Err(EvaluatorError::MissingConfig(
                    "--judge must be enabled when judge evaluator is selected",
                ));
            }
            let cache_key = format!("{}:{}", run_id.0, case.id.0);
            if let Some(cached) = judge_cache.lock().await.get(&cache_key).cloned() {
                return Ok(cached);
            }
            let evaluator = JudgeEvaluator::from_case(
                case,
                config.judge_model.as_deref(),
                config.judge_max_tokens,
            )?;
            let scores = evaluator.evaluate(case, result)?;
            judge_cache.lock().await.insert(cache_key, scores.clone());
            Ok(scores)
        }
        other => Err(EvaluatorError::UnknownEvaluator(other.to_string())),
    }
}

fn resolve_evaluator_id(case: &Case, config: &EvalConfig) -> String {
    if config.evaluator == "all" {
        case.evaluator_config
            .as_ref()
            .map(|config| config.evaluator.clone())
            .unwrap_or_else(|| EvaluatorKind::Rules.as_str().to_string())
    } else {
        config.evaluator.clone()
    }
}

fn aggregate_case_results(
    case: &Case,
    results: &[CaseResult],
    runs: u32,
    effectiveness_threshold: f32,
) -> CaseResult {
    let status = aggregate_status(results);
    let metrics = aggregate_metrics(results, runs);
    let eval_scores = aggregate_eval_scores(results, effectiveness_threshold);
    let representative = results
        .iter()
        .find(|result| result.status == CaseStatus::Passed)
        .unwrap_or_else(|| results.first().expect("at least one run"));

    let error = if status == CaseStatus::Flaky {
        Some("inconsistent results across runs".to_string())
    } else {
        representative.error.clone()
    };

    CaseResult {
        case_id: case.id.clone(),
        status,
        error,
        trace: representative.trace.clone(),
        metrics,
        eval_scores,
    }
}

fn aggregate_status(results: &[CaseResult]) -> CaseStatus {
    let unique: HashSet<CaseStatus> = results.iter().map(|result| result.status.clone()).collect();
    if unique.len() == 1 {
        unique.into_iter().next().unwrap_or(CaseStatus::Error)
    } else {
        CaseStatus::Flaky
    }
}

fn aggregate_metrics(results: &[CaseResult], runs: u32) -> agentcarousel_core::Metrics {
    let mut metrics = agentcarousel_core::Metrics::default();
    let count = results.len() as u64;
    if count == 0 {
        return metrics;
    }

    let sum_latency: u64 = results
        .iter()
        .map(|result| result.metrics.total_latency_ms)
        .sum();
    let sum_llm: u32 = results.iter().map(|result| result.metrics.llm_calls).sum();
    let sum_tool: u32 = results.iter().map(|result| result.metrics.tool_calls).sum();
    let sum_steps: u32 = results
        .iter()
        .map(|result| result.metrics.total_steps)
        .sum();

    let (tokens_in_sum, tokens_in_count) = sum_optional_u64(results, |metrics| metrics.tokens_in);
    let (tokens_out_sum, tokens_out_count) =
        sum_optional_u64(results, |metrics| metrics.tokens_out);
    let (cost_sum, cost_count) = sum_optional_f64(results, |metrics| metrics.estimated_cost_usd);

    let mean_latency = sum_latency as f64 / count as f64;
    metrics.total_latency_ms = mean_latency.round() as u64;
    metrics.llm_calls = sum_llm / count as u32;
    metrics.tool_calls = sum_tool / count as u32;
    metrics.total_steps = sum_steps / count as u32;
    metrics.tokens_in = tokens_in_count.map(|count| tokens_in_sum / count);
    metrics.tokens_out = tokens_out_count.map(|count| tokens_out_sum / count);
    metrics.estimated_cost_usd = cost_count.map(|count| cost_sum / count as f64);
    if count > 1 {
        let latency_variance = results
            .iter()
            .map(|result| {
                let diff = result.metrics.total_latency_ms as f64 - mean_latency;
                diff * diff
            })
            .sum::<f64>()
            / count as f64;
        metrics.latency_variance_ms2 = Some(latency_variance);
        metrics.latency_stddev_ms = Some(latency_variance.sqrt());
    }
    let (effectiveness_variance, effectiveness_stddev) = effectiveness_variance_stats(results);
    metrics.effectiveness_variance = effectiveness_variance;
    metrics.effectiveness_stddev = effectiveness_stddev;
    metrics.runs_attempted = runs;
    metrics.runs_succeeded = results
        .iter()
        .filter(|result| result.status == CaseStatus::Passed)
        .count() as u32;
    if runs > 0 {
        metrics.error_rate =
            Some(1.0 - (metrics.runs_succeeded as f32 / metrics.runs_attempted as f32));
    }
    metrics.consistency_score = Some(consistency_score(results));
    metrics.provider_errors = sum_provider_errors(results);
    metrics
}

fn sum_optional_u64(
    results: &[CaseResult],
    getter: fn(&agentcarousel_core::Metrics) -> Option<u64>,
) -> (u64, Option<u64>) {
    let mut sum = 0;
    let mut count = 0;
    for result in results {
        if let Some(value) = getter(&result.metrics) {
            sum += value;
            count += 1;
        }
    }
    if count == 0 {
        (0, None)
    } else {
        (sum, Some(count))
    }
}

fn sum_optional_f64(
    results: &[CaseResult],
    getter: fn(&agentcarousel_core::Metrics) -> Option<f64>,
) -> (f64, Option<u64>) {
    let mut sum = 0.0;
    let mut count = 0;
    for result in results {
        if let Some(value) = getter(&result.metrics) {
            sum += value;
            count += 1;
        }
    }
    if count == 0 {
        (0.0, None)
    } else {
        (sum, Some(count))
    }
}

fn effectiveness_variance_stats(results: &[CaseResult]) -> (Option<f32>, Option<f32>) {
    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut count = 0.0_f64;
    for result in results {
        if let Some(scores) = result.eval_scores.as_ref() {
            let value = scores.effectiveness_score as f64;
            sum += value;
            sum_sq += value * value;
            count += 1.0;
        }
    }
    if count <= 1.0 {
        return (None, None);
    }
    let mean = sum / count;
    let variance = (sum_sq / count) - (mean * mean);
    let variance = variance.max(0.0);
    let stddev = variance.sqrt();
    (Some(variance as f32), Some(stddev as f32))
}

fn sum_provider_errors(results: &[CaseResult]) -> ProviderErrorMetrics {
    let mut metrics = ProviderErrorMetrics::default();
    for result in results {
        metrics.status_429 += result.metrics.provider_errors.status_429;
        metrics.status_500 += result.metrics.provider_errors.status_500;
        metrics.status_503 += result.metrics.provider_errors.status_503;
        metrics.status_504 += result.metrics.provider_errors.status_504;
    }
    metrics
}

fn apply_provider_error_metrics(result: &mut CaseResult) {
    let Some(error) = result.error.as_deref() else {
        return;
    };
    let Some(status) = extract_http_status(error) else {
        return;
    };
    match status {
        429 => result.metrics.provider_errors.status_429 += 1,
        500 => result.metrics.provider_errors.status_500 += 1,
        503 => result.metrics.provider_errors.status_503 += 1,
        504 => result.metrics.provider_errors.status_504 += 1,
        _ => {}
    }
}

fn extract_http_status(error: &str) -> Option<u16> {
    let candidates = [429_u16, 500, 503, 504];
    for code in candidates {
        let code_str = code.to_string();
        let patterns = [
            format!("({code_str}"),
            format!(" {code_str} "),
            format!(" {code_str}:"),
            format!(" {code_str})"),
        ];
        if patterns.iter().any(|pattern| error.contains(pattern)) {
            return Some(code);
        }
    }
    None
}

fn aggregate_eval_scores(
    results: &[CaseResult],
    effectiveness_threshold: f32,
) -> Option<EvalScores> {
    let collected: Vec<&EvalScores> = results
        .iter()
        .filter_map(|result| result.eval_scores.as_ref())
        .collect();
    if collected.is_empty() {
        return None;
    }

    let evaluator = collected
        .first()
        .map(|scores| scores.evaluator.clone())
        .unwrap_or_else(|| EvaluatorKind::Rules.as_str().to_string());
    let effectiveness_score = collected
        .iter()
        .map(|scores| scores.effectiveness_score)
        .sum::<f32>()
        / collected.len() as f32;

    let mut rubric_map: HashMap<String, (f32, f32, u32, Option<String>)> = HashMap::new();
    for scores in &collected {
        for rubric in &scores.rubric_scores {
            let entry =
                rubric_map
                    .entry(rubric.rubric_id.clone())
                    .or_insert((0.0, rubric.weight, 0, None));
            entry.0 += rubric.score;
            entry.2 += 1;
            if entry.3.is_none() {
                entry.3 = rubric.rationale.clone();
            }
        }
    }
    let rubric_scores = rubric_map
        .into_iter()
        .map(
            |(rubric_id, (sum_score, weight, count, rationale))| RubricScore {
                rubric_id,
                score: if count == 0 {
                    0.0
                } else {
                    sum_score / count as f32
                },
                weight,
                rationale,
            },
        )
        .collect();

    let judge_rationale = collected
        .iter()
        .find_map(|scores| scores.judge_rationale.clone());

    Some(EvalScores {
        evaluator,
        rubric_scores,
        effectiveness_score,
        passed: effectiveness_score >= effectiveness_threshold,
        judge_rationale,
    })
}

fn consistency_score(results: &[CaseResult]) -> f32 {
    if results.len() <= 1 {
        return 1.0;
    }
    let mut counts: HashMap<String, u32> = HashMap::new();
    for result in results {
        let signature = format!(
            "{:?}|{}",
            result.status,
            result.trace.final_output.clone().unwrap_or_default()
        );
        *counts.entry(signature).or_insert(0) += 1;
    }
    let max = counts.values().copied().max().unwrap_or(0) as f32;
    max / results.len() as f32
}

fn flatten_cases(fixtures: Vec<FixtureFile>) -> Vec<Case> {
    let mut cases = Vec::new();
    for fixture in fixtures {
        let defaults = fixture.defaults.clone();
        for mut case in fixture.cases {
            apply_defaults(&mut case, &defaults);
            cases.push(case);
        }
    }
    cases
}

fn apply_defaults(case: &mut Case, defaults: &Option<agentcarousel_core::CaseDefaults>) {
    if let Some(defaults) = defaults {
        if case.timeout_secs.is_none() {
            case.timeout_secs = defaults.timeout_secs;
        }
        if case.tags.is_empty() {
            if let Some(tags) = defaults.tags.as_ref() {
                case.tags = tags.clone();
            }
        }
        if case.evaluator_config.is_none() {
            if let Some(evaluator) = defaults.evaluator.as_ref() {
                case.evaluator_config = Some(agentcarousel_core::EvaluatorConfig {
                    evaluator: evaluator.clone(),
                    golden_path: None,
                    golden_threshold: None,
                    process_cmd: None,
                    judge_prompt: None,
                });
            }
        }
    }
}

fn build_summary(results: &[CaseResult]) -> RunSummary {
    let total = results.len() as u32;
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut flaky = 0;
    let mut errored = 0;
    let mut timed_out = 0;
    let mut latency_sum = 0u64;
    let mut effectiveness_sum = 0.0;
    let mut effectiveness_count = 0u32;
    let mut provider_errors = ProviderErrorMetrics::default();

    for result in results {
        latency_sum += result.metrics.total_latency_ms;
        provider_errors.status_429 += result.metrics.provider_errors.status_429;
        provider_errors.status_500 += result.metrics.provider_errors.status_500;
        provider_errors.status_503 += result.metrics.provider_errors.status_503;
        provider_errors.status_504 += result.metrics.provider_errors.status_504;
        if let Some(scores) = result.eval_scores.as_ref() {
            effectiveness_sum += scores.effectiveness_score;
            effectiveness_count += 1;
        }
        match result.status {
            agentcarousel_core::CaseStatus::Passed => passed += 1,
            agentcarousel_core::CaseStatus::Failed => failed += 1,
            agentcarousel_core::CaseStatus::Skipped => skipped += 1,
            agentcarousel_core::CaseStatus::Flaky => flaky += 1,
            agentcarousel_core::CaseStatus::TimedOut => timed_out += 1,
            agentcarousel_core::CaseStatus::Error => errored += 1,
        }
    }

    let effective_total = total.saturating_sub(flaky);
    let pass_rate = if effective_total == 0 {
        0.0
    } else {
        passed as f32 / effective_total as f32
    };
    let mean_latency_ms = if total == 0 {
        0.0
    } else {
        latency_sum as f64 / total as f64
    };
    let mean_effectiveness_score = if effectiveness_count == 0 {
        None
    } else {
        Some(effectiveness_sum / effectiveness_count as f32)
    };
    let overall_status = if failed == 0 && timed_out == 0 && errored == 0 && flaky == 0 {
        OverallStatus::Pass
    } else {
        OverallStatus::Fail
    };

    RunSummary {
        total,
        passed,
        failed,
        skipped,
        flaky,
        errored,
        timed_out,
        pass_rate,
        mean_latency_ms,
        mean_effectiveness_score,
        provider_errors,
        overall_status,
    }
}
