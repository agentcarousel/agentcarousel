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
use super::output::JsonOutput;
use super::GlobalOptions;

/// Run fixtures with mock generation (no API keys required).
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc test fixtures/customer-support/cases.yaml\n  agc test fixtures/ --filter-tags smoke\n  agc test fixtures/ --concurrency 4 --format json\n\nExit codes:\n  0  all cases passed\n  1  one or more cases failed\n  4  runtime error (IO, network)\n  5  fixture path not found"
)]
pub struct TestArgs {
    /// Fixture files or dirs (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Override the run id stored in the history DB for this run.
    #[arg(long)]
    pub run_id: Option<String>,
    /// Glob matched against full case ids (`skill/case-id`).
    #[arg(short = 'f', long)]
    filter: Option<String>,
    /// Comma-separated case tags to include (e.g. `smoke,fast`).
    #[arg(
        short = 'g',
        long = "filter-tags",
        value_name = "TAG",
        value_delimiter = ','
    )]
    filter_tags: Option<Vec<String>>,
    /// Maximum number of cases to run in parallel.
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    /// Per-case timeout in seconds.
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    /// Use mock responses instead of calling an API (`true` / `false`).
    #[arg(short = 'o', long)]
    offline: Option<bool>,
    /// Disable the config-level offline default (force live mode unless overridden per-case).
    #[arg(short = 'O', long)]
    no_offline_default: bool,
    /// Directory to load or write mock responses from.
    #[arg(short = 'm', long)]
    mock_dir: Option<PathBuf>,
    /// Stop after the first failing case.
    #[arg(short = 'F', long)]
    fail_fast: bool,
    /// Output format: `human` (default) or `json`.
    #[arg(short = 'p', long)]
    format: Option<String>,
    /// Cancel the entire run after N seconds (per-case --timeout still applies per case).
    #[arg(long)]
    timeout_run: Option<u64>,
    /// Base URL for a custom agent endpoint (required when generator model is 'custom').
    #[arg(long)]
    generator_endpoint: Option<String>,
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
        run_timeout_secs: args.timeout_run,
        offline,
        mock_dir,
        generation_mode: GenerationMode::MockOnly,
        generator_model: Some(config.generator.model.clone()),
        generator_max_tokens: config.generator.max_tokens,
        generator_endpoint: args.generator_endpoint.clone(),
        fail_fast: args.fail_fast,
        mock_strict: std::env::var("agentcarousel_MOCK_STRICT").ok().as_deref() == Some("1"),
        command: "test".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: args.run_id.clone(),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let run = runtime.block_on(run_fixtures(fixtures, runner_config));

    let _ = persist_run(&run);

    let failed = run.cases.iter().any(|case| {
        matches!(
            case.status,
            CaseStatus::Failed | CaseStatus::TimedOut | CaseStatus::Error | CaseStatus::Flaky
        )
    });

    if globals.json {
        let value = serde_json::to_value(&run).unwrap_or(serde_json::Value::Null);
        JsonOutput::ok("test", value).print();
    } else {
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
    }

    if failed {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}
