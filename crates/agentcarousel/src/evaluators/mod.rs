//! Pluggable **evaluators** that score a finished [`crate::CaseResult`] against fixture rubrics
//! or external references: [`RulesEvaluator`], [`GoldenEvaluator`], [`ProcessEvaluator`],
//! [`JudgeEvaluator`], and the [`Evaluator`] trait.

mod assertions;
mod golden;
mod judge;
mod process;
mod rules;
mod trait_def;

pub use golden::{
    evaluate_for_promotion, GoldenEvaluator, PromotionMeta, PromotionResult,
    PROMOTE_CRITICAL_THRESHOLD, PROMOTE_EFFECTIVENESS_THRESHOLD,
};
pub use judge::JudgeEvaluator;
pub use process::ProcessEvaluator;
pub use rules::{evaluate_case, RuleEvaluation, RulesEvaluator};
pub use trait_def::{Evaluator, EvaluatorError, EvaluatorKind};
