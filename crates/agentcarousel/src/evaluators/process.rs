use agentcarousel_core::{Case, CaseResult, EvalScores, RubricScore};
use serde::Deserialize;
use std::io::Write;
use std::process::{Command, Stdio};

use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};

#[derive(Debug, Clone)]
pub struct ProcessEvaluator {
    pub command: Vec<String>,
}

impl ProcessEvaluator {
    pub fn from_case(case: &Case) -> Result<Self, EvaluatorError> {
        let config = case
            .evaluator_config
            .as_ref()
            .ok_or(EvaluatorError::MissingConfig("process_cmd"))?;
        let command = config
            .process_cmd
            .clone()
            .ok_or(EvaluatorError::MissingConfig("process_cmd"))?;
        if command.is_empty() {
            return Err(EvaluatorError::MissingConfig("process_cmd"));
        }
        Ok(Self { command })
    }
}

#[derive(Debug, Deserialize)]
struct ProcessEvalResponse {
    scores: Option<Vec<ProcessScore>>,
    passed: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ProcessScore {
    rubric_id: String,
    score: f32,
    rationale: Option<String>,
}

impl Evaluator for ProcessEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Process.as_str()
    }

    fn evaluate(&self, case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let payload = serde_json::json!({ "case": case, "result": result });
        let mut command = Command::new(&self.command[0]);
        if self.command.len() > 1 {
            command.args(&self.command[1..]);
        }
        let mut child = command
            .env_remove("agentcarousel_JUDGE_KEY")
            .env_remove("AGENTCAROUSEL_JUDGE_KEY")
            .env_remove("AGENTCAROUSEL_API_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| EvaluatorError::ProcessFailed(err.to_string()))?;

        if let Some(stdin) = child.stdin.as_mut() {
            let input = serde_json::to_vec(&payload)
                .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
            stdin
                .write_all(&input)
                .map_err(|err| EvaluatorError::ProcessFailed(err.to_string()))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|err| EvaluatorError::ProcessFailed(err.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let message = if stderr.is_empty() {
                "process evaluator exited with non-zero status".to_string()
            } else {
                stderr
            };
            return Err(EvaluatorError::ProcessFailed(message));
        }

        let response: ProcessEvalResponse = serde_json::from_slice(&output.stdout)
            .map_err(|err| EvaluatorError::InvalidOutput(err.to_string()))?;
        let rubric_scores: Vec<RubricScore> = response
            .scores
            .unwrap_or_default()
            .into_iter()
            .map(|score| RubricScore {
                rubric_id: score.rubric_id,
                score: score.score,
                weight: 1.0,
                rationale: score.rationale,
            })
            .collect();
        let effectiveness_score = if rubric_scores.is_empty() {
            if response.passed.unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        } else {
            let total: f32 = rubric_scores.iter().map(|score| score.score).sum();
            total / rubric_scores.len() as f32
        };

        Ok(EvalScores {
            evaluator: self.id().to_string(),
            rubric_scores,
            effectiveness_score,
            passed: response.passed.unwrap_or(effectiveness_score >= 1.0),
            judge_rationale: None,
        })
    }
}
