//! Pluggable **evaluators** that score a finished [`crate::CaseResult`] against fixture rubrics
//! or external references: [`RulesEvaluator`], [`GoldenEvaluator`], [`ProcessEvaluator`],
//! [`JudgeEvaluator`], and the [`Evaluator`] trait.

mod assertions;
mod golden;
mod judge;
mod process;
mod rules;
mod trait_def;

pub use golden::GoldenEvaluator;
pub use judge::JudgeEvaluator;
pub use process::ProcessEvaluator;
pub use rules::{evaluate_case, RuleEvaluation, RulesEvaluator};
pub use trait_def::{Evaluator, EvaluatorError, EvaluatorKind};
