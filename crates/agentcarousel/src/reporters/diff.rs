use agentcarousel_core::{CaseRegression, CaseStatus, RegressionKind, Run, RunDiff};
use serde_json::json;
use std::collections::HashMap;

pub fn diff_runs(run_a: &Run, run_b: &Run, regression_threshold: f32) -> RunDiff {
    let lookup: HashMap<_, _> = run_a
        .cases
        .iter()
        .map(|case| (case.case_id.0.clone(), case))
        .collect();

    let mut regressions = Vec::new();
    for case in &run_b.cases {
        let Some(before_case) = lookup.get(&case.case_id.0) else {
            continue;
        };

        if status_rank(case.status.clone()) > status_rank(before_case.status.clone()) {
            regressions.push(CaseRegression {
                case_id: case.case_id.clone(),
                kind: RegressionKind::StatusChange,
                before: json!(before_case.status),
                after: json!(case.status),
            });
        }

        let before_latency = before_case.metrics.total_latency_ms as f64;
        let after_latency = case.metrics.total_latency_ms as f64;
        if before_latency > 0.0
            && after_latency > before_latency * (1.0 + regression_threshold as f64)
        {
            regressions.push(CaseRegression {
                case_id: case.case_id.clone(),
                kind: RegressionKind::LatencyIncrease,
                before: json!(before_latency),
                after: json!(after_latency),
            });
        }

        let before_effectiveness = before_case
            .eval_scores
            .as_ref()
            .map(|scores| scores.effectiveness_score);
        let after_effectiveness = case
            .eval_scores
            .as_ref()
            .map(|scores| scores.effectiveness_score);
        if let (Some(before_score), Some(after_score)) = (before_effectiveness, after_effectiveness)
        {
            if after_score < before_score - regression_threshold {
                regressions.push(CaseRegression {
                    case_id: case.case_id.clone(),
                    kind: RegressionKind::EffectivenessDropped,
                    before: json!(before_score),
                    after: json!(after_score),
                });
            }
        }

        let before_error_rate = before_case.metrics.error_rate;
        let after_error_rate = case.metrics.error_rate;
        if let (Some(before_rate), Some(after_rate)) = (before_error_rate, after_error_rate) {
            if after_rate > before_rate + regression_threshold {
                regressions.push(CaseRegression {
                    case_id: case.case_id.clone(),
                    kind: RegressionKind::ErrorRateIncreased,
                    before: json!(before_rate),
                    after: json!(after_rate),
                });
            }
        }
    }

    let has_regressions = !regressions.is_empty();
    RunDiff {
        run_a: run_a.id.clone(),
        run_b: run_b.id.clone(),
        regressions,
        improvements: Vec::new(),
        has_regressions,
    }
}

pub fn print_diff(diff: &RunDiff) {
    if !diff.has_regressions {
        println!("no regressions detected");
        return;
    }
    println!("regressions detected:");
    for regression in &diff.regressions {
        println!(
            "- {}: {:?} ({:?} -> {:?})",
            regression.case_id.0, regression.kind, regression.before, regression.after
        );
    }
}

fn status_rank(status: CaseStatus) -> u8 {
    match status {
        CaseStatus::Passed => 0,
        CaseStatus::Skipped => 0,
        CaseStatus::Flaky => 1,
        CaseStatus::Failed => 2,
        CaseStatus::TimedOut => 3,
        CaseStatus::Error => 4,
    }
}
