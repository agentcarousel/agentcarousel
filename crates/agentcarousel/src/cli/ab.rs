use agentcarousel_core::{
    annotate_run_cost, fmt_cost, fmt_tokens, judge_key_candidates, judge_provider_from_model,
    prefetch_pricing, CaseStatus, FixtureFile, Message, Role,
};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::persist_run;
use agentcarousel_runner::{run_eval, EvalConfig, GenerationMode, GeneratorProvider, RunnerConfig};
use clap::{ArgAction, Parser, ValueEnum};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::io::{stderr, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{apply_case_filter, apply_tag_filter, collect_fixture_paths};
use super::output::JsonOutput;
use super::GlobalOptions;

#[derive(Debug, Clone, ValueEnum)]
enum AbExecutionMode {
    Mock,
    Live,
}

/// Compare two system prompts on the same fixture suite and see which one wins.
///
/// agc ab runs your test suite against two prompt variants concurrently, then prints a side-by-side table showing pass rates, effectiveness scores, and which specific cases changed between A and B. Both runs are saved to history so you can review them later.
///
/// Returns exit code 1 if variant B regresses relative to A (lower pass rate or effectiveness score), so you can use it as a gate in CI.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc ab --a prompts/old.md --b prompts/new.md fixtures/            # mock (fast, no API key)\n  agc ab --a p1.md --b p2.md fixtures/ --execution-mode live        # live generation\n  agc ab --a p1.md --b p2.md fixtures/ --judge                      # add LLM judge scoring\n  agc ab --a p1.md --b p2.md fixtures/ --model gemini-2.5-flash     # pick a specific model\n  agc ab --a p1.md --b p2.md fixtures/ --json                       # machine-readable output\n\nExit codes:\n  0  B is equivalent to or better than A\n  1  B regresses relative to A\n  4  runtime error (network, disk, config)"
)]
pub struct AbArgs {
    /// Fixture files or directories to run (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Path to the system prompt file for variant A.
    #[arg(long, value_name = "FILE")]
    a: PathBuf,
    /// Path to the system prompt file for variant B.
    #[arg(long, value_name = "FILE")]
    b: PathBuf,
    /// Evaluator: `rules` | `golden` | `process` | `judge` | `all`.
    #[arg(short = 'e', long, default_value = "rules")]
    evaluator: String,
    /// Enable the LLM judge for judge-scored cases (requires API keys).
    #[arg(short = 'j', long)]
    judge: bool,
    /// Model to use for judge scoring (overrides config `judge.model`).
    #[arg(short = 'J', long)]
    judge_model: Option<String>,
    /// `mock` (default) or `live` — whether to call a real generator API.
    #[arg(short = 'x', long, value_enum, default_value_t = AbExecutionMode::Mock)]
    execution_mode: AbExecutionMode,
    /// Generator model name (overrides config `generator.model`).
    #[arg(short = 'm', long)]
    model: Option<String>,
    /// Number of times to run each case per variant (use >1 for flakiness detection).
    #[arg(short = 'n', long, default_value_t = 1)]
    runs: u32,
    /// Glob matched against full case ids (`skill/case-id`).
    #[arg(short = 'f', long)]
    filter: Option<String>,
    /// Comma-separated case tags to include (e.g. `smoke,fast`).
    #[arg(long = "filter-tags", value_name = "TAG", value_delimiter = ',')]
    filter_tags: Option<Vec<String>>,
    /// Per-variant case concurrency (default: 1 for live, 4 for mock).
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    /// Per-case timeout in seconds.
    #[arg(short = 't', long)]
    timeout: Option<u64>,
    /// Effectiveness delta threshold for declaring a winner (default: 0.05).
    #[arg(long, default_value_t = 0.05_f32)]
    threshold: f32,
    /// Show a variant-level progress bar on stderr (default: on when stderr is a TTY).
    #[arg(short = 'P', long, action = ArgAction::SetTrue)]
    progress: bool,
    /// Never show the A/B progress bar.
    #[arg(short = 'N', long, action = ArgAction::SetTrue)]
    no_progress: bool,
}

#[derive(Debug, Serialize)]
pub struct AbOutput {
    pub prompt_a: String,
    pub prompt_b: String,
    pub run_id_a: String,
    pub run_id_b: String,
    pub total_cases: u32,
    pub pass_rate_a: f32,
    pub pass_rate_b: f32,
    pub effectiveness_a: Option<f32>,
    pub effectiveness_b: Option<f32>,
    pub winner: String,
    pub regression: bool,
    pub cases_flipped_to_b: u32,
    pub cases_flipped_to_a: u32,
    pub cases: Vec<AbCaseComparison>,
}

#[derive(Debug, Serialize)]
pub struct AbCaseComparison {
    pub case_id: String,
    pub status_a: CaseStatus,
    pub status_b: CaseStatus,
    pub effectiveness_a: Option<f32>,
    pub effectiveness_b: Option<f32>,
    pub delta: Option<f32>,
    pub winner: String,
    pub flipped: bool,
}

pub fn run_ab(args: AbArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let prompt_a = match std::fs::read_to_string(&args.a) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read --a {}: {e}", args.a.display());
            return ExitCode::ConfigError.as_i32();
        }
    };
    let prompt_b = match std::fs::read_to_string(&args.b) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read --b {}: {e}", args.b.display());
            return ExitCode::ConfigError.as_i32();
        }
    };

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

    let generator_model = args
        .model
        .clone()
        .unwrap_or_else(|| config.generator.model.clone());
    let generation_mode = match args.execution_mode {
        AbExecutionMode::Mock => GenerationMode::MockOnly,
        AbExecutionMode::Live => GenerationMode::Live,
    };
    if matches!(generation_mode, GenerationMode::Live) {
        let provider = GeneratorProvider::from_model(&generator_model);
        if resolve_key(provider.key_candidates()).is_none() {
            eprintln!(
                "error: set one of {} to run live generation for model '{}'",
                provider.key_candidates().join(", "),
                generator_model
            );
            return ExitCode::ConfigError.as_i32();
        }
    }

    let fixture_paths = collect_fixture_paths(&args.paths);
    if fixture_paths.is_empty() {
        eprintln!("error: no fixture files found in the specified paths");
        return ExitCode::ConfigError.as_i32();
    }

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
    let concurrency = args.concurrency.or(config.runner.concurrency).unwrap_or(
        if matches!(generation_mode, GenerationMode::Live) {
            1
        } else {
            4
        },
    );

    if !globals.json && !globals.quiet {
        eprintln!(
            "{} A/B — {} case(s) × 2 variants — running in parallel",
            style("⚖").bold(),
            total_cases,
        );
    }

    let show_progress = !args.no_progress
        && !globals.quiet
        && !globals.json
        && (args.progress || stderr().is_terminal());

    let fixtures_a = inject_system_prompt(fixtures.clone(), &prompt_a);
    let fixtures_b = inject_system_prompt(fixtures, &prompt_b);
    let eval_config_a = build_eval_config(
        "a",
        &args,
        config,
        concurrency,
        judge_active,
        &judge_model,
        &generator_model,
    );
    let eval_config_b = build_eval_config(
        "b",
        &args,
        config,
        concurrency,
        judge_active,
        &judge_model,
        &generator_model,
    );

    prefetch_pricing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    let (run_a, run_b) = runtime.block_on(async {
        let pb: Option<ProgressBar> = if show_progress {
            let pb = ProgressBar::new(2);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/2 variants {msg}",
                )
                .expect("progress template")
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
            );
            pb.enable_steady_tick(Duration::from_millis(120));
            Some(pb)
        } else {
            None
        };

        let pb_a = pb.clone();
        let pb_b = pb.clone();
        let (run_a, run_b) = tokio::join!(
            async move {
                let r = run_eval(fixtures_a, eval_config_a).await;
                if let Some(pb) = pb_a {
                    pb.inc(1);
                }
                r
            },
            async move {
                let r = run_eval(fixtures_b, eval_config_b).await;
                if let Some(pb) = pb_b {
                    pb.inc(1);
                }
                r
            },
        );
        if let Some(pb) = pb {
            pb.finish_and_clear();
        }
        (run_a, run_b)
    });

    let judge_model_opt = if args.judge {
        Some(judge_model.as_str())
    } else {
        None
    };
    let mut run_a = run_a;
    let mut run_b = run_b;
    annotate_run_cost(&mut run_a, &generator_model, judge_model_opt);
    annotate_run_cost(&mut run_b, &generator_model, judge_model_opt);
    let ab_cmd_line = std::env::args().collect::<Vec<_>>().join(" ");
    let judge_model_str = judge_model_opt.map(|s| s.to_string());
    run_a.summary.generator_model = Some(generator_model.clone());
    run_b.summary.generator_model = Some(generator_model.clone());
    run_a.summary.judge_model = judge_model_str.clone();
    run_b.summary.judge_model = judge_model_str;
    run_a.summary.command_line = Some(ab_cmd_line.clone());
    run_b.summary.command_line = Some(ab_cmd_line);

    let _ = persist_run(&run_a);
    let _ = persist_run(&run_b);

    let output = build_ab_output(&args.a, &args.b, &run_a, &run_b, args.threshold);
    let regression = output.regression;

    if globals.json {
        JsonOutput::ok("ab", output).print();
    } else {
        print_ab_terminal(&output);
        print_ab_cost_lines(&run_a, &run_b);
    }

    if regression {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn inject_system_prompt(mut fixtures: Vec<FixtureFile>, system_prompt: &str) -> Vec<FixtureFile> {
    for fixture in &mut fixtures {
        for case in &mut fixture.cases {
            case.input.messages.retain(|m| m.role != Role::System);
            case.input.messages.insert(
                0,
                Message {
                    role: Role::System,
                    content: system_prompt.to_string(),
                },
            );
        }
    }
    fixtures
}

fn build_eval_config(
    variant: &str,
    args: &AbArgs,
    config: &ResolvedConfig,
    concurrency: usize,
    judge: bool,
    judge_model: &str,
    generator_model: &str,
) -> EvalConfig {
    let generation_mode = match args.execution_mode {
        AbExecutionMode::Mock => GenerationMode::MockOnly,
        AbExecutionMode::Live => GenerationMode::Live,
    };
    let runner = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        run_timeout_secs: None,
        offline: if matches!(generation_mode, GenerationMode::Live) {
            false
        } else {
            config.runner.offline
        },
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode,
        generator_model: Some(generator_model.to_string()),
        generator_max_tokens: config.generator.max_tokens,
        generator_endpoint: None,
        fail_fast: false,
        mock_strict: false,
        command: format!("ab-{variant}"),
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

fn build_ab_output(
    path_a: &Path,
    path_b: &Path,
    run_a: &agentcarousel_core::Run,
    run_b: &agentcarousel_core::Run,
    threshold: f32,
) -> AbOutput {
    use std::collections::HashMap;

    let cases_a: HashMap<&str, &agentcarousel_core::CaseResult> = run_a
        .cases
        .iter()
        .map(|c| (c.case_id.0.as_str(), c))
        .collect();

    let mut case_comparisons: Vec<AbCaseComparison> = Vec::new();
    let mut flipped_to_b = 0u32;
    let mut flipped_to_a = 0u32;

    for case_b in &run_b.cases {
        let Some(case_a) = cases_a.get(case_b.case_id.0.as_str()) else {
            continue;
        };

        let eff_a = case_a.eval_scores.as_ref().map(|s| s.effectiveness_score);
        let eff_b = case_b.eval_scores.as_ref().map(|s| s.effectiveness_score);
        let delta = match (eff_a, eff_b) {
            (Some(a), Some(b)) => Some(b - a),
            _ => None,
        };

        let a_passed = case_a.status == CaseStatus::Passed;
        let b_passed = case_b.status == CaseStatus::Passed;
        let flipped = case_a.status != case_b.status;

        if !a_passed && b_passed {
            flipped_to_b += 1;
        }
        if a_passed && !b_passed {
            flipped_to_a += 1;
        }

        let case_winner = match (a_passed, b_passed, delta) {
            (true, false, _) => "a",
            (false, true, _) => "b",
            (_, _, Some(d)) if d > threshold => "b",
            (_, _, Some(d)) if d < -threshold => "a",
            _ => "tie",
        }
        .to_string();

        case_comparisons.push(AbCaseComparison {
            case_id: case_b.case_id.0.clone(),
            status_a: case_a.status.clone(),
            status_b: case_b.status.clone(),
            effectiveness_a: eff_a,
            effectiveness_b: eff_b,
            delta,
            winner: case_winner,
            flipped,
        });
    }

    let pass_rate_a = run_a.summary.pass_rate;
    let pass_rate_b = run_b.summary.pass_rate;
    let eff_a = run_a.summary.mean_effectiveness_score;
    let eff_b = run_b.summary.mean_effectiveness_score;

    let overall_delta = match (eff_a, eff_b) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };

    let winner = match overall_delta {
        Some(d) if d > threshold => "b",
        Some(d) if d < -threshold => "a",
        _ => {
            if pass_rate_b > pass_rate_a + threshold {
                "b"
            } else if pass_rate_a > pass_rate_b + threshold {
                "a"
            } else {
                "tie"
            }
        }
    }
    .to_string();

    let regression = winner == "a"
        || overall_delta.is_some_and(|d| d < -threshold)
        || case_comparisons
            .iter()
            .any(|c| c.winner == "a" && c.flipped);

    AbOutput {
        prompt_a: path_a.display().to_string(),
        prompt_b: path_b.display().to_string(),
        run_id_a: run_a.id.0.clone(),
        run_id_b: run_b.id.0.clone(),
        total_cases: run_b.summary.total,
        pass_rate_a,
        pass_rate_b,
        effectiveness_a: eff_a,
        effectiveness_b: eff_b,
        winner: winner.clone(),
        regression,
        cases_flipped_to_b: flipped_to_b,
        cases_flipped_to_a: flipped_to_a,
        cases: case_comparisons,
    }
}

fn print_ab_terminal(output: &AbOutput) {
    let skill = String::new();
    let _ = skill;

    let prompt_a_short = short_path(&output.prompt_a);
    let prompt_b_short = short_path(&output.prompt_b);
    let label_w = prompt_a_short.len().max(prompt_b_short.len()).max(6);

    println!();
    println!(
        "  {} A/B — {} case(s)\n",
        style("⚖").bold(),
        output.total_cases
    );

    println!(
        "  {:<3}  {:<label_w$}  {:>9}  {:>7}  {}",
        style("Var").bold(),
        style("Prompt").bold(),
        style("Pass rate").bold(),
        style("Score").bold(),
        style("Run ID").bold(),
    );
    let sep_w = 3 + 2 + label_w + 2 + 9 + 2 + 7 + 2 + 10;
    println!("  {}", "─".repeat(sep_w));

    let score_a = output
        .effectiveness_a
        .map(|s| format!("{s:.2}"))
        .unwrap_or_else(|| "—".to_string());
    let score_b = output
        .effectiveness_b
        .map(|s| format!("{s:.2}"))
        .unwrap_or_else(|| "—".to_string());

    println!(
        "  {:<3}  {:<label_w$}  {:>8.0}%  {:>7}  {}",
        style("A").bold(),
        prompt_a_short,
        output.pass_rate_a * 100.0,
        score_a,
        output.run_id_a,
    );
    println!(
        "  {:<3}  {:<label_w$}  {:>8.0}%  {:>7}  {}",
        style("B").bold(),
        prompt_b_short,
        output.pass_rate_b * 100.0,
        score_b,
        output.run_id_b,
    );
    println!("  {}", "─".repeat(sep_w));

    let delta_line = match (output.effectiveness_a, output.effectiveness_b) {
        (Some(a), Some(b)) => {
            let d = b - a;
            let arrow = if d > 0.0 { "▲" } else { "▼" };
            format!("  Δscore {d:+.2} {arrow}")
        }
        _ => {
            let d = output.pass_rate_b - output.pass_rate_a;
            let arrow = if d > 0.0 { "▲" } else { "▼" };
            format!("  Δpass {d:+.0}% {arrow}")
        }
    };

    let winner_label = match output.winner.as_str() {
        "a" => style("Winner: A").red().bold().to_string(),
        "b" => style("Winner: B").green().bold().to_string(),
        _ => style("Tie").dim().to_string(),
    };
    println!("\n  {winner_label}{delta_line}");

    if output.cases_flipped_to_a > 0 || output.cases_flipped_to_b > 0 {
        println!();
        if output.cases_flipped_to_b > 0 {
            println!(
                "  {} {} case(s) newly {} with B",
                style("✓").green(),
                output.cases_flipped_to_b,
                style("passing").green(),
            );
        }
        if output.cases_flipped_to_a > 0 {
            println!(
                "  {} {} case(s) newly {} with B",
                style("✗").red(),
                output.cases_flipped_to_a,
                style("failing").red(),
            );
        }
    }

    let changed: Vec<&AbCaseComparison> = output
        .cases
        .iter()
        .filter(|c| c.flipped || c.delta.is_some_and(|d| d.abs() > 0.05))
        .collect();

    if !changed.is_empty() {
        println!();
        let id_w = changed
            .iter()
            .map(|c| c.case_id.len())
            .max()
            .unwrap_or(4)
            .clamp(4, 40);
        println!(
            "\n  {:<id_w$}  {:>8}  {:>8}  {:>7}  {}",
            style("Case").bold(),
            style("A").bold(),
            style("B").bold(),
            style("Delta").bold(),
            style("Winner").bold(),
        );
        println!("  {}", "─".repeat(id_w + 2 + 8 + 2 + 8 + 2 + 7 + 2 + 6));
        for c in &changed {
            let eff_a = c
                .effectiveness_a
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| format!("{:?}", c.status_a).to_lowercase());
            let eff_b = c
                .effectiveness_b
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| format!("{:?}", c.status_b).to_lowercase());
            let delta_str = c
                .delta
                .map(|d| format!("{d:+.2}"))
                .unwrap_or_else(|| "—".to_string());
            let winner_col = match c.winner.as_str() {
                "a" => style("A").red().to_string(),
                "b" => style("B").green().to_string(),
                _ => style("tie").dim().to_string(),
            };
            let short_id: String = c.case_id.chars().take(id_w).collect();
            println!("  {short_id:<id_w$}  {eff_a:>8}  {eff_b:>8}  {delta_str:>7}  {winner_col}",);
        }
    }

    println!();
    if output.regression {
        println!("  {}", style("Exit 1 — B regresses relative to A").red());
    } else {
        println!("  {}", style("No regression detected").green());
    }
    println!();
}

fn short_path(p: &str) -> String {
    let path = std::path::Path::new(p);
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

fn is_judge_evaluator_active(evaluator: &str) -> bool {
    evaluator == "judge" || evaluator == "all"
}

fn resolve_key(candidates: &[&str]) -> Option<String> {
    candidates.iter().find_map(|k| std::env::var(k).ok())
}

fn print_ab_cost_lines(run_a: &agentcarousel_core::Run, run_b: &agentcarousel_core::Run) {
    let a = &run_a.summary;
    let b = &run_b.summary;
    let any = a.total_cost_usd.is_some()
        || b.total_cost_usd.is_some()
        || a.tokens_in.is_some()
        || b.tokens_in.is_some();
    if !any {
        return;
    }

    for (label, s) in [("A", a), ("B", b)] {
        let total_in = s
            .tokens_in
            .map(|g| g + s.judge_tokens_in.unwrap_or(0))
            .or(s.judge_tokens_in);
        let total_out = s
            .tokens_out
            .map(|g| g + s.judge_tokens_out.unwrap_or(0))
            .or(s.judge_tokens_out);
        if total_in.is_none() && s.total_cost_usd.is_none() {
            continue;
        }
        let tok = format!("{}↑ {}↓", fmt_tokens(total_in), fmt_tokens(total_out));
        let line = match s.total_cost_usd {
            Some(_) => format!(
                "{}  tokens {}  ·  {}",
                label,
                tok,
                fmt_cost(s.total_cost_usd)
            ),
            None => format!("{}  tokens {}", label, tok),
        };
        println!("  {}", style(line).dim());
    }
}

#[cfg(test)]
mod tests {
    use super::{inject_system_prompt, is_judge_evaluator_active};
    use agentcarousel_core::{Case, CaseId, CaseInput, Expected, FixtureFile, Message, Role};

    fn make_fixture(messages: Vec<Message>) -> FixtureFile {
        FixtureFile {
            schema_version: 1,
            skill_or_agent: "test".to_string(),
            defaults: None,
            cases: vec![Case {
                id: CaseId("c1".to_string()),
                description: None,
                tags: vec![],
                input: CaseInput {
                    messages,
                    context: None,
                    env_overrides: None,
                },
                expected: Expected {
                    tool_sequence: None,
                    output: None,
                    rubric: None,
                },
                evaluator_config: None,
                timeout_secs: None,
                seed: None,
            }],
            bundle_id: None,
            bundle_version: None,
            certification_track: None,
            risk_tier: None,
            data_handling: None,
        }
    }

    #[test]
    fn inject_replaces_existing_system_message() {
        let msgs = vec![
            Message {
                role: Role::System,
                content: "old".to_string(),
            },
            Message {
                role: Role::User,
                content: "hello".to_string(),
            },
        ];
        let result = inject_system_prompt(vec![make_fixture(msgs)], "new system");
        let case = &result[0].cases[0];
        assert_eq!(case.input.messages[0].role, Role::System);
        assert_eq!(case.input.messages[0].content, "new system");
        assert_eq!(case.input.messages.len(), 2);
    }

    #[test]
    fn inject_prepends_when_no_system_message() {
        let msgs = vec![Message {
            role: Role::User,
            content: "hello".to_string(),
        }];
        let result = inject_system_prompt(vec![make_fixture(msgs)], "my system");
        let case = &result[0].cases[0];
        assert_eq!(case.input.messages[0].role, Role::System);
        assert_eq!(case.input.messages[0].content, "my system");
        assert_eq!(case.input.messages.len(), 2);
    }

    #[test]
    fn judge_active_logic() {
        assert!(is_judge_evaluator_active("judge"));
        assert!(is_judge_evaluator_active("all"));
        assert!(!is_judge_evaluator_active("rules"));
    }
}
