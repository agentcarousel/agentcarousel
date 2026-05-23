use agentcarousel_reporters::{fetch_run, find_previous_run, find_tagged_run, list_runs, tag_run};
use clap::{Parser, Subcommand};
use console::style;
use serde::Serialize;

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::registry_client::{resolve_token, RegistryClient};
use super::GlobalOptions;

const DEFAULT_THRESHOLD: f32 = 0.05;

/// Check whether a new run is better or worse than a saved baseline.
///
/// agc compare looks at pass rate, effectiveness scores, and per-case status changes between two runs and reports any regressions. Use it as a CI gate to catch quality drops before they reach production.
///
/// Use `agc compare tag` to save a run as a named baseline, then compare future runs against it. Tags can be stored in the local history database or pushed to the shared registry.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc compare -l --baseline <run-id>                     # compare latest run to a saved baseline\n  agc compare -l --baseline <run-id> --threshold 0.05   # allow up to 5% regression\n  agc compare <run-id> --baseline <run-id>               # compare two specific runs\n  agc compare <run-id> --registry                        # fetch baseline from the shared registry\n  agc compare tag <run-id> --name prod-baseline          # save a run as a named baseline\n  agc compare tag <run-id> --name prod-baseline --registry  # push baseline to registry\n\nExit codes:\n  0  no regression detected (or improvement)\n  1  regression exceeds threshold\n  4  runtime error (disk, database)\n  5  run not found in history"
)]
pub struct CompareArgs {
    #[command(subcommand)]
    command: Option<CompareCommand>,

    /// The current run id to compare (omit to use latest run).
    #[arg(value_name = "RUN_ID")]
    run_id: Option<String>,

    /// Use the latest run in history as the current run.
    #[arg(short = 'l', long, conflicts_with = "run_id")]
    latest: bool,

    /// Baseline run id or named tag to compare against.
    #[arg(long)]
    baseline: Option<String>,

    /// Fetch the baseline from the registry instead of local SQLite.
    #[arg(long)]
    registry: bool,

    /// Fall back to local SQLite if --registry baseline fetch fails (default: true).
    #[arg(long, default_value_t = true)]
    local_fallback: bool,

    /// Regression threshold for overall effectiveness delta (default: 0.05).
    #[arg(long, default_value_t = DEFAULT_THRESHOLD)]
    threshold: f32,

    /// Significance level (alpha) for the Mann-Whitney U test (default: 0.05).
    /// When N≥5 matched cases have effectiveness scores, exit 1 only when the
    /// delta exceeds --threshold AND p < --significance.
    #[arg(long, default_value_t = 0.05_f32)]
    significance: f32,

    /// Registry URL (overrides AGENTCAROUSEL_REGISTRY_URL env and config).
    #[arg(long)]
    registry_url: Option<String>,

    /// API token for registry write operations (overrides AGENTCAROUSEL_API_TOKEN env).
    #[arg(long)]
    token: Option<String>,
}

#[derive(Debug, Subcommand)]
enum CompareCommand {
    /// Tag a run as a named baseline for future comparisons.
    Tag {
        /// Run id to tag.
        run_id: String,
        /// Name to store (e.g. `prod-baseline`).
        #[arg(long)]
        name: String,
        /// Push this baseline to the registry instead of local SQLite.
        #[arg(long)]
        registry: bool,
        /// Registry URL (overrides env).
        #[arg(long)]
        registry_url: Option<String>,
        /// API token for registry write operations.
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct CompareResult {
    pub baseline_run_id: String,
    pub current_run_id: String,
    pub skill_or_agent: Option<String>,
    pub regression: bool,
    pub overall_effectiveness_delta: Option<f32>,
    pub pass_rate_delta: f32,
    pub threshold: f32,
    pub significance: f32,
    /// Per-case effectiveness scores from the baseline run (matched cases only).
    pub samples_baseline: Vec<f64>,
    /// Per-case effectiveness scores from the current run (matched cases only).
    pub samples_current: Vec<f64>,
    /// Mann-Whitney U p-value. `None` when fewer than 5 matched scored cases.
    pub p_value: Option<f64>,
    /// Whether the effectiveness delta is statistically significant at `significance` level.
    pub significant: Option<bool>,
    pub cases: Vec<CaseCompare>,
}

#[derive(Debug, Serialize)]
pub struct CaseCompare {
    pub case_id: String,
    pub baseline_effectiveness: Option<f32>,
    pub current_effectiveness: Option<f32>,
    pub delta: Option<f32>,
    pub regression: bool,
}

pub fn run_compare(args: CompareArgs, globals: &GlobalOptions) -> i32 {
    if let Some(CompareCommand::Tag {
        run_id,
        name,
        registry,
        registry_url,
        token,
    }) = args.command
    {
        return run_tag(
            &run_id,
            &name,
            registry,
            registry_url.as_deref(),
            token.as_deref(),
            globals,
        );
    }
    run_compare_runs(args, globals)
}

fn run_tag(
    run_id: &str,
    name: &str,
    registry: bool,
    registry_url: Option<&str>,
    token_flag: Option<&str>,
    globals: &GlobalOptions,
) -> i32 {
    if registry {
        // Push baseline to registry (agc-oisz)
        let url = match std::env::var("AGENTCAROUSEL_REGISTRY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| registry_url.map(str::to_string))
        {
            Some(u) => u,
            None => {
                emit_error(
                    globals,
                    "compare tag",
                    "no_registry_url",
                    "Registry URL required for --registry. Set AGENTCAROUSEL_REGISTRY_URL or pass --registry-url.",
                    vec![],
                    String::new(),
                );
                return ExitCode::ValidationFailed.as_i32();
            }
        };
        let token = match resolve_token(token_flag) {
            Some(t) => t,
            None => {
                emit_error(
                    globals,
                    "compare tag",
                    "no_token",
                    "Registry token required for --registry. Set AGENTCAROUSEL_API_TOKEN.",
                    vec!["export AGENTCAROUSEL_API_TOKEN=<your-token>".to_string()],
                    String::new(),
                );
                return ExitCode::ValidationFailed.as_i32();
            }
        };
        let client = match RegistryClient::new(&url, &token) {
            Ok(c) => c,
            Err(err) => {
                emit_error(
                    globals,
                    "compare tag",
                    "client_error",
                    &err,
                    vec![],
                    String::new(),
                );
                return ExitCode::RuntimeError.as_i32();
            }
        };
        // bundle_id from run name convention: use the `name` arg as bundle_id (CI usage)
        match client.set_bundle_baseline(name, run_id) {
            Ok(res) => {
                if globals.json {
                    JsonOutput::ok("compare tag", &res).print();
                } else {
                    let set_at = res.get("set_at").and_then(|v| v.as_str()).unwrap_or("");
                    println!("registry baseline set: run={run_id} bundle={name} set_at={set_at}");
                }
                ExitCode::Ok.as_i32()
            }
            Err(err) if err.contains("not an improvement") => {
                emit_error(
                    globals,
                    "compare tag",
                    "not_an_improvement",
                    "Registry rejected baseline: run is not an improvement over the current baseline.",
                    vec!["Only a strictly better run (higher pass_rate or composite_score) can replace the baseline.".to_string()],
                    err,
                );
                ExitCode::Failed.as_i32()
            }
            Err(err) => {
                emit_error(
                    globals,
                    "compare tag",
                    "runtime_error",
                    &err,
                    vec![],
                    String::new(),
                );
                ExitCode::RuntimeError.as_i32()
            }
        }
    } else {
        // Local SQLite tag
        match tag_run(name, run_id) {
            Ok(()) => {
                if globals.json {
                    JsonOutput::ok(
                        "compare tag",
                        serde_json::json!({ "name": name, "run_id": run_id }),
                    )
                    .print();
                } else {
                    println!("tagged run {run_id} as '{name}'");
                }
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                if globals.json {
                    JsonOutput::err(
                        "compare tag",
                        JsonError::new("runtime_error", err.to_string()),
                    )
                    .print();
                } else {
                    eprintln!("error: {err}");
                }
                ExitCode::RuntimeError.as_i32()
            }
        }
    }
}

fn run_compare_runs(args: CompareArgs, globals: &GlobalOptions) -> i32 {
    let current = match resolve_current_run(args.run_id.as_deref(), args.latest, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };

    let baseline = match resolve_baseline(
        &args.baseline,
        &current,
        args.registry,
        args.local_fallback,
        args.registry_url.as_deref(),
        globals,
    ) {
        Ok(r) => r,
        Err(code) => return code,
    };

    let result = build_compare_result(&baseline, &current, args.threshold, args.significance);
    let regression = result.regression;

    if globals.json {
        JsonOutput::ok("compare", &result).print();
    } else {
        print_compare_terminal(&result);
    }

    if regression {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn resolve_current_run(
    run_id: Option<&str>,
    latest: bool,
    globals: &GlobalOptions,
) -> Result<agentcarousel_core::Run, i32> {
    if let Some(id) = run_id {
        fetch_run(id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "run_not_found",
                &format!("Run '{id}' not found in history database."),
                vec!["Run 'agc report list' to see available run IDs.".to_string()],
                err.to_string(),
            );
            ExitCode::NotFound.as_i32()
        })
    } else if latest {
        let listings = list_runs(1).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            ExitCode::RuntimeError.as_i32()
        })?;
        let id = listings.into_iter().next().ok_or_else(|| {
            emit_error(
                globals,
                "compare",
                "no_runs",
                "No runs in history database.",
                vec!["Run 'agc eval' first.".to_string()],
                String::new(),
            );
            ExitCode::NotFound.as_i32()
        })?;
        fetch_run(&id.id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            ExitCode::RuntimeError.as_i32()
        })
    } else {
        emit_error(
            globals,
            "compare",
            "invalid_args",
            "Specify a RUN_ID or pass -l/--latest.",
            vec!["Example: agc compare -l --baseline <run-id>".to_string()],
            String::new(),
        );
        Err(ExitCode::ValidationFailed.as_i32())
    }
}

fn resolve_baseline(
    baseline_arg: &Option<String>,
    current: &agentcarousel_core::Run,
    registry: bool,
    local_fallback: bool,
    registry_url: Option<&str>,
    globals: &GlobalOptions,
) -> Result<agentcarousel_core::Run, i32> {
    // --registry: fetch baseline from registry, optionally fall back to local
    if registry {
        let bundle_id = current.skill_or_agent.as_deref().unwrap_or("");
        if bundle_id.is_empty() {
            emit_error(
                globals,
                "compare",
                "no_bundle_id",
                "Cannot fetch registry baseline: current run has no skill_or_agent.",
                vec!["Pass --baseline <run-id> for explicit local baseline.".to_string()],
                String::new(),
            );
            return Err(ExitCode::ValidationFailed.as_i32());
        }

        let url = std::env::var("AGENTCAROUSEL_REGISTRY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| registry_url.map(str::to_string));

        if let Some(url) = url {
            if let Ok(client) = RegistryClient::new(&url, "") {
                match client.get_bundle_baseline(bundle_id) {
                    Ok(res) => {
                        if let Some(run_id) = res.get("run_id").and_then(|v| v.as_str()) {
                            if let Ok(run) = fetch_run(run_id) {
                                return Ok(run);
                            }
                            // run not in local history — build a synthetic baseline
                            let pass_rate =
                                res.get("pass_rate").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                            let composite = res
                                .get("composite_score")
                                .and_then(|v| v.as_f64())
                                .map(|v| v as f32);
                            return Ok(synthetic_baseline_run(
                                run_id, bundle_id, pass_rate, composite,
                            ));
                        }
                    }
                    Err(err) if !local_fallback => {
                        emit_error(
                            globals,
                            "compare",
                            "registry_baseline_failed",
                            &format!("Failed to fetch registry baseline: {err}"),
                            vec!["Pass --local-fallback to fall back to local SQLite.".to_string()],
                            err,
                        );
                        return Err(ExitCode::RuntimeError.as_i32());
                    }
                    Err(_) => {} // fall through to local
                }
            }
        } else if !local_fallback {
            emit_error(
                globals,
                "compare",
                "no_registry_url",
                "Registry URL required for --registry. Set AGENTCAROUSEL_REGISTRY_URL or pass --registry-url.",
                vec![],
                String::new(),
            );
            return Err(ExitCode::ValidationFailed.as_i32());
        }
        // fall through to local resolution
    }

    // Resolution order: explicit --baseline, named tag, auto (previous run for same skill)
    let baseline_id: Option<String> = if let Some(ref spec) = baseline_arg {
        if fetch_run(spec).is_ok() {
            Some(spec.clone())
        } else {
            find_tagged_run(spec).ok().flatten().or(Some(spec.clone()))
        }
    } else {
        None
    };

    if let Some(id) = baseline_id {
        return fetch_run(&id).map_err(|err| {
            emit_error(
                globals,
                "compare",
                "run_not_found",
                &format!("Baseline run '{id}' not found in history database."),
                vec!["Run 'agc report list' to see available run IDs.".to_string()],
                err.to_string(),
            );
            ExitCode::NotFound.as_i32()
        });
    }

    // Auto-baseline: most recent prior run for same skill
    let skill = match current.skill_or_agent.as_deref() {
        Some(s) => s,
        None => {
            emit_error(
                globals,
                "compare",
                "no_baseline",
                "Cannot auto-select baseline: current run has no skill_or_agent. Pass --baseline <run-id>.",
                vec![],
                String::new(),
            );
            return Err(ExitCode::NotFound.as_i32());
        }
    };

    match find_previous_run(skill, &current.id.0) {
        Ok(Some(run)) => Ok(run),
        Ok(None) => {
            emit_error(
                globals,
                "compare",
                "no_baseline",
                &format!("No previous run found for skill '{skill}'. Pass --baseline <run-id>."),
                vec!["Run 'agc report list' to see available runs.".to_string()],
                String::new(),
            );
            Err(ExitCode::NotFound.as_i32())
        }
        Err(err) => {
            emit_error(
                globals,
                "compare",
                "runtime_error",
                &err.to_string(),
                vec![],
                err.to_string(),
            );
            Err(ExitCode::RuntimeError.as_i32())
        }
    }
}

/// Build a minimal synthetic Run from registry baseline metrics when the run isn't in local history.
fn synthetic_baseline_run(
    run_id: &str,
    skill: &str,
    pass_rate: f32,
    composite_score: Option<f32>,
) -> agentcarousel_core::Run {
    use agentcarousel_core::{OverallStatus, ProviderErrorMetrics, Run, RunId, RunSummary};
    use chrono::Utc;
    let summary = RunSummary {
        total: 0,
        passed: 0,
        failed: 0,
        skipped: 0,
        flaky: 0,
        errored: 0,
        timed_out: 0,
        pass_rate,
        mean_latency_ms: 0.0,
        mean_effectiveness_score: composite_score,
        provider_errors: ProviderErrorMetrics {
            status_429: 0,
            status_500: 0,
            status_503: 0,
            status_504: 0,
        },
        overall_status: if pass_rate >= 1.0 {
            OverallStatus::Pass
        } else {
            OverallStatus::Fail
        },
        tokens_in: None,
        tokens_out: None,
        mean_tokens_per_judged_case: None,
        latency_p50_ms: None,
        latency_p95_ms: None,
        latency_p99_ms: None,
        judge_tokens_in: None,
        judge_tokens_out: None,
        gen_cost_usd: None,
        judge_cost_usd: None,
        total_cost_usd: None,
        generator_model: None,
        judge_model: None,
        command_line: None,
    };
    Run {
        id: RunId(run_id.to_string()),
        schema_version: 1,
        started_at: Utc::now(),
        finished_at: None,
        command: String::new(),
        git_sha: None,
        agentcarousel_version: String::new(),
        config_hash: String::new(),
        cases: vec![],
        summary,
        fixture_bundle_id: None,
        fixture_bundle_version: None,
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
        skill_or_agent: Some(skill.to_string()),
        runner_offline: false,
        runner_mock_strict: false,
        runner_mock_only: false,
        prompt_audit: None,
    }
}

fn build_compare_result(
    baseline: &agentcarousel_core::Run,
    current: &agentcarousel_core::Run,
    threshold: f32,
    significance: f32,
) -> CompareResult {
    use std::collections::HashMap;

    let baseline_cases: HashMap<&str, &agentcarousel_core::CaseResult> = baseline
        .cases
        .iter()
        .map(|c| (c.case_id.0.as_str(), c))
        .collect();

    let mut cases = Vec::new();
    let mut samples_baseline: Vec<f64> = Vec::new();
    let mut samples_current: Vec<f64> = Vec::new();

    for case in &current.cases {
        let baseline_eff = baseline_cases
            .get(case.case_id.0.as_str())
            .and_then(|b| b.eval_scores.as_ref())
            .map(|s| s.effectiveness_score);
        let current_eff = case.eval_scores.as_ref().map(|s| s.effectiveness_score);
        let delta = match (baseline_eff, current_eff) {
            (Some(b), Some(c)) => Some(c - b),
            _ => None,
        };
        let regression = delta.is_some_and(|d| d < -threshold);
        if let (Some(b), Some(c)) = (baseline_eff, current_eff) {
            samples_baseline.push(b as f64);
            samples_current.push(c as f64);
        }
        cases.push(CaseCompare {
            case_id: case.case_id.0.clone(),
            baseline_effectiveness: baseline_eff,
            current_effectiveness: current_eff,
            delta,
            regression,
        });
    }

    let baseline_pass_rate = baseline.summary.pass_rate;
    let current_pass_rate = current.summary.pass_rate;
    let pass_rate_delta = current_pass_rate - baseline_pass_rate;

    let baseline_eff = baseline.summary.mean_effectiveness_score;
    let current_eff = current.summary.mean_effectiveness_score;
    let overall_effectiveness_delta = match (baseline_eff, current_eff) {
        (Some(b), Some(c)) => Some(c - b),
        _ => None,
    };

    let p_value = stats::mann_whitney_u_pvalue(&samples_baseline, &samples_current);
    let significant = p_value.map(|p| p < significance as f64);

    let overall_regression = match (overall_effectiveness_delta, p_value) {
        (Some(d), Some(p)) => d < -threshold && p < significance as f64,
        (Some(d), None) => d < -threshold,
        _ => false,
    };
    let regression = overall_regression || cases.iter().any(|c| c.regression);

    CompareResult {
        baseline_run_id: baseline.id.0.clone(),
        current_run_id: current.id.0.clone(),
        skill_or_agent: current.skill_or_agent.clone(),
        regression,
        overall_effectiveness_delta,
        pass_rate_delta,
        threshold,
        significance,
        samples_baseline,
        samples_current,
        p_value,
        significant,
        cases,
    }
}

fn print_compare_terminal(result: &CompareResult) {
    let skill = result.skill_or_agent.as_deref().unwrap_or("unknown");
    println!(
        "\n  Comparing run {} → {}  ({})\n",
        &result.baseline_run_id[..result.baseline_run_id.len().min(8)],
        &result.current_run_id[..result.current_run_id.len().min(8)],
        skill,
    );

    if let Some(delta) = result.overall_effectiveness_delta {
        let arrow = if delta < 0.0 { "▼" } else { "▲" };
        let label = if result.regression {
            style("⚠ REGRESSION").yellow().bold()
        } else {
            style("✓ OK").green().bold()
        };
        let sig_note = match result.p_value {
            Some(p) if result.significant == Some(true) => {
                format!("  p={p:.3} ★ significant")
            }
            Some(p) => format!("  p={p:.3} (not significant)"),
            None => "  (N<5, no significance test)".to_string(),
        };
        println!(
            "  Overall effectiveness   {:+.2}   {}  {}{}",
            delta, arrow, label, sig_note
        );
    }

    let arrow = if result.pass_rate_delta < 0.0 {
        "▼"
    } else {
        "▲"
    };
    println!(
        "  Pass rate               {:+.0}%   {}",
        result.pass_rate_delta * 100.0,
        arrow,
    );

    let regressions: Vec<&CaseCompare> = result.cases.iter().filter(|c| c.regression).collect();
    if !regressions.is_empty() {
        println!("\n  Regressions:");
        println!("  ┌─────────────────────────────┬────────┬────────┬───────┐");
        println!("  │ Case                        │ Before │ After  │ Delta │");
        println!("  ├─────────────────────────────┼────────┼────────┼───────┤");
        for c in &regressions {
            let before = c
                .baseline_effectiveness
                .map_or("  —   ".to_string(), |v| format!(" {v:.2} "));
            let after = c
                .current_effectiveness
                .map_or("  —   ".to_string(), |v| format!(" {v:.2} "));
            let delta = c.delta.map_or("  —  ".to_string(), |v| format!("{v:+.2}"));
            let short_id: String = c.case_id.chars().take(29).collect();
            println!(
                "  │ {:<29} │{:^8}│{:^8}│{:^7}│",
                short_id, before, after, delta
            );
        }
        println!("  └─────────────────────────────┴────────┴────────┴───────┘");
    }

    println!();
    if result.regression {
        println!(
            "  {}",
            style(format!(
                "Exit 1 — regression exceeds threshold ({:.2})",
                result.threshold
            ))
            .red()
        );
    } else {
        println!("  {}", style("No regression detected").green());
    }
    println!();
}

mod stats {
    /// Mann-Whitney U test (two-tailed). Returns `None` when either sample has
    /// fewer than 5 observations — not enough data for a meaningful result.
    pub(super) fn mann_whitney_u_pvalue(a: &[f64], b: &[f64]) -> Option<f64> {
        if a.len() < 5 || b.len() < 5 {
            return None;
        }
        let n_a = a.len() as f64;
        let n_b = b.len() as f64;
        let mut u = 0.0_f64;
        for &ai in a {
            for &bj in b {
                if ai > bj {
                    u += 1.0;
                } else if (ai - bj).abs() < f64::EPSILON {
                    u += 0.5;
                }
            }
        }
        let mean_u = n_a * n_b / 2.0;
        let var_u = n_a * n_b * (n_a + n_b + 1.0) / 12.0;
        if var_u <= 0.0 {
            return None;
        }
        let z = (u - mean_u) / var_u.sqrt();
        Some(2.0 * normal_sf(z.abs()))
    }

    fn normal_sf(z: f64) -> f64 {
        0.5 * erfc(z / std::f64::consts::SQRT_2)
    }

    /// Complementary error function — Abramowitz & Stegun 7.1.26, max error < 1.5e-7.
    fn erfc(x: f64) -> f64 {
        if x < 0.0 {
            return 2.0 - erfc(-x);
        }
        let t = 1.0 / (1.0 + 0.3275911 * x);
        let poly = t
            * (0.254_829_592
                + t * (-0.284_496_736
                    + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
        poly * (-x * x).exp()
    }

    #[cfg(test)]
    mod tests {
        use super::mann_whitney_u_pvalue;

        #[test]
        fn identical_samples_give_high_pvalue() {
            let a = vec![0.8, 0.9, 0.7, 0.85, 0.8];
            let p = mann_whitney_u_pvalue(&a, &a).unwrap();
            assert!(
                p > 0.5,
                "identical distributions should not be significant: p={p}"
            );
        }

        #[test]
        fn clearly_different_samples_give_low_pvalue() {
            let a = vec![0.1, 0.15, 0.12, 0.11, 0.13];
            let b = vec![0.9, 0.92, 0.88, 0.91, 0.95];
            let p = mann_whitney_u_pvalue(&a, &b).unwrap();
            assert!(
                p < 0.05,
                "clearly different distributions should be significant: p={p}"
            );
        }

        #[test]
        fn returns_none_when_too_few_samples() {
            let a = vec![0.8, 0.9, 0.7, 0.85];
            let b = vec![0.8, 0.9, 0.7, 0.85, 0.8];
            assert!(mann_whitney_u_pvalue(&a, &b).is_none());
        }
    }
}

fn emit_error(
    globals: &GlobalOptions,
    command: &'static str,
    code: &'static str,
    message: &str,
    suggestions: Vec<String>,
    _detail: String,
) {
    if globals.json {
        JsonOutput::err(
            command,
            JsonError::new(code, message).with_suggestions(suggestions),
        )
        .print();
    } else {
        eprintln!("error: {message}");
        for s in &suggestions {
            eprintln!("  hint: {s}");
        }
    }
}
