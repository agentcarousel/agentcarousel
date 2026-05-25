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
use std::sync::atomic::{AtomicBool, Ordering};
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

pub fn flatten_cases(fixtures: Vec<FixtureFile>) -> Vec<Case> {
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
        let case_input = case.input.messages.clone();
        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(case.timeout_secs.unwrap_or(config.timeout_secs)),
            super::executor::run_case(case, mock_engine, config),
        )
        .await;
        let result = match timeout {
            Ok(result) => result,
            Err(_) => super::executor::timeout_result(case_id, case_input),
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
        let case_input = case.input.messages.clone();
        let handle = tokio::spawn(async move {
            let _permit = permit;
            let timeout = tokio::time::timeout(
                std::time::Duration::from_secs(case.timeout_secs.unwrap_or(config.timeout_secs)),
                super::executor::run_case(case, &mock_engine, &config),
            )
            .await;
            match timeout {
                Ok(result) => result,
                Err(_) => super::executor::timeout_result(case_id, case_input),
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
                input: Vec::new(),
                discrimination_score: None,
                discrimination_label: None,
            }),
        }
    }
    results
}

/// Submit cases to the Anthropic batch API without blocking on results.
///
/// Saves a rich [`BatchStateRecord`] (including fixture paths and judge config) so
/// `agc batch fetch` can reconstruct the full eval pipeline later. Returns the batch ID.
pub async fn submit_batch_only(
    cases: &[agentcarousel_core::Case],
    config: &super::RunnerConfig,
    fixture_paths: Vec<String>,
    judge_model: Option<String>,
) -> Result<String, super::batch::BatchError> {
    use super::batch::{AnthropicBatch, BatchStateRecord, BatchStateStore, CaseBatchItem};
    use super::generator::{
        build_user_prompt, resolve_generator_key, resolve_system_prompt, GeneratorProvider,
    };

    let model = config
        .generator_model
        .as_deref()
        .ok_or_else(|| super::batch::BatchError::Fatal("generator model not configured".into()))?
        .to_string();
    let provider = GeneratorProvider::from_model(&model);
    if !matches!(provider, GeneratorProvider::Anthropic) {
        return Err(super::batch::BatchError::Fatal(format!(
            "fire-and-forget batch is only supported for Anthropic models; got {provider:?}"
        )));
    }
    let api_key = resolve_generator_key(provider)
        .map_err(|e| super::batch::BatchError::Fatal(e.to_string()))?;

    let max_tokens = config.generator_max_tokens.unwrap_or(2048);
    let items: Vec<CaseBatchItem> = cases
        .iter()
        .map(|case| CaseBatchItem {
            case_id: case.id.clone(),
            system: resolve_system_prompt(case),
            user_prompt: build_user_prompt(case),
            model: model.clone(),
            max_tokens,
            seed: case.seed,
        })
        .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| super::batch::BatchError::Fatal(format!("reqwest build failed: {e}")))?;

    let dispatcher = AnthropicBatch::new(api_key);
    let (batch_id, case_ids) = dispatcher.submit_only(&items, &client).await?;

    // Overwrite state with the rich record that includes fixture paths and judge config.
    let rich_state = BatchStateRecord {
        batch_id: batch_id.clone(),
        provider: "anthropic".to_string(),
        status: "in_progress".to_string(),
        created_at: chrono::Utc::now(),
        case_ids,
        fixture_paths,
        model,
        max_tokens,
        judge_model,
    };
    let _ = BatchStateStore::save(&rich_state, &std::path::PathBuf::from(".agc/batch_state"));

    Ok(batch_id)
}

/// Map a batch dispatch result to `Vec<CaseResult>`, falling back to Error for failures.
fn map_batch_results(
    dispatch_result: Result<Vec<super::batch::BatchCaseResult>, super::batch::BatchError>,
    cases: Vec<agentcarousel_core::Case>,
    case_map: std::collections::HashMap<String, Vec<agentcarousel_core::Message>>,
) -> Vec<agentcarousel_core::CaseResult> {
    use agentcarousel_core::{CaseResult, CaseStatus, ExecutionTrace, Metrics};
    match dispatch_result {
        Ok(batch_results) => batch_results
            .into_iter()
            .map(|br| {
                let input = case_map.get(&br.case_id.0).cloned().unwrap_or_default();
                let (status, error) = if br.error.is_some() {
                    (CaseStatus::Error, br.error)
                } else {
                    (CaseStatus::Passed, None)
                };
                let has_output = br.output.is_some();
                CaseResult {
                    case_id: br.case_id,
                    status,
                    error,
                    trace: ExecutionTrace {
                        steps: Vec::new(),
                        final_output: br.output,
                        redacted: false,
                    },
                    metrics: Metrics {
                        tokens_in: br.tokens_in,
                        tokens_out: br.tokens_out,
                        llm_calls: if has_output { 1 } else { 0 },
                        ..Metrics::default()
                    },
                    eval_scores: None,
                    input,
                    discrimination_score: None,
                    discrimination_label: None,
                }
            })
            .collect(),
        Err(e) => cases
            .into_iter()
            .map(|case| CaseResult {
                case_id: case.id,
                status: CaseStatus::Error,
                error: Some(e.to_string()),
                trace: ExecutionTrace {
                    steps: Vec::new(),
                    final_output: None,
                    redacted: false,
                },
                metrics: Metrics::default(),
                eval_scores: None,
                input: case.input.messages,
                discrimination_score: None,
                discrimination_label: None,
            })
            .collect(),
    }
}

pub(super) async fn run_batch(
    cases: Vec<agentcarousel_core::Case>,
    config: &super::RunnerConfig,
) -> Vec<agentcarousel_core::CaseResult> {
    use super::batch::{
        AnthropicBatch, BatchDispatcher, BatchStateStore, CaseBatchItem, OpenAiBatch,
    };
    use super::generator::{
        build_user_prompt, resolve_generator_key, resolve_system_prompt, GeneratorProvider,
    };
    use agentcarousel_core::{CaseResult, CaseStatus, ExecutionTrace, Metrics};

    // ── Collect-only path: results from an already-submitted batch ───────────────
    if let Some(ref collect_id) = config.batch_collect_id {
        let state_dir = std::path::PathBuf::from(".agc/batch_state");
        let state = match BatchStateStore::load(collect_id, &state_dir) {
            Ok(s) => s,
            Err(e) => {
                return cases
                    .into_iter()
                    .map(|case| CaseResult {
                        case_id: case.id,
                        status: CaseStatus::Error,
                        error: Some(format!("batch state load failed: {e}")),
                        trace: ExecutionTrace {
                            steps: Vec::new(),
                            final_output: None,
                            redacted: false,
                        },
                        metrics: Metrics::default(),
                        eval_scores: None,
                        input: case.input.messages,
                        discrimination_score: None,
                        discrimination_label: None,
                    })
                    .collect();
            }
        };
        let api_key = match resolve_generator_key(GeneratorProvider::Anthropic) {
            Ok(k) => k,
            Err(e) => {
                return cases
                    .into_iter()
                    .map(|case| CaseResult {
                        case_id: case.id,
                        status: CaseStatus::Error,
                        error: Some(e.to_string()),
                        trace: ExecutionTrace {
                            steps: Vec::new(),
                            final_output: None,
                            redacted: false,
                        },
                        metrics: Metrics::default(),
                        eval_scores: None,
                        input: case.input.messages,
                        discrimination_score: None,
                        discrimination_label: None,
                    })
                    .collect();
            }
        };
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return cases
                    .into_iter()
                    .map(|case| CaseResult {
                        case_id: case.id,
                        status: CaseStatus::Error,
                        error: Some(format!("reqwest build failed: {e}")),
                        trace: ExecutionTrace {
                            steps: Vec::new(),
                            final_output: None,
                            redacted: false,
                        },
                        metrics: Metrics::default(),
                        eval_scores: None,
                        input: case.input.messages,
                        discrimination_score: None,
                        discrimination_label: None,
                    })
                    .collect();
            }
        };
        let case_map: std::collections::HashMap<_, _> = cases
            .iter()
            .map(|c| (c.id.0.clone(), c.input.messages.clone()))
            .collect();
        let dispatcher = AnthropicBatch::new(api_key);
        let total = state.case_ids.len();
        let dispatch_result = dispatcher
            .collect_batch(collect_id, &state.case_ids, &client, total)
            .await;
        return map_batch_results(dispatch_result, cases, case_map);
    }

    if config.fail_fast {
        // batch mode is all-or-nothing; fail-fast is incompatible
        return cases
            .into_iter()
            .map(|case| CaseResult {
                case_id: case.id,
                status: CaseStatus::Error,
                error: Some("--fail-fast is not supported with --execution-mode batch".to_string()),
                trace: ExecutionTrace {
                    steps: Vec::new(),
                    final_output: None,
                    redacted: false,
                },
                metrics: Metrics::default(),
                eval_scores: None,
                input: case.input.messages,
                discrimination_score: None,
                discrimination_label: None,
            })
            .collect();
    }

    let model = match config.generator_model.as_deref() {
        Some(m) => m.to_string(),
        None => {
            return cases
                .into_iter()
                .map(|case| CaseResult {
                    case_id: case.id,
                    status: CaseStatus::Error,
                    error: Some("generator model is not configured for batch mode".to_string()),
                    trace: ExecutionTrace {
                        steps: Vec::new(),
                        final_output: None,
                        redacted: false,
                    },
                    metrics: Metrics::default(),
                    eval_scores: None,
                    input: case.input.messages,
                    discrimination_score: None,
                    discrimination_label: None,
                })
                .collect();
        }
    };

    let provider = GeneratorProvider::from_model(&model);
    let max_tokens = config.generator_max_tokens.unwrap_or(2048);

    // Build a lookup map so we can reconstruct CaseResult.input after dispatch
    let case_map: std::collections::HashMap<_, _> = cases
        .iter()
        .map(|c| (c.id.0.clone(), c.input.messages.clone()))
        .collect();

    // Map Case → CaseBatchItem
    let batch_items: Vec<CaseBatchItem> = cases
        .iter()
        .map(|case| CaseBatchItem {
            case_id: case.id.clone(),
            system: resolve_system_prompt(case),
            user_prompt: build_user_prompt(case),
            model: model.clone(),
            max_tokens,
            seed: case.seed,
        })
        .collect();

    // Dispatch to the appropriate batch dispatcher
    let dispatch_result = match provider {
        GeneratorProvider::Anthropic => match resolve_generator_key(provider) {
            Ok(key) => AnthropicBatch::new(key).dispatch(batch_items).await,
            Err(e) => {
                return cases
                    .into_iter()
                    .map(|case| CaseResult {
                        case_id: case.id,
                        status: CaseStatus::Error,
                        error: Some(e.to_string()),
                        trace: ExecutionTrace {
                            steps: Vec::new(),
                            final_output: None,
                            redacted: false,
                        },
                        metrics: Metrics::default(),
                        eval_scores: None,
                        input: case.input.messages,
                        discrimination_score: None,
                        discrimination_label: None,
                    })
                    .collect()
            }
        },
        GeneratorProvider::OpenAi => match resolve_generator_key(provider) {
            Ok(key) => OpenAiBatch::new(key).dispatch(batch_items).await,
            Err(e) => {
                return cases
                    .into_iter()
                    .map(|case| CaseResult {
                        case_id: case.id,
                        status: CaseStatus::Error,
                        error: Some(e.to_string()),
                        trace: ExecutionTrace {
                            steps: Vec::new(),
                            final_output: None,
                            redacted: false,
                        },
                        metrics: Metrics::default(),
                        eval_scores: None,
                        input: case.input.messages,
                        discrimination_score: None,
                        discrimination_label: None,
                    })
                    .collect()
            }
        },
        GeneratorProvider::Gemini | GeneratorProvider::OpenRouter | GeneratorProvider::Custom => {
            eprintln!(
                "warn: batch mode is not supported for {provider:?}; falling back to parallel live generation"
            );
            return run_parallel(
                cases,
                &agentcarousel_fixtures::MockEngine::default(),
                config,
            )
            .await;
        }
    };

    map_batch_results(dispatch_result, cases, case_map)
}

pub(super) async fn run_eval_cases(
    cases: Vec<Case>,
    mock_engine: &MockEngine,
    config: &EvalConfig,
    run_id: &RunId,
    judge_cache: Arc<Mutex<BoundedCache>>,
) -> Vec<CaseResult> {
    // Batch mode: collect all outputs up front, then run the evaluator pipeline on each.
    if config.runner.generation_mode == super::GenerationMode::Batch {
        let batch_case_results = run_batch(cases.clone(), &config.runner).await;
        // Build a map from case_id → CaseResult for the batch outputs
        let mut output_map: std::collections::HashMap<String, CaseResult> = batch_case_results
            .into_iter()
            .map(|r| (r.case_id.0.clone(), r))
            .collect();

        let judge_unavailable = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut results = Vec::with_capacity(cases.len());

        for case in cases {
            let case_id_str = case.id.0.clone();
            let mut result = output_map
                .remove(&case_id_str)
                .unwrap_or_else(|| CaseResult {
                    case_id: case.id.clone(),
                    status: CaseStatus::Error,
                    error: Some("batch result missing for case".to_string()),
                    trace: agentcarousel_core::ExecutionTrace {
                        steps: Vec::new(),
                        final_output: None,
                        redacted: false,
                    },
                    metrics: agentcarousel_core::Metrics::default(),
                    eval_scores: None,
                    input: case.input.messages.clone(),
                    discrimination_score: None,
                    discrimination_label: None,
                });

            if result.status == CaseStatus::Passed {
                match evaluate_case_result(&case, &result, config, run_id, &judge_cache).await {
                    Ok(scores) => {
                        result.metrics.judge_tokens_in = scores.judge_tokens_in;
                        result.metrics.judge_tokens_out = scores.judge_tokens_out;
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
                        if err.is_fatal_for_run()
                            && !judge_unavailable.swap(true, std::sync::atomic::Ordering::AcqRel)
                        {
                            eprintln!("warn: judge permanently unavailable ({}), skipping evaluator for remaining batch cases", err);
                        }
                        result.status = CaseStatus::Error;
                        result.error = Some(err.to_string());
                    }
                }
            }
            super::aggregation::apply_provider_error_metrics(&mut result);
            results.push(result);
        }
        return results;
    }

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
    let judge_unavailable = Arc::new(AtomicBool::new(false));
    let generator_unavailable = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();

    for case in cases {
        let permit = semaphore.clone().acquire_owned().await.unwrap();

        // After acquiring a slot, check whether a completed task found the judge or generator
        // permanently broken. Skip remaining cases to avoid wasting tokens.
        let judge_gone = judge_unavailable.load(Ordering::Acquire);
        let gen_gone = generator_unavailable.load(Ordering::Acquire);
        if judge_gone || gen_gone {
            drop(permit);
            let case_id = case.id.clone();
            let pb = progress_bar.clone();
            let err_msg = if gen_gone {
                "generator unavailable — case skipped to avoid wasting tokens"
            } else {
                "judge unavailable — generator skipped to avoid wasting tokens"
            };
            handles.push(tokio::spawn(async move {
                if let Some(pb) = pb {
                    pb.inc(1);
                }
                CaseResult {
                    case_id,
                    status: CaseStatus::Error,
                    error: Some(err_msg.to_string()),
                    trace: agentcarousel_core::ExecutionTrace {
                        steps: Vec::new(),
                        final_output: None,
                        redacted: false,
                    },
                    metrics: agentcarousel_core::Metrics::default(),
                    eval_scores: None,
                    input: Vec::new(),
                    discrimination_score: None,
                    discrimination_label: None,
                }
            }));
            continue;
        }

        let mock_engine = mock_engine.clone();
        let config = config.clone();
        let run_id = run_id.clone();
        let judge_cache = judge_cache.clone();
        let judge_unavailable = judge_unavailable.clone();
        let generator_unavailable = generator_unavailable.clone();
        let pb = progress_bar.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let result = run_case_eval(
                case,
                &mock_engine,
                &config,
                &run_id,
                judge_cache,
                judge_unavailable,
                generator_unavailable,
            )
            .await;
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
    judge_unavailable: Arc<AtomicBool>,
    generator_unavailable: Arc<AtomicBool>,
) -> CaseResult {
    let runs = std::cmp::max(1, config.runs);
    let mut per_run_results = Vec::new();
    let base_seed = case.seed.unwrap_or(config.seed);

    for run_index in 0..runs {
        let mut run_case = case.clone();
        run_case.seed = Some(base_seed.wrapping_add(run_index as u64));
        let mut result = super::executor::run_case_unscored(
            run_case,
            mock_engine,
            &config.runner,
            Some(generator_unavailable.clone()),
        )
        .await;

        if result.status == CaseStatus::Passed {
            match evaluate_case_result(&case, &result, config, run_id, &judge_cache).await {
                Ok(scores) => {
                    result.metrics.judge_tokens_in = scores.judge_tokens_in;
                    result.metrics.judge_tokens_out = scores.judge_tokens_out;
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
                    if err.is_fatal_for_run() && !judge_unavailable.swap(true, Ordering::AcqRel) {
                        eprintln!(
                            "warn: judge permanently unavailable ({}), skipping generator for remaining cases",
                            err
                        );
                    }
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
            let case_owned = case.clone();
            let result_owned = result.clone();
            let scores =
                tokio::task::spawn_blocking(move || evaluator.evaluate(&case_owned, &result_owned))
                    .await
                    .map_err(|_| {
                        EvaluatorError::JudgeFailed("judge task panicked".to_string())
                    })??;
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

/// Run blank-prompt and degraded-prompt passes to compute per-case discrimination scores.
///
/// Returns a `Vec` aligned with `cases` — one `(score, label)` per case.
/// Uses `run_parallel` for all three internal passes (mock engine is unused here;
/// pass `MockEngine::default()` since generation_mode is Live or Batch).
///
/// Caller passes the `current_passed` outcomes from the already-completed main run.
pub async fn run_discrimination(
    cases: Vec<agentcarousel_core::Case>,
    config: &super::RunnerConfig,
    current_passed: &[bool],
) -> Vec<(f32, String)> {
    use super::generator::resolve_system_prompt;
    use agentcarousel_core::{CaseStatus, Message, Role};
    use agentcarousel_fixtures::MockEngine;

    let mock_engine = MockEngine::default();

    // Build blank-prompt cases (system message = empty string)
    let blank_cases: Vec<agentcarousel_core::Case> = cases
        .iter()
        .map(|c| {
            let mut case = c.clone();
            if let Some(sys_msg) = case
                .input
                .messages
                .iter_mut()
                .find(|m| m.role == Role::System)
            {
                sys_msg.content = String::new();
            } else {
                case.input.messages.insert(
                    0,
                    Message {
                        role: Role::System,
                        content: String::new(),
                    },
                );
            }
            case
        })
        .collect();

    // Build degraded-prompt cases (first 20% of resolved system prompt)
    let degraded_cases: Vec<agentcarousel_core::Case> = cases
        .iter()
        .map(|c| {
            let full_system = resolve_system_prompt(c);
            let cutoff = (full_system.len() as f32 * 0.2) as usize;
            // Round up to the next char boundary
            let cutoff = full_system
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i < cutoff.max(1))
                .last()
                .map(|i| i + 1)
                .unwrap_or(0);
            let degraded_system = full_system[..cutoff].to_string();

            let mut case = c.clone();
            if let Some(sys_msg) = case
                .input
                .messages
                .iter_mut()
                .find(|m| m.role == Role::System)
            {
                sys_msg.content = degraded_system;
            } else {
                case.input.messages.insert(
                    0,
                    Message {
                        role: Role::System,
                        content: degraded_system,
                    },
                );
            }
            case
        })
        .collect();

    // Run both passes concurrently
    let (blank_results, degraded_results) = tokio::join!(
        run_parallel(blank_cases, &mock_engine, config),
        run_parallel(degraded_cases, &mock_engine, config),
    );

    // Compute score per case
    cases
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let cur = *current_passed.get(i).unwrap_or(&false);
            let blank_ok = blank_results
                .get(i)
                .map(|r| r.status == CaseStatus::Passed)
                .unwrap_or(false);
            let deg_ok = degraded_results
                .get(i)
                .map(|r| r.status == CaseStatus::Passed)
                .unwrap_or(false);

            let degraded_passed = blank_ok || deg_ok;
            let score = (cur as i32 - degraded_passed as i32) as f32;
            let label = if score > 0.2 {
                "high".to_string()
            } else if score <= 0.0 {
                "low".to_string()
            } else {
                "marginal".to_string()
            };
            (score, label)
        })
        .collect()
}
