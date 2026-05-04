use agentcarousel_core::{
    judge_key_candidates, judge_provider_from_model, CaseStatus, CertificationContext,
    JudgeProvider,
};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::{persist_run, print_json, print_junit, print_terminal};
use agentcarousel_runner::{run_eval, EvalConfig, GenerationMode, GeneratorProvider, RunnerConfig};
use clap::{ArgAction, Parser, ValueEnum};
use console::style;
use std::io::{stderr, IsTerminal};
use std::path::PathBuf;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{
    apply_case_filter, apply_tag_filter, collect_fixture_paths, default_concurrency,
};
use super::GlobalOptions;

const GENERATOR_GEMINI_KEY_ENV_CANDIDATES: [&str; 6] = [
    "AGENTCAROUSEL_GENERATOR_KEY",
    "agentcarousel_GENERATOR_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];
const GENERATOR_OPENAI_KEY_ENV_CANDIDATES: [&str; 5] = [
    "AGENTCAROUSEL_GENERATOR_KEY",
    "agentcarousel_GENERATOR_KEY",
    "OPENAI_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];
const GENERATOR_ANTHROPIC_KEY_ENV_CANDIDATES: [&str; 5] = [
    "AGENTCAROUSEL_GENERATOR_KEY",
    "agentcarousel_GENERATOR_KEY",
    "ANTHROPIC_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];
const GENERATOR_OPENROUTER_KEY_ENV_CANDIDATES: [&str; 5] = [
    "OPENROUTER_API_KEY",
    "AGENTCAROUSEL_GENERATOR_KEY",
    "agentcarousel_GENERATOR_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];

#[derive(Debug, Clone, ValueEnum)]
enum EvalExecutionMode {
    Mock,
    Live,
}

/// Run evaluation (mock or live; add --judge when fixtures use judge evaluators).
#[derive(Debug, Parser)]
pub struct EvalArgs {
    /// Fixture files or dirs (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    #[arg(short = 'n', long, default_value_t = 1)]
    runs: u32,
    #[arg(short = 's', long, default_value_t = 0)]
    seed: u64,
    /// Which evaluator runs: `rules`, `golden`, `process`, `judge`, or `all` (use each case's evaluator from the fixture).
    ///
    /// Mixed fixtures (e.g. rules + golden + judge per case) require `all` for `--judge` to take effect.
    #[arg(short = 'e', long, default_value = "rules")]
    evaluator: String,
    /// Call the LLM judge API for cases that use the judge evaluator (needs API keys; see error text if missing).
    ///
    /// No effect unless the active evaluator can select judge — typically `--evaluator all` or `--evaluator judge`.
    #[arg(short = 'j', long)]
    judge: bool,
    #[arg(short = 'J', long)]
    judge_model: Option<String>,
    #[arg(short = 'x', long, value_enum, default_value_t = EvalExecutionMode::Mock)]
    execution_mode: EvalExecutionMode,
    #[arg(short = 'm', long)]
    model: Option<String>,
    #[arg(short = 'M', long)]
    disable_max_tokens: bool,
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    #[arg(short = 'f', long)]
    format: Option<String>,
    #[arg(short = 'F', long)]
    filter: Option<String>,
    /// Comma-separated case tags to include (e.g. certification, smoke).
    #[arg(long = "filter-tags", value_name = "TAG", value_delimiter = ',')]
    filter_tags: Option<Vec<String>>,
    #[arg(short = 'C', long)]
    certification_context: Option<CliCertificationContext>,
    #[arg(short = 'i', long)]
    carousel_iteration: Option<u32>,
    #[arg(short = 'p', long)]
    policy_version: Option<String>,
    /// Show a case-level progress bar on stderr (default: on for non-JSON/JUnit output when stderr is a TTY; use with `--format json` so only stderr shows progress).
    #[arg(short = 'P', long, action = ArgAction::SetTrue)]
    progress: bool,
    /// Never show the eval case progress bar.
    #[arg(short = 'N', long, action = ArgAction::SetTrue)]
    no_progress: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum CliCertificationContext {
    Local,
    Msp,
    Ci,
}

impl From<CliCertificationContext> for CertificationContext {
    fn from(value: CliCertificationContext) -> Self {
        match value {
            CliCertificationContext::Local => CertificationContext::Local,
            CliCertificationContext::Msp => CertificationContext::Msp,
            CliCertificationContext::Ci => CertificationContext::Ci,
        }
    }
}

pub fn run_eval_command(args: EvalArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    if globals.verbose >= 2 {
        // Enable deeper evaluator diagnostics for this invocation.
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
    if matches!(args.execution_mode, EvalExecutionMode::Live)
        && resolve_generator_key(generator_provider).is_none()
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
    };

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

    // Live provider-backed evals are rate-limited and can return 429/503 under burst load.
    // Unless the user explicitly overrides concurrency, default live runs to serialized
    // execution to reduce transient provider errors.
    let concurrency = if matches!(generation_mode, GenerationMode::Live)
        && args.concurrency.is_none()
        && config.runner.concurrency.is_none()
    {
        1
    } else {
        args.concurrency
            .or(config.runner.concurrency)
            .or_else(default_concurrency)
            .unwrap_or(1)
    };
    let format = args
        .format
        .clone()
        .unwrap_or_else(|| config.output.format.clone());
    let show_progress = !args.no_progress
        && !globals.quiet
        && (args.progress || ((format != "json" && format != "junit") && stderr().is_terminal()));
    if !globals.quiet && format != "json" && format != "junit" {
        if args.judge && !judge_enabled {
            eprintln!(
                "{} --judge is set but the judge evaluator is not active (--evaluator is {:?}). \
For fixtures like cmmc-assessor that set judge per case, use --evaluator all (and keep --judge).",
                style("hint:").yellow().bold(),
                args.evaluator
            );
        }
        print_preflight(
            &generation_mode,
            &generator_model,
            judge_enabled,
            &judge_model,
            args.runs,
            concurrency,
        );
    }
    let runner = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        offline: if matches!(generation_mode, GenerationMode::Live) {
            false
        } else {
            config.runner.offline
        },
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode,
        generator_model: Some(generator_model),
        generator_max_tokens: if args.disable_max_tokens {
            None
        } else {
            config.generator.max_tokens
        },
        fail_fast: false,
        mock_strict: std::env::var("agentcarousel_MOCK_STRICT").ok().as_deref() == Some("1"),
        command: "eval".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: globals.run_id.clone(),
    };

    let eval_config = EvalConfig {
        runner,
        runs: args.runs,
        seed: args.seed,
        evaluator: if args.evaluator == "rules" {
            config.eval.default_evaluator.clone()
        } else {
            args.evaluator
        },
        judge: judge_enabled,
        judge_model: Some(judge_model),
        effectiveness_threshold: config.eval.effectiveness_threshold,
        judge_max_tokens: if args.disable_max_tokens {
            None
        } else {
            config.judge.max_tokens
        },
        certification_context: args.certification_context.map(Into::into),
        carousel_iteration: args.carousel_iteration,
        policy_version: args.policy_version,
        progress: show_progress,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");
    let run = runtime.block_on(run_eval(fixtures, eval_config));

    let _ = persist_run(&run);
    let format_str = format.as_str();
    match format_str {
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

    if !globals.quiet && format_str != "json" && format_str != "junit" {
        print_postflight_hints(&run);
    }
    // Keep machine-readable stdout clean for json/junit; still emit run id + next step.
    let hint_to_stderr = globals.quiet || format_str == "json" || format_str == "junit";
    print_eval_saved_run_hint(&run, hint_to_stderr);

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

fn print_preflight(
    mode: &GenerationMode,
    generator_model: &str,
    judge_enabled: bool,
    judge_model: &str,
    runs: u32,
    concurrency: usize,
) {
    println!("{} {}", style("Agentcarousel").bold(), style("eval").cyan());
    println!(
        "  mode: {}  runs: {}  concurrency: {}",
        style(format!("{mode:?}")).yellow(),
        runs,
        concurrency
    );
    println!("  generator: {}", style(generator_model).green());
    if judge_enabled {
        println!("  judge: {}", style(judge_model).green());
    } else {
        println!("  judge: {}", style("disabled").yellow());
    }
    if matches!(mode, GenerationMode::MockOnly) {
        println!(
            "  {} try --execution-mode live --model gemini-1.5-pro or openrouter/free",
            style("tip:").yellow(),
        );
    }
}

fn cli_invocation_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "agentcarousel".to_string())
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
            "{} use --verbose for diagnostics or --format json to inspect outputs",
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
    // For --evaluator all, detect judge per-case from fixture metadata so we can
    // enforce --judge/key requirements before runtime execution starts.
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
    match provider {
        GeneratorProvider::Gemini => &GENERATOR_GEMINI_KEY_ENV_CANDIDATES,
        GeneratorProvider::OpenAi => &GENERATOR_OPENAI_KEY_ENV_CANDIDATES,
        GeneratorProvider::Anthropic => &GENERATOR_ANTHROPIC_KEY_ENV_CANDIDATES,
        GeneratorProvider::OpenRouter => &GENERATOR_OPENROUTER_KEY_ENV_CANDIDATES,
    }
}

fn resolve_generator_key(provider: GeneratorProvider) -> Option<String> {
    generator_key_candidates(provider)
        .iter()
        .find_map(|key| std::env::var(key).ok())
}
