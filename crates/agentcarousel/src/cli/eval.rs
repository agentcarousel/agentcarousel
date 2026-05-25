use agentcarousel_core::{
    annotate_run_cost, judge_key_candidates, judge_provider_from_model, prefetch_pricing,
    CaseStatus, JudgeProvider,
};
use agentcarousel_evaluators::run_prompt_audit;
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::{persist_run, print_json, print_terminal};
use agentcarousel_runner::{
    flatten_cases, run_discrimination_eval, run_eval, submit_batch_only, EvalConfig,
    GenerationMode, GeneratorProvider, RunnerConfig,
};
use clap::{Parser, ValueEnum};
use console::style;
use std::io::{stderr, IsTerminal};
use std::path::PathBuf;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{
    apply_case_filter, apply_tag_filter, collect_fixture_paths, default_concurrency,
};
use super::output::JsonOutput;
use super::GlobalOptions;

#[derive(Debug, Clone, ValueEnum)]
enum EvalExecutionMode {
    Mock,
    Live,
    /// Submit all cases to the provider's async batch API (~50% cost saving).
    Batch,
}

/// Run your test suite and see which cases pass, fail, or need attention.
#[derive(Debug, Parser)]
#[command(
    long_about = "Run your test suite and see which cases pass, fail, or need attention.\n\nBy default, agc eval uses pre-recorded mock responses so no API key is required and runs finish in seconds. Switch to --execution-mode live to call a real model API. Add --judge to score outputs with an LLM judge on top of rule-based checks.\n\nToken counts and USD cost are shown automatically after each run when data is available.",
    after_help = "Examples:\n  agc eval fixtures/                                      # mock run, rules evaluator (fast, no API key)\n  agc eval fixtures/ --execution-mode live               # call a real model API\n  agc eval fixtures/ --execution-mode live --judge       # live generation + LLM judge scoring\n  agc eval fixtures/ --evaluator judge --judge           # force judge scoring on every case\n  agc eval fixtures/ --filter-tags smoke --json          # CI-friendly JSON output\n  agc eval fixtures/ --execution-mode batch              # async batch API (~50% cheaper)\n\nTo promote a saved run to golden:  agc promote <run_id>\n\nExit codes:\n  0  all cases passed\n  1  one or more cases failed or scored below threshold\n  4  runtime error (network, disk, config)\n  5  fixture path not found"
)]
pub struct EvalArgs {
    /// Fixture files or dirs (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Override the run id stored in the history DB for this run.
    #[arg(long)]
    pub run_id: Option<String>,
    /// Number of times to run each case (use >1 for flakiness detection).
    #[arg(short = 'n', long, default_value_t = 1)]
    runs: u32,
    /// Random seed for mock generation (0 = deterministic default).
    #[arg(short = 's', long, default_value_t = 0)]
    seed: u64,
    /// `rules` | `golden` | `process` | `judge` | `all` (default `rules` uses config; `all` uses each case’s `evaluator_config` in YAML).
    ///
    /// - **`judge`** — every run case is scored with the judge (ignores per-case evaluator choice).
    /// - **`all`** — each case uses its fixture’s evaluator; **only `judge` cases call the judge API** (use with `--judge` and keys). Required for mixed rules/golden/judge fixtures.
    #[arg(short = 'e', long, default_value = "rules")]
    evaluator: String,
    /// Enable the LLM judge for judge-scored cases (requires API keys; errors list env vars if missing).
    ///
    /// Useless unless the active mode can select judge: **`--evaluator judge`** (all cases judged) or **`--evaluator all`** (only cases with `evaluator: judge` in YAML).
    #[arg(short = 'j', long)]
    judge: bool,
    /// Model to use for judge scoring (overrides config `judge.model`).
    #[arg(short = 'J', long)]
    judge_model: Option<String>,
    /// `mock` (default) or `live` — whether to call a real generator API.
    #[arg(short = 'x', long, value_enum, default_value_t = EvalExecutionMode::Mock)]
    execution_mode: EvalExecutionMode,
    /// Generator model name (overrides config `generator.model`).
    #[arg(short = 'm', long)]
    model: Option<String>,
    /// Omit `max_tokens` from generator requests (unsupported for Anthropic models).
    #[arg(short = 'M', long)]
    disable_max_tokens: bool,
    /// Maximum number of cases to run in parallel.
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    /// Per-case timeout in seconds.
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    /// Glob matched against full case ids (`skill/case-id`). Example: `my-skill/judge-*` to run only judge-named cases; combine with `--evaluator all --judge`.
    #[arg(short = 'F', long)]
    filter: Option<String>,
    /// Comma-separated tags; keep only cases having any listed tag. Tag judge-only rows (e.g. `judge`) and pass `--filter-tags judge` to skip rules/golden cases.
    #[arg(long = "filter-tags", value_name = "TAG", value_delimiter = ',')]
    filter_tags: Option<Vec<String>>,
    /// Base URL for a custom agent endpoint (required when --model is 'custom').
    #[arg(long)]
    generator_endpoint: Option<String>,
    /// Run 3 eval passes (current / blank / degraded prompt) and attach a discrimination
    /// score to each case. High-discrimination cases (score > 0.2) are valuable tests;
    /// low-discrimination cases (score ≤ 0) pass even with a degraded prompt and may
    /// be noise. Requires --execution-mode live or batch.
    #[arg(long = "measure-discrimination", short = 'D')]
    measure_discrimination: bool,
}

pub fn run_eval_command(args: EvalArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    if globals.verbose >= 2 {
        std::env::set_var("AGENTCAROUSEL_DEBUG_JUDGE", "1");
    }

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

    let judge_selected = is_judge_selected(&args, &fixtures);
    if judge_selected && !args.judge {
        eprintln!("error: judge evaluator selected; rerun with --judge");
        return ExitCode::ConfigError.as_i32();
    }
    // When --judge is set with the default "rules" evaluator, auto-select the judge
    // evaluator so `agc eval --judge` does what the user expects without also requiring
    // --evaluator judge.
    let (judge_selected, effective_evaluator) =
        if args.judge && !judge_selected && args.evaluator == "rules" {
            (true, "judge".to_string())
        } else {
            (judge_selected, args.evaluator.clone())
        };
    let judge_model = args
        .judge_model
        .clone()
        .unwrap_or_else(|| config.judge.model.clone());
    let judge_provider = judge_provider_from_model(&judge_model);
    if judge_selected && resolve_judge_key(judge_provider).is_none() {
        eprintln!(
            "error: set one of {} to run --judge for model '{}'\n  tip: {}",
            judge_key_candidates(judge_provider).join(", "),
            judge_model,
            key_example(judge_key_candidates(judge_provider))
        );
        return ExitCode::ConfigError.as_i32();
    }
    let judge_enabled = args.judge && judge_selected;
    let generator_model = args
        .model
        .clone()
        .unwrap_or_else(|| config.generator.model.clone());
    let generator_provider = GeneratorProvider::from_model(&generator_model);
    if args.disable_max_tokens
        && (matches!(generator_provider, GeneratorProvider::Anthropic)
            || (judge_selected && matches!(judge_provider, JudgeProvider::Anthropic)))
    {
        eprintln!("error: --disable-max-tokens is not supported with Anthropic models");
        return ExitCode::ConfigError.as_i32();
    }
    if matches!(
        args.execution_mode,
        EvalExecutionMode::Live | EvalExecutionMode::Batch
    ) && resolve_generator_key(generator_provider).is_none()
    {
        eprintln!(
            "error: set one of {} to run live generation for model '{}'\n  tip: {}",
            generator_key_candidates(generator_provider).join(", "),
            generator_model,
            key_example(generator_key_candidates(generator_provider))
        );
        return ExitCode::ConfigError.as_i32();
    }
    let generation_mode = match args.execution_mode {
        EvalExecutionMode::Mock => GenerationMode::MockOnly,
        EvalExecutionMode::Live => GenerationMode::Live,
        EvalExecutionMode::Batch => GenerationMode::Batch,
    };

    if args.measure_discrimination && matches!(args.execution_mode, EvalExecutionMode::Mock) {
        eprintln!("error: --measure-discrimination requires --execution-mode live or batch");
        return ExitCode::ConfigError.as_i32();
    }

    if globals.verbose > 0 {
        eprintln!(
            "debug: eval setup mode={:?} generator_model={} judge_model={} judge_enabled={} fixtures={}",
            generation_mode,
            generator_model,
            judge_model,
            judge_enabled,
            fixtures.len()
        );
    }

    let concurrency = if matches!(
        generation_mode,
        GenerationMode::Live | GenerationMode::Batch
    ) && args.concurrency.is_none()
        && config.runner.concurrency.is_none()
    {
        1
    } else {
        args.concurrency
            .or(config.runner.concurrency)
            .or_else(default_concurrency)
            .unwrap_or(1)
    };
    let total_cases_for_hint: usize = fixtures.iter().map(|f| f.cases.len()).sum();
    if !globals.quiet
        && config.output.format != "json"
        && matches!(args.execution_mode, EvalExecutionMode::Live)
        && total_cases_for_hint > 50
    {
        eprintln!(
            "{} {} cases detected in live mode; use --execution-mode batch for ~50% cost savings",
            style("hint:").yellow().bold(),
            total_cases_for_hint
        );
    }
    let format = config.output.format.clone();
    let show_progress = !globals.quiet && (format != "json" && stderr().is_terminal());
    if !globals.quiet && format != "json" && args.judge && !judge_enabled {
        eprintln!(
            "{} --judge is set but no judge evaluator is active (--evaluator is {:?}). \
For fixtures that set judge per case, use --evaluator all (and keep --judge).",
            style("hint:").yellow().bold(),
            effective_evaluator
        );
    }
    let runner = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        offline: if matches!(
            generation_mode,
            GenerationMode::Live | GenerationMode::Batch
        ) {
            false
        } else {
            config.runner.offline
        },
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode,
        generator_model: Some(generator_model.clone()),
        generator_max_tokens: if args.disable_max_tokens {
            None
        } else {
            config.generator.max_tokens
        },
        generator_endpoint: args.generator_endpoint.clone(),
        fail_fast: false,
        mock_strict: std::env::var("agentcarousel_MOCK_STRICT").ok().as_deref() == Some("1"),
        command: "eval".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: args.run_id.clone(),
        batch_collect_id: None,
    };

    // Clone runner config for discrimination pass (before it is moved into eval_config).
    let discrimination_runner = if args.measure_discrimination {
        Some(runner.clone())
    } else {
        None
    };

    // Clone fixtures for discrimination pass (before they are moved into run_eval).
    let discrimination_fixtures = if args.measure_discrimination {
        Some(fixtures.clone())
    } else {
        None
    };

    let eval_config = EvalConfig {
        runner,
        runs: args.runs,
        seed: args.seed,
        evaluator: if effective_evaluator == "rules" {
            config.eval.default_evaluator.clone()
        } else {
            effective_evaluator
        },
        judge: judge_enabled,
        judge_model: Some(judge_model.clone()),
        effectiveness_threshold: config.eval.effectiveness_threshold,
        judge_max_tokens: if args.disable_max_tokens {
            None
        } else {
            config.judge.max_tokens
        },
        progress: show_progress,
    };

    // ── Batch fire-and-forget ─────────────────────────────────────────────────
    // Submit to the Anthropic batch API, save state for `agc batch fetch`, and exit.
    if matches!(generation_mode, GenerationMode::Batch) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("tokio runtime");
        let cases_for_batch = flatten_cases(fixtures);
        let fixture_paths: Vec<String> = args
            .paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let judge_model_for_batch = if judge_enabled {
            Some(eval_config.judge_model.clone().unwrap_or_default())
        } else {
            None
        };
        match runtime.block_on(submit_batch_only(
            &cases_for_batch,
            &eval_config.runner,
            fixture_paths,
            judge_model_for_batch,
        )) {
            Ok(batch_id) => {
                let n = cases_for_batch.len();
                eprintln!(
                    "Batch submitted: {} ({} cases)\n  status : agc batch status {}\n  fetch  : agc batch fetch {}",
                    batch_id, n, batch_id, batch_id
                );
                return ExitCode::Ok.as_i32();
            }
            Err(e) => {
                eprintln!("error: batch submit failed: {e}");
                return ExitCode::RuntimeError.as_i32();
            }
        }
    }

    prefetch_pricing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let mut run = runtime.block_on(run_eval(fixtures, eval_config));

    // Discrimination pass: run blank/degraded variants and attach scores to cases.
    if let (Some(disc_fixtures), Some(disc_runner)) =
        (discrimination_fixtures, discrimination_runner)
    {
        let current_passed: Vec<bool> = run
            .cases
            .iter()
            .map(|c| c.status == agentcarousel_core::CaseStatus::Passed)
            .collect();

        let disc_cases = flatten_cases(disc_fixtures);

        let scores = runtime.block_on(run_discrimination_eval(
            disc_cases,
            disc_runner,
            current_passed,
        ));

        for (case_result, (score, label)) in run.cases.iter_mut().zip(scores) {
            case_result.discrimination_score = Some(score);
            case_result.discrimination_label = Some(label);
        }
    }

    let judge_model_for_cost = if args.judge {
        Some(judge_model.as_str())
    } else {
        None
    };
    annotate_run_cost(&mut run, &generator_model, judge_model_for_cost);
    run.summary.generator_model = Some(generator_model.clone());
    run.summary.judge_model = if judge_enabled {
        Some(judge_model.clone())
    } else {
        None
    };
    run.summary.command_line = Some(std::env::args().collect::<Vec<_>>().join(" "));

    // Run the prompt-audit second pass when: judge is on, >50% of cases failed, and a
    // prompt.md exists. Skipped silently on errors (audit is best-effort).
    let total_cases = run.summary.total;
    let failed_cases = run.summary.failed;
    let fail_rate = if total_cases > 0 {
        failed_cases as f64 / total_cases as f64
    } else {
        0.0
    };
    if judge_enabled && fail_rate > 0.5 {
        if let Some(prompt_text) = load_prompt_md(&args.paths) {
            let show_audit_spinner =
                !globals.quiet && format != "json" && !globals.json && stderr().is_terminal();
            let audit_spinner: Option<indicatif::ProgressBar> = if show_audit_spinner {
                use indicatif::{ProgressBar, ProgressStyle};
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::with_template("{spinner:.green} {msg}")
                        .expect("spinner template")
                        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
                );
                pb.set_message("Running prompt audit...");
                pb.enable_steady_tick(std::time::Duration::from_millis(120));
                Some(pb)
            } else {
                None
            };
            let audit_max_tokens = config.judge.max_tokens.map(|t| t.max(2048));
            match run_prompt_audit(&prompt_text, &run.cases, &judge_model, audit_max_tokens) {
                Ok(audit) => {
                    if let Some(ref pb) = audit_spinner {
                        pb.finish_and_clear();
                    }
                    if let Some(ref mut cost) = run.summary.total_cost_usd {
                        if let Some(ti) = audit.judge_tokens_in {
                            if let Some(pricing) = agentcarousel_core::lookup_pricing(&judge_model)
                            {
                                *cost += pricing.prompt_usd_per_token * ti as f64
                                    + pricing.completion_usd_per_token
                                        * audit.judge_tokens_out.unwrap_or(0) as f64;
                            }
                        }
                    }
                    run.prompt_audit = Some(audit);
                }
                Err(err) => {
                    if let Some(ref pb) = audit_spinner {
                        pb.finish_and_clear();
                    }
                    if globals.verbose > 0 {
                        eprintln!("prompt audit skipped: {err}");
                    }
                }
            }
        }
    } else if judge_enabled && failed_cases > 1 {
        // Hint when there are multiple failures but not enough to auto-trigger the audit.
        if !globals.quiet && format != "json" && !globals.json {
            let bin = cli_invocation_name();
            let id = run.id.0.as_str();
            println!(
                "{} {} case(s) failed; run `{} audit run {}` to analyse the prompt",
                style("hint:").yellow().bold(),
                failed_cases,
                bin,
                id
            );
        }
    }

    let _ = persist_run(&run);

    let format_str = format.as_str();

    if globals.json {
        let value = serde_json::to_value(&run).unwrap_or(serde_json::Value::Null);
        JsonOutput::ok("eval", value).print();
    } else {
        match format_str {
            "json" => print_json(&run),
            _ => {
                if globals.quiet {
                    agentcarousel_reporters::print_terminal_summary(&run);
                } else {
                    print_terminal(&run, globals.verbose > 0);
                }
            }
        }

        if !globals.quiet && format_str != "json" {
            print_postflight_hints(&run);
        }
        if globals.quiet || format_str == "json" {
            print_eval_saved_run_hint(&run, globals.quiet || format_str == "json");
        }
    }

    if args.measure_discrimination && !globals.quiet && format != "json" && !globals.json {
        let high = run
            .cases
            .iter()
            .filter(|c| c.discrimination_label.as_deref() == Some("high"))
            .count();
        let low = run
            .cases
            .iter()
            .filter(|c| c.discrimination_label.as_deref() == Some("low"))
            .count();
        println!(
            "{} discrimination: {} high-value, {} low-value cases",
            style("info:").cyan().bold(),
            high,
            low
        );
        if low > 0 {
            println!(
                "  {} {} case(s) pass even with blank/degraded prompt — consider revising",
                style("hint:").yellow().bold(),
                low
            );
        }
    }

    if has_eval_failures(&run, config.eval.effectiveness_threshold) {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn has_eval_failures(run: &agentcarousel_core::Run, threshold: f32) -> bool {
    run.cases.iter().any(|case| {
        matches!(
            case.status,
            CaseStatus::Failed | CaseStatus::TimedOut | CaseStatus::Error | CaseStatus::Flaky
        ) || case
            .eval_scores
            .as_ref()
            .map(|scores| scores.effectiveness_score < threshold)
            .unwrap_or(true)
    })
}

fn cli_invocation_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "agc".to_string())
}

fn print_eval_saved_run_hint(run: &agentcarousel_core::Run, to_stderr: bool) {
    let bin = cli_invocation_name();
    let id = run.id.0.as_str();
    let line1 = format!("run id: {id}");
    let line2 = format!("next: {bin} report show {id}");
    if to_stderr {
        eprintln!("{line1}");
        eprintln!("{line2}");
    } else {
        println!("{line1}");
        println!("{line2}");
    }
}

fn print_postflight_hints(run: &agentcarousel_core::Run) {
    let provider_errors = &run.summary.provider_errors;
    if provider_errors.status_429
        + provider_errors.status_500
        + provider_errors.status_503
        + provider_errors.status_504
        > 0
    {
        println!(
            "{} provider errors detected; consider rerunning or lowering concurrency",
            style("hint:").yellow(),
        );
    }
    if run.summary.errored > 0 || run.summary.timed_out > 0 {
        println!(
            "{} use --verbose for diagnostics or --json to inspect outputs",
            style("hint:").yellow(),
        );
    }
}

fn key_example(keys: &[&str]) -> String {
    keys.first()
        .map(|key| format!("export {}=your_key_here", key))
        .unwrap_or_else(|| "export YOUR_API_KEY=your_key_here".to_string())
}

fn is_judge_selected(args: &EvalArgs, fixtures: &[agentcarousel_core::FixtureFile]) -> bool {
    if args.evaluator == "judge" {
        return true;
    }
    if args.evaluator != "all" {
        return false;
    }
    fixtures
        .iter()
        .flat_map(|fixture| fixture.cases.iter())
        .any(|case| {
            case.evaluator_config
                .as_ref()
                .map(|config| config.evaluator == "judge")
                .unwrap_or(false)
        })
}

fn resolve_judge_key(provider: JudgeProvider) -> Option<String> {
    judge_key_candidates(provider)
        .iter()
        .find_map(|key| std::env::var(key).ok())
}

fn generator_key_candidates(provider: GeneratorProvider) -> &'static [&'static str] {
    provider.key_candidates()
}

fn resolve_generator_key(provider: GeneratorProvider) -> Option<String> {
    provider
        .key_candidates()
        .iter()
        .find_map(|key| std::env::var(key).ok())
}

/// Look for a `prompt.md` adjacent to the fixture paths and return its contents.
fn load_prompt_md(paths: &[PathBuf]) -> Option<String> {
    for path in paths {
        let dir = if path.is_dir() {
            path.clone()
        } else {
            path.parent()?.to_path_buf()
        };
        let candidate = dir.join("prompt.md");
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            if !text.trim().is_empty() {
                return Some(text);
            }
        }
    }
    None
}
