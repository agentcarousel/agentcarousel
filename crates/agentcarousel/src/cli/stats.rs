use agentcarousel_core::{CaseStatus, Run};
use agentcarousel_reporters::list_full_runs;
use clap::Parser;
use console::style;
use std::collections::HashMap;
use std::path::PathBuf;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::GlobalOptions;

/// Show historical pass-rate trends, per-case flakiness, and latency from run history.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc stats\n  agc stats --skill customer-support\n  agc stats --limit 100 --format json"
)]
pub struct StatsArgs {
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Filter to a specific skill or agent name.
    #[arg(long)]
    skill: Option<String>,
    /// Maximum number of runs to analyse (newest first).
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// Output format: `human` (default) or `json`.
    #[arg(long, default_value = "human")]
    format: String,
}

pub fn run_stats(args: StatsArgs, _config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let runs = match list_full_runs(args.limit) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::RuntimeError.as_i32();
        }
    };

    let runs: Vec<Run> = if let Some(skill) = &args.skill {
        runs.into_iter()
            .filter(|r| r.skill_or_agent.as_deref() == Some(skill.as_str()))
            .collect()
    } else {
        runs
    };

    if runs.is_empty() {
        if !globals.quiet {
            eprintln!("no runs in history");
        }
        return ExitCode::Ok.as_i32();
    }

    let mut case_statuses: HashMap<String, Vec<CaseStatus>> = HashMap::new();
    let pass_rates: Vec<(String, f32)> = runs
        .iter()
        .map(|r| {
            let label = r.started_at.format("%Y-%m-%d %H:%M").to_string();
            for cr in &r.cases {
                case_statuses
                    .entry(cr.case_id.0.clone())
                    .or_default()
                    .push(cr.status.clone());
            }
            (label, r.summary.pass_rate)
        })
        .collect();

    let mut flakiness: Vec<(String, f32)> = case_statuses
        .iter()
        .filter(|(_, statuses)| statuses.len() > 1)
        .map(|(id, statuses)| {
            let non_pass = statuses
                .iter()
                .filter(|s| !matches!(s, CaseStatus::Passed))
                .count();
            let flak = non_pass as f32 / statuses.len() as f32;
            (id.clone(), flak)
        })
        .collect();
    flakiness.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let latency_trend: Vec<f64> = runs.iter().map(|r| r.summary.mean_latency_ms).collect();

    if args.format == "json" {
        let output = serde_json::json!({
            "run_count": runs.len(),
            "pass_rate_trend": pass_rates.iter().map(|(t, r)| serde_json::json!({"at": t, "pass_rate": r})).collect::<Vec<_>>(),
            "mean_latency_trend_ms": latency_trend,
            "flakiest_cases": flakiness.iter().take(10).map(|(id, f)| serde_json::json!({"case_id": id, "flakiness": f})).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_default()
        );
        return ExitCode::Ok.as_i32();
    }

    let skill_label = args.skill.as_deref().unwrap_or("all skills");
    println!(
        "\n  {} — last {} runs ({})",
        style("AgentCarousel Stats").bold(),
        runs.len(),
        skill_label
    );
    println!("  ──────────────────────────────────────────");

    println!("\n  Pass rate trend (newest → oldest):");
    for (at, rate) in pass_rates.iter().take(10) {
        let bar = rate_bar(*rate, 20);
        println!("    {} {} {:.0}%", style(at).dim(), bar, rate * 100.0);
    }
    if pass_rates.len() > 10 {
        println!("    … and {} older runs", pass_rates.len() - 10);
    }

    if !flakiness.is_empty() {
        println!("\n  Flakiest cases:");
        for (id, flak) in flakiness.iter().take(5) {
            let label = id.rsplit_once('/').map(|(_, s)| s).unwrap_or(id.as_str());
            println!("    {:.0}%  {}", flak * 100.0, style(label).yellow());
        }
    }

    let mean_latency: f64 = if latency_trend.is_empty() {
        0.0
    } else {
        latency_trend.iter().sum::<f64>() / latency_trend.len() as f64
    };
    println!("\n  Mean latency across all runs: {:.0}ms", mean_latency);
    println!();

    ExitCode::Ok.as_i32()
}

fn rate_bar(rate: f32, width: usize) -> String {
    let filled = (rate * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}
