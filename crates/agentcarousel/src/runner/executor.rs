use super::sandbox::Sandbox;
use super::tracer::{SecretScrubber, Tracer};
use super::{generator, GenerationMode, RunnerConfig};
use agentcarousel_core::{
    Case, CaseResult, CaseStatus, ExecutionTrace, Metrics, StepKind, TraceStep,
};
use agentcarousel_evaluators::evaluate_case;
use agentcarousel_fixtures::MockEngine;
use serde_json::json;
use std::time::Instant;

/// Run a single [`crate::Case`]: replay tool expectations against [`crate::MockEngine`], apply
/// output rules when enabled, and return a [`crate::CaseResult`].
pub async fn run_case(case: Case, mock_engine: &MockEngine, config: &RunnerConfig) -> CaseResult {
    run_case_inner(case, mock_engine, config, true).await
}

/// Same execution path as [`run_case`](crate::run_case) but skips embedded rules scoring
/// (used by the eval harness before attaching evaluator scores).
pub async fn run_case_unscored(
    case: Case,
    mock_engine: &MockEngine,
    config: &RunnerConfig,
) -> CaseResult {
    run_case_inner(case, mock_engine, config, false).await
}

async fn run_case_inner(
    case: Case,
    mock_engine: &MockEngine,
    config: &RunnerConfig,
    evaluate_rules: bool,
) -> CaseResult {
    let start = Instant::now();
    let mut trace = ExecutionTrace {
        steps: Vec::new(),
        final_output: None,
        redacted: false,
    };
    let mut metrics = Metrics::default();
    let mut error = None;
    let mut status = CaseStatus::Passed;

    let _sandbox = Sandbox::apply(&case.input.env_overrides, config.offline);

    if let Some(tool_sequence) = case.expected.tool_sequence.as_ref() {
        for expectation in tool_sequence {
            let args = expectation.args_match.clone().unwrap_or_else(|| json!({}));
            let step_index = trace.steps.len() as u32;
            trace.steps.push(TraceStep {
                index: step_index,
                kind: StepKind::ToolCall,
                tool: Some(expectation.tool.clone()),
                args: Some(args.clone()),
                result: None,
                latency_ms: 0,
                tokens_in: None,
                tokens_out: None,
            });
            metrics.tool_calls += 1;

            let response = mock_engine.match_response(&expectation.tool, &args);
            match response {
                Some(response) => {
                    trace.steps.push(TraceStep {
                        index: trace.steps.len() as u32,
                        kind: StepKind::ToolResult,
                        tool: Some(expectation.tool.clone()),
                        args: None,
                        result: Some(response),
                        latency_ms: 0,
                        tokens_in: None,
                        tokens_out: None,
                    });
                }
                None if config.mock_strict => {
                    status = CaseStatus::Error;
                    error = Some(mock_engine.describe_miss(&expectation.tool, &args));
                    break;
                }
                None if config.offline => {
                    status = CaseStatus::Error;
                    error = Some(mock_engine.describe_miss(&expectation.tool, &args));
                    break;
                }
                None => {
                    trace.steps.push(TraceStep {
                        index: trace.steps.len() as u32,
                        kind: StepKind::ToolResult,
                        tool: Some(expectation.tool.clone()),
                        args: None,
                        result: None,
                        latency_ms: 0,
                        tokens_in: None,
                        tokens_out: None,
                    });
                }
            }
        }
    }

    if status == CaseStatus::Passed {
        let case_id = case.id.0.clone();
        let args = json!({ "case_id": case_id });
        if let Some(response) = mock_engine.match_response("agent_response", &args) {
            trace.final_output = Some(extract_output(response));
        } else {
            match config.generation_mode {
                GenerationMode::Live => {
                    let live_start = Instant::now();
                    match generator::generate_case_output(&case, config).await {
                        Ok(generated) => {
                            trace.final_output = Some(generated.output);
                            trace.steps.push(TraceStep {
                                index: trace.steps.len() as u32,
                                kind: StepKind::LlmCall,
                                tool: None,
                                args: None,
                                result: Some(generator::generation_step_result(
                                    generator::GeneratorProvider::from_model(
                                        config.generator_model.as_deref().unwrap_or_default(),
                                    ),
                                    config.generator_model.as_deref().unwrap_or("unknown-model"),
                                )),
                                latency_ms: live_start.elapsed().as_millis() as u64,
                                tokens_in: generated
                                    .tokens_in
                                    .and_then(|value| u32::try_from(value).ok()),
                                tokens_out: generated
                                    .tokens_out
                                    .and_then(|value| u32::try_from(value).ok()),
                            });
                            metrics.llm_calls += 1;
                            metrics.tokens_in = generated.tokens_in;
                            metrics.tokens_out = generated.tokens_out;
                        }
                        Err(err) => {
                            status = CaseStatus::Error;
                            error = Some(format!("live generation failed: {err}"));
                        }
                    }
                }
                GenerationMode::MockOnly => {
                    if case.expected.output.is_some() && (config.mock_strict || config.offline) {
                        status = CaseStatus::Error;
                        error = Some(mock_engine.describe_miss("agent_response", &args));
                    } else {
                        status = CaseStatus::Error;
                        error = Some(
                            "missing agent_response mock; rerun eval with --execution-mode live"
                                .to_string(),
                        );
                    }
                }
            }
        }
        trace.steps.push(TraceStep {
            index: trace.steps.len() as u32,
            kind: StepKind::AgentDecision,
            tool: None,
            args: None,
            result: None,
            latency_ms: 0,
            tokens_in: None,
            tokens_out: None,
        });
    }

    if status == CaseStatus::Passed && evaluate_rules {
        let evaluation = evaluate_case(&case, &trace);
        if !evaluation.passed {
            status = CaseStatus::Failed;
            error = Some(evaluation.failures.join("; "));
        }
    }

    metrics.total_steps = trace.steps.len() as u32;
    metrics.total_latency_ms = start.elapsed().as_millis() as u64;

    let mut tracer = Tracer::new(SecretScrubber::default());
    tracer.scrub_trace(&mut trace);

    CaseResult {
        case_id: case.id,
        status,
        error,
        trace,
        metrics,
        eval_scores: None,
    }
}

pub fn timeout_result(case_id: agentcarousel_core::CaseId) -> CaseResult {
    CaseResult {
        case_id,
        status: CaseStatus::TimedOut,
        error: Some("case timed out".to_string()),
        trace: ExecutionTrace {
            steps: Vec::new(),
            final_output: None,
            redacted: false,
        },
        metrics: Metrics::default(),
        eval_scores: None,
    }
}

fn extract_output(response: serde_json::Value) -> String {
    match response {
        serde_json::Value::String(value) => value,
        serde_json::Value::Object(mut map) => map
            .remove("content")
            .and_then(|value| value.as_str().map(|value| value.to_string()))
            .unwrap_or_else(|| serde_json::Value::Object(map).to_string()),
        other => other.to_string(),
    }
}
