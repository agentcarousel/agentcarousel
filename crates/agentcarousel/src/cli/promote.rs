use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::fetch_run;
use clap::Parser;
use console::style;
use std::path::PathBuf;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::export::export_run_artifact;
use super::fixture_utils::collect_fixture_paths;
use super::output::{JsonError, JsonOutput};
use super::registry_client::{resolve_token, RegistryClient};
use super::GlobalOptions;
use crate::evaluators::{
    evaluate_for_promotion, PromotionResult, PROMOTE_CRITICAL_THRESHOLD,
    PROMOTE_EFFECTIVENESS_THRESHOLD,
};

const DEFAULT_REGISTRY_URL: &str = "https://api.agentcarousel.com";

/// Promote golden files from a saved run and optionally publish to the registry.
///
/// Loads the run from local history, re-evaluates the promotion gate (effectiveness ≥ 0.90,
/// all critical rubric items ≥ 0.95), writes golden files for passing cases, and prints a
/// summary. With --registry, the run evidence is also submitted to the agentcarousel cloud API.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc promote 3NDEKTSV4R                              # apply gate to saved run\n  agc promote 3NDEKTSV4R --registry               # promote locally + submit to registry\n  agc promote 3NDEKTSV4R fixtures/my-skill/       # specify fixture path explicitly\n\nExit codes:\n  0  all eligible cases promoted (or none eligible)\n  1  one or more cases blocked by the quality gate\n  4  run not found in history or registry error"
)]
pub struct PromoteArgs {
    /// Run ID to load from history.
    run_id: String,
    /// Fixture dirs or files used to load case metadata (rubric + golden_path).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// API token for registry submission (falls back to AGENTCAROUSEL_API_TOKEN or stored credentials).
    #[arg(long)]
    token: Option<String>,
    /// Submit run evidence to the agentcarousel registry after local promotion.
    #[arg(long)]
    registry: bool,
    /// Registry base URL (default: https://api.agentcarousel.com).
    #[arg(long)]
    registry_url: Option<String>,
}

pub fn run_promote(args: PromoteArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    // 1. Load run from local history.
    let run = match fetch_run(&args.run_id) {
        Ok(r) => r,
        Err(err) => {
            let msg = format!("run '{}' not found in history: {err}", args.run_id);
            if globals.json {
                JsonOutput::err("promote", JsonError::new("run_not_found", &msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            return ExitCode::RuntimeError.as_i32();
        }
    };
    let run_id = run.id.0.clone();

    // 2. Load fixture cases so we have rubric + golden_path metadata.
    let fixture_paths = collect_fixture_paths(&args.paths);
    let mut case_map: std::collections::HashMap<String, agentcarousel_core::Case> =
        std::collections::HashMap::new();
    for path in fixture_paths {
        match load_fixture(&path) {
            Ok(fixture) => {
                for case in fixture.cases {
                    case_map.insert(case.id.0.clone(), case);
                }
            }
            Err(err) => {
                eprintln!("warning: failed to load fixture {}: {err}", path.display());
            }
        }
    }

    if case_map.is_empty() {
        let msg = "no fixture cases loaded — check paths (default: fixtures/)";
        if globals.json {
            JsonOutput::err("promote", JsonError::new("no_fixtures", msg)).print();
        } else {
            eprintln!("error: {msg}");
        }
        return ExitCode::RuntimeError.as_i32();
    }

    // 3. Apply the promotion gate to every case result in the run.
    let mut promotion_results: Vec<PromotionResult> = Vec::new();
    for case_result in &run.cases {
        if let Some(case) = case_map.get(&case_result.case_id.0) {
            if let Some(pr) = evaluate_for_promotion(case, case_result, Some(run_id.as_str())) {
                promotion_results.push(pr);
            }
        }
    }

    if promotion_results.is_empty() {
        let msg = "no cases with golden evaluator config found — combine with --evaluator judge";
        if globals.json {
            JsonOutput::ok(
                "promote",
                serde_json::json!({ "promoted": 0, "blocked": 0, "message": msg }),
            )
            .print();
        } else {
            println!("{msg}");
        }
        return ExitCode::Ok.as_i32();
    }

    let promoted_count = promotion_results.iter().filter(|r| r.promoted).count();
    let blocked_count = promotion_results.iter().filter(|r| !r.promoted).count();

    // 4. Print summary (terminal mode).
    if !globals.quiet && !globals.json {
        print_promotion_summary(&promotion_results);
    }

    // 5. Registry submission when --registry is requested.
    let registry_response = if args.registry {
        let registry_url = args
            .registry_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(config.msp.registry_endpoint.as_deref())
            .unwrap_or(DEFAULT_REGISTRY_URL);

        let token = match resolve_token(args.token.as_deref()) {
            Some(t) => t,
            None => {
                let msg = "no API token found — run `agc login` or export AGENTCAROUSEL_API_TOKEN";
                if globals.json {
                    JsonOutput::err("promote", JsonError::new("auth_required", msg)).print();
                } else {
                    eprintln!("error: {msg}");
                }
                return ExitCode::RuntimeError.as_i32();
            }
        };

        let bundle_id = match run.fixture_bundle_id.as_deref() {
            Some(id) => id.to_string(),
            None => {
                let msg = "run has no fixture_bundle_id — re-run with a bundle-packed fixture";
                if globals.json {
                    JsonOutput::err("promote", JsonError::new("missing_bundle_id", msg)).print();
                } else {
                    eprintln!("error: {msg}");
                }
                return ExitCode::RuntimeError.as_i32();
            }
        };

        let client = match RegistryClient::new(registry_url, &token) {
            Ok(c) => c,
            Err(err) => {
                if globals.json {
                    JsonOutput::err("promote", JsonError::new("client_error", &err)).print();
                } else {
                    eprintln!("error: {err}");
                }
                return ExitCode::RuntimeError.as_i32();
            }
        };

        let evidence_path = match export_run_artifact(run_id.as_str(), None) {
            Ok(p) => p,
            Err(err) => {
                let msg = format!("failed to export evidence: {err}");
                if globals.json {
                    JsonOutput::err("promote", JsonError::new("export_error", &msg)).print();
                } else {
                    eprintln!("error: {msg}");
                }
                return ExitCode::RuntimeError.as_i32();
            }
        };

        match client.submit_run_evidence(&bundle_id, &evidence_path) {
            Ok(resp) => {
                if !globals.json && !globals.quiet {
                    println!(
                        "  {} submitted to {} (bundle {})",
                        style("registry").bold(),
                        registry_url,
                        bundle_id
                    );
                    if let Some(id) = resp.get("run_id").and_then(|v| v.as_str()) {
                        println!("  registry run id: {}", style(id).cyan());
                    }
                }
                Some(resp)
            }
            Err(err) => {
                let msg = format!("registry submission failed: {err}");
                if globals.json {
                    JsonOutput::err("promote", JsonError::new("registry_error", &msg)).print();
                } else {
                    eprintln!("error: {msg}");
                }
                return ExitCode::RuntimeError.as_i32();
            }
        }
    } else {
        None
    };

    if globals.json {
        let mut payload = serde_json::json!({
            "run_id": run_id,
            "promoted": promoted_count,
            "blocked": blocked_count,
        });
        if let Some(reg) = registry_response {
            payload["registry"] = reg;
        }
        JsonOutput::ok("promote", payload).print();
    }

    if blocked_count > 0 {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn print_promotion_summary(results: &[PromotionResult]) {
    let col_w = results
        .iter()
        .map(|r| r.case_id.len())
        .max()
        .unwrap_or(20)
        .clamp(20, 60);

    println!();
    println!(
        "  {}  (effectiveness ≥ {:.2} · critical ≥ {:.2})",
        style("Promotion summary").bold(),
        PROMOTE_EFFECTIVENESS_THRESHOLD,
        PROMOTE_CRITICAL_THRESHOLD,
    );

    let bar = "─".repeat(col_w + 2);
    println!("  ┌{}┬──────────┬──────────┬─────────────┐", bar);
    println!(
        "  │ {:<col_w$} │ Effect.  │ Baseline │ Status      │",
        "Case",
        col_w = col_w
    );
    println!("  ├{}┼──────────┼──────────┼─────────────┤", bar);
    for r in results {
        let baseline_str = r
            .golden_baseline
            .map(|b| format!("  {:.2}  ", b))
            .unwrap_or_else(|| "  --    ".to_string());
        let status = if r.promoted {
            style("▲ promoted ".to_string()).green().to_string()
        } else {
            style("✗ blocked  ".to_string()).red().to_string()
        };
        println!(
            "  │ {:<col_w$} │  {:.2}    │{}│ {} │",
            r.case_id,
            r.effectiveness,
            baseline_str,
            status,
            col_w = col_w
        );
    }
    println!("  └{}┴──────────┴──────────┴─────────────┘", bar);

    for r in results
        .iter()
        .filter(|r| !r.promoted && !r.blocked_by.is_empty())
    {
        println!();
        println!("  {}", style(&r.case_id).bold());
        for (id, score) in &r.blocked_by {
            println!(
                "    {}  {}  {:.2}  (needs {:.2})",
                style("CRITICAL").red().bold(),
                id,
                score,
                PROMOTE_CRITICAL_THRESHOLD,
            );
        }
        println!(
            "    {}: improve skill prompt or golden and re-run with judge scoring",
            style("hint").yellow()
        );
    }
    println!();
}
