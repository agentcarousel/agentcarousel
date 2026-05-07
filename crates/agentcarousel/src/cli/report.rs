use agentcarousel_core::Run;
use agentcarousel_reporters::{
    diff_runs, fetch_run, list_runs, print_diff, print_json, print_terminal, RunListing,
};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::Path;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;

/// List/show/diff runs in the local history DB (same DB as test/eval; see config).
#[derive(Debug, Parser)]
pub struct ReportArgs {
    #[command(subcommand)]
    command: ReportCommand,
}

#[derive(Debug, Subcommand)]
enum ReportCommand {
    /// Recent run ids (newest first).
    List {
        #[arg(short = 'l', long, default_value_t = 20)]
        limit: usize,
        #[arg(short = 'j', long)]
        json: bool,
    },
    /// One run (human-readable terminal, same formatting as eval/test, or `--json`).
    /// Pass a run id from `report list`, or a path to `run.json` / a directory that contains it (e.g. an evidence folder).
    Show {
        run_id: String,
        #[arg(short = 'j', long)]
        json: bool,
    },
    /// Compare two runs (regressions vs configured threshold).
    Diff { run_id_a: String, run_id_b: String },
}

pub fn run_report(args: ReportArgs, config: &ResolvedConfig) -> i32 {
    match args.command {
        ReportCommand::List { limit, json } => report_list(limit, json),
        ReportCommand::Show { run_id, json } => report_show(&run_id, json),
        ReportCommand::Diff { run_id_a, run_id_b } => {
            report_diff(&run_id_a, &run_id_b, config.report.regression_threshold)
        }
    }
}

fn report_list(limit: usize, json: bool) -> i32 {
    match list_runs(limit) {
        Ok(runs) => {
            if json {
                let payload =
                    serde_json::to_string_pretty(&runs).unwrap_or_else(|_| "[]".to_string());
                println!("{payload}");
            } else {
                print_list(&runs);
            }
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

/// Load a run from the history database, or from `run.json` (file path or parent directory).
fn load_run_for_show(run_ref: &str) -> Result<Run, String> {
    let path = Path::new(run_ref);
    if path.exists() {
        let json_path = if path.is_dir() {
            path.join("run.json")
        } else {
            path.to_path_buf()
        };
        if !json_path.is_file() {
            return Err(format!(
                "expected {} to be a run.json file or a directory containing run.json",
                run_ref
            ));
        }
        let raw = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", json_path.display()))
    } else {
        fetch_run(run_ref).map_err(|e| e.to_string())
    }
}

fn report_show(run_id: &str, json: bool) -> i32 {
    match load_run_for_show(run_id) {
        Ok(run) => {
            if json {
                print_json(&run);
            } else {
                print_terminal(&run);
            }
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn report_diff(run_id_a: &str, run_id_b: &str, threshold: f32) -> i32 {
    let run_a = match fetch_run(run_id_a) {
        Ok(run) => run,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::RuntimeError.as_i32();
        }
    };
    let run_b = match fetch_run(run_id_b) {
        Ok(run) => run,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::RuntimeError.as_i32();
        }
    };

    let diff = diff_runs(&run_a, &run_b, threshold);
    print_diff(&diff);
    if diff.has_regressions {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn print_list(runs: &[RunListing]) {
    if runs.is_empty() {
        println!("no runs recorded");
        return;
    }
    for run in runs {
        println!("{}  {}", run.id, run.started_at.to_rfc3339());
    }
}
