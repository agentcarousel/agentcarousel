use agentcarousel_core::CaseStatus;
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::{persist_run, print_json, print_junit, print_terminal};
use agentcarousel_runner::{run_fixtures, GenerationMode, RunnerConfig};
use clap::Parser;
use std::path::PathBuf;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{
    apply_case_filter, apply_tag_filter, collect_fixture_paths, default_concurrency,
};
use super::GlobalOptions;

/// Run fixtures (mock generation by default; see runner config).
#[derive(Debug, Parser)]
pub struct TestArgs {
    /// Fixture files or dirs (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    #[arg(short = 'f', long)]
    filter: Option<String>,
    /// Comma-separated case tags to run (e.g. smoke).
    #[arg(
        short = 'g',
        long = "filter-tags",
        value_name = "TAG",
        value_delimiter = ','
    )]
    filter_tags: Option<Vec<String>>,
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    #[arg(short = 'o', long)]
    offline: Option<bool>,
    #[arg(short = 'O', long)]
    no_offline_default: bool,
    #[arg(short = 'm', long)]
    mock_dir: Option<PathBuf>,
    #[arg(short = 'F', long)]
    fail_fast: bool,
    #[arg(short = 'p', long)]
    format: Option<String>,
}

pub fn run_test(args: TestArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let fixture_paths = collect_fixture_paths(&args.paths);
    let mut fixtures = Vec::new();
    for path in fixture_paths {
        match load_fixture(&path) {
            Ok(fixture) => {
                let fixture = apply_case_filter(fixture, args.filter.as_deref());
                let fixture = apply_tag_filter(fixture, args.filter_tags.as_deref());
                fixtures.push(fixture);
            }
            Err(err) => {
                eprintln!("error: failed to load fixture {}: {err}", path.display());
                return ExitCode::ConfigError.as_i32();
            }
        }
    }

    let concurrency = args
        .concurrency
        .or(config.runner.concurrency)
        .or_else(default_concurrency)
        .unwrap_or(1);
    let default_offline = config.runner.offline;
    let offline = if args.no_offline_default {
        false
    } else {
        args.offline.unwrap_or(default_offline)
    };
    let mock_dir = args
        .mock_dir
        .clone()
        .unwrap_or_else(|| config.runner.mock_dir.clone());
    let format = args
        .format
        .clone()
        .unwrap_or_else(|| config.output.format.clone());

    let runner_config = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        offline,
        mock_dir,
        generation_mode: GenerationMode::MockOnly,
        generator_model: Some(config.generator.model.clone()),
        generator_max_tokens: config.generator.max_tokens,
        fail_fast: args.fail_fast,
        mock_strict: std::env::var("agentcarousel_MOCK_STRICT").ok().as_deref() == Some("1"),
        command: "test".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: globals.run_id.clone(),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let run = runtime.block_on(run_fixtures(fixtures, runner_config));

    let _ = persist_run(&run);
    match format.as_str() {
        "json" => print_json(&run),
        "junit" => print_junit(&run),
        _ => {
            if globals.quiet {
                agentcarousel_reporters::print_terminal_summary(&run);
            } else {
                print_terminal(&run);
            }
        }
    }

    if run.cases.iter().any(|case| {
        matches!(
            case.status,
            CaseStatus::Failed | CaseStatus::TimedOut | CaseStatus::Error | CaseStatus::Flaky
        )
    }) {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}
