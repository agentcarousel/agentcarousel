//! Skill lifecycle pipeline: `agc pipeline onboard` and `agc pipeline improve`.
//!
//! These commands chain the existing agc CLI commands into a complete skill
//! improvement workflow.  Each step calls the existing `run_*` functions directly
//! rather than spawning subprocesses.

use chrono::Utc;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use super::candidate_store::{
    candidates_path, find_workspace_root, CandidateEntry, CandidateStatus, CandidateStore,
    MetricSnapshot,
};
use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::local_config::LocalProfile;
use super::GlobalOptions;

// ── CLI types ─────────────────────────────────────────────────────────────────

/// Skill lifecycle pipeline commands.
#[derive(Debug, Parser)]
#[command(about = "Run the skill lifecycle pipeline (onboard or improve a skill).")]
pub struct PipelineArgs {
    #[command(subcommand)]
    pub command: PipelineCommand,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum PipelineCommand {
    /// Onboard a new skill: generate → validate → test → eval → tag baseline.
    #[command(
        after_help = "Examples:\n  agc pipeline onboard customer-support\n  agc pipeline onboard customer-support --target-model ollama/gemma4 --target-endpoint http://localhost:11434/api/generate\n  agc pipeline onboard customer-support --dry-run"
    )]
    Onboard(OnboardArgs),
    /// Improve a skill: iterative eval → optimize → A/B gate loop.
    #[command(
        after_help = "Examples:\n  agc pipeline improve customer-support\n  agc pipeline improve customer-support --target-score 0.90 --max-rounds 5"
    )]
    Improve(ImproveArgs),
}

/// Arguments for `agc pipeline onboard`.
#[derive(Debug, Parser)]
pub struct OnboardArgs {
    /// Skill name (resolves to fixtures/<skill>/ for fixture paths).
    pub skill: String,
    /// Generator model to target (overrides local profile and config).
    #[arg(long)]
    pub target_model: Option<String>,
    /// Generator endpoint URL for Ollama or custom models.
    #[arg(long)]
    pub target_endpoint: Option<String>,
    /// Judge model (overrides config).
    #[arg(long)]
    pub judge_model: Option<String>,
    /// Judge endpoint URL for Ollama or custom judge models.
    #[arg(long)]
    pub judge_endpoint: Option<String>,
    /// Target pass rate for the skill (0.0–1.0; default: 0.85).
    #[arg(long, default_value_t = 0.85_f32)]
    pub target_score: f32,
    /// Print what would happen without calling any APIs.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments for `agc pipeline improve`.
#[derive(Debug, Parser)]
pub struct ImproveArgs {
    /// Skill name (resolves to fixtures/<skill>/).
    pub skill: String,
    /// Target pass rate (overrides local profile; default: 0.85).
    #[arg(long)]
    pub target_score: Option<f32>,
    /// Maximum improvement rounds (overrides local profile; default: 5).
    #[arg(long)]
    pub max_rounds: Option<u32>,
    /// Generator endpoint URL for Ollama or custom models.
    #[arg(long)]
    pub target_endpoint: Option<String>,
    /// Judge endpoint URL for Ollama or custom judge models.
    #[arg(long)]
    pub judge_endpoint: Option<String>,
    /// Budget in USD (0.0 = unlimited, useful for Ollama).
    #[arg(long, default_value_t = 0.0_f64)]
    pub budget: f64,
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

pub fn run_pipeline(args: PipelineArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    match args.command {
        PipelineCommand::Onboard(a) => run_onboard(a, config, globals),
        PipelineCommand::Improve(a) => run_improve(a, config, globals),
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

fn fixture_path(skill: &str) -> PathBuf {
    PathBuf::from("fixtures").join(skill)
}

fn prompt_path(skill: &str) -> PathBuf {
    fixture_path(skill).join("prompt.md")
}

fn prompt_bak_path(skill: &str) -> PathBuf {
    fixture_path(skill).join("prompt.md.bak")
}

/// Load the candidate store from the nearest workspace root.
fn load_store() -> (CandidateStore, PathBuf) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let root = find_workspace_root(&cwd).unwrap_or(cwd);
    let path = candidates_path(&root);
    let store = CandidateStore::load(&path);
    (store, path)
}

fn save_store(store: &CandidateStore, path: &Path) {
    if let Err(e) = store.save(path) {
        eprintln!("warning: could not save candidates.json: {e}");
    }
}

/// Build a `MetricSnapshot` from the current run history for `skill`.
fn snapshot_metrics(skill: &str) -> MetricSnapshot {
    use super::metrics::compute_metrics_for_export;
    let (_, results, _) = compute_metrics_for_export(Some(skill), 20, None);
    let mut snap = MetricSnapshot::default();
    for r in &results {
        let score_01 = r.score_0_to_100.map(|s| (s / 100.0) as f32);
        match r.id {
            "injection_resistance" => snap.injection_resistance = score_01,
            "drift_index" => snap.behavioral_drift = score_01,
            "behavioral_coverage" => snap.coverage = score_01,
            "confidence_calibration" => snap.calibration = score_01,
            _ => {}
        }
    }
    snap
}

/// Run `agc eval` against a skill's fixtures (live + judge) and return `(run_id, pass_rate)`.
/// Returns `Err(exit_code)` if eval fails.
///
/// # TODO: refactor run_eval_command to return Result<Run, ExitCode>
fn run_eval_and_get_score(
    fixture_dir: &Path,
    target_model: &str,
    generator_endpoint: Option<&str>,
    judge_model: &str,
    judge_endpoint: Option<&str>,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> Result<(String, f32), i32> {
    use super::eval::{run_eval_command, EvalArgs};
    use clap::Parser;

    let mut argv = vec![
        "eval".to_string(),
        fixture_dir.to_string_lossy().into_owned(),
        "--execution-mode".to_string(),
        "live".to_string(),
        "--judge".to_string(),
        "--model".to_string(),
        target_model.to_string(),
        "--judge-model".to_string(),
        judge_model.to_string(),
    ];
    if let Some(ep) = generator_endpoint {
        argv.push("--generator-endpoint".to_string());
        argv.push(ep.to_string());
    }
    if let Some(jep) = judge_endpoint {
        argv.push("--judge-endpoint".to_string());
        argv.push(jep.to_string());
    }

    let args = EvalArgs::parse_from(&argv);
    let exit_code = run_eval_command(args, config, globals);
    if exit_code != ExitCode::Ok.as_i32() && exit_code != ExitCode::Failed.as_i32() {
        // Exit codes 0 (all pass) and 1 (some fail) both mean eval completed.
        // Any other code (4, 5) means a runtime/config error.
        return Err(exit_code);
    }

    // Capture the most recent run.
    // TODO: refactor run_eval_command to return Result<Run, ExitCode>
    use agentcarousel_reporters::{fetch_run, list_runs};
    let listings = list_runs(1).map_err(|e| {
        eprintln!("error: could not list runs after eval: {e}");
        ExitCode::RuntimeError.as_i32()
    })?;
    let listing = listings.into_iter().next().ok_or_else(|| {
        eprintln!("error: no run found in history after eval");
        ExitCode::RuntimeError.as_i32()
    })?;
    let run = fetch_run(&listing.id).map_err(|e| {
        eprintln!("error: could not fetch run {}: {e}", listing.id);
        ExitCode::RuntimeError.as_i32()
    })?;
    Ok((listing.id, run.summary.pass_rate))
}

// ── Onboard ───────────────────────────────────────────────────────────────────

fn run_onboard(args: OnboardArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let local = LocalProfile::load();

    // Resolve target model: CLI > local profile > config.
    let target_model = args
        .target_model
        .clone()
        .or_else(|| local.generator.as_ref().and_then(|g| g.model.clone()))
        .unwrap_or_else(|| config.generator.model.clone());

    let generator_endpoint = args
        .target_endpoint
        .as_deref()
        .or(local.generator_endpoint())
        .or(config.generator.endpoint.as_deref())
        .map(|s| s.to_string());

    let judge_model = args
        .judge_model
        .clone()
        .or_else(|| local.judge.as_ref().and_then(|j| j.model.clone()))
        .unwrap_or_else(|| config.judge.model.clone());

    let judge_endpoint = args
        .judge_endpoint
        .as_deref()
        .or_else(|| local.judge_endpoint())
        // When the judge is a local/custom model and no dedicated endpoint is given,
        // inherit the generator endpoint — same ollama server handles both roles.
        .or_else(|| {
            if judge_model.starts_with("ollama/") || judge_model.starts_with("custom/") {
                generator_endpoint.as_deref()
            } else {
                None
            }
        })
        .map(|s| s.to_string());

    let fixture_dir = fixture_path(&args.skill);

    if !fixture_dir.is_dir() && !args.dry_run {
        eprintln!(
            "error: fixtures directory not found: {}",
            fixture_dir.display()
        );
        return ExitCode::NotFound.as_i32();
    }

    if !globals.quiet {
        println!();
        println!(
            "  Pipeline: onboarding skill '{}' (model: {})",
            args.skill, target_model
        );
        println!();
    }

    if args.dry_run {
        println!("  [dry-run] Step 1/4 — generate: agc generate --from-prompt {} --count 5 --model {} {}", prompt_path(&args.skill).display(), target_model, generator_endpoint.as_deref().map(|ep| format!("--generator-endpoint {ep}")).unwrap_or_default());
        println!(
            "  [dry-run] Step 2/4 — validate: agc validate {}",
            fixture_dir.display()
        );
        println!("  [dry-run] Step 3/4 — eval:     agc eval {} --execution-mode live --judge --model {} --judge-model {}", fixture_dir.display(), target_model, judge_model);
        println!("  [dry-run] Step 3.5/4 — metrics + audit");
        println!("  [dry-run] Step 4/4 — tag baseline");
        return ExitCode::Ok.as_i32();
    }

    // Mark as Onboarding immediately so `agc candidates` shows it.
    let (mut store, store_path) = load_store();
    let mut entry = CandidateEntry {
        skill: args.skill.clone(),
        status: CandidateStatus::Onboarding,
        target_model: target_model.clone(),
        target_endpoint: generator_endpoint.clone(),
        baseline_run_id: None,
        baseline_score: None,
        current_score: None,
        baseline_metrics: None,
        current_metrics: None,
        target_score: args.target_score,
        improvement_rounds: 0,
        last_updated: now_rfc3339(),
        onboarded_at: None,
    };
    store.upsert(entry.clone());
    save_store(&store, &store_path);

    // ── Step 1/4: generate ────────────────────────────────────────────────────
    if !globals.quiet {
        println!("  [1/4] Generating fixture cases...");
    }
    {
        use super::generate::{run_generate, GenerateArgs};
        let mut argv = vec![
            "generate".to_string(),
            "--from-prompt".to_string(),
            prompt_path(&args.skill).to_string_lossy().into_owned(),
            "--count".to_string(),
            "5".to_string(),
            "--model".to_string(),
            target_model.clone(),
        ];
        if let Some(ref ep) = generator_endpoint {
            argv.push("--generator-endpoint".to_string());
            argv.push(ep.clone());
        }
        let gen_args = GenerateArgs::parse_from(&argv);
        let code = run_generate(gen_args, globals);
        if code != ExitCode::Ok.as_i32() {
            eprintln!("error: generate step failed (exit {code})");
            entry.last_updated = now_rfc3339();
            store.upsert(entry);
            save_store(&store, &store_path);
            return code;
        }
    }

    // ── Step 2/4: validate ────────────────────────────────────────────────────
    if !globals.quiet {
        println!("  [2/4] Validating fixtures...");
    }
    {
        use super::validate::{run_validate, ValidateArgs};
        let argv = vec![
            "validate".to_string(),
            fixture_dir.to_string_lossy().into_owned(),
        ];
        let val_args = ValidateArgs::parse_from(&argv);
        let code = run_validate(val_args, config, globals);
        if code != ExitCode::Ok.as_i32() {
            eprintln!("error: validate step failed (exit {code})");
            return code;
        }
    }

    // ── Step 3/4: eval (live + judge) ─────────────────────────────────────────
    if !globals.quiet {
        println!("  [3/4] Running live eval with judge...");
    }
    let (run_id, pass_rate) = match run_eval_and_get_score(
        &fixture_dir,
        &target_model,
        generator_endpoint.as_deref(),
        &judge_model,
        judge_endpoint.as_deref(),
        config,
        globals,
    ) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    if pass_rate < args.target_score && !globals.quiet {
        println!(
            "  warning: pass rate {:.0}% is below target {:.0}% — proceeding anyway",
            pass_rate * 100.0,
            args.target_score * 100.0
        );
    }

    // ── Step 3.5/4: compliance metrics + audit ───────────────────────────────
    if !globals.quiet {
        println!("  [3.5/4] Computing compliance metrics...");
    }
    let baseline_metrics = snapshot_metrics(&args.skill);
    if !globals.quiet {
        println!(
            "  metrics: injection={} drift={} coverage={} calibration={}",
            fmt_metric(baseline_metrics.injection_resistance),
            fmt_metric(baseline_metrics.behavioral_drift),
            fmt_metric(baseline_metrics.coverage),
            fmt_metric(baseline_metrics.calibration),
        );
    }

    // Run audit for informational preview (does not modify prompt.md).
    if !globals.quiet {
        println!("  [4.5/5] Running prompt audit (informational)...");
    }
    {
        use super::audit::{run_audit_command, AuditArgs};
        let mut argv = vec![
            "audit".to_string(),
            "run".to_string(),
            run_id.clone(),
            "--model".to_string(),
            judge_model.clone(),
        ];
        if let Some(ref jep) = judge_endpoint {
            argv.push("--judge-endpoint".to_string());
            argv.push(jep.clone());
        }
        let audit_args = AuditArgs::parse_from(&argv);
        // Audit is best-effort; ignore exit code.
        let _ = run_audit_command(audit_args, config, globals);
    }

    // ── Step 4/4: tag baseline ────────────────────────────────────────────────
    if !globals.quiet {
        println!("  [4/4] Tagging baseline run...");
    }
    {
        use agentcarousel_reporters::tag_run;
        let tag_name = format!("{}-baseline", args.skill);
        if let Err(e) = tag_run(&tag_name, &run_id) {
            eprintln!("warning: could not tag run as baseline: {e}");
        } else if !globals.quiet {
            println!(
                "  ✓ baseline tagged: run {} · {:.0}%",
                run_id,
                pass_rate * 100.0
            );
        }
    }

    // Save candidate as Stable with baseline data.
    let onboarded_at = now_rfc3339();
    entry.status = CandidateStatus::Stable;
    entry.baseline_run_id = Some(run_id.clone());
    entry.baseline_score = Some(pass_rate);
    entry.current_score = Some(pass_rate);
    entry.baseline_metrics = Some(baseline_metrics);
    entry.target_score = args.target_score;
    entry.last_updated = onboarded_at.clone();
    entry.onboarded_at = Some(onboarded_at);
    store.upsert(entry);
    save_store(&store, &store_path);

    if !globals.quiet {
        println!();
        println!(
            "  Onboard complete: '{}' is Stable at {:.0}% (target: {:.0}%)",
            args.skill,
            pass_rate * 100.0,
            args.target_score * 100.0,
        );
        println!(
            "  Run `agc pipeline improve {}` to start the improvement loop.",
            args.skill
        );
        println!();
    }

    ExitCode::Ok.as_i32()
}

// ── Improve ───────────────────────────────────────────────────────────────────

#[allow(unused_assignments)]
fn run_improve(args: ImproveArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let local = LocalProfile::load();

    let target_score = args
        .target_score
        .or_else(|| local.target_score())
        .unwrap_or(0.85_f32);

    let max_rounds = args
        .max_rounds
        .or_else(|| local.max_rounds())
        .unwrap_or(5_u32);

    let (mut store, store_path) = load_store();
    let entry_opt = store.get(&args.skill).cloned();

    let mut entry = match entry_opt {
        Some(e) if e.status != CandidateStatus::Onboarding => e,
        Some(_) => {
            eprintln!(
                "error: skill '{}' is still Onboarding — run `agc pipeline onboard {}` first",
                args.skill, args.skill
            );
            return ExitCode::ConfigError.as_i32();
        }
        None => {
            eprintln!(
                "error: skill '{}' not found in candidates — run `agc pipeline onboard {}` first",
                args.skill, args.skill
            );
            return ExitCode::ConfigError.as_i32();
        }
    };

    let target_model = entry.target_model.clone();
    let generator_endpoint = args
        .target_endpoint
        .as_deref()
        .or(entry.target_endpoint.as_deref())
        .or(local.generator_endpoint())
        .or(config.generator.endpoint.as_deref())
        .map(|s| s.to_string());

    let judge_model = local
        .judge
        .as_ref()
        .and_then(|j| j.model.clone())
        .unwrap_or_else(|| config.judge.model.clone());

    let judge_endpoint = args
        .judge_endpoint
        .as_deref()
        .or_else(|| local.judge_endpoint())
        // Inherit generator endpoint when judge is a local/custom model with no dedicated endpoint.
        .or_else(|| {
            if judge_model.starts_with("ollama/") || judge_model.starts_with("custom/") {
                generator_endpoint.as_deref()
            } else {
                None
            }
        })
        .map(|s| s.to_string());

    let fixture_dir = fixture_path(&args.skill);

    if !globals.quiet {
        println!();
        println!(
            "  Pipeline: improving skill '{}' (target: {:.0}%, max rounds: {})",
            args.skill,
            target_score * 100.0,
            max_rounds,
        );
        println!();
    }

    let mut no_improvement_streak = 0u32;
    // Tracks the most recent re-eval run ID for stuck-detection audit.
    // Starts as the baseline run ID so audit works even before any re-eval completes.
    let mut current_run_id: Option<String> = entry.baseline_run_id.clone();

    for round in 0..max_rounds {
        if !globals.quiet {
            println!("  Round {}/{}", round + 1, max_rounds);
        }

        // ── a. Eval → current_score ───────────────────────────────────────────
        // run_id from the first eval is not stored; re-eval run ID is used for audit.
        let (_first_run_id, current_score) = match run_eval_and_get_score(
            &fixture_dir,
            &target_model,
            generator_endpoint.as_deref(),
            &judge_model,
            judge_endpoint.as_deref(),
            config,
            globals,
        ) {
            Ok(pair) => pair,
            Err(code) => return code,
        };

        // ── b. Metrics snapshot ───────────────────────────────────────────────
        let current_metrics = snapshot_metrics(&args.skill);
        let injection_regressed =
            check_injection_regression(entry.baseline_metrics.as_ref(), &current_metrics);
        if injection_regressed && !globals.quiet {
            println!("  warning: injection resistance dropped vs baseline — optimization may have weakened adversarial guards");
        }

        // ── c. Check target reached ───────────────────────────────────────────
        if current_score >= target_score && !injection_regressed {
            entry.status = CandidateStatus::Stable;
            entry.current_score = Some(current_score);
            entry.current_metrics = Some(current_metrics);
            entry.last_updated = now_rfc3339();
            store.upsert(entry.clone());
            save_store(&store, &store_path);
            if !globals.quiet {
                println!(
                    "  Target reached: {:.0}% >= {:.0}%",
                    current_score * 100.0,
                    target_score * 100.0
                );
            }
            return ExitCode::Ok.as_i32();
        }

        if !globals.quiet {
            println!(
                "  Round {}: score={:.0}% < target={:.0}% — optimizing",
                round + 1,
                current_score * 100.0,
                target_score * 100.0,
            );
        }

        // ── e. Optimize (1 iteration) ─────────────────────────────────────────
        {
            use super::optimize::{run_optimize_command, OptimizeArgs};
            let mut argv = vec![
                "optimize".to_string(),
                fixture_dir.to_string_lossy().into_owned(),
                "--iterations".to_string(),
                "1".to_string(),
                "--target-score".to_string(),
                format!("{}", target_score),
                "--budget".to_string(),
                format!("{}", args.budget),
                "--model".to_string(),
                target_model.clone(),
                "--judge-model".to_string(),
                judge_model.clone(),
            ];
            if let Some(ref ep) = generator_endpoint {
                argv.push("--generator-endpoint".to_string());
                argv.push(ep.clone());
            }
            if let Some(ref jep) = judge_endpoint {
                argv.push("--judge-endpoint".to_string());
                argv.push(jep.clone());
            }
            let opt_args = OptimizeArgs::parse_from(&argv);
            let code = run_optimize_command(opt_args, config, globals);
            if code != ExitCode::Ok.as_i32() && code != ExitCode::Failed.as_i32() {
                eprintln!("error: optimize step failed (exit {code})");
                return code;
            }
        }

        // ── f. A/B gate ───────────────────────────────────────────────────────
        let bak = prompt_bak_path(&args.skill);
        let prompt = prompt_path(&args.skill);
        if bak.exists() && prompt.exists() {
            use super::ab::{run_ab, AbArgs};
            let mut argv = vec![
                "ab".to_string(),
                fixture_dir.to_string_lossy().into_owned(),
                "--a".to_string(),
                bak.to_string_lossy().into_owned(),
                "--b".to_string(),
                prompt.to_string_lossy().into_owned(),
                "--execution-mode".to_string(),
                "live".to_string(),
                "--model".to_string(),
                target_model.clone(),
                "--judge".to_string(),
                "--judge-model".to_string(),
                judge_model.clone(),
                "--threshold".to_string(),
                "0.05".to_string(),
            ];
            if let Some(ref ep) = generator_endpoint {
                argv.push("--generator-endpoint".to_string());
                argv.push(ep.clone());
            }
            if let Some(ref jep) = judge_endpoint {
                argv.push("--judge-endpoint".to_string());
                argv.push(jep.clone());
            }
            let ab_args = AbArgs::parse_from(&argv);
            let ab_code = run_ab(ab_args, config, globals);
            if ab_code == ExitCode::Failed.as_i32() {
                if !globals.quiet {
                    println!("  A/B gate: optimized prompt regressed — reverting");
                }
                // Restore prompt.md from backup.
                if let Err(e) = std::fs::copy(&bak, &prompt) {
                    eprintln!("error: could not revert prompt.md from backup: {e}");
                }
                // Skip re-eval this round.
                continue;
            }
        }

        // ── g. Re-eval → new_score ────────────────────────────────────────────
        let (new_run_id, new_score) = match run_eval_and_get_score(
            &fixture_dir,
            &target_model,
            generator_endpoint.as_deref(),
            &judge_model,
            judge_endpoint.as_deref(),
            config,
            globals,
        ) {
            Ok(pair) => pair,
            Err(code) => return code,
        };
        current_run_id = Some(new_run_id);

        // ── h. Post-optimize metrics ──────────────────────────────────────────
        let post_metrics = snapshot_metrics(&args.skill);
        let post_injection_regressed =
            check_injection_regression(entry.baseline_metrics.as_ref(), &post_metrics);

        // ── i. Update candidate ───────────────────────────────────────────────
        if new_score > current_score && !post_injection_regressed {
            entry.status = CandidateStatus::Improving;
            entry.current_score = Some(new_score);
            let injection_display = fmt_metric(post_metrics.injection_resistance);
            entry.current_metrics = Some(post_metrics);
            entry.improvement_rounds += 1;
            entry.last_updated = now_rfc3339();
            store.upsert(entry.clone());
            save_store(&store, &store_path);
            no_improvement_streak = 0;
            if !globals.quiet {
                println!(
                    "  Improved: {:.0}% -> {:.0}%  [injection: {}]",
                    current_score * 100.0,
                    new_score * 100.0,
                    injection_display,
                );
            }
        } else if post_injection_regressed {
            if !globals.quiet {
                println!("  Optimization degraded injection resistance — reverting prompt.md");
            }
            if bak.exists() {
                if let Err(e) = std::fs::copy(&bak, &prompt) {
                    eprintln!("error: could not revert prompt.md from backup: {e}");
                }
            }
            no_improvement_streak += 1;
        } else {
            no_improvement_streak += 1;
            if !globals.quiet {
                println!("  No improvement this round ({:.0}%)", new_score * 100.0);
            }
        }

        // Check for stuck condition.
        if no_improvement_streak >= 2 {
            if !globals.quiet {
                println!(
                    "  warning: stuck after {} rounds — running audit for diagnosis",
                    no_improvement_streak
                );
            }
            // Run audit to surface diagnosis.
            if let Some(ref audit_run_id) = current_run_id {
                use super::audit::{run_audit_command, AuditArgs};
                let mut argv = vec![
                    "audit".to_string(),
                    "run".to_string(),
                    audit_run_id.clone(),
                    "--model".to_string(),
                    judge_model.clone(),
                ];
                if let Some(ref jep) = judge_endpoint {
                    argv.push("--judge-endpoint".to_string());
                    argv.push(jep.clone());
                }
                let audit_args = AuditArgs::parse_from(&argv);
                let _ = run_audit_command(audit_args, config, globals);
                if !globals.quiet {
                    println!();
                    println!("  Audit complete. Review findings above or run:");
                    println!(
                        "    agc audit suggest {} --apply   # append suggestions to prompt.md",
                        audit_run_id
                    );
                    println!("  Then re-run: agc pipeline improve {}", args.skill);
                }
            } else if !globals.quiet {
                println!("  No completed re-eval run ID available for audit.");
                println!("  Then re-run: agc pipeline improve {}", args.skill);
            }
            entry.status = CandidateStatus::Improving;
            entry.last_updated = now_rfc3339();
            store.upsert(entry);
            save_store(&store, &store_path);
            return ExitCode::Failed.as_i32();
        }
    }

    // Loop completed: save final status.
    let final_score = entry.current_score.unwrap_or(0.0);
    entry.status = if final_score >= target_score {
        CandidateStatus::Stable
    } else {
        CandidateStatus::Improving
    };
    entry.last_updated = now_rfc3339();
    store.upsert(entry);
    save_store(&store, &store_path);

    if final_score >= target_score {
        ExitCode::Ok.as_i32()
    } else {
        ExitCode::Failed.as_i32()
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn fmt_metric(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{:.0}%", v * 100.0),
        None => "—".to_string(),
    }
}

/// Returns true if injection resistance regressed by more than 5 percentage points.
fn check_injection_regression(baseline: Option<&MetricSnapshot>, current: &MetricSnapshot) -> bool {
    match (
        baseline.and_then(|b| b.injection_resistance),
        current.injection_resistance,
    ) {
        (Some(b), Some(c)) => c < b - 0.05,
        _ => false,
    }
}
