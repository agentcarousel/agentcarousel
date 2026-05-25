use agentcarousel_core::{annotate_run_cost, CaseResult, CaseStatus, FixtureFile, Message, Role};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::persist_run;
use agentcarousel_runner::{call_llm, run_eval, EvalConfig, GenerationMode, RunnerConfig};
use clap::Parser;
use console::style;
use std::path::PathBuf;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::fixture_utils::collect_fixture_paths;
use super::GlobalOptions;

/// Automated system prompt optimization loop.
///
/// Runs eval → analyzes failures → synthesizes prompt candidates → scores them → applies the
/// best and repeats. Stops when the target pass rate is reached, the budget is exhausted, or
/// max iterations are hit.
#[derive(Debug, Parser)]
#[command(
    long_about = "Automated prompt optimization loop.\n\nEach iteration: (1) eval to find failing cases, (2) analyze failures by rubric dimension, (3) synthesize 3 prompt candidates via LLM, (4) score each candidate, (5) apply the best one. Writes the winning prompt to prompt.md and saves an optimization report.\n\nRequires ANTHROPIC_API_KEY (or the key for your chosen model).",
    after_help = "Examples:\n  agc optimize fixtures/my-skill/\n  agc optimize fixtures/my-skill/ --target-score 0.95 --max-iter 5 --budget 15\n  agc optimize fixtures/my-skill/ --model claude-opus-4-7 --judge-model claude-opus-4-7"
)]
pub struct OptimizeArgs {
    /// Fixture files or directory to optimize against.
    #[arg(value_name = "PATH")]
    path: PathBuf,
    /// Stop when pass rate reaches this value (0.0–1.0).
    #[arg(long, default_value_t = 0.9)]
    target_score: f32,
    /// Maximum USD to spend across all eval and LLM calls.
    #[arg(long, default_value_t = 10.0)]
    budget: f64,
    /// Maximum number of optimization iterations.
    #[arg(long, default_value_t = 5)]
    max_iter: u32,
    /// Generator model for eval runs (default: from config).
    #[arg(long)]
    model: Option<String>,
    /// Judge model for scoring and analysis (default: from config).
    #[arg(long)]
    judge_model: Option<String>,
    /// Config file path (default: agentcarousel.toml).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Write optimization report JSON to this file (default: optimize-report-<run_id>.json).
    #[arg(long)]
    output: Option<PathBuf>,
    /// Show what would happen without running any LLM calls.
    #[arg(long)]
    dry_run: bool,
}

// ── Report types ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Debug)]
struct OptimizeReport {
    skill: String,
    baseline_score: f32,
    final_score: f32,
    target_score: f32,
    target_reached: bool,
    iterations_run: u32,
    score_trajectory: Vec<f32>,
    total_cost_usd: f64,
    baseline_prompt: String,
    final_prompt: String,
    iterations: Vec<IterationRecord>,
}

#[derive(serde::Serialize, Debug)]
struct IterationRecord {
    iteration: u32,
    score_before: f32,
    failure_count: usize,
    top_rubric_failure: Option<String>,
    candidates: Vec<CandidateRecord>,
    applied: Option<usize>,
    score_after: f32,
    cost_usd: f64,
}

#[derive(serde::Serialize, Debug)]
struct CandidateRecord {
    index: usize,
    score: f32,
    delta: f32,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn run_optimize_command(
    args: OptimizeArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> i32 {
    let model = args
        .model
        .clone()
        .unwrap_or_else(|| config.generator.model.clone());
    let judge_model = args
        .judge_model
        .clone()
        .unwrap_or_else(|| config.judge.model.clone());

    // Collect fixture paths and load fixtures.
    let fixture_paths = collect_fixture_paths(std::slice::from_ref(&args.path));
    if fixture_paths.is_empty() {
        eprintln!("error: no fixture files found at '{}'", args.path.display());
        return ExitCode::NotFound.as_i32();
    }

    let mut fixtures: Vec<FixtureFile> = Vec::new();
    for path in &fixture_paths {
        match load_fixture(path) {
            Ok(f) => fixtures.push(f),
            Err(e) => {
                eprintln!("error: failed to load '{}': {e}", path.display());
                return ExitCode::NotFound.as_i32();
            }
        }
    }

    let total_cases: usize = fixtures.iter().map(|f| f.cases.len()).sum();
    if total_cases == 0 {
        eprintln!("error: fixtures contain no cases");
        return ExitCode::NotFound.as_i32();
    }

    // Determine prompt.md path from the first fixture's skill.
    let skill = fixtures[0].skill_or_agent.clone();
    let prompt_path = PathBuf::from("fixtures").join(&skill).join("prompt.md");
    if !prompt_path.exists() {
        eprintln!(
            "error: prompt.md not found at '{}'\n  hint: agc optimize requires a skill directory with a prompt.md",
            prompt_path.display()
        );
        return ExitCode::NotFound.as_i32();
    }

    let baseline_prompt = match std::fs::read_to_string(&prompt_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not read '{}': {e}", prompt_path.display());
            return ExitCode::RuntimeError.as_i32();
        }
    };

    if !globals.quiet {
        println!(
            "\n{} {} · {} cases · target={:.0}% · budget=${:.0} · max_iter={}",
            style("agc optimize").bold(),
            style(&skill).cyan(),
            total_cases,
            args.target_score * 100.0,
            args.budget,
            args.max_iter,
        );
        println!("  prompt: {}", prompt_path.display());
        println!("  model: {}  judge: {}\n", model, judge_model);
    }

    if args.dry_run {
        println!(
            "{} dry-run mode — no API calls will be made",
            style("note:").yellow().bold()
        );
        println!(
            "  Would optimize: {} cases across {} fixtures",
            total_cases,
            fixtures.len()
        );
        println!("  Prompt path: {}", prompt_path.display());
        return ExitCode::Ok.as_i32();
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    let report = runtime.block_on(optimize_loop(
        fixtures,
        fixture_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        baseline_prompt,
        prompt_path.clone(),
        &model,
        &judge_model,
        &args,
        config,
        globals,
    ));

    // Print summary.
    println!(
        "\n{} score trajectory: {}",
        style("Optimization complete.").bold(),
        report
            .score_trajectory
            .iter()
            .map(|s| format!("{:.0}%", s * 100.0))
            .collect::<Vec<_>>()
            .join(" → ")
    );
    if report.target_reached {
        println!(
            "  {} target {:.0}% reached!",
            style("✓").green().bold(),
            report.target_score * 100.0
        );
    } else {
        println!(
            "  {} target {:.0}% not reached (final {:.0}%)",
            style("✗").yellow(),
            report.target_score * 100.0,
            report.final_score * 100.0
        );
    }
    if report.total_cost_usd > 0.0 {
        println!("  estimated cost: ${:.4}", report.total_cost_usd);
    }

    // Write report file.
    let report_path = args.output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "optimize-report-{}.json",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ))
    });
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&report_path, json) {
                eprintln!(
                    "warn: could not write report to '{}': {e}",
                    report_path.display()
                );
            } else {
                println!("  report: {}", report_path.display());
            }
        }
        Err(e) => eprintln!("warn: could not serialize report: {e}"),
    }

    if report.final_score >= args.target_score {
        ExitCode::Ok.as_i32()
    } else {
        ExitCode::Failed.as_i32()
    }
}

// ── Optimization loop ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn optimize_loop(
    fixtures: Vec<FixtureFile>,
    _fixture_paths: Vec<String>,
    baseline_prompt: String,
    prompt_path: PathBuf,
    model: &str,
    judge_model: &str,
    args: &OptimizeArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> OptimizeReport {
    let skill = fixtures[0].skill_or_agent.clone();
    let mut current_prompt = baseline_prompt.clone();
    let mut total_cost: f64 = 0.0;
    let mut score_trajectory: Vec<f32> = Vec::new();
    let mut iteration_records: Vec<IterationRecord> = Vec::new();

    // ── Baseline eval ─────────────────────────────────────────────────────────
    if !globals.quiet {
        println!(
            "  {} baseline eval ({} cases)…",
            style("⟳").dim(),
            fixtures.iter().map(|f| f.cases.len()).sum::<usize>()
        );
    }
    let baseline_run = eval_with_prompt(
        fixtures.clone(),
        &current_prompt,
        model,
        judge_model,
        config,
        globals,
    )
    .await;
    let _ = persist_run(&baseline_run);
    let baseline_score = baseline_run.summary.pass_rate;
    total_cost += baseline_run.summary.total_cost_usd.unwrap_or(0.0);
    score_trajectory.push(baseline_score);

    if !globals.quiet {
        let failures = count_failures(&baseline_run);
        println!(
            "  Baseline: {:.0}% pass rate ({}/{} passing, {} failing)",
            baseline_score * 100.0,
            baseline_run.summary.passed,
            baseline_run.summary.total,
            failures,
        );
    }

    if baseline_score >= args.target_score {
        if !globals.quiet {
            println!(
                "  {} already at target — no optimization needed",
                style("✓").green()
            );
        }
        return OptimizeReport {
            skill,
            baseline_score,
            final_score: baseline_score,
            target_score: args.target_score,
            target_reached: true,
            iterations_run: 0,
            score_trajectory,
            total_cost_usd: total_cost,
            baseline_prompt,
            final_prompt: current_prompt,
            iterations: iteration_records,
        };
    }

    // ── Iteration loop ────────────────────────────────────────────────────────
    let mut current_score = baseline_score;

    for iter in 0..args.max_iter {
        if current_score >= args.target_score {
            break;
        }
        if total_cost >= args.budget {
            if !globals.quiet {
                eprintln!(
                    "  {} budget ${:.2} exhausted (spent ${:.4})",
                    style("warn:").yellow(),
                    args.budget,
                    total_cost
                );
            }
            break;
        }

        if !globals.quiet {
            println!(
                "\n  {} Iteration {}/{} (current score: {:.0}%)",
                style("→").bold(),
                iter + 1,
                args.max_iter,
                current_score * 100.0
            );
        }

        // 1. Collect failures from the current run.
        let current_run = if iter == 0 {
            &baseline_run
        } else {
            // Re-eval was done at end of previous iteration; use that.
            // We re-eval below so we always have fresh results.
            &baseline_run // placeholder — overwritten in logic below
        };
        let failing_cases = collect_failures(current_run);
        if failing_cases.is_empty() {
            break;
        }

        // 2. Cluster failures by rubric dimension.
        let top_rubric = top_failure_dimension(&failing_cases);
        if !globals.quiet {
            let msg = top_rubric.as_deref().unwrap_or("(no rubric data)");
            println!(
                "    Failures: {}  top dimension: {}",
                failing_cases.len(),
                msg
            );
        }

        // 3. Analyze failures with the judge model.
        let failure_summary = format_failure_summary(&failing_cases, &current_prompt);
        let feedback =
            match analyze_failures_llm(&failure_summary, &current_prompt, judge_model).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("  warn: analysis LLM call failed: {e} — skipping iteration");
                    continue;
                }
            };
        if !globals.quiet {
            let first_line = feedback.lines().next().unwrap_or("(no feedback)");
            println!(
                "    Analysis: {}",
                first_line.chars().take(120).collect::<String>()
            );
        }

        // 4. Synthesize 3 prompt candidates.
        let candidates = match synthesize_candidates_llm(&current_prompt, &feedback, model).await {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => {
                eprintln!("  warn: synthesis returned no candidates — skipping iteration");
                continue;
            }
            Err(e) => {
                eprintln!("  warn: synthesis LLM call failed: {e} — skipping iteration");
                continue;
            }
        };

        // 5. Score each candidate.
        if !globals.quiet {
            println!("    Scoring {} candidates…", candidates.len());
        }
        let mut candidate_records: Vec<CandidateRecord> = Vec::new();
        let mut best_score = current_score;
        let mut best_idx: Option<usize> = None;
        let mut best_candidate_prompt: Option<String> = None;

        for (i, candidate) in candidates.iter().enumerate() {
            let run = eval_with_prompt(
                fixtures.clone(),
                candidate,
                model,
                judge_model,
                config,
                globals,
            )
            .await;
            let _ = persist_run(&run);
            let score = run.summary.pass_rate;
            total_cost += run.summary.total_cost_usd.unwrap_or(0.0);
            let delta = score - current_score;

            if !globals.quiet {
                let arrow = if score > current_score {
                    style(format!("(+{:.0}%)", delta * 100.0))
                        .green()
                        .to_string()
                } else if delta < 0.0 {
                    style(format!("({:.0}%)", delta * 100.0)).red().to_string()
                } else {
                    style("(±0%)".to_string()).dim().to_string()
                };
                println!(
                    "      candidate {}: {:.0}%  {}",
                    i + 1,
                    score * 100.0,
                    arrow
                );
            }

            candidate_records.push(CandidateRecord {
                index: i + 1,
                score,
                delta,
            });

            if score > best_score {
                best_score = score;
                best_idx = Some(i + 1);
                best_candidate_prompt = Some(candidate.clone());
            }
        }

        // 6. Apply best if it improves on current.
        let score_after;
        let applied;
        if let Some(new_prompt) = best_candidate_prompt {
            current_prompt = new_prompt;
            if let Err(e) = std::fs::write(&prompt_path, &current_prompt) {
                eprintln!("  warn: could not write prompt.md: {e}");
            }
            // Re-eval to get official score with the applied prompt.
            let reeval = eval_with_prompt(
                fixtures.clone(),
                &current_prompt,
                model,
                judge_model,
                config,
                globals,
            )
            .await;
            let _ = persist_run(&reeval);
            total_cost += reeval.summary.total_cost_usd.unwrap_or(0.0);
            score_after = reeval.summary.pass_rate;
            applied = best_idx;
            current_score = score_after;
            if !globals.quiet {
                println!(
                    "    {} Applied candidate {} ({:.0}% → {:.0}%)",
                    style("✓").green(),
                    best_idx.unwrap_or(0),
                    (score_after - (best_score - current_score + current_score)) * 100.0,
                    score_after * 100.0
                );
            }
        } else {
            score_after = current_score;
            applied = None;
            if !globals.quiet {
                println!(
                    "    {} No candidate improved on current score — holding",
                    style("→").dim()
                );
            }
        }

        score_trajectory.push(score_after);
        let iter_cost = total_cost - iteration_records.iter().map(|r| r.cost_usd).sum::<f64>();
        iteration_records.push(IterationRecord {
            iteration: iter + 1,
            score_before: current_score - (score_after - current_score),
            failure_count: failing_cases.len(),
            top_rubric_failure: top_rubric,
            candidates: candidate_records,
            applied,
            score_after,
            cost_usd: iter_cost,
        });
    }

    let final_score = *score_trajectory.last().unwrap_or(&baseline_score);

    OptimizeReport {
        skill,
        baseline_score,
        final_score,
        target_score: args.target_score,
        target_reached: final_score >= args.target_score,
        iterations_run: iteration_records.len() as u32,
        score_trajectory,
        total_cost_usd: total_cost,
        baseline_prompt,
        final_prompt: current_prompt,
        iterations: iteration_records,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Run eval with a specific system prompt injected into every case.
async fn eval_with_prompt(
    fixtures: Vec<FixtureFile>,
    system_prompt: &str,
    model: &str,
    judge_model: &str,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> agentcarousel_core::Run {
    let injected = inject_system_prompt(fixtures, system_prompt);
    let runner = RunnerConfig {
        concurrency: config.runner.concurrency.unwrap_or(4),
        timeout_secs: config.runner.timeout_secs,
        offline: false,
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode: GenerationMode::Live,
        generator_model: Some(model.to_string()),
        generator_max_tokens: config.generator.max_tokens,
        generator_endpoint: None,
        fail_fast: false,
        mock_strict: false,
        command: "optimize-eval".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: "optimize".to_string(),
        run_id: None,
        batch_collect_id: None,
    };
    let eval_config = EvalConfig {
        runner,
        runs: 1,
        seed: 0,
        evaluator: config.eval.default_evaluator.clone(),
        judge: true,
        judge_model: Some(judge_model.to_string()),
        effectiveness_threshold: config.eval.effectiveness_threshold,
        judge_max_tokens: config.judge.max_tokens,
        progress: !globals.quiet,
    };
    let mut run = run_eval(injected, eval_config).await;
    annotate_run_cost(&mut run, model, Some(judge_model));
    run
}

/// Inject `system_prompt` as an explicit system message into every case.
/// Cases that already have an inline system message have it replaced.
fn inject_system_prompt(fixtures: Vec<FixtureFile>, system_prompt: &str) -> Vec<FixtureFile> {
    fixtures
        .into_iter()
        .map(|mut fixture| {
            fixture.cases = fixture
                .cases
                .into_iter()
                .map(|mut case| {
                    // Remove any existing system message.
                    case.input.messages.retain(|m| m.role != Role::System);
                    // Prepend the new system message.
                    case.input.messages.insert(
                        0,
                        Message {
                            role: Role::System,
                            content: system_prompt.to_string(),
                        },
                    );
                    case
                })
                .collect();
            fixture
        })
        .collect()
}

fn count_failures(run: &agentcarousel_core::Run) -> usize {
    run.cases
        .iter()
        .filter(|c| c.status != CaseStatus::Passed)
        .count()
}

fn collect_failures(run: &agentcarousel_core::Run) -> Vec<&CaseResult> {
    run.cases
        .iter()
        .filter(|c| c.status != CaseStatus::Passed)
        .collect()
}

/// Find the rubric dimension with the most failures.
fn top_failure_dimension(failures: &[&CaseResult]) -> Option<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for case in failures {
        if let Some(scores) = &case.eval_scores {
            for rs in &scores.rubric_scores {
                if rs.score < 0.5 {
                    *counts.entry(rs.rubric_id.clone()).or_insert(0) += 1;
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(dim, _)| dim)
}

/// Build a concise failure summary for the analysis prompt.
fn format_failure_summary(failures: &[&CaseResult], current_prompt: &str) -> String {
    let mut lines = vec![
        format!("Current system prompt:\n{}\n", current_prompt),
        format!("{} failing cases:\n", failures.len()),
    ];
    for (i, case) in failures.iter().take(10).enumerate() {
        lines.push(format!("  Case {}: {}", i + 1, case.case_id.0));
        if let Some(scores) = &case.eval_scores {
            let failing_rubrics: Vec<_> = scores
                .rubric_scores
                .iter()
                .filter(|rs| rs.score < 0.5)
                .map(|rs| {
                    let rationale = rs.rationale.as_deref().unwrap_or("no rationale");
                    format!(
                        "    - {}: score={:.2} — {}",
                        rs.rubric_id, rs.score, rationale
                    )
                })
                .collect();
            if !failing_rubrics.is_empty() {
                lines.extend(failing_rubrics);
            }
        }
        if let Some(err) = &case.error {
            lines.push(format!(
                "    - error: {}",
                err.chars().take(200).collect::<String>()
            ));
        }
        if let Some(output) = &case.trace.final_output {
            lines.push(format!(
                "    - output: {}",
                output.chars().take(300).collect::<String>()
            ));
        }
    }
    if failures.len() > 10 {
        lines.push(format!("  … and {} more", failures.len() - 10));
    }
    lines.join("\n")
}

/// Call the judge model to analyze failures and return actionable feedback.
async fn analyze_failures_llm(
    failure_summary: &str,
    _current_prompt: &str,
    judge_model: &str,
) -> Result<String, String> {
    let prompt = format!(
        r#"You are an expert prompt engineer. Analyze these AI agent test failures and identify the root cause.

{failure_summary}

Respond concisely with:
1. The primary failure pattern (one sentence)
2. The specific prompt weakness causing it (one sentence)
3. The single most impactful change to make (one sentence, concrete and actionable)

Keep your response under 200 words."#
    );
    let result = call_llm(judge_model, &prompt, Some(512)).await?;
    Ok(result.output)
}

/// Call the model to synthesize 3 improved prompt candidates.
async fn synthesize_candidates_llm(
    current_prompt: &str,
    feedback: &str,
    model: &str,
) -> Result<Vec<String>, String> {
    let prompt = format!(
        r#"You are an expert prompt engineer. Given this system prompt and failure analysis, generate 3 improved versions.

CURRENT SYSTEM PROMPT:
{current_prompt}

FAILURE ANALYSIS:
{feedback}

Generate exactly 3 distinct improved system prompt variants that address the identified failures. Each should be a complete, standalone system prompt.

Respond with valid JSON only (no markdown, no explanation):
{{"variants": ["<complete prompt 1>", "<complete prompt 2>", "<complete prompt 3>"]}}"#
    );
    let result = call_llm(model, &prompt, Some(2048)).await?;

    // Parse the JSON response.
    let text = result.output.trim();
    // Strip markdown code fences if present.
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    let parsed: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|e| format!("JSON parse error: {e} — raw: {json_text:.200}"))?;

    let variants = parsed["variants"]
        .as_array()
        .ok_or("response missing 'variants' array")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    if variants.is_empty() {
        return Err("variants array was empty".to_string());
    }

    Ok(variants)
}
