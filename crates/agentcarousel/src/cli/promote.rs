use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::fetch_run;
use clap::Parser;
use console::style;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    after_help = "Examples:\n  agc promote 3NDEKTSV4R                              # apply gate to saved run\n  agc promote 3NDEKTSV4R --registry               # promote locally + submit to registry\n  agc promote 3NDEKTSV4R fixtures/my-skill/       # specify fixture path explicitly\n  agc promote 3NDEKTSV4R --bootstrap-golden       # write golden files from run output (no gate)\n\nExit codes:\n  0  all eligible cases promoted (or none eligible)\n  1  one or more cases blocked by the quality gate\n  4  run not found in history or registry error"
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
    /// Write run outputs directly to golden files, bypassing the quality gate.
    /// Creates the golden/ directory and patches evaluator_config in cases.yaml for any case
    /// that does not already have a golden evaluator configured. Use this to seed golden files
    /// for a skill that has never had them before.
    #[arg(long)]
    bootstrap_golden: bool,
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
    let mut case_map: HashMap<String, agentcarousel_core::Case> = HashMap::new();
    let mut case_file_map: HashMap<String, PathBuf> = HashMap::new();
    for path in fixture_paths {
        match load_fixture(&path) {
            Ok(fixture) => {
                for case in fixture.cases {
                    case_file_map.insert(case.id.0.clone(), path.clone());
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

    // 2b. Bootstrap mode: write golden files directly from run output, no gate.
    if args.bootstrap_golden {
        return bootstrap_golden_cases(&run, &case_map, &case_file_map, globals);
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
                let msg = "no API token found — export AGENTCAROUSEL_API_TOKEN";
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

// ── Bootstrap golden ──────────────────────────────────────────────────────────

struct BootstrapEntry {
    case_id: String,
    golden_path: PathBuf,
    /// None = written successfully; Some(reason) = skipped.
    skip_reason: Option<String>,
}

fn bootstrap_golden_cases(
    run: &agentcarousel_core::Run,
    case_map: &HashMap<String, agentcarousel_core::Case>,
    case_file_map: &HashMap<String, PathBuf>,
    globals: &GlobalOptions,
) -> i32 {
    let mut entries: Vec<BootstrapEntry> = Vec::new();
    // fixture_path -> Vec<(case_id, abs golden_path)> — for YAML patching
    let mut patch_map: HashMap<PathBuf, Vec<(String, PathBuf)>> = HashMap::new();

    for case_result in &run.cases {
        let case_id = &case_result.case_id.0;

        let output = match &case_result.trace.final_output {
            Some(o) if !o.is_empty() => o.clone(),
            _ => {
                entries.push(BootstrapEntry {
                    case_id: case_id.clone(),
                    golden_path: PathBuf::new(),
                    skip_reason: Some("no output in run".to_string()),
                });
                continue;
            }
        };

        if let Some(case) = case_map.get(case_id) {
            if let Some(cfg) = &case.evaluator_config {
                let reason = if cfg.evaluator == "golden" {
                    "already has golden evaluator config"
                } else {
                    "has explicit evaluator config — remove it first to bootstrap golden"
                };
                let existing_path = cfg.golden_path.clone().unwrap_or_default();
                entries.push(BootstrapEntry {
                    case_id: case_id.clone(),
                    golden_path: existing_path,
                    skip_reason: Some(reason.to_string()),
                });
                continue;
            }
        }

        let fixture_path = match case_file_map.get(case_id) {
            Some(p) => p.clone(),
            None => {
                entries.push(BootstrapEntry {
                    case_id: case_id.clone(),
                    golden_path: PathBuf::new(),
                    skip_reason: Some("case not found in loaded fixtures".to_string()),
                });
                continue;
            }
        };

        let fixture_dir = fixture_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let golden_dir = fixture_dir.join("golden");
        let leaf = case_id.rsplit('/').next().unwrap_or(case_id.as_str());
        let golden_path = golden_dir.join(format!("{leaf}.txt"));

        if let Err(e) = std::fs::create_dir_all(&golden_dir) {
            eprintln!("error: failed to create {}: {e}", golden_dir.display());
            return ExitCode::RuntimeError.as_i32();
        }
        if let Err(e) = std::fs::write(&golden_path, &output) {
            eprintln!("error: failed to write {}: {e}", golden_path.display());
            return ExitCode::RuntimeError.as_i32();
        }

        entries.push(BootstrapEntry {
            case_id: case_id.clone(),
            golden_path: golden_path.clone(),
            skip_reason: None,
        });
        patch_map
            .entry(fixture_path)
            .or_default()
            .push((case_id.clone(), golden_path));
    }

    // Patch cases.yaml for every file that had at least one new golden written.
    let mut yaml_warnings: Vec<String> = Vec::new();
    for (fixture_path, patches) in &patch_map {
        if let Err(e) = patch_cases_yaml(fixture_path, patches) {
            yaml_warnings.push(format!("{}: {e}", fixture_path.display()));
        }
    }

    let written: Vec<&BootstrapEntry> =
        entries.iter().filter(|e| e.skip_reason.is_none()).collect();
    let skipped: Vec<&BootstrapEntry> =
        entries.iter().filter(|e| e.skip_reason.is_some()).collect();

    if globals.json {
        let cases_json: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                let mut obj = serde_json::json!({
                    "case_id": e.case_id,
                    "status": if e.skip_reason.is_none() { "bootstrapped" } else { "skipped" },
                });
                if !e.golden_path.as_os_str().is_empty() {
                    obj["golden_path"] =
                        serde_json::Value::String(e.golden_path.display().to_string());
                }
                if let Some(reason) = &e.skip_reason {
                    obj["reason"] = serde_json::Value::String(reason.clone());
                }
                obj
            })
            .collect();
        JsonOutput::ok(
            "promote",
            serde_json::json!({
                "run_id": run.id.0,
                "bootstrapped": written.len(),
                "skipped": skipped.len(),
                "cases": cases_json,
                "yaml_warnings": yaml_warnings,
            }),
        )
        .print();
        return ExitCode::Ok.as_i32();
    }

    if !globals.quiet {
        print_bootstrap_summary(&written, &skipped, &patch_map, &yaml_warnings);
    }

    ExitCode::Ok.as_i32()
}

/// Inject `evaluator_config: {evaluator: golden, golden_path: ...}` into each listed case.
/// Uses a serde_yaml round-trip — preserves structure but drops YAML comments.
fn patch_cases_yaml(path: &Path, patches: &[(String, PathBuf)]) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc: serde_yaml::Value = serde_yaml::from_str(&raw).map_err(|e| e.to_string())?;

    let cases = doc
        .get_mut("cases")
        .and_then(|v| v.as_sequence_mut())
        .ok_or("no 'cases' sequence in fixture")?;

    for case in cases.iter_mut() {
        let id = case
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if let Some((_, golden_path)) = patches.iter().find(|(pid, _)| *pid == id) {
            let golden_str = golden_path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");

            let mut cfg = serde_yaml::Mapping::new();
            cfg.insert(
                serde_yaml::Value::String("evaluator".into()),
                serde_yaml::Value::String("golden".into()),
            );
            cfg.insert(
                serde_yaml::Value::String("golden_path".into()),
                serde_yaml::Value::String(golden_str),
            );
            cfg.insert(
                serde_yaml::Value::String("golden_threshold".into()),
                serde_yaml::Value::Number(serde_yaml::Number::from(0.85_f64)),
            );

            if let Some(mapping) = case.as_mapping_mut() {
                mapping.insert(
                    serde_yaml::Value::String("evaluator_config".into()),
                    serde_yaml::Value::Mapping(cfg),
                );
            }
        }
    }

    let patched = serde_yaml::to_string(&doc).map_err(|e| e.to_string())?;
    std::fs::write(path, patched).map_err(|e| e.to_string())?;
    Ok(())
}

fn print_bootstrap_summary(
    written: &[&BootstrapEntry],
    skipped: &[&BootstrapEntry],
    patch_map: &HashMap<PathBuf, Vec<(String, PathBuf)>>,
    yaml_warnings: &[String],
) {
    println!();
    println!(
        "  {}   {} written{}",
        style("Bootstrap summary").bold(),
        written.len(),
        if skipped.is_empty() {
            String::new()
        } else {
            format!("  {} skipped", skipped.len())
        }
    );

    if !written.is_empty() {
        let col_w = written
            .iter()
            .map(|e| e.case_id.len())
            .max()
            .unwrap_or(20)
            .clamp(20, 50);
        let bar = "─".repeat(col_w + 2);
        println!("  ┌{}┬──────────────────────────────────────────────┐", bar);
        println!(
            "  │ {:<col_w$} │ Golden file                                  │",
            "Case",
            col_w = col_w,
        );
        println!("  ├{}┼──────────────────────────────────────────────┤", bar);
        for e in written {
            let path_str = e.golden_path.display().to_string();
            let path_display = if path_str.len() > 44 {
                format!("…{}", &path_str[path_str.len() - 43..])
            } else {
                path_str
            };
            println!(
                "  │ {:<col_w$} │ {:<44} │",
                e.case_id,
                path_display,
                col_w = col_w,
            );
        }
        println!("  └{}┴──────────────────────────────────────────────┘", bar);
    }

    if !skipped.is_empty() {
        println!();
        println!("  {}", style("Skipped").dim());
        for e in skipped {
            println!(
                "    {}  {}",
                style(&e.case_id).dim(),
                style(e.skip_reason.as_deref().unwrap_or("")).dim()
            );
        }
    }

    if !yaml_warnings.is_empty() {
        println!();
        for w in yaml_warnings {
            println!("  {} {}", style("warn:").yellow(), w);
        }
        println!(
            "  {}",
            style("  add evaluator_config manually to the cases above").yellow()
        );
    } else if !patch_map.is_empty() {
        println!();
        for (path, patches) in patch_map {
            println!(
                "  {} {} ({} case{})",
                style("patched").green(),
                path.display(),
                patches.len(),
                if patches.len() == 1 { "" } else { "s" }
            );
        }
    }

    if !written.is_empty() {
        println!();
        println!(
            "  {}",
            style("Next: agc eval <fixtures/> --evaluator golden").dim()
        );
    }
    println!();
}
