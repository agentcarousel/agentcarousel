use agentcarousel_core::{
    annotate_run_cost, fmt_cost, fmt_tokens, judge_key_candidates, judge_provider_from_model,
    prefetch_pricing, FixtureFile,
};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::persist_run;
use agentcarousel_runner::{run_eval, EvalConfig, GenerationMode, GeneratorProvider, RunnerConfig};
use clap::{ArgAction, Parser};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::io::{stderr, IsTerminal};
use std::path::PathBuf;
use std::time::Duration;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{apply_case_filter, apply_tag_filter, collect_fixture_paths};
use super::output::JsonOutput;
use super::GlobalOptions;

/// Run the same fixtures against multiple models and rank them side-by-side.
///
/// agc carousel runs every model in parallel and prints a ranked table showing pass rate, effectiveness score, latency, token usage, and USD cost per model. Every run is saved to history so you can dig into individual results with agc report or agc compare afterward.
///
/// There are two main evaluation modes — choose the one that fits your goal:
///
///   rules (default)  Checks pass/fail based on assertions in the fixture. Fast and free — no extra API calls.
///   judge            Scores each output on a rubric using a second LLM. Richer signal, but costs more and takes longer.
///
/// Recommended workflow for the most complete ranking:
///   1. Record golden outputs:   agc eval fixtures/ --execution-mode live --judge --promote-golden
///   2. Rules baseline:          agc carousel --models m1,m2,... fixtures/
///   3. Judge scoring:           agc carousel --models m1,m2,... fixtures/ -e judge --judge
///   4. Compare top two:         agc compare <run-a> --baseline <run-b>
///
/// Requires live API keys for every provider in --models.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc carousel --models gpt-4o,gemini-2.5-flash,claude-sonnet-4-6 fixtures/my-skill/\n  agc carousel --models gpt-4o,gemini-2.5-flash fixtures/ -e judge --judge\n  agc carousel --models gpt-4o,gpt-4o-mini fixtures/my-skill/ --json\n  agc carousel --models openrouter/deepseek/deepseek-chat:free fixtures/ -e judge --judge\n\nOpenRouter models: prefix with 'openrouter/' or use the slash-separated model id directly.\n  Example: openrouter/deepseek/deepseek-chat:free\n\nExit codes:\n  0  all models passed\n  1  one or more models had failures\n  4  runtime error"
)]
pub struct CarouselArgs {
    /// Fixture files or directories to run (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Comma-separated list of generator models to run (required).
    #[arg(long, value_delimiter = ',', required = true)]
    models: Vec<String>,
    /// Evaluator: `rules` | `golden` | `process` | `judge` | `all`.
    #[arg(short = 'e', long, default_value = "rules")]
    evaluator: String,
    /// Enable the LLM judge for judge-scored cases (requires API keys).
    #[arg(short = 'j', long)]
    judge: bool,
    /// Model to use for judge scoring (overrides config `judge.model`).
    #[arg(short = 'J', long)]
    judge_model: Option<String>,
    /// Number of times to run each case per model (use >1 for flakiness detection).
    #[arg(short = 'n', long, default_value_t = 1)]
    runs: u32,
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
    /// Per-model case concurrency (default: 1 for live generation).
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    /// Per-case timeout in seconds.
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    /// Show a model-level progress bar on stderr (default: on when stderr is a TTY).
    #[arg(short = 'P', long, action = ArgAction::SetTrue)]
    progress: bool,
    /// Never show the carousel progress bar.
    #[arg(short = 'N', long, action = ArgAction::SetTrue)]
    no_progress: bool,
}

#[derive(Debug, Serialize)]
pub struct CarouselOutput {
    pub models: Vec<ModelRow>,
    pub total_cases: u32,
    pub fixture_count: usize,
}

#[derive(Debug, Serialize)]
pub struct ModelRow {
    pub rank: usize,
    pub model: String,
    pub run_id: String,
    pub passed: u32,
    pub total: u32,
    pub pass_rate: f32,
    pub effectiveness_score: Option<f32>,
    pub latency_p50_ms: Option<f64>,
    pub any_failures: bool,
    pub gen_tokens_in: Option<u64>,
    pub gen_tokens_out: Option<u64>,
    pub judge_tokens_in: Option<u64>,
    pub judge_tokens_out: Option<u64>,
    pub gen_cost_usd: Option<f64>,
    pub judge_cost_usd: Option<f64>,
    pub total_cost_usd: Option<f64>,
}

pub fn run_carousel(args: CarouselArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    if args.models.is_empty() {
        eprintln!("error: --models is required and must name at least one model");
        return ExitCode::ConfigError.as_i32();
    }

    // Validate judge keys upfront if judge is enabled.
    let judge_model = args
        .judge_model
        .clone()
        .unwrap_or_else(|| config.judge.model.clone());
    let judge_provider = judge_provider_from_model(&judge_model);
    let judge_active = args.judge && is_judge_evaluator_active(&args.evaluator);
    if judge_active && resolve_key(judge_key_candidates(judge_provider)).is_none() {
        eprintln!(
            "error: set one of {} to use --judge with model '{}'",
            judge_key_candidates(judge_provider).join(", "),
            judge_model
        );
        return ExitCode::ConfigError.as_i32();
    }

    // Validate generator keys upfront for each model.
    for model in &args.models {
        let provider = GeneratorProvider::from_model(model);
        if resolve_key(provider.key_candidates()).is_none() {
            eprintln!(
                "error: set one of {} to run live generation for model '{}'",
                provider.key_candidates().join(", "),
                model
            );
            return ExitCode::ConfigError.as_i32();
        }
    }

    // Load and filter fixtures once; clone per model.
    let fixture_paths = collect_fixture_paths(&args.paths);
    if fixture_paths.is_empty() {
        eprintln!("error: no fixture files found in the specified paths");
        return ExitCode::ConfigError.as_i32();
    }
    let fixture_count = fixture_paths.len();

    let mut fixtures: Vec<FixtureFile> = Vec::new();
    for path in fixture_paths {
        match load_fixture(&path) {
            Ok(f) => {
                let f = apply_case_filter(f, args.filter.as_deref());
                let f = apply_tag_filter(f, args.filter_tags.as_deref());
                fixtures.push(f);
            }
            Err(err) => {
                eprintln!("error: failed to load fixture {}: {err}", path.display());
                return ExitCode::ConfigError.as_i32();
            }
        }
    }
    if fixtures.is_empty() {
        eprintln!("error: all fixture files are empty or were filtered to zero cases");
        return ExitCode::ConfigError.as_i32();
    }

    let total_cases = fixtures.iter().map(|f| f.cases.len() as u32).sum::<u32>();
    let concurrency = args.concurrency.or(config.runner.concurrency).unwrap_or(1);

    if !globals.json && !globals.quiet {
        eprintln!(
            "{} carousel — {} model(s) × {} case(s) — running in parallel",
            style("🎠").bold(),
            args.models.len(),
            total_cases,
        );
    }

    let show_progress = !args.no_progress
        && !globals.quiet
        && !globals.json
        && (args.progress || stderr().is_terminal());

    prefetch_pricing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    // Spawn one eval task per model; collect results as each completes.
    let results: Vec<(String, agentcarousel_core::Run)> = runtime.block_on(async {
        let pb: Option<ProgressBar> = if show_progress {
            let pb = ProgressBar::new(args.models.len() as u64);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} models",
                )
                .expect("progress template")
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );
            pb.enable_steady_tick(Duration::from_millis(120));
            Some(pb)
        } else {
            None
        };

        let mut set = tokio::task::JoinSet::new();
        for model in &args.models {
            let fixtures_clone = fixtures.clone();
            let eval_config = build_eval_config(
                model,
                &args,
                config,
                concurrency,
                judge_active,
                &judge_model,
            );
            let model_clone = model.clone();
            set.spawn(async move { (model_clone, run_eval(fixtures_clone, eval_config).await) });
        }

        let mut out = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok((model, run)) => {
                    if let Some(pb) = &pb {
                        pb.inc(1);
                    }
                    out.push((model, run));
                }
                Err(e) => eprintln!("error: model task panicked: {e}"),
            }
        }
        if let Some(pb) = pb {
            pb.finish_and_clear();
        }
        out
    });

    if results.is_empty() {
        eprintln!("error: all model runs failed");
        return ExitCode::RuntimeError.as_i32();
    }

    let judge_model_for_cost = if judge_active {
        Some(judge_model.as_str())
    } else {
        None
    };

    // Annotate cost and persist all runs.
    let mut results = results;
    let carousel_cmd_line = std::env::args().collect::<Vec<_>>().join(" ");
    for (model, run) in &mut results {
        annotate_run_cost(run, model, judge_model_for_cost);
        run.summary.generator_model = Some(model.clone());
        run.summary.judge_model = judge_model_for_cost.map(|s| s.to_string());
        run.summary.command_line = Some(carousel_cmd_line.clone());
        let _ = persist_run(run);
    }

    // Build ranked rows: sort by effectiveness score desc, then pass_rate desc.
    let mut rows: Vec<ModelRow> = results
        .iter()
        .map(|(model, run)| {
            let s = &run.summary;
            ModelRow {
                rank: 0,
                model: model.clone(),
                run_id: run.id.0.clone(),
                passed: s.passed,
                total: s.total,
                pass_rate: s.pass_rate,
                effectiveness_score: s.mean_effectiveness_score,
                latency_p50_ms: s.latency_p50_ms,
                any_failures: s.failed > 0 || s.errored > 0 || s.timed_out > 0,
                gen_tokens_in: s.tokens_in,
                gen_tokens_out: s.tokens_out,
                judge_tokens_in: s.judge_tokens_in,
                judge_tokens_out: s.judge_tokens_out,
                gen_cost_usd: s.gen_cost_usd,
                judge_cost_usd: s.judge_cost_usd,
                total_cost_usd: s.total_cost_usd,
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.effectiveness_score
            .unwrap_or(b.pass_rate)
            .partial_cmp(&a.effectiveness_score.unwrap_or(a.pass_rate))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                b.pass_rate
                    .partial_cmp(&a.pass_rate)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    for (i, row) in rows.iter_mut().enumerate() {
        row.rank = i + 1;
    }

    let any_failure = rows.iter().any(|r| r.any_failures);

    if globals.json {
        let out = CarouselOutput {
            models: rows,
            total_cases,
            fixture_count,
        };
        JsonOutput::ok("carousel", out).print();
        return if any_failure {
            ExitCode::Failed.as_i32()
        } else {
            ExitCode::Ok.as_i32()
        };
    }

    print_table(&rows, total_cases);

    if any_failure {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn build_eval_config(
    model: &str,
    args: &CarouselArgs,
    config: &ResolvedConfig,
    concurrency: usize,
    judge: bool,
    judge_model: &str,
) -> EvalConfig {
    let runner = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        run_timeout_secs: None,
        offline: false,
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode: GenerationMode::Live,
        generator_model: Some(model.to_string()),
        generator_max_tokens: config.generator.max_tokens,
        generator_endpoint: None,
        fail_fast: false,
        mock_strict: false,
        command: "carousel".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: None,
    };
    EvalConfig {
        runner,
        runs: args.runs,
        seed: 0,
        evaluator: if args.evaluator == "rules" {
            config.eval.default_evaluator.clone()
        } else {
            args.evaluator.clone()
        },
        judge,
        judge_model: Some(judge_model.to_string()),
        judge_max_tokens: config.judge.max_tokens,
        effectiveness_threshold: config.eval.effectiveness_threshold,
        certification_context: None,
        carousel_iteration: None,
        policy_version: None,
        progress: false,
    }
}

fn print_table(rows: &[ModelRow], total_cases: u32) {
    let has_cost = rows
        .iter()
        .any(|r| r.total_cost_usd.is_some() || r.gen_tokens_in.is_some());
    let model_w = rows.iter().map(|r| r.model.len()).max().unwrap_or(5).max(5);
    let run_id_w = 10;
    let cost_w = if has_cost { 22 } else { 0 };
    let sep_w = 4
        + 2
        + model_w
        + 2
        + 10
        + 2
        + 9
        + 2
        + 11
        + if has_cost { 2 + cost_w } else { 0 }
        + 2
        + run_id_w;

    println!();
    if has_cost {
        println!(
            "  {:<4}  {:<model_w$}  {:>10}  {:>9}  {:>11}  {:<cost_w$}  {}",
            style("Rank").bold(),
            style("Model").bold(),
            style("Passed").bold(),
            style("Score").bold(),
            style("Latency p50").bold(),
            style("Tokens / Cost ($USD)").bold(),
            style("Run ID").bold(),
        );
    } else {
        println!(
            "  {:<4}  {:<model_w$}  {:>10}  {:>9}  {:>11}  {}",
            style("Rank").bold(),
            style("Model").bold(),
            style("Passed").bold(),
            style("Score").bold(),
            style("Latency p50").bold(),
            style("Run ID").bold(),
        );
    }
    println!("  {}", "─".repeat(sep_w));

    for row in rows {
        let rank_str = format!("#{}", row.rank);
        let passed_str = format!("{} / {}", row.passed, total_cases);
        let score_str = row
            .effectiveness_score
            .map(|s| format!("{:.2}", s))
            .unwrap_or_else(|| "—".to_string());
        let latency_str = row
            .latency_p50_ms
            .map(|ms| format!("{:.1} s", ms / 1000.0))
            .unwrap_or_else(|| "—".to_string());
        let run_prefix: String = row.run_id.chars().take(run_id_w).collect();

        let rank_col = if row.rank == 1 {
            style(rank_str).green().bold().to_string()
        } else if row.any_failures {
            style(rank_str).yellow().to_string()
        } else {
            style(rank_str).dim().to_string()
        };

        let model_col = if row.rank == 1 {
            style(&row.model).green().to_string()
        } else {
            row.model.clone()
        };

        if has_cost {
            let total_in = row
                .gen_tokens_in
                .map(|g| g + row.judge_tokens_in.unwrap_or(0))
                .or(row.judge_tokens_in);
            let total_out = row
                .gen_tokens_out
                .map(|g| g + row.judge_tokens_out.unwrap_or(0))
                .or(row.judge_tokens_out);
            let tok_str = format!("↑{} ↓{}", fmt_tokens(total_in), fmt_tokens(total_out));
            let cost_str = fmt_cost(row.total_cost_usd);
            let cost_col = if row.rank == 1 {
                format!(
                    "{} {}",
                    style(&tok_str).cyan(),
                    style(&cost_str).green().bold()
                )
            } else {
                format!(
                    "{} {}",
                    style(&tok_str).cyan().dim(),
                    style(&cost_str).yellow()
                )
            };
            println!(
                "  {rank_col:<4}  {model_col:<model_w$}  {passed_str:>10}  {score_str:>9}  {latency_str:>11}  {cost_col:<cost_w$}  {run_prefix}",
            );
        } else {
            println!(
                "  {rank_col:<4}  {model_col:<model_w$}  {passed_str:>10}  {score_str:>9}  {latency_str:>11}  {run_prefix}",
            );
        }
    }

    println!("  {}", "─".repeat(sep_w));

    if let Some(best) = rows.first() {
        let score_label = best
            .effectiveness_score
            .map(|s| format!("  score {:.2}", s))
            .unwrap_or_default();
        println!(
            "\n  {} {}{}  ({:.0}% pass rate)",
            style("Best:").bold(),
            style(&best.model).green().bold(),
            score_label,
            best.pass_rate * 100.0,
        );
    }

    if rows.len() >= 2 {
        let a = &rows[0].run_id;
        let b = &rows[1].run_id;
        println!(
            "  {}  agc compare {} --baseline {}",
            style("Compare top 2:").dim(),
            a,
            b,
        );
    }

    println!();
}

fn is_judge_evaluator_active(evaluator: &str) -> bool {
    evaluator == "judge" || evaluator == "all"
}

fn resolve_key(candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::is_judge_evaluator_active;

    #[test]
    fn judge_active_for_judge_and_all() {
        assert!(is_judge_evaluator_active("judge"));
        assert!(is_judge_evaluator_active("all"));
        assert!(!is_judge_evaluator_active("rules"));
        assert!(!is_judge_evaluator_active("golden"));
    }
}
