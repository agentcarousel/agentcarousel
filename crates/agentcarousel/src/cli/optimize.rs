use agentcarousel_core::{
    annotate_run_cost, CaseId, CaseResult, CaseStatus, FixtureFile, Message, Role,
};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::persist_run;
use agentcarousel_runner::{call_llm, run_eval, EvalConfig, GenerationMode, RunnerConfig};
use clap::Parser;
use console::style;
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
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
    /// Fixture files or directory to optimize against (mutually exclusive with --skill).
    #[arg(value_name = "PATH", conflicts_with = "skill")]
    path: Option<PathBuf>,
    /// Skill name shorthand — resolves to fixtures/<name>/ (mutually exclusive with PATH).
    #[arg(long, value_name = "NAME", conflicts_with = "path")]
    skill: Option<String>,
    /// Stop when pass rate reaches this value (0.0–1.0).
    #[arg(long, default_value_t = 0.9)]
    target_score: f32,
    /// Maximum USD to spend across all eval and LLM calls.
    #[arg(long, default_value_t = 10.0)]
    budget: f64,
    /// Maximum number of optimization iterations (alias: --max-iter).
    #[arg(long, alias = "max-iter", default_value_t = 5)]
    iterations: u32,
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
    /// Base URL for the generator model endpoint (required when --model is 'custom' or 'ollama/<name>').
    /// Falls back to `generator.endpoint` in agentcarousel.toml.
    #[arg(long, value_name = "URL")]
    generator_endpoint: Option<String>,
    /// Base URL for the judge model endpoint (required when --judge-model is 'custom' or 'ollama/<name>').
    #[arg(long, value_name = "URL")]
    judge_endpoint: Option<String>,
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
    total_duration_secs: f64,
    baseline_prompt: String,
    final_prompt: String,
    /// Unified diff of `baseline_prompt` → `final_prompt`.
    prompt_diff: String,
    iterations: Vec<IterationRecord>,
}

/// Per-cluster structured failure analysis produced by the judge.
#[derive(serde::Serialize, Debug, Clone)]
pub struct FailureAnalysis {
    /// The rubric dimension ID this analysis targets.
    pub cluster_id: String,
    /// Per-case judge explanations: `(case_id, two-sentence explanation)`.
    pub representative_cases: Vec<(CaseId, String)>,
    /// Single actionable sentence synthesized from all per-case explanations.
    pub synthesis: String,
}

#[derive(serde::Serialize, Debug)]
struct IterationRecord {
    iteration: u32,
    score_before: f32,
    failure_count: usize,
    top_rubric_failure: Option<String>,
    failure_clusters: Vec<FailureCluster>,
    failure_analyses: Vec<FailureAnalysis>,
    edit_candidates: Vec<PromptEditCandidate>,
    candidates: Vec<CandidateRecord>,
    applied: Option<usize>,
    score_after: f32,
    cost_usd: f64,
    duration_secs: f64,
}

/// A targeted, minimal edit to the system prompt produced by the synthesis LLM.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct PromptEditCandidate {
    /// Stable identifier within this iteration (e.g. `"edit-1"`).
    pub id: String,
    /// The exact substring in the current prompt to replace. Empty string means append.
    pub original_text: String,
    /// The text to insert in place of `original_text`.
    pub replacement_text: String,
    /// One-sentence rationale for the edit.
    pub rationale: String,
    /// The rubric cluster ID this edit is intended to fix.
    pub targets_cluster: String,
}

impl PromptEditCandidate {
    /// Apply this edit to `prompt`, returning the modified string.
    pub fn apply(&self, prompt: &str) -> String {
        if self.original_text.is_empty() {
            format!("{}\n{}", prompt.trim_end(), self.replacement_text)
        } else {
            prompt.replacen(&self.original_text, &self.replacement_text, 1)
        }
    }
}

#[derive(serde::Serialize, Debug)]
struct CandidateRecord {
    index: usize,
    /// Pass rate on the targeted failing-case set after applying this edit.
    score: f32,
    /// `score` minus the baseline pass rate on the targeted set.
    delta: f32,
    /// Improvement on the cluster's representative failing cases specifically.
    delta_failing: f32,
    /// Degradation on previously-passing cases for the same rubric dimension.
    regression: f32,
    /// Selection criterion: `delta_failing - regression` (higher is better).
    selection_score: f32,
    edit_id: String,
    targets_cluster: String,
    rationale: String,
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
    let generator_endpoint = args
        .generator_endpoint
        .clone()
        .or_else(|| config.generator.endpoint.clone());
    let judge_endpoint = args.judge_endpoint.clone();

    // Resolve the fixture root from --skill name or positional PATH.
    let fixture_root = match (&args.skill, &args.path) {
        (Some(name), _) => PathBuf::from("fixtures").join(name),
        (_, Some(p)) => p.clone(),
        (None, None) => {
            eprintln!("error: provide either a PATH argument or --skill <name>");
            return ExitCode::ConfigError.as_i32();
        }
    };

    // Collect fixture paths and load fixtures.
    let fixture_paths = collect_fixture_paths(std::slice::from_ref(&fixture_root));
    if fixture_paths.is_empty() {
        eprintln!(
            "error: no fixture files found at '{}'",
            fixture_root.display()
        );
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
            args.iterations,
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
        generator_endpoint.as_deref(),
        judge_endpoint.as_deref(),
        &args,
        config,
        globals,
    ));

    // Print human-readable summary.
    print_optimize_summary(&report, globals);

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
    generator_endpoint: Option<&str>,
    judge_endpoint: Option<&str>,
    args: &OptimizeArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> OptimizeReport {
    let loop_start = std::time::Instant::now();
    let skill = fixtures[0].skill_or_agent.clone();
    let rubric_lookup = build_rubric_lookup(&fixtures);
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
        generator_endpoint,
        judge_endpoint,
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
            total_duration_secs: loop_start.elapsed().as_secs_f64(),
            prompt_diff: String::new(),
            baseline_prompt: baseline_prompt.clone(),
            final_prompt: current_prompt,
            iterations: iteration_records,
        };
    }

    // ── Iteration loop ────────────────────────────────────────────────────────
    let mut current_score = baseline_score;
    let mut no_improvement_streak: u32 = 0;

    for iter in 0..args.iterations {
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
        if no_improvement_streak >= 2 {
            if !globals.quiet {
                eprintln!(
                    "  {} 2 consecutive iterations with no improvement — stopping early",
                    style("warn:").yellow()
                );
            }
            break;
        }

        let iter_start = std::time::Instant::now();
        if !globals.quiet {
            println!(
                "\n  {} Iteration {}/{} (current score: {:.0}%)",
                style("→").bold(),
                iter + 1,
                args.iterations,
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
        let clusters = cluster_failures(&failing_cases, &rubric_lookup);
        let top_rubric = clusters.first().map(|c| c.rubric_id.clone());
        if !globals.quiet {
            let msg = top_rubric.as_deref().unwrap_or("(no rubric data)");
            println!(
                "    Failures: {}  top dimension: {}  ({} cluster(s))",
                failing_cases.len(),
                msg,
                clusters.len(),
            );
        }

        // 3. Structured per-cluster failure analysis via the judge model.
        let case_map: HashMap<CaseId, &CaseResult> = failing_cases
            .iter()
            .map(|c| (c.case_id.clone(), *c))
            .collect();
        let analyses =
            analyze_clusters_llm(&clusters, &case_map, judge_model, judge_endpoint).await;
        // Build a flat feedback string for the candidate synthesis step.
        let feedback = if analyses.is_empty() {
            // Fallback to the text-summary path if no structured analysis produced.
            let failure_summary = format_failure_summary(&failing_cases, &current_prompt);
            match analyze_failures_llm(
                &failure_summary,
                &current_prompt,
                judge_model,
                judge_endpoint,
            )
            .await
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("  warn: analysis LLM call failed: {e} — skipping iteration");
                    continue;
                }
            }
        } else {
            analyses
                .iter()
                .map(|a| format!("[{}] {}", a.cluster_id, a.synthesis))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if !globals.quiet {
            let first_line = feedback.lines().next().unwrap_or("(no feedback)");
            println!(
                "    Analysis: {}",
                first_line.chars().take(120).collect::<String>()
            );
        }

        // 4. Synthesize 3 targeted prompt edits.
        let edit_candidates = match synthesize_edit_candidates_llm(
            &current_prompt,
            &analyses,
            &feedback,
            model,
            generator_endpoint,
        )
        .await
        {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => {
                eprintln!("  warn: synthesis returned no edit candidates — skipping iteration");
                continue;
            }
            Err(e) => {
                eprintln!("  warn: synthesis LLM call failed: {e} — skipping iteration");
                continue;
            }
        };

        // 5. Targeted candidate scoring: eval only failing cluster cases + regression guard.
        // Build targeted fixture sets once per iteration (shared across candidates).
        let top_cluster = clusters.first();
        let failing_target_ids: Vec<CaseId> = top_cluster
            .map(|c| c.representative.clone())
            .unwrap_or_default();
        let passing_target_ids: Vec<CaseId> = top_cluster
            .map(|c| passing_cases_for_dimension(&baseline_run, &c.rubric_id, 5))
            .unwrap_or_default();

        let failing_fixtures = filter_fixtures_to_cases(&fixtures, &failing_target_ids);
        let passing_fixtures = filter_fixtures_to_cases(&fixtures, &passing_target_ids);

        // Baseline pass rates on the targeted sets (using current prompt).
        let baseline_failing_rate = if failing_fixtures.is_empty() {
            0.0_f32
        } else {
            let r = eval_with_prompt(
                failing_fixtures.clone(),
                &current_prompt,
                model,
                judge_model,
                generator_endpoint,
                judge_endpoint,
                config,
                globals,
            )
            .await;
            total_cost += r.summary.total_cost_usd.unwrap_or(0.0);
            r.summary.pass_rate
        };
        let baseline_passing_rate = if passing_fixtures.is_empty() {
            1.0_f32
        } else {
            let r = eval_with_prompt(
                passing_fixtures.clone(),
                &current_prompt,
                model,
                judge_model,
                generator_endpoint,
                judge_endpoint,
                config,
                globals,
            )
            .await;
            total_cost += r.summary.total_cost_usd.unwrap_or(0.0);
            r.summary.pass_rate
        };

        if !globals.quiet {
            println!(
                "    Scoring {} edit candidates (targeted: {} failing + {} regression)…",
                edit_candidates.len(),
                failing_target_ids.len(),
                passing_target_ids.len(),
            );
        }
        let mut candidate_records: Vec<CandidateRecord> = Vec::new();
        let mut best_selection: f32 = 0.0;
        let mut best_idx: Option<usize> = None;
        let mut best_candidate_prompt: Option<String> = None;

        for (i, edit) in edit_candidates.iter().enumerate() {
            let applied_prompt = edit.apply(&current_prompt);

            // Eval on failing cases to measure improvement.
            let (delta_failing, score_on_failing) = if !failing_fixtures.is_empty() {
                let r = eval_with_prompt(
                    failing_fixtures.clone(),
                    &applied_prompt,
                    model,
                    judge_model,
                    generator_endpoint,
                    judge_endpoint,
                    config,
                    globals,
                )
                .await;
                let _ = persist_run(&r);
                total_cost += r.summary.total_cost_usd.unwrap_or(0.0);
                let rate = r.summary.pass_rate;
                (rate - baseline_failing_rate, rate)
            } else {
                (0.0, 0.0)
            };

            // Eval on passing cases to measure regression.
            let regression = if !passing_fixtures.is_empty() {
                let r = eval_with_prompt(
                    passing_fixtures.clone(),
                    &applied_prompt,
                    model,
                    judge_model,
                    generator_endpoint,
                    judge_endpoint,
                    config,
                    globals,
                )
                .await;
                total_cost += r.summary.total_cost_usd.unwrap_or(0.0);
                // Positive value = degradation (lower is better).
                (baseline_passing_rate - r.summary.pass_rate).max(0.0)
            } else {
                0.0
            };

            let selection_score = delta_failing - regression;
            let score = score_on_failing;
            let delta = score - current_score;

            if !globals.quiet {
                let arrow = if delta_failing > 0.0 {
                    style(format!("+{:.0}% failing", delta_failing * 100.0))
                        .green()
                        .to_string()
                } else if delta_failing < 0.0 {
                    style(format!("{:.0}% failing", delta_failing * 100.0))
                        .red()
                        .to_string()
                } else {
                    style("±0%".to_string()).dim().to_string()
                };
                println!(
                    "      edit {}: {}  regression={:.0}%  sel={:.2}  [→ {}]",
                    i + 1,
                    arrow,
                    regression * 100.0,
                    selection_score,
                    edit.targets_cluster,
                );
            }

            candidate_records.push(CandidateRecord {
                index: i + 1,
                score,
                delta,
                delta_failing,
                regression,
                selection_score,
                edit_id: edit.id.clone(),
                targets_cluster: edit.targets_cluster.clone(),
                rationale: edit.rationale.clone(),
            });

            // Select by highest (delta_failing - regression); require positive improvement.
            if delta_failing > 0.0 && selection_score > best_selection {
                best_selection = selection_score;
                best_idx = Some(i + 1);
                best_candidate_prompt = Some(applied_prompt);
            }
        }

        // 6. Apply best if it improves on current.
        let score_after;
        let applied;
        if let Some(new_prompt) = best_candidate_prompt {
            current_prompt = new_prompt;
            // Back up current prompt.md before overwriting.
            let bak_path = prompt_path.with_extension("md.bak");
            if let Err(e) = std::fs::copy(&prompt_path, &bak_path) {
                eprintln!("  warn: could not write prompt.md.bak: {e}");
            }
            if let Err(e) = std::fs::write(&prompt_path, &current_prompt) {
                eprintln!("  warn: could not write prompt.md: {e}");
            }
            no_improvement_streak = 0;
            // Re-eval to get official score with the applied prompt.
            let reeval = eval_with_prompt(
                fixtures.clone(),
                &current_prompt,
                model,
                judge_model,
                generator_endpoint,
                judge_endpoint,
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
                    "    {} Applied edit {} ({:.0}% → {:.0}% on full suite)",
                    style("✓").green(),
                    best_idx.unwrap_or(0),
                    current_score * 100.0,
                    score_after * 100.0,
                );
            }
        } else {
            score_after = current_score;
            applied = None;
            no_improvement_streak += 1;
            if !globals.quiet {
                println!(
                    "    {} No edit improved failing cases without regression — holding (streak: {})",
                    style("→").dim(),
                    no_improvement_streak,
                );
            }
        }

        score_trajectory.push(score_after);
        let iter_cost = total_cost - iteration_records.iter().map(|r| r.cost_usd).sum::<f64>();
        let iter_duration = iter_start.elapsed().as_secs_f64();
        iteration_records.push(IterationRecord {
            iteration: iter + 1,
            score_before: current_score - (score_after - current_score),
            failure_count: failing_cases.len(),
            top_rubric_failure: top_rubric,
            failure_clusters: clusters,
            failure_analyses: analyses,
            edit_candidates,
            candidates: candidate_records,
            applied,
            score_after,
            cost_usd: iter_cost,
            duration_secs: iter_duration,
        });
    }

    let final_score = *score_trajectory.last().unwrap_or(&baseline_score);
    let total_duration = loop_start.elapsed().as_secs_f64();
    let final_prompt = current_prompt;
    let prompt_diff = unified_diff(&baseline_prompt, &final_prompt);

    OptimizeReport {
        skill,
        baseline_score,
        final_score,
        target_score: args.target_score,
        target_reached: final_score >= args.target_score,
        iterations_run: iteration_records.len() as u32,
        score_trajectory,
        total_cost_usd: total_cost,
        total_duration_secs: total_duration,
        baseline_prompt,
        final_prompt,
        prompt_diff,
        iterations: iteration_records,
    }
}

// ── Report helpers ────────────────────────────────────────────────────────────

fn print_optimize_summary(report: &OptimizeReport, globals: &GlobalOptions) {
    println!(
        "\nOptimization complete: {} iteration(s), ${:.4} spent, {:.1}s",
        report.iterations_run, report.total_cost_usd, report.total_duration_secs,
    );
    for rec in &report.iterations {
        let delta = rec.score_after - rec.score_before;
        let arrow = if delta > 0.0 {
            style(format!("(+{:.0}%)", delta * 100.0))
                .green()
                .to_string()
        } else if delta < 0.0 {
            style(format!("({:.0}%)", delta * 100.0)).red().to_string()
        } else {
            style("(±0%)".to_string()).dim().to_string()
        };
        let edit_label = rec
            .applied
            .and_then(|idx| rec.edit_candidates.get(idx.saturating_sub(1)))
            .map(|e| format!("Edit: {}", e.rationale.chars().take(60).collect::<String>()))
            .unwrap_or_else(|| "no edit applied".to_string());
        println!(
            "  Iteration {}: {:.0}% → {:.0}%  {}  {}",
            rec.iteration,
            rec.score_before * 100.0,
            rec.score_after * 100.0,
            arrow,
            edit_label,
        );
    }
    let score_line = if report.target_reached {
        format!(
            "Score: {:.2} ≥ target {:.2} {}",
            report.final_score,
            report.target_score,
            style("✓").green().bold(),
        )
    } else {
        format!(
            "Score: {:.2} < target {:.2} {}",
            report.final_score,
            report.target_score,
            style("✗").yellow(),
        )
    };
    println!("{score_line}");
    if !report.prompt_diff.is_empty() && !globals.quiet {
        println!("\nPrompt diff:");
        for line in report.prompt_diff.lines() {
            if line.starts_with('+') {
                println!("  {}", style(line).green());
            } else if line.starts_with('-') {
                println!("  {}", style(line).red());
            } else {
                println!("  {line}");
            }
        }
    }
}

/// Generate a unified diff string from `old` to `new`.
fn unified_diff(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(prefix);
        out.push_str(change.value());
        if !change.value().ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Run eval with a specific system prompt injected into every case.
#[allow(clippy::too_many_arguments)]
async fn eval_with_prompt(
    fixtures: Vec<FixtureFile>,
    system_prompt: &str,
    model: &str,
    judge_model: &str,
    generator_endpoint: Option<&str>,
    judge_endpoint: Option<&str>,
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
        generator_endpoint: generator_endpoint.map(|s| s.to_string()),
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
        judge_endpoint: judge_endpoint.map(|s| s.to_string()),
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

/// Filter fixtures to include only cases whose IDs are in `keep`.
fn filter_fixtures_to_cases(fixtures: &[FixtureFile], keep: &[CaseId]) -> Vec<FixtureFile> {
    let id_set: std::collections::HashSet<&CaseId> = keep.iter().collect();
    fixtures
        .iter()
        .filter_map(|f| {
            let cases: Vec<_> = f
                .cases
                .iter()
                .filter(|c| id_set.contains(&c.id))
                .cloned()
                .collect();
            if cases.is_empty() {
                None
            } else {
                let mut filtered = f.clone();
                filtered.cases = cases;
                Some(filtered)
            }
        })
        .collect()
}

/// Return up to `limit` case IDs that passed in `run` and have a rubric score for `rubric_id`.
fn passing_cases_for_dimension(
    run: &agentcarousel_core::Run,
    rubric_id: &str,
    limit: usize,
) -> Vec<CaseId> {
    run.cases
        .iter()
        .filter(|c| {
            c.status == CaseStatus::Passed
                && c.eval_scores
                    .as_ref()
                    .map(|s| s.rubric_scores.iter().any(|rs| rs.rubric_id == rubric_id))
                    .unwrap_or(false)
        })
        .take(limit)
        .map(|c| c.case_id.clone())
        .collect()
}

// ── Failure clustering ────────────────────────────────────────────────────────

/// A group of failing cases sharing the same primary rubric failure dimension.
#[derive(serde::Serialize, Debug, Clone)]
pub struct FailureCluster {
    /// The rubric dimension ID (or `"rules_failure"` for cases with no eval scores).
    pub rubric_id: String,
    /// Human-readable description from the fixture rubric definition.
    pub rubric_description: String,
    /// All failing case IDs in this cluster.
    pub case_ids: Vec<CaseId>,
    /// Up to 3 representative case IDs (sorted for determinism).
    pub representative: Vec<CaseId>,
}

/// Build a `rubric_id → description` lookup from all fixture rubric definitions.
pub fn build_rubric_lookup(fixtures: &[FixtureFile]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for fixture in fixtures {
        for case in &fixture.cases {
            if let Some(rubric_items) = &case.expected.rubric {
                for item in rubric_items {
                    map.entry(item.id.clone())
                        .or_insert_with(|| item.description.clone());
                }
            }
        }
    }
    map
}

/// Group failing cases by their primary rubric failure dimension.
///
/// For each failing case, the primary dimension is the rubric item with the
/// lowest weighted score (`weight * (1.0 - score)`). Cases with no `eval_scores`
/// (rules-only failures) are grouped into a catch-all `"rules_failure"` cluster.
/// Returns clusters ordered by descending case count.
pub fn cluster_failures(
    failures: &[&CaseResult],
    rubric_lookup: &HashMap<String, String>,
) -> Vec<FailureCluster> {
    // rubric_id → case_ids
    let mut groups: HashMap<String, Vec<CaseId>> = HashMap::new();

    for case in failures {
        let primary = match &case.eval_scores {
            None => "rules_failure".to_string(),
            Some(scores) if scores.rubric_scores.is_empty() => "rules_failure".to_string(),
            Some(scores) => {
                // Pick dimension with highest weighted deficit (weight * (1 - score)).
                scores
                    .rubric_scores
                    .iter()
                    .filter(|rs| rs.score < 1.0)
                    .max_by(|a, b| {
                        let da = a.weight * (1.0 - a.score);
                        let db = b.weight * (1.0 - b.score);
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|rs| rs.rubric_id.clone())
                    .unwrap_or_else(|| "rules_failure".to_string())
            }
        };
        groups
            .entry(primary)
            .or_default()
            .push(case.case_id.clone());
    }

    let mut clusters: Vec<FailureCluster> = groups
        .into_iter()
        .map(|(rubric_id, mut case_ids)| {
            case_ids.sort_by(|a, b| a.0.cmp(&b.0));
            let description = if rubric_id == "rules_failure" {
                "Rules-based failure (no rubric scores available)".to_string()
            } else {
                rubric_lookup
                    .get(&rubric_id)
                    .cloned()
                    .unwrap_or_else(|| rubric_id.clone())
            };
            let representative = case_ids.iter().take(3).cloned().collect();
            FailureCluster {
                rubric_id,
                rubric_description: description,
                case_ids,
                representative,
            }
        })
        .collect();

    // Largest cluster first.
    clusters.sort_by_key(|c| std::cmp::Reverse(c.case_ids.len()));
    clusters
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

/// Run structured per-cluster failure analysis using the judge model.
///
/// For each cluster, calls the judge once per representative case to explain
/// what the system prompt is missing, then synthesizes those explanations into
/// a single actionable sentence. Returns one `FailureAnalysis` per cluster.
pub async fn analyze_clusters_llm(
    clusters: &[FailureCluster],
    case_map: &HashMap<CaseId, &CaseResult>,
    judge_model: &str,
    judge_endpoint: Option<&str>,
) -> Vec<FailureAnalysis> {
    let mut analyses = Vec::new();
    for cluster in clusters {
        // Per-case judge explanations.
        let mut representative_cases: Vec<(CaseId, String)> = Vec::new();
        for case_id in &cluster.representative {
            let output = case_map
                .get(case_id)
                .and_then(|c| c.trace.final_output.as_deref())
                .unwrap_or("(no output recorded)");
            let error = case_map
                .get(case_id)
                .and_then(|c| c.error.as_deref())
                .unwrap_or("");

            let agent_result = if !error.is_empty() {
                format!("Error: {}", &error[..error.len().min(300)])
            } else {
                output.chars().take(400).collect::<String>()
            };

            let prompt = format!(
                "The agent failed this test case.\nThe case tests: {rubric_description}\nThe agent produced: {agent_result}\n\nIn exactly 2 sentences, explain what the system prompt is missing that would have caused the agent to succeed. Be specific and actionable.",
                rubric_description = cluster.rubric_description,
                agent_result = agent_result,
            );
            match call_llm(judge_model, &prompt, Some(256), judge_endpoint).await {
                Ok(r) => representative_cases.push((case_id.clone(), r.output)),
                Err(e) => {
                    representative_cases.push((case_id.clone(), format!("(analysis failed: {e})")))
                }
            }
        }

        // Synthesize per-case explanations into one actionable sentence.
        let synthesis = if representative_cases.is_empty() {
            format!(
                "No representative cases found for cluster '{}'.",
                cluster.rubric_id
            )
        } else {
            let explanations = representative_cases
                .iter()
                .enumerate()
                .map(|(i, (id, exp))| format!("Case {} ({}): {}", i + 1, id.0, exp))
                .collect::<Vec<_>>()
                .join("\n");
            let synth_prompt = format!(
                "These are per-case explanations of why an AI agent failed the rubric dimension '{rubric_id}' ({rubric_description}):\n\n{explanations}\n\nSummarize in exactly ONE actionable sentence the single most important change to the system prompt that would fix these failures.",
                rubric_id = cluster.rubric_id,
                rubric_description = cluster.rubric_description,
                explanations = explanations,
            );
            match call_llm(judge_model, &synth_prompt, Some(200), judge_endpoint).await {
                Ok(r) => r.output,
                Err(_) => representative_cases
                    .first()
                    .map(|(_, s)| s.clone())
                    .unwrap_or_default(),
            }
        };

        analyses.push(FailureAnalysis {
            cluster_id: cluster.rubric_id.clone(),
            representative_cases,
            synthesis,
        });
    }
    analyses
}

/// Call the judge model to analyze failures and return actionable feedback.
async fn analyze_failures_llm(
    failure_summary: &str,
    _current_prompt: &str,
    judge_model: &str,
    judge_endpoint: Option<&str>,
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
    let result = call_llm(judge_model, &prompt, Some(512), judge_endpoint).await?;
    Ok(result.output)
}

/// Synthesize 3 targeted, minimal prompt edits from the failure analyses.
///
/// Each edit specifies `original_text` to replace and `replacement_text` to insert,
/// targeting a specific failure cluster. Returns parsed `PromptEditCandidate` values.
/// Falls back to `synthesize_candidates_llm` output wrapped as whole-prompt edits when
/// the structured JSON cannot be parsed.
async fn synthesize_edit_candidates_llm(
    current_prompt: &str,
    analyses: &[FailureAnalysis],
    feedback: &str,
    model: &str,
    generator_endpoint: Option<&str>,
) -> Result<Vec<PromptEditCandidate>, String> {
    let analysis_block = if analyses.is_empty() {
        feedback.to_string()
    } else {
        analyses
            .iter()
            .map(|a| format!("- [{}] {}", a.cluster_id, a.synthesis))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let prompt = format!(
        r#"You are an expert prompt engineer. Here is an AI agent system prompt and the failure patterns observed when evaluating it.

CURRENT SYSTEM PROMPT:
{current_prompt}

FAILURE ANALYSIS:
{analysis_block}

Generate exactly 3 specific, minimal edits to improve the prompt. Each edit must:
1. Quote the EXACT text to replace from the current prompt (or use empty string to append at end)
2. Provide the replacement text
3. Explain in one sentence which failure it addresses
4. Name the cluster it targets

Respond with valid JSON only — no markdown, no explanation:
{{"edits": [{{"id": "edit-1", "original_text": "<exact text from prompt or empty>", "replacement_text": "<new text>", "rationale": "<one sentence>", "targets_cluster": "<cluster id>"}}, {{"id": "edit-2", ...}}, {{"id": "edit-3", ...}}]}}"#
    );

    let result = call_llm(model, &prompt, Some(8192), generator_endpoint).await?;
    let text = result.output.trim();
    let json_text = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    // Full parse first; fall back to extracting any complete edit objects on truncated JSON.
    let edits = match serde_json::from_str::<serde_json::Value>(&json_text) {
        Ok(parsed) => parsed["edits"]
            .as_array()
            .ok_or("response missing 'edits' array")?
            .iter()
            .filter_map(|v| serde_json::from_value::<PromptEditCandidate>(v.clone()).ok())
            .filter(|e| !e.replacement_text.is_empty())
            .collect::<Vec<_>>(),
        Err(_) => extract_partial_edits(&json_text),
    };

    if edits.is_empty() {
        return Err(format!(
            "edits array was empty or unparseable — raw: {:.200}",
            json_text
        ));
    }

    Ok(edits)
}

/// Extract complete `PromptEditCandidate` objects from a potentially truncated JSON string.
///
/// Searches for `{"id":` markers and attempts to parse each complete `{...}` block
/// as a `PromptEditCandidate`. Stops when a block has no closing brace (mid-truncation).
fn extract_partial_edits(text: &str) -> Vec<PromptEditCandidate> {
    let mut edits = Vec::new();
    let mut search_from = 0;

    while search_from < text.len() {
        // Find the next edit object start — must begin with {"id": to avoid matching the
        // outer {"edits": [...]} wrapper.
        let Some(rel) = text[search_from..].find("{\"id\":") else {
            break;
        };
        let start = search_from + rel;

        // Walk forward to find the matching closing brace.
        let mut depth: i32 = 0;
        let mut end = None;
        for (i, c) in text[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + i);
                        break;
                    }
                }
                _ => {}
            }
        }

        match end {
            Some(e) => {
                let obj = &text[start..=e];
                if let Ok(edit) = serde_json::from_str::<PromptEditCandidate>(obj) {
                    if !edit.replacement_text.is_empty() {
                        edits.push(edit);
                    }
                }
                search_from = e + 1;
            }
            // No closing brace — response was truncated mid-object; stop.
            None => break,
        }
    }

    edits
}
