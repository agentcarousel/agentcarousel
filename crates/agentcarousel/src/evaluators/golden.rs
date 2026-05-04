use agentcarousel_core::{Case, CaseResult, EvalScores, RubricScore};
use similar::TextDiff;
use std::fs;

use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};

const DEFAULT_GOLDEN_THRESHOLD: f32 = 0.9;

#[derive(Debug, Clone)]
pub struct GoldenEvaluator {
    pub golden_path: std::path::PathBuf,
    pub threshold: f32,
}

impl GoldenEvaluator {
    pub fn from_case(case: &Case) -> Result<Self, EvaluatorError> {
        let config = case
            .evaluator_config
            .as_ref()
            .ok_or(EvaluatorError::MissingConfig("golden_path"))?;
        let golden_path = config
            .golden_path
            .clone()
            .ok_or(EvaluatorError::MissingConfig("golden_path"))?;
        let threshold = config.golden_threshold.unwrap_or(DEFAULT_GOLDEN_THRESHOLD);
        Ok(Self {
            golden_path,
            threshold,
        })
    }
}

impl Evaluator for GoldenEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Golden.as_str()
    }

    fn evaluate(&self, _case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let expected =
            fs::read_to_string(&self.golden_path).map_err(|source| EvaluatorError::GoldenRead {
                path: self.golden_path.clone(),
                source,
            })?;
        let actual = result.trace.final_output.clone().unwrap_or_default();
        let diff = TextDiff::from_lines(&expected, &actual);
        let ratio = diff.ratio() as f32;
        let passed = ratio >= self.threshold;

        Ok(EvalScores {
            evaluator: self.id().to_string(),
            rubric_scores: vec![RubricScore {
                rubric_id: "golden".to_string(),
                score: ratio,
                weight: 1.0,
                rationale: Some(format!(
                    "similarity {:.2} (threshold {:.2})",
                    ratio, self.threshold
                )),
            }],
            effectiveness_score: ratio,
            passed,
            judge_rationale: None,
        })
    }
}
