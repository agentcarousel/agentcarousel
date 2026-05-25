use agentcarousel_core::{annotate_run_cost, CaseStatus};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::{persist_run, print_terminal_summary};
use agentcarousel_runner::{run_eval, BatchStateStore, EvalConfig, GenerationMode, RunnerConfig};
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::GlobalOptions;

#[derive(Debug, Parser)]
#[command(about = "Manage async batch jobs (status, fetch results).")]
pub struct BatchArgs {
    #[command(subcommand)]
    pub command: BatchCommand,
}

#[derive(Debug, Subcommand)]
pub enum BatchCommand {
    /// Show the current status of a batch job.
    Status {
        /// Batch ID returned by `agc eval --execution-mode batch`.
        batch_id: String,
    },
    /// Wait for a batch to complete and run the eval pipeline on its results.
    Fetch {
        /// Batch ID returned by `agc eval --execution-mode batch`.
        batch_id: String,
        /// Config file path (default: agentcarousel.toml in current directory).
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

pub fn run_batch_command(args: BatchArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    match args.command {
        BatchCommand::Status { batch_id } => run_status(&batch_id, globals),
        BatchCommand::Fetch { batch_id, .. } => run_fetch(&batch_id, config, globals),
    }
}

fn run_status(batch_id: &str, globals: &GlobalOptions) -> i32 {
    let api_key = match std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
    {
        Some(k) => k,
        None => {
            eprintln!("error: ANTHROPIC_API_KEY not set");
            return ExitCode::ConfigError.as_i32();
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    let result = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let resp = client
            .get(format!(
                "https://api.anthropic.com/v1/messages/batches/{batch_id}"
            ))
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await?;
        let status_code = resp.status();
        let body: serde_json::Value = resp.json().await?;
        Ok::<(reqwest::StatusCode, serde_json::Value), reqwest::Error>((status_code, body))
    });

    match result {
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::RuntimeError.as_i32()
        }
        Ok((status_code, body)) if !status_code.is_success() => {
            eprintln!("error: API returned {status_code}: {body}");
            ExitCode::RuntimeError.as_i32()
        }
        Ok((_, body)) => {
            let processing_status = body["processing_status"].as_str().unwrap_or("unknown");
            let total = body["request_counts"]["processing"].as_u64().unwrap_or(0)
                + body["request_counts"]["succeeded"].as_u64().unwrap_or(0)
                + body["request_counts"]["errored"].as_u64().unwrap_or(0)
                + body["request_counts"]["canceled"].as_u64().unwrap_or(0)
                + body["request_counts"]["expired"].as_u64().unwrap_or(0);
            let succeeded = body["request_counts"]["succeeded"].as_u64().unwrap_or(0);
            let errored = body["request_counts"]["errored"].as_u64().unwrap_or(0);

            println!(
                "Batch {}  status={}",
                style(batch_id).bold(),
                match processing_status {
                    "ended" => style(processing_status).green().to_string(),
                    "in_progress" => style(processing_status).yellow().to_string(),
                    _ => style(processing_status).dim().to_string(),
                }
            );
            println!(
                "  {total} total  |  {} succeeded  |  {} errored",
                style(succeeded).green(),
                if errored > 0 {
                    style(errored).red().to_string()
                } else {
                    style(errored).dim().to_string()
                }
            );
            if processing_status == "ended" {
                println!(
                    "\n  {}  agc batch fetch {batch_id}",
                    style("Ready to collect:").bold()
                );
            } else if !globals.quiet {
                println!("\n  Check again later with:  agc batch status {batch_id}");
            }
            ExitCode::Ok.as_i32()
        }
    }
}

fn run_fetch(batch_id: &str, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let state_dir = PathBuf::from(".agc/batch_state");
    let state = match BatchStateStore::load(batch_id, &state_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "error: could not load batch state for '{}': {}\n  hint: run this command from the same directory where you ran 'agc eval --execution-mode batch'",
                batch_id, e
            );
            return ExitCode::RuntimeError.as_i32();
        }
    };

    if state.fixture_paths.is_empty() {
        eprintln!(
            "error: batch state has no fixture paths (was this batch submitted with an older version?)\n  hint: re-run 'agc eval --execution-mode batch' to generate a new batch"
        );
        return ExitCode::RuntimeError.as_i32();
    }

    // Re-read fixtures from the stored paths.
    let mut fixtures = Vec::new();
    for path_str in &state.fixture_paths {
        let path = PathBuf::from(path_str);
        match load_fixture(&path) {
            Ok(f) => fixtures.push(f),
            Err(e) => {
                eprintln!("error: failed to re-read fixture '{}': {}", path_str, e);
                return ExitCode::RuntimeError.as_i32();
            }
        }
    }

    if fixtures.is_empty() {
        eprintln!("error: no fixture cases found in stored paths");
        return ExitCode::RuntimeError.as_i32();
    }

    if !globals.quiet {
        let n: usize = fixtures.iter().map(|f| f.cases.len()).sum();
        eprintln!(
            "Collecting batch {} ({} cases from {} fixtures)…",
            batch_id,
            n,
            fixtures.len()
        );
    }

    let judge_enabled = state.judge_model.is_some();
    let judge_model = state
        .judge_model
        .clone()
        .unwrap_or_else(|| config.judge.model.clone());

    let runner = RunnerConfig {
        concurrency: 1,
        timeout_secs: config.runner.timeout_secs,
        offline: false,
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode: GenerationMode::Batch,
        generator_model: Some(state.model.clone()),
        generator_max_tokens: Some(state.max_tokens),
        generator_endpoint: None,
        fail_fast: false,
        mock_strict: false,
        command: "batch-fetch".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: None,
        batch_collect_id: Some(batch_id.to_string()),
    };

    let eval_config = EvalConfig {
        runner,
        runs: 1,
        seed: 0,
        evaluator: config.eval.default_evaluator.clone(),
        judge: judge_enabled,
        judge_model: Some(judge_model.clone()),
        effectiveness_threshold: config.eval.effectiveness_threshold,
        judge_max_tokens: config.judge.max_tokens,
        progress: !globals.quiet,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    let mut run = runtime.block_on(run_eval(fixtures, eval_config));

    let generator_model = state.model.clone();
    let judge_model_for_cost = if judge_enabled {
        Some(judge_model.as_str())
    } else {
        None
    };
    annotate_run_cost(&mut run, &generator_model, judge_model_for_cost);

    let error_count = run
        .cases
        .iter()
        .filter(|c| c.status == CaseStatus::Error)
        .count();

    // Persist to history.
    if let Err(e) = persist_run(&run) {
        eprintln!("warn: could not persist run to history: {e}");
    }

    // Print summary only — no per-case error spam.
    print_terminal_summary(&run);

    if error_count > 0 && !globals.quiet {
        eprintln!(
            "\n{} {error_count} error(s) — run 'agc report show {}' for details",
            style("hint:").yellow().bold(),
            run.id.0
        );
    }

    let passed = run.summary.passed;
    let total = run.summary.total;
    if passed == total {
        ExitCode::Ok.as_i32()
    } else {
        ExitCode::Failed.as_i32()
    }
}
