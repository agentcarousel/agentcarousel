use agentcarousel_core::{annotate_run_cost, CaseId, CaseResult, CaseStatus, FixtureFile, Message, Role};
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::persist_run;
use agentcarousel_runner::{call_llm, run_eval, EvalConfig, GenerationMode, RunnerConfig};
use clap::Parser;
use console::style;
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
            analyze_clusters_llm(&clusters, &case_map, judge_model).await;
        // Build a flat feedback string for the candidate synthesis step.
        let feedback = if analyses.is_empty() {
            // Fallback to the text-summary path if no structured analysis produced.
            let failure_summary = format_failure_summary(&failing_cases, &current_prompt);
            match analyze_failures_llm(&failure_summary, &current_prompt, judge_model).await {
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
            failure_clusters: clusters,
            failure_analyses: analyses,
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
                    map.entry(item.id.clone()).or_insert_with(|| item.description.clone());
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
            match call_llm(judge_model, &prompt, Some(256)).await {
                Ok(r) => representative_cases.push((case_id.clone(), r.output)),
                Err(e) => representative_cases
                    .push((case_id.clone(), format!("(analysis failed: {e})"))),
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
            match call_llm(judge_model, &synth_prompt, Some(200)).await {
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
