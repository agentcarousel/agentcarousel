use clap::Parser;
use std::path::PathBuf;

use super::candidate_store::{
    candidates_path, find_workspace_root, CandidateEntry, CandidateStore,
};
use super::exit_codes::ExitCode;
use super::output::JsonOutput;
use super::GlobalOptions;

/// List skill pipeline candidates and their current lifecycle state.
///
/// Shows all skills registered in the candidate registry (.agents/candidates.json) with
/// their evaluation scores, compliance metrics, and improvement progress.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc candidates                       # show all candidates\n  agc candidates --status stable        # filter by status\n  agc candidates --json                 # machine-readable output\n\nExit codes:\n  0  success\n  4  runtime error (workspace root not found)"
)]
pub struct CandidatesArgs {
    /// Filter by status: onboarding | improving | stable | graduated.
    #[arg(long)]
    pub status: Option<String>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

pub fn run_candidates(args: CandidatesArgs, globals: &GlobalOptions) -> i32 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace_root = match find_workspace_root(&cwd) {
        Some(root) => root,
        None => {
            // If no workspace root found, try using cwd directly (still
            // useful even without a config file for ad-hoc use).
            cwd.clone()
        }
    };

    let store_path = candidates_path(&workspace_root);
    let store = CandidateStore::load(&store_path);

    // Filter by status if requested.
    let status_filter = args.status.as_deref().map(|s| s.to_lowercase());
    let entries: Vec<&CandidateEntry> = store
        .all_sorted()
        .into_iter()
        .filter(|e| {
            if let Some(ref filter) = status_filter {
                e.status.to_string().to_lowercase() == *filter
            } else {
                true
            }
        })
        .collect();

    if globals.json {
        let value = serde_json::to_value(&entries).unwrap_or(serde_json::Value::Array(vec![]));
        JsonOutput::ok("candidates", value).print();
        return ExitCode::Ok.as_i32();
    }

    // ── Terminal table ────────────────────────────────────────────────────────
    println!();
    println!("  Candidates ({})", store_path.display());
    println!();

    if entries.is_empty() {
        println!("  (no candidates found)");
        println!();
        return ExitCode::Ok.as_i32();
    }

    let skill_w = entries
        .iter()
        .map(|e| e.skill.len())
        .max()
        .unwrap_or(5)
        .max(5);

    // Header row.
    println!(
        "  {:<skill_w$}  {:<10}  {:>10}  {:>9}  {:>9}  {:>6}  Updated",
        "Skill", "Status", "Eval(B→C)", "Coverage", "Injection", "Rounds",
    );
    // Separator.
    let sep_len = skill_w + 2 + 10 + 2 + 10 + 2 + 9 + 2 + 9 + 2 + 6 + 2 + 10;
    println!("  {}", "─".repeat(sep_len));

    for entry in &entries {
        let eval_col = format_score_transition(entry.baseline_score, entry.current_score);
        let coverage_col = format_metric(
            entry
                .current_metrics
                .as_ref()
                .and_then(|m| m.coverage)
                .or_else(|| entry.baseline_metrics.as_ref().and_then(|m| m.coverage)),
        );
        let injection_col = format_metric(
            entry
                .current_metrics
                .as_ref()
                .and_then(|m| m.injection_resistance)
                .or_else(|| {
                    entry
                        .baseline_metrics
                        .as_ref()
                        .and_then(|m| m.injection_resistance)
                }),
        );
        let updated_col = entry
            .last_updated
            .split('T')
            .next()
            .unwrap_or(&entry.last_updated)
            .to_string();

        println!(
            "  {:<skill_w$}  {:<10}  {:>10}  {:>9}  {:>9}  {:>6}  {}",
            entry.skill,
            entry.status.to_string(),
            eval_col,
            coverage_col,
            injection_col,
            entry.improvement_rounds,
            updated_col,
        );
    }

    println!();
    ExitCode::Ok.as_i32()
}

/// Format a baseline→current score transition as "78%→89%" or "—" if both absent.
fn format_score_transition(baseline: Option<f32>, current: Option<f32>) -> String {
    match (baseline, current) {
        (Some(b), Some(c)) => format!("{:.0}%→{:.0}%", b * 100.0, c * 100.0),
        (Some(b), None) => format!("{:.0}%→—", b * 100.0),
        (None, Some(c)) => format!("—→{:.0}%", c * 100.0),
        (None, None) => "—".to_string(),
    }
}

/// Format a single metric score (0.0–1.0) as "86%" or "—".
fn format_metric(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{:.0}%", v * 100.0),
        None => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_score_transition_both_set() {
        assert_eq!(format_score_transition(Some(0.78), Some(0.89)), "78%→89%");
    }

    #[test]
    fn format_score_transition_none_none() {
        assert_eq!(format_score_transition(None, None), "—");
    }

    #[test]
    fn format_metric_some() {
        assert_eq!(format_metric(Some(0.86)), "86%");
    }

    #[test]
    fn format_metric_none() {
        assert_eq!(format_metric(None), "—");
    }
}
