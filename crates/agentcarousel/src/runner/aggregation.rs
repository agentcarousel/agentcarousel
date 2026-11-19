use agentcarousel_core::{
    Case, CaseId, CaseResult, CaseStatus, EvalScores, ExecutionTrace, Metrics, OverallStatus,
    ProviderErrorMetrics, RubricScore, RunSummary,
};
use agentcarousel_evaluators::EvaluatorKind;
use std::collections::{HashMap, HashSet};

/// Not-tested placeholder for a suite case that produced no result (assurance spec INV-1).
fn not_tested_result(
    case_id: CaseId,
    tags: Vec<String>,
    critical: bool,
    reason: &str,
) -> CaseResult {
    CaseResult {
        case_id,
        status: CaseStatus::Skipped,
        error: None,
        trace: ExecutionTrace {
            steps: Vec::new(),
            final_output: None,
            redacted: false,
        },
        metrics: Metrics::default(),
        eval_scores: None,
        input: Vec::new(),
        discrimination_score: None,
        discrimination_label: None,
        tags,
        skip_reason: Some(reason.to_string()),
        critical,
    }
}

/// A case the runner was asked to execute: id, tags, and whether it is critical.
pub(super) type PlannedCase = (CaseId, Vec<String>, bool);

/// Record every suite case that has no result: planned cases never reached (e.g. after
/// `--fail-fast`) as `not_executed`, and filtered-out cases as `excluded_by_filter`.
pub(super) fn append_not_tested(
    results: &mut Vec<CaseResult>,
    planned: &[PlannedCase],
    excluded: Vec<Case>,
) {
    let seen: HashSet<String> = results.iter().map(|r| r.case_id.0.clone()).collect();
    for (case_id, tags, critical) in planned {
        if !seen.contains(&case_id.0) {
            results.push(not_tested_result(
                case_id.clone(),
                tags.clone(),
                *critical,
                "not_executed",
            ));
        }
    }
    for case in excluded {
        if !seen.contains(&case.id.0) {
            let critical = case.is_critical();
            results.push(not_tested_result(
                case.id,
                case.tags,
                critical,
                "excluded_by_filter",
            ));
        }
    }
}

/// Copy each planned case's criticality onto its result.
pub(super) fn mark_critical(results: &mut [CaseResult], planned: &[PlannedCase]) {
    let critical: HashSet<&str> = planned
        .iter()
        .filter(|(_, _, critical)| *critical)
        .map(|(id, _, _)| id.0.as_str())
        .collect();
    for result in results.iter_mut() {
        if critical.contains(result.case_id.0.as_str()) {
            result.critical = true;
        }
    }
}

pub(super) fn aggregate_case_results(
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
        input: case.input.messages.clone(),
        discrimination_score: None,
        discrimination_label: None,
        tags: case.tags.clone(),
        skip_reason: None,
        critical: false,
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

fn aggregate_metrics(results: &[CaseResult], runs: u32) -> Metrics {
    let mut metrics = Metrics::default();
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

    let (tokens_in_sum, tokens_in_count) = sum_optional_u64(results, |m| m.tokens_in);
    let (tokens_out_sum, tokens_out_count) = sum_optional_u64(results, |m| m.tokens_out);
    let (cost_sum, cost_count) = sum_optional_f64(results, |m| m.estimated_cost_usd);
    let (judge_in_sum, judge_in_count) = sum_optional_u64(results, |m| m.judge_tokens_in);
    let (judge_out_sum, judge_out_count) = sum_optional_u64(results, |m| m.judge_tokens_out);

    let mean_latency = sum_latency as f64 / count as f64;
    metrics.total_latency_ms = mean_latency.round() as u64;
    metrics.llm_calls = sum_llm / count as u32;
    metrics.tool_calls = sum_tool / count as u32;
    metrics.total_steps = sum_steps / count as u32;
    metrics.tokens_in = tokens_in_count.map(|count| tokens_in_sum / count);
    metrics.tokens_out = tokens_out_count.map(|count| tokens_out_sum / count);
    metrics.estimated_cost_usd = cost_count.map(|count| cost_sum / count as f64);
    metrics.judge_tokens_in = judge_in_count.map(|count| judge_in_sum / count);
    metrics.judge_tokens_out = judge_out_count.map(|count| judge_out_sum / count);
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
    getter: fn(&Metrics) -> Option<u64>,
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
    getter: fn(&Metrics) -> Option<f64>,
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
    let variance = ((sum_sq / count) - (mean * mean)).max(0.0);
    let stddev = variance.sqrt();
    (Some(variance as f32), Some(stddev as f32))
}

pub(super) fn sum_provider_errors(results: &[CaseResult]) -> ProviderErrorMetrics {
    let mut metrics = ProviderErrorMetrics::default();
    for result in results {
        metrics.status_429 += result.metrics.provider_errors.status_429;
        metrics.status_500 += result.metrics.provider_errors.status_500;
        metrics.status_503 += result.metrics.provider_errors.status_503;
        metrics.status_504 += result.metrics.provider_errors.status_504;
    }
    metrics
}

pub(super) fn apply_provider_error_metrics(result: &mut CaseResult) {
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
            format!("({code_str})"),
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

    type RubricAccumulator = (
        f32,
        f32,
        u32,
        Option<String>,
        Option<agentcarousel_core::ScoreSource>,
        bool,
    );
    let mut rubric_map: HashMap<String, RubricAccumulator> = HashMap::new();
    for scores in &collected {
        for rubric in &scores.rubric_scores {
            let entry = rubric_map.entry(rubric.rubric_id.clone()).or_insert((
                0.0,
                rubric.weight,
                0,
                None,
                None,
                false,
            ));
            entry.0 += rubric.score;
            entry.2 += 1;
            if entry.3.is_none() {
                entry.3 = rubric.rationale.clone();
            }
            entry.4 = merge_score_source(entry.4, rubric.source);
            entry.5 |= rubric.critical;
        }
    }
    let rubric_scores = rubric_map
        .into_iter()
        .map(
            |(rubric_id, (sum_score, weight, count, rationale, source, critical))| RubricScore {
                rubric_id,
                score: if count == 0 {
                    0.0
                } else {
                    sum_score / count as f32
                },
                weight,
                rationale,
                source,
                critical,
            },
        )
        .collect();

    let judge_rationale = collected
        .iter()
        .find_map(|scores| scores.judge_rationale.clone());

    let judge_tokens_in = collected
        .iter()
        .filter_map(|s| s.judge_tokens_in)
        .reduce(|a, b| a + b);
    let judge_tokens_out = collected
        .iter()
        .filter_map(|s| s.judge_tokens_out)
        .reduce(|a, b| a + b);

    Some(EvalScores {
        evaluator,
        rubric_scores,
        effectiveness_score,
        passed: effectiveness_score >= effectiveness_threshold,
        judge_rationale,
        judge_tokens_in,
        judge_tokens_out,
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

pub(super) fn build_summary(results: &[CaseResult]) -> RunSummary {
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

    let mut tokens_in_sum = 0u64;
    let mut tokens_out_sum = 0u64;
    let mut has_tokens = false;
    let mut token_case_count = 0u64;
    let mut judge_tokens_in_sum = 0u64;
    let mut judge_tokens_out_sum = 0u64;
    let mut has_judge_tokens = false;

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
        if result.metrics.tokens_in.is_some() || result.metrics.tokens_out.is_some() {
            has_tokens = true;
            token_case_count += 1;
            tokens_in_sum += result.metrics.tokens_in.unwrap_or(0);
            tokens_out_sum += result.metrics.tokens_out.unwrap_or(0);
        }
        if result.metrics.judge_tokens_in.is_some() || result.metrics.judge_tokens_out.is_some() {
            has_judge_tokens = true;
            judge_tokens_in_sum += result.metrics.judge_tokens_in.unwrap_or(0);
            judge_tokens_out_sum += result.metrics.judge_tokens_out.unwrap_or(0);
        }
        match result.status {
            CaseStatus::Passed => passed += 1,
            CaseStatus::Failed => failed += 1,
            CaseStatus::Skipped => skipped += 1,
            CaseStatus::Flaky => flaky += 1,
            CaseStatus::TimedOut => timed_out += 1,
            CaseStatus::Error => errored += 1,
        }
    }

    let effective_total = total.saturating_sub(flaky).saturating_sub(skipped);
    let pass_rate = if effective_total == 0 {
        0.0
    } else {
        passed as f32 / effective_total as f32
    };
    // Latency describes executed cases only; not-tested placeholders have none.
    let executed = total.saturating_sub(skipped);
    let mean_latency_ms = if executed == 0 {
        0.0
    } else {
        latency_sum as f64 / executed as f64
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

    let (tokens_in, tokens_out, mean_tokens_per_judged_case) = if has_tokens {
        let mean = (tokens_in_sum + tokens_out_sum).checked_div(token_case_count);
        (Some(tokens_in_sum), Some(tokens_out_sum), mean)
    } else {
        (None, None, None)
    };

    let mut latencies: Vec<f64> = results
        .iter()
        .filter(|r| r.status != CaseStatus::Skipped)
        .map(|r| r.metrics.total_latency_ms as f64)
        .collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let latency_p50_ms = percentile(&latencies, 50.0);
    let latency_p95_ms = percentile(&latencies, 95.0);
    let latency_p99_ms = percentile(&latencies, 99.0);

    let (judge_tokens_in, judge_tokens_out) = if has_judge_tokens {
        (Some(judge_tokens_in_sum), Some(judge_tokens_out_sum))
    } else {
        (None, None)
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
        tokens_in,
        tokens_out,
        mean_tokens_per_judged_case,
        latency_p50_ms,
        latency_p95_ms,
        latency_p99_ms,
        judge_tokens_in,
        judge_tokens_out,
        gen_cost_usd: None,
        judge_cost_usd: None,
        total_cost_usd: None,
        generator_model: None,
        judge_model: None,
        command_line: None,
    }
}

fn percentile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.len() < 2 {
        return None;
    }
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    Some(sorted[idx.min(sorted.len() - 1)])
}

/// Combine the score provenance of one rubric item across repetitions. When methods differ,
/// an LLM judge anywhere makes the averaged score judge-derived (assurance spec INV-4).
fn merge_score_source(
    current: Option<agentcarousel_core::ScoreSource>,
    next: Option<agentcarousel_core::ScoreSource>,
) -> Option<agentcarousel_core::ScoreSource> {
    use agentcarousel_core::ScoreSource;
    match (current, next) {
        (None, other) | (other, None) => other,
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(ScoreSource::LlmJudge), _) | (_, Some(ScoreSource::LlmJudge)) => {
            Some(ScoreSource::LlmJudge)
        }
        (Some(ScoreSource::NotEvaluated), other) | (other, Some(ScoreSource::NotEvaluated)) => {
            other
        }
        (Some(a), Some(_)) => Some(a),
    }
}

#[cfg(test)]
mod score_source_tests {
    use super::merge_score_source;
    use agentcarousel_core::ScoreSource::*;

    #[test]
    fn judge_anywhere_dominates_mixed_sources() {
        assert_eq!(
            merge_score_source(Some(Deterministic), Some(LlmJudge)),
            Some(LlmJudge)
        );
        assert_eq!(
            merge_score_source(Some(LlmJudge), Some(Deterministic)),
            Some(LlmJudge)
        );
    }

    #[test]
    fn not_evaluated_yields_to_an_evaluated_source() {
        assert_eq!(
            merge_score_source(Some(NotEvaluated), Some(Deterministic)),
            Some(Deterministic)
        );
        assert_eq!(merge_score_source(None, Some(Golden)), Some(Golden));
        assert_eq!(
            merge_score_source(Some(Process), Some(Process)),
            Some(Process)
        );
    }
}
