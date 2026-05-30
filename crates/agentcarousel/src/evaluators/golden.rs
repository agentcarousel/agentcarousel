use agentcarousel_core::{Case, CaseResult, EvalScores, RubricItem, RubricScore};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use std::fs;

use super::trait_def::{Evaluator, EvaluatorError, EvaluatorKind};

const DEFAULT_GOLDEN_THRESHOLD: f32 = 0.9;
pub const PROMOTE_EFFECTIVENESS_THRESHOLD: f32 = 0.90;
pub const PROMOTE_CRITICAL_THRESHOLD: f32 = 0.95;
const CRITICAL_WEIGHT_FALLBACK: f32 = 0.45;

#[derive(Debug, Clone)]
pub struct GoldenEvaluator {
    pub golden_path: std::path::PathBuf,
    pub threshold: f32,
    pub normalize_whitespace: bool,
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
        let normalize_whitespace = config.golden_normalize_whitespace.unwrap_or(false);
        Ok(Self {
            golden_path,
            threshold,
            normalize_whitespace,
        })
    }
}

/// Collapse runs of whitespace to a single space and strip leading/trailing whitespace per line.
fn normalize(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Evaluator for GoldenEvaluator {
    fn id(&self) -> &'static str {
        EvaluatorKind::Golden.as_str()
    }

    fn evaluate(&self, _case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError> {
        let actual = result.trace.final_output.clone().unwrap_or_default();

        let expected =
            fs::read_to_string(&self.golden_path).map_err(|source| EvaluatorError::GoldenRead {
                path: self.golden_path.clone(),
                source,
            })?;

        let (cmp_expected, cmp_actual) = if self.normalize_whitespace {
            (normalize(&expected), normalize(&actual))
        } else {
            (expected, actual)
        };

        let diff = TextDiff::from_lines(&cmp_expected, &cmp_actual);
        let ratio = diff.ratio();
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
            judge_tokens_in: None,
            judge_tokens_out: None,
        })
    }
}

/// Persisted sidecar alongside a golden file; records the effectiveness score at last promotion.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromotionMeta {
    pub effectiveness: f32,
    pub promoted_at: String,
    pub run_id: Option<String>,
}

/// Outcome of a single promotion gate check.
pub struct PromotionResult {
    pub case_id: String,
    /// True if the gate passed and the golden file was actually written.
    pub promoted: bool,
    /// Rubric item IDs (and their scores) that failed the critical threshold.
    pub blocked_by: Vec<(String, f32)>,
    pub effectiveness: f32,
    /// Score from the previous `PromotionMeta` sidecar, if one exists.
    pub golden_baseline: Option<f32>,
    pub delta: Option<f32>,
}

/// Evaluate whether `result` clears the promotion gate for `case` and, if it does,
/// write the actual output to the case's `golden_path` and update the sidecar.
///
/// Returns `None` when the case has no golden evaluator config or no eval scores.
pub fn evaluate_for_promotion(
    case: &Case,
    result: &CaseResult,
    run_id: Option<&str>,
) -> Option<PromotionResult> {
    let config = case.evaluator_config.as_ref()?;
    let golden_path = config.golden_path.as_ref()?.clone();

    let eval_scores = result.eval_scores.as_ref()?;
    let effectiveness = eval_scores.effectiveness_score;

    let rubric = case.expected.rubric.as_deref().unwrap_or(&[]);
    let has_explicit_critical = rubric.iter().any(|r| r.critical == Some(true));
    let critical_items: Vec<&RubricItem> = if has_explicit_critical {
        rubric.iter().filter(|r| r.critical == Some(true)).collect()
    } else {
        rubric
            .iter()
            .filter(|r| r.weight >= CRITICAL_WEIGHT_FALLBACK)
            .collect()
    };

    let mut sidecar_raw = golden_path.as_os_str().to_owned();
    sidecar_raw.push(".meta.json");
    let sidecar_path = std::path::PathBuf::from(sidecar_raw);

    let baseline_meta: Option<PromotionMeta> = fs::read_to_string(&sidecar_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let golden_baseline = baseline_meta.as_ref().map(|m| m.effectiveness);

    let mut blocked_by: Vec<(String, f32)> = Vec::new();
    for item in &critical_items {
        let score = eval_scores
            .rubric_scores
            .iter()
            .find(|s| s.rubric_id == item.id)
            .map(|s| s.score)
            .unwrap_or(0.0);
        if score < PROMOTE_CRITICAL_THRESHOLD {
            blocked_by.push((item.id.clone(), score));
        }
    }

    let gate_passed = blocked_by.is_empty() && effectiveness >= PROMOTE_EFFECTIVENESS_THRESHOLD;
    let delta = golden_baseline.map(|b| effectiveness - b);

    let promoted = if gate_passed {
        let actual = result.trace.final_output.clone().unwrap_or_default();
        if let Some(parent) = golden_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        if fs::write(&golden_path, &actual).is_ok() {
            let meta = PromotionMeta {
                effectiveness,
                promoted_at: Utc::now().format("%Y-%m-%d").to_string(),
                run_id: run_id.map(|s| s.to_string()),
            };
            if let Ok(json) = serde_json::to_string_pretty(&meta) {
                let _ = fs::write(&sidecar_path, json);
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    Some(PromotionResult {
        case_id: result.case_id.0.clone(),
        promoted,
        blocked_by,
        effectiveness,
        golden_baseline,
        delta,
    })
}
