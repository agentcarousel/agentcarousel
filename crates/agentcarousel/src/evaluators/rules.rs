use agentcarousel_core::{
    Case, CaseResult, EvalScores, ExecutionTrace, RubricScore, StepKind, ToolOrder,
};
use serde_json::Value;

use super::assertions::check_output;
use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};

#[derive(Debug, Clone)]
pub struct RuleEvaluation {
    pub passed: bool,
    pub failures: Vec<String>,
}

#[derive(Debug, Default)]
pub struct RulesEvaluator;

pub fn evaluate_case(case: &Case, trace: &ExecutionTrace) -> RuleEvaluation {
    let mut failures = Vec::new();

    if let Some(sequence) = case.expected.tool_sequence.as_ref() {
        if !sequence.is_empty() {
            failures.extend(check_tool_sequence(sequence, trace));
        }
    }

    if let Some(assertions) = case.expected.output.as_ref() {
        for assertion in assertions {
            if let Err(error) = check_output(assertion, trace) {
                failures.push(error);
            }
        }
    }

    RuleEvaluation {
        passed: failures.is_empty(),
        failures,
    }
}

impl Evaluator for RulesEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Rules.as_str()
    }

    fn evaluate(&self, case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let evaluation = evaluate_case(case, &result.trace);
        let mut rubric_scores = score_rubric(case, &result.trace);

        if !evaluation.passed {
            rubric_scores.push(RubricScore {
                rubric_id: "rules".to_string(),
                score: 0.0,
                weight: 1.0,
                rationale: Some(evaluation.failures.join("; ")),
            });
        }

        let effectiveness_score = if !evaluation.passed {
            0.0
        } else if rubric_scores.is_empty() {
            1.0
        } else {
            weighted_average(&rubric_scores)
        };

        let passed = if rubric_scores.is_empty() {
            evaluation.passed
        } else {
            evaluation.passed && rubric_scores.iter().all(|score| score.score >= 1.0)
        };

        Ok(EvalScores {
            evaluator: self.id().to_string(),
            rubric_scores,
            effectiveness_score,
            passed,
            judge_rationale: None,
        })
    }
}

fn check_tool_sequence(
    sequence: &[agentcarousel_core::ToolCallExpectation],
    trace: &ExecutionTrace,
) -> Vec<String> {
    let actual_steps: Vec<&agentcarousel_core::TraceStep> = trace
        .steps
        .iter()
        .filter(|step| step.kind == StepKind::ToolCall)
        .collect();

    let actual_tools: Vec<String> = actual_steps
        .iter()
        .filter_map(|step| step.tool.clone())
        .collect();

    let mut failures = Vec::new();
    let strict_expected: Vec<&agentcarousel_core::ToolCallExpectation> = sequence
        .iter()
        .filter(|expectation| expectation.order == ToolOrder::Strict)
        .collect();

    if !strict_expected.is_empty()
        && (actual_tools.len() != strict_expected.len()
            || !strict_expected
                .iter()
                .zip(actual_steps.iter())
                .all(|(expected, actual)| tool_matches(expected, actual)))
    {
        failures.push(format!(
            "tool sequence mismatch: expected {:?}, got {:?}",
            strict_expected
                .iter()
                .map(|expectation| expectation.tool.clone())
                .collect::<Vec<_>>(),
            actual_tools
        ));
    }

    let subsequence_expected: Vec<&agentcarousel_core::ToolCallExpectation> = sequence
        .iter()
        .filter(|expectation| expectation.order == ToolOrder::Subsequence)
        .collect();
    if !subsequence_expected.is_empty() && !is_subsequence(&subsequence_expected, &actual_steps) {
        failures.push(format!(
            "tool subsequence mismatch: expected {:?} within {:?}",
            subsequence_expected
                .iter()
                .map(|expectation| expectation.tool.clone())
                .collect::<Vec<_>>(),
            actual_tools
        ));
    }

    let unordered_expected: Vec<&agentcarousel_core::ToolCallExpectation> = sequence
        .iter()
        .filter(|expectation| expectation.order == ToolOrder::Unordered)
        .collect();
    for expected in unordered_expected {
        if !actual_steps
            .iter()
            .any(|actual| tool_matches(expected, actual))
        {
            failures.push(format!("missing unordered tool call: {}", expected.tool));
        }
    }

    failures
}

fn is_subsequence(
    expected: &[&agentcarousel_core::ToolCallExpectation],
    actual: &[&agentcarousel_core::TraceStep],
) -> bool {
    let mut index = 0;
    for item in actual {
        if expected
            .get(index)
            .map(|expectation| tool_matches(expectation, item))
            .unwrap_or(false)
        {
            index += 1;
            if index == expected.len() {
                return true;
            }
        }
    }
    expected.is_empty()
}

fn tool_matches(
    expected: &agentcarousel_core::ToolCallExpectation,
    actual: &agentcarousel_core::TraceStep,
) -> bool {
    let tool_matches = actual.tool.as_deref() == Some(&expected.tool);
    let args_matches = match (expected.args_match.as_ref(), actual.args.as_ref()) {
        (None, _) => true,
        (Some(expected), Some(actual)) => is_subset(expected, actual),
        _ => false,
    };
    tool_matches && args_matches
}

fn is_subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            expected_map.iter().all(|(key, value)| {
                actual_map
                    .get(key)
                    .map(|actual_value| is_subset(value, actual_value))
                    .unwrap_or(false)
            })
        }
        (Value::Array(expected_arr), Value::Array(actual_arr)) => expected_arr == actual_arr,
        _ => expected == actual,
    }
}

fn score_rubric(case: &Case, trace: &ExecutionTrace) -> Vec<RubricScore> {
    let Some(rubric) = case.expected.rubric.as_ref() else {
        return Vec::new();
    };
    rubric
        .iter()
        .map(|item| {
            let mut rationale = None;
            let score = if let Some(auto_check) = item.auto_check.as_ref() {
                match check_output(auto_check, trace) {
                    Ok(()) => 1.0,
                    Err(err) => {
                        rationale = Some(err);
                        0.0
                    }
                }
            } else {
                rationale = Some("requires judge or manual review".to_string());
                0.0
            };
            RubricScore {
                rubric_id: item.id.clone(),
                score,
                weight: item.weight,
                rationale,
            }
        })
        .collect()
}

fn weighted_average(scores: &[RubricScore]) -> f32 {
    let total_weight: f32 = scores.iter().map(|score| score.weight).sum();
    if total_weight <= f32::EPSILON {
        return 0.0;
    }
    let weighted_sum: f32 = scores.iter().map(|score| score.score * score.weight).sum();
    weighted_sum / total_weight
}
