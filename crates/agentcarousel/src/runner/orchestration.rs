use agentcarousel_core::{
    Case, CaseDefaults, CaseResult, CaseStatus, EvalScores, EvaluatorConfig, FixtureFile, RunId,
};
use agentcarousel_evaluators::{
    Evaluator, EvaluatorError, EvaluatorKind, GoldenEvaluator, JudgeEvaluator, ProcessEvaluator,
    RulesEvaluator,
};
use agentcarousel_fixtures::MockEngine;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

use super::{EvalConfig, RunnerConfig};

pub(super) struct BoundedCache {
    map: HashMap<String, EvalScores>,
    order: VecDeque<String>,
    capacity: usize,
}

impl BoundedCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<&EvalScores> {
        self.map.get(key)
    }

    pub(super) fn insert(&mut self, key: String, value: EvalScores) {
        if self.map.contains_key(&key) {
            return;
        }
        if self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

pub(super) fn skill_display_label(fixtures: &[FixtureFile]) -> Option<String> {
    let mut names: Vec<String> = fixtures.iter().map(|f| f.skill_or_agent.clone()).collect();
    names.sort();
    names.dedup();
    match names.len() {
        0 => None,
        1 => Some(names[0].clone()),
        _ => Some(names.join(", ")),
    }
}

pub(super) fn bundle_metadata(fixtures: &[FixtureFile]) -> (Option<String>, Option<String>) {
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

pub(super) fn flatten_cases(fixtures: Vec<FixtureFile>) -> Vec<Case> {
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

fn apply_defaults(case: &mut Case, defaults: &Option<CaseDefaults>) {
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
                case.evaluator_config = Some(EvaluatorConfig {
                    evaluator: evaluator.clone(),
                    golden_path: None,
                    golden_threshold: None,
                    process_cmd: None,
                    judge_prompt: None,
                    effectiveness_threshold: None,
                });
            }
        }
    }
}

pub(super) async fn run_sequential(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &RunnerConfig,
) -> Vec<CaseResult> {
    let mut results = Vec::new();
    for case in cases {
        let case_id = case.id.clone();
        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(case.timeout_secs.unwrap_or(config.timeout_secs)),
            super::executor::run_case(case, mock_engine, config),
        )
        .await;
        let result = match timeout {
            Ok(result) => result,
            Err(_) => super::executor::timeout_result(case_id),
        };
        let should_stop = result.status != CaseStatus::Passed;
        results.push(result);
        if config.fail_fast && should_stop {
            break;
        }
    }
    results
}

pub(super) async fn run_parallel(
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
                super::executor::run_case(case, &mock_engine, &config),
            )
            .await;
            match timeout {
                Ok(result) => result,
                Err(_) => super::executor::timeout_result(case_id),
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

pub(super) async fn run_eval_cases(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &EvalConfig,
    run_id: &RunId,
    judge_cache: Arc<Mutex<BoundedCache>>,
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

pub(super) async fn run_case_eval(
    case: Case,
    mock_engine: &MockEngine,
    config: &EvalConfig,
    run_id: &RunId,
    judge_cache: Arc<Mutex<BoundedCache>>,
) -> CaseResult {
    let runs = std::cmp::max(1, config.runs);
    let mut per_run_results = Vec::new();
    let base_seed = case.seed.unwrap_or(config.seed);

    for run_index in 0..runs {
        let mut run_case = case.clone();
        run_case.seed = Some(base_seed.wrapping_add(run_index as u64));
        let mut result =
            super::executor::run_case_unscored(run_case, mock_engine, &config.runner).await;

        if result.status == CaseStatus::Passed {
            match evaluate_case_result(&case, &result, config, run_id, &judge_cache).await {
                Ok(scores) => {
                    result.eval_scores = Some(scores.clone());
                    let threshold = case
                        .evaluator_config
                        .as_ref()
                        .and_then(|c| c.effectiveness_threshold)
                        .unwrap_or(config.effectiveness_threshold);
                    if scores.effectiveness_score < threshold {
                        result.status = CaseStatus::Failed;
                        result.error = Some(format!(
                            "effectiveness {:.2} below threshold {:.2}",
                            scores.effectiveness_score, threshold
                        ));
                    }
                }
                Err(err) => {
                    result.status = CaseStatus::Error;
                    result.error = Some(err.to_string());
                }
            }
        }

        super::aggregation::apply_provider_error_metrics(&mut result);
        per_run_results.push(result);
    }

    let threshold = case
        .evaluator_config
        .as_ref()
        .and_then(|c| c.effectiveness_threshold)
        .unwrap_or(config.effectiveness_threshold);
    super::aggregation::aggregate_case_results(&case, &per_run_results, runs, threshold)
}

async fn evaluate_case_result(
    case: &Case,
    result: &CaseResult,
    config: &EvalConfig,
    run_id: &RunId,
    judge_cache: &Arc<Mutex<BoundedCache>>,
) -> Result<EvalScores, EvaluatorError> {
    let evaluator_id = resolve_evaluator_id(case, config);
    match evaluator_id.as_str() {
        "rules" => RulesEvaluator.evaluate(case, result),
        "golden" => {
            let mut evaluator = GoldenEvaluator::from_case(case)?;
            evaluator.update = config.update_golden;
            evaluator.evaluate(case, result)
        }
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
