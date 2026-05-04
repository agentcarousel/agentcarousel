use agentcarousel_reporters::{fetch_run, list_runs};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::export::export_run_artifact;
use super::registry_client::{resolve_registry_url, RegistryClient};
use super::GlobalOptions;

/// Publish a bundle to registry (register + submit run evidence).
#[derive(Debug, Parser)]
pub struct PublishArgs {
    /// Bundle directory or explicit bundle.manifest.json path.
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,
    /// Registry API URL (alias: --registry-url). Falls back to config/env.
    #[arg(long = "url", alias = "registry-url")]
    url: Option<String>,
    /// Explicit run id (default: latest run matching manifest bundle_id + bundle_version).
    #[arg(short = 'r', long)]
    run_id: Option<String>,
    /// Submit all runs matching manifest bundle_id + bundle_version.
    #[arg(short = 'a', long, default_value_t = false)]
    all_runs: bool,
    /// Limit number of matching runs processed when --all-runs is set (newest first).
    #[arg(short = 'l', long)]
    limit: Option<usize>,
    /// Explicit evidence tarball path.
    #[arg(short = 'e', long)]
    evidence: Option<PathBuf>,
    /// Resolve and print values without API writes.
    #[arg(short = 'n', long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct BundleManifestMeta {
    bundle_id: String,
    bundle_version: String,
    skill_or_agent: Option<String>,
}

pub fn run_publish(args: PublishArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    match publish_bundle(args, config, globals) {
        Ok(payload) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
            );
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn publish_bundle(
    args: PublishArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> Result<Value, String> {
    let (manifest, meta, _root) = load_bundle_manifest(&args.path)?;
    let endpoint = resolve_registry_url(args.url.as_deref(), config)?;
    let registry_bundle_id = compute_registry_bundle_id(&meta);
    if args.all_runs && args.run_id.is_some() {
        return Err("cannot combine --all-runs with --run-id".to_string());
    }
    if args.all_runs && args.evidence.is_some() {
        return Err("cannot combine --all-runs with --evidence (single file)".to_string());
    }

    let token = if args.dry_run {
        None
    } else {
        Some(resolve_registry_token()?)
    };

    let selected_run_ids = if args.all_runs {
        matching_run_ids(&meta, config, args.limit)?
    } else {
        vec![match args.run_id.as_deref().or(globals.run_id.as_deref()) {
            Some(id) => id.to_string(),
            None => latest_matching_run_id(&meta, config)?,
        }]
    };

    if args.dry_run {
        let selected_run_id = selected_run_ids.first().cloned().unwrap_or_default();
        let evidence_hint = if let Some(path) = &args.evidence {
            path.clone()
        } else if selected_run_id.is_empty() {
            PathBuf::from("<none>")
        } else {
            locate_or_export_evidence(&selected_run_id, meta.skill_or_agent.as_deref())?
        };
        return Ok(json!({
            "mode": "dry-run",
            "registry_url": endpoint,
            "registry_bundle_id": registry_bundle_id,
            "run_id": selected_run_id,
            "run_ids": selected_run_ids,
            "run_count": selected_run_ids.len(),
            "evidence": evidence_hint.display().to_string(),
            "bundle_id": meta.bundle_id,
            "bundle_version": meta.bundle_version
        }));
    }

    let token = token.expect("token is resolved for non-dry-run path");
    let client = RegistryClient::new(&endpoint, &token)?;
    let bundle_response = client.push_bundle_manifest(&manifest)?;

    let mut run_results = Vec::new();
    for run_id in &selected_run_ids {
        let evidence_path = if let Some(path) = &args.evidence {
            path.clone()
        } else {
            locate_or_export_evidence(run_id, meta.skill_or_agent.as_deref())?
        };
        if !evidence_path.exists() {
            return Err(format!(
                "evidence file not found for run {}: {}",
                run_id,
                evidence_path.display()
            ));
        }
        let run_response = client.submit_run_evidence(&registry_bundle_id, &evidence_path)?;
        let status = if run_response
            .get("duplicate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "already_uploaded"
        } else {
            "uploaded"
        };
        run_results.push(json!({
            "run_id": run_id,
            "status": status,
            "evidence_path": evidence_path.display().to_string(),
            "response": run_response
        }));
    }

    let trust_state_url = format!("{}/v1/bundles/{}/trust-state", endpoint, registry_bundle_id);
    Ok(json!({
        "bundle": bundle_response,
        "runs": run_results,
        "selected_run_id": selected_run_ids.first().cloned().unwrap_or_default(),
        "selected_run_ids": selected_run_ids,
        "registry_bundle_id": registry_bundle_id,
        "trust_state_url": trust_state_url
    }))
}

fn resolve_registry_token() -> Result<String, String> {
    if let Ok(value) = std::env::var("AGENTCAROUSEL_API_TOKEN") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    Err("registry token missing: export AGENTCAROUSEL_API_TOKEN".to_string())
}

fn load_bundle_manifest(path: &Path) -> Result<(Value, BundleManifestMeta, PathBuf), String> {
    let (manifest_path, root) = resolve_manifest_path(path)?;
    let contents = fs::read_to_string(&manifest_path).map_err(|err| err.to_string())?;
    let manifest: Value = serde_json::from_str(&contents).map_err(|err| err.to_string())?;
    let meta: BundleManifestMeta =
        serde_json::from_value(manifest.clone()).map_err(|err| err.to_string())?;
    Ok((manifest, meta, root))
}

fn resolve_manifest_path(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) != Some("bundle.manifest.json") {
            return Err(format!(
                "expected bundle directory or bundle.manifest.json path, got file {}",
                path.display()
            ));
        }
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        return Ok((path.to_path_buf(), root));
    }
    let manifest_path = path.join("bundle.manifest.json");
    if !manifest_path.exists() {
        return Err(format!(
            "bundle.manifest.json not found at {}",
            manifest_path.display()
        ));
    }
    Ok((manifest_path, path.to_path_buf()))
}

fn compute_registry_bundle_id(meta: &BundleManifestMeta) -> String {
    if let Some(skill) = &meta.skill_or_agent {
        return format!("{skill}-{}", meta.bundle_version);
    }
    format!(
        "{}-{}",
        meta.bundle_id.replace('/', "-"),
        meta.bundle_version
    )
}

fn latest_matching_run_id(
    meta: &BundleManifestMeta,
    config: &ResolvedConfig,
) -> Result<String, String> {
    let mut ids = matching_run_ids(meta, config, Some(1))?;
    if let Some(id) = ids.pop() {
        return Ok(id);
    }
    Err(format!(
        "no run found for bundle {}@{}; pass --run-id (-r) explicitly or run `agentcarousel report list`",
        meta.bundle_id, meta.bundle_version
    ))
}

fn matching_run_ids(
    meta: &BundleManifestMeta,
    config: &ResolvedConfig,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let history_limit = config.report.max_history_runs.unwrap_or(500) as usize;
    let listings = list_runs(history_limit).map_err(|err| err.to_string())?;
    let mut ids = Vec::new();
    let mut skipped = 0usize;
    for listing in listings {
        let run = match fetch_run(&listing.id) {
            Ok(run) => run,
            Err(err) => {
                skipped += 1;
                eprintln!(
                    "warning: skipping unreadable run {} from history: {}",
                    listing.id, err
                );
                continue;
            }
        };
        if run.fixture_bundle_id.as_deref() == Some(meta.bundle_id.as_str())
            && run.fixture_bundle_version.as_deref() == Some(meta.bundle_version.as_str())
        {
            ids.push(listing.id);
            if let Some(max) = limit {
                if ids.len() >= max {
                    break;
                }
            }
        }
    }
    if ids.is_empty() {
        if skipped > 0 {
            let skipped_label = if skipped == 1 {
                "entry was"
            } else {
                "entries were"
            };
            return Err(format!(
                "no readable run found for bundle {}@{} ({} unreadable history {} skipped); run eval/test again or pass --run-id (-r)",
                meta.bundle_id,
                meta.bundle_version,
                skipped,
                skipped_label
            ));
        }
        return Err(format!(
            "no run found for bundle {}@{}; run eval/test first or pass --run-id (-r)",
            meta.bundle_id, meta.bundle_version
        ));
    }
    Ok(ids)
}

fn locate_or_export_evidence(
    run_id: &str,
    skill_or_agent: Option<&str>,
) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(skill) = skill_or_agent {
        candidates.push(PathBuf::from(format!(
            "reports/evidence-packs/{skill}/agentcarousel-evidence-{run_id}.tar.gz"
        )));
    }
    candidates.push(PathBuf::from(format!(
        "reports/evidence-packs/agentcarousel-evidence-{run_id}.tar.gz"
    )));
    candidates.push(PathBuf::from(format!(
        "agentcarousel-evidence-{run_id}.tar.gz"
    )));
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.to_path_buf());
        }
    }
    export_run_artifact(run_id, None)
}
