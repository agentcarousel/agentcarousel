use agentcarousel_core::{Case, CaseResult, EvalScores, RubricScore};
use similar::TextDiff;
use std::fs;

use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};

const DEFAULT_GOLDEN_THRESHOLD: f32 = 0.9;

#[derive(Debug, Clone)]
pub struct GoldenEvaluator {
    pub golden_path: std::path::PathBuf,
    pub threshold: f32,
    /// When true, write actual output to `golden_path` and return pass instead of diffing.
    pub update: bool,
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
            update: false,
        })
    }
}

impl Evaluator for GoldenEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Golden.as_str()
    }

    fn evaluate(&self, _case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let actual = result.trace.final_output.clone().unwrap_or_default();

        if self.update {
            if let Some(parent) = self.golden_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&self.golden_path, &actual).map_err(|source| EvaluatorError::GoldenRead {
                path: self.golden_path.clone(),
                source,
            })?;
            eprintln!("  updated golden: {}", self.golden_path.display());
            return Ok(EvalScores {
                evaluator: self.id().to_string(),
                rubric_scores: vec![RubricScore {
                    rubric_id: "golden".to_string(),
                    score: 1.0,
                    weight: 1.0,
                    rationale: Some("updated".to_string()),
                }],
                effectiveness_score: 1.0,
                passed: true,
                judge_rationale: None,
            });
        }

        let expected =
            fs::read_to_string(&self.golden_path).map_err(|source| EvaluatorError::GoldenRead {
                path: self.golden_path.clone(),
                source,
            })?;
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
