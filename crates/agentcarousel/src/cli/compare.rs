use agentcarousel_reporters::{fetch_run, find_previous_run, find_tagged_run, list_runs, tag_run};
use clap::{Parser, Subcommand};
use console::style;
use serde::Serialize;

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const DEFAULT_THRESHOLD: f32 = 0.05;

/// Compare eval runs and gate on regressions.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc compare -l --baseline <run-id>\n  agc compare -l --baseline <run-id> --threshold 0.05\n  agc compare <run-id> --baseline <run-id>\n  agc compare tag <run-id> --name prod-baseline\n  agc compare -l  # auto-baseline: previous run for same skill\n\nExit codes:\n  0  no regression (or improvement)\n  1  regression exceeds threshold\n  4  runtime error (IO, database)\n  5  run not found in history"
)]
pub struct CompareArgs {
    #[command(subcommand)]
    command: Option<CompareCommand>,

    /// The current run id to compare (omit to use latest run).
    #[arg(value_name = "RUN_ID")]
    run_id: Option<String>,

    /// Use the latest run in history as the current run.
    #[arg(short = 'l', long, conflicts_with = "run_id")]
    latest: bool,

    /// Baseline run id or named tag to compare against.
    #[arg(long)]
    baseline: Option<String>,

    /// Regression threshold for overall effectiveness delta (default: 0.05).
    #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
    threshold: f32,
}

#[derive(Debug, Subcommand)]
enum CompareCommand {
    /// Tag a run as a named baseline for future comparisons.
    Tag {
        /// Run id to tag.
        run_id: String,
        /// Name to store (e.g. `prod-baseline`).
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Serialize)]
pub struct CompareResult {
    pub baseline_run_id: String,
    pub current_run_id: String,
    pub skill_or_agent: Option<String>,
    pub regression: bool,
    pub overall_effectiveness_delta: Option<f32>,
    pub pass_rate_delta: f32,
    pub threshold: f32,
    pub cases: Vec<CaseCompare>,
}

#[derive(Debug, Serialize)]
pub struct CaseCompare {
    pub case_id: String,
    pub baseline_effectiveness: Option<f32>,
    pub current_effectiveness: Option<f32>,
    pub delta: Option<f32>,
    pub regression: bool,
}

pub fn run_compare(args: CompareArgs, globals: &GlobalOptions) -> i32 {
    if let Some(CompareCommand::Tag { run_id, name }) = args.command {
        return run_tag(&run_id, &name, globals);
    }
    run_compare_runs(args, globals)
}

fn run_tag(run_id: &str, name: &str, globals: &GlobalOptions) -> i32 {
    match tag_run(name, run_id) {
        Ok(()) => {
            if globals.json {
                JsonOutput::ok(
                    "compare tag",
                    serde_json::json!({ "name": name, "run_id": run_id }),
                )
                .print();
            } else {
                println!("tagged run {run_id} as '{name}'");
            }
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            if globals.json {
                JsonOutput::err(
                    "compare tag",
                    JsonError::new("runtime_error", err.to_string()),
                )
                .print();
            } else {
                eprintln!("error: {err}");
            }
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn run_compare_runs(args: CompareArgs, globals: &GlobalOptions) -> i32 {
    let current = match resolve_current_run(args.run_id.as_deref(), args.latest, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };

    let baseline = match resolve_baseline(&args.baseline, &current, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };

    let result = build_compare_result(&baseline, &current, args.threshold);
    let regression = result.regression;

    if globals.json {
        JsonOutput::ok("compare", &result).print();
    } else {
        print_compare_terminal(&result);
    }

    if regression {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn resolve_current_run(
    run_id: Option<&str>,
    latest: bool,
    globals: &GlobalOptions,
) -> Result<agentcarousel_core::Run, i32> {
    if let Some(id) = run_id {
        fetch_run(id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "run_not_found",
                &format!("Run '{id}' not found in history database."),
                vec!["Run 'agc report list' to see available run IDs.".to_string()],
                err.to_string(),
            );
            ExitCode::NotFound.as_i32()
        })
    } else if latest {
        let listings = list_runs(1).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            ExitCode::RuntimeError.as_i32()
        })?;
        let id = listings.into_iter().next().ok_or_else(|| {
            emit_error(
                globals,
                "compare",
                "no_runs",
                "No runs in history database.",
                vec!["Run 'agc eval' first.".to_string()],
                String::new(),
            );
            ExitCode::NotFound.as_i32()
        })?;
        fetch_run(&id.id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            ExitCode::RuntimeError.as_i32()
        })
    } else {
        emit_error(
            globals,
            "compare",
            "invalid_args",
            "Specify a RUN_ID or pass -l/--latest.",
            vec!["Example: agc compare -l --baseline <run-id>".to_string()],
            String::new(),
        );
        Err(ExitCode::ValidationFailed.as_i32())
    }
}

fn resolve_baseline(
    baseline_arg: &Option<String>,
    current: &agentcarousel_core::Run,
    globals: &GlobalOptions,
) -> Result<agentcarousel_core::Run, i32> {
    // Resolution order: explicit --baseline, named tag, auto (previous run for same skill)
    let baseline_id: Option<String> = if let Some(ref spec) = baseline_arg {
        // Try as run ID first; if not found try as a named tag
        if fetch_run(spec).is_ok() {
            Some(spec.clone())
        } else {
            find_tagged_run(spec).ok().flatten().or(Some(spec.clone()))
        }
    } else {
        None
    };

    if let Some(id) = baseline_id {
        return fetch_run(&id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "run_not_found",
                &format!("Baseline run '{id}' not found in history database."),
                vec!["Run 'agc report list' to see available run IDs.".to_string()],
                err.to_string(),
            );
            ExitCode::NotFound.as_i32()
        });
    }

    // Auto-baseline: most recent prior run for same skill
    let skill = match current.skill_or_agent.as_deref() {
        Some(s) => s,
        None => {
            emit_error(
                globals,
                "compare",
                "no_baseline",
                "Cannot auto-select baseline: current run has no skill_or_agent. Pass --baseline <run-id>.",
                vec![],
                String::new(),
            );
            return Err(ExitCode::NotFound.as_i32());
        }
    };

    match find_previous_run(skill, &current.id.0) {
        Ok(Some(run)) => Ok(run),
        Ok(None) => {
            emit_error(
                globals,
                "compare",
                "no_baseline",
                &format!("No previous run found for skill '{skill}'. Pass --baseline <run-id>."),
                vec!["Run 'agc report list' to see available runs.".to_string()],
                String::new(),
            );
            Err(ExitCode::NotFound.as_i32())
        }
        Err(err) => {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            Err(ExitCode::RuntimeError.as_i32())
        }
    }
}

fn build_compare_result(
    baseline: &agentcarousel_core::Run,
    current: &agentcarousel_core::Run,
    threshold: f32,
) -> CompareResult {
    use std::collections::HashMap;

    let baseline_cases: HashMap<&str, &agentcarousel_core::CaseResult> = baseline
        .cases
        .iter()
        .map(|c| (c.case_id.0.as_str(), c))
        .collect();

    let mut cases = Vec::new();
    for case in &current.cases {
        let baseline_eff = baseline_cases
            .get(case.case_id.0.as_str())
            .and_then(|b| b.eval_scores.as_ref())
            .map(|s| s.effectiveness_score);
        let current_eff = case.eval_scores.as_ref().map(|s| s.effectiveness_score);
        let delta = match (baseline_eff, current_eff) {
            (Some(b), Some(c)) => Some(c - b),
            _ => None,
        };
        let regression = delta.is_some_and(|d| d < -threshold);
        cases.push(CaseCompare {
            case_id: case.case_id.0.clone(),
            baseline_effectiveness: baseline_eff,
            current_effectiveness: current_eff,
            delta,
            regression,
        });
    }

    let baseline_pass_rate = baseline.summary.pass_rate;
    let current_pass_rate = current.summary.pass_rate;
    let pass_rate_delta = current_pass_rate - baseline_pass_rate;

    let baseline_eff = baseline.summary.mean_effectiveness_score;
    let current_eff = current.summary.mean_effectiveness_score;
    let overall_effectiveness_delta = match (baseline_eff, current_eff) {
        (Some(b), Some(c)) => Some(c - b),
        _ => None,
    };

    let regression = overall_effectiveness_delta.is_some_and(|d| d < -threshold)
        || cases.iter().any(|c| c.regression);

    CompareResult {
        baseline_run_id: baseline.id.0.clone(),
        current_run_id: current.id.0.clone(),
        skill_or_agent: current.skill_or_agent.clone(),
        regression,
        overall_effectiveness_delta,
        pass_rate_delta,
        threshold,
        cases,
    }
}

fn print_compare_terminal(result: &CompareResult) {
    let skill = result.skill_or_agent.as_deref().unwrap_or("unknown");
    println!(
        "\n  Comparing run {} → {}  ({})\n",
        &result.baseline_run_id[..result.baseline_run_id.len().min(8)],
        &result.current_run_id[..result.current_run_id.len().min(8)],
        skill,
    );

    if let Some(delta) = result.overall_effectiveness_delta {
        let arrow = if delta < 0.0 { "▼" } else { "▲" };
        let label = if result.regression {
            style("⚠ REGRESSION").yellow().bold()
        } else {
            style("✓ OK").green().bold()
        };
        println!(
            "  Overall effectiveness   {:+.2}   {}  {}",
            delta, arrow, label
        );
    }

    let arrow = if result.pass_rate_delta < 0.0 {
        "▼"
    } else {
        "▲"
    };
    println!(
        "  Pass rate               {:+.0}%   {}",
        result.pass_rate_delta * 100.0,
        arrow,
    );

    let regressions: Vec<&CaseCompare> = result.cases.iter().filter(|c| c.regression).collect();
    if !regressions.is_empty() {
        println!("\n  Regressions:");
        println!("  ┌─────────────────────────────┬────────┬────────┬───────┐");
        println!("  │ Case                        │ Before │ After  │ Delta │");
        println!("  ├─────────────────────────────┼────────┼────────┼───────┤");
        for c in &regressions {
            let before = c
                .baseline_effectiveness
                .map_or("  —   ".to_string(), |v| format!(" {v:.2} "));
            let after = c
                .current_effectiveness
                .map_or("  —   ".to_string(), |v| format!(" {v:.2} "));
            let delta = c.delta.map_or("  —  ".to_string(), |v| format!("{v:+.2}"));
            let short_id: String = c.case_id.chars().take(29).collect();
            println!(
                "  │ {:<29} │{:^8}│{:^8}│{:^7}│",
                short_id, before, after, delta
            );
        }
        println!("  └─────────────────────────────┴────────┴────────┴───────┘");
    }

    println!();
    if result.regression {
        println!(
            "  {}",
            style(format!(
                "Exit 1 — regression exceeds threshold ({:.2})",
                result.threshold
            ))
            .red()
        );
    } else {
        println!("  {}", style("No regression detected").green());
    }
    println!();
}

fn emit_error(
    globals: &GlobalOptions,
    command: &'static str,
    code: &'static str,
    message: &str,
    suggestions: Vec<String>,
    _detail: String,
) {
    if globals.json {
        JsonOutput::err(
            command,
            JsonError::new(code, message).with_suggestions(suggestions),
        )
        .print();
    } else {
        eprintln!("error: {message}");
        for s in &suggestions {
            eprintln!("  hint: {s}");
        }
    }
}
