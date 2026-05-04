use agentcarousel_reporters::{fetch_run, list_runs};
use chrono::Utc;
use clap::Parser;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;

// In-crate copy for `cargo package`; keep aligned with repo `fixtures/schemas/skill-definition.schema.json`.
const SKILL_DEFINITION_SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/schemas/skill-definition.schema.json"
));

use super::exit_codes::ExitCode;
use super::GlobalOptions;

/// Export run(s) as evidence .tar.gz (run.json + fingerprints per run).
#[derive(Debug, Parser)]
pub struct ExportArgs {
    /// Run id to export (from `report list` or eval/test hint lines; omit with `--last`).
    #[arg(value_name = "RUN_ID")]
    run_id_positional: Option<String>,
    /// Export the N most recent runs (newest first). Typical: `1`, `5`, or `10` (max 50).
    #[arg(short = 'l', long, value_name = "N")]
    last: Option<usize>,
    /// Output path for a single run (default: `./agentcarousel-evidence-<run_id>.tar.gz`).
    #[arg(short = 'o', long)]
    out: Option<PathBuf>,
    /// With `--last`, write each tarball under this directory (created if missing; default: cwd).
    #[arg(short = 'd', long, value_name = "DIR")]
    out_dir: Option<PathBuf>,
}

const EXPORT_LAST_MAX: usize = 50;

pub fn run_export(args: ExportArgs, globals: &GlobalOptions) -> i32 {
    let run_id = globals.run_id.as_ref().or(args.run_id_positional.as_ref());
    match (run_id, args.last) {
        (Some(_), Some(_)) => {
            eprintln!("error: specify either RUN_ID or --last N, not both");
            ExitCode::RuntimeError.as_i32()
        }
        (None, None) => {
            eprintln!(
                "error: specify RUN_ID or --last N (e.g. export --last 5 --out-dir ./evidence)"
            );
            ExitCode::RuntimeError.as_i32()
        }
        (Some(run_id), None) => {
            if args.out_dir.is_some() {
                eprintln!("error: --out-dir is only valid with --last");
                return ExitCode::RuntimeError.as_i32();
            }
            match export_run_artifact(run_id, args.out.as_deref()) {
                Ok(path) => {
                    println!("created {}", path.display());
                    ExitCode::Ok.as_i32()
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::RuntimeError.as_i32()
                }
            }
        }
        (None, Some(n)) => {
            if args.out.is_some() {
                eprintln!("error: with --last, use --out-dir for the output directory (not --out)");
                return ExitCode::RuntimeError.as_i32();
            }
            match export_last_n(n, args.out_dir.as_deref()) {
                Ok(paths) => {
                    for path in paths {
                        println!("created {}", path.display());
                    }
                    ExitCode::Ok.as_i32()
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::RuntimeError.as_i32()
                }
            }
        }
    }
}

fn export_last_n(n: usize, out_dir: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    if n == 0 || n > EXPORT_LAST_MAX {
        return Err(format!(
            "--last N must be between 1 and {EXPORT_LAST_MAX} (got {n})"
        ));
    }
    let listings = list_runs(n).map_err(|e| e.to_string())?;
    if listings.is_empty() {
        println!("no runs recorded");
        return Ok(Vec::new());
    }
    if listings.len() < n {
        eprintln!(
            "note: only {} run(s) in history (requested {})",
            listings.len(),
            n
        );
    }
    let base = out_dir.unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for listing in listings {
        let path = base.join(format!("agentcarousel-evidence-{}.tar.gz", listing.id));
        paths.push(export_run_artifact(&listing.id, Some(&path))?);
    }
    Ok(paths)
}

pub(crate) fn export_run_artifact(run_id: &str, out: Option<&Path>) -> Result<PathBuf, String> {
    let run = fetch_run(run_id).map_err(|err| err.to_string())?;
    // Stage export files in a temp directory before archiving.
    let root = std::env::temp_dir().join(format!("agentcarousel-evidence-{run_id}"));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|err| err.to_string())?;
    }
    fs::create_dir_all(&root).map_err(|err| err.to_string())?;

    let run_json_path = root.join("run.json");
    write_json(&run_json_path, &run)?;

    let bundle_lock_path = root.join("fixture_bundle.lock");
    let schema_hash = fixture_schema_sha256();
    let bundle_lock = json!({
        "fixture_bundle_id": run.fixture_bundle_id,
        "fixture_bundle_version": run.fixture_bundle_version,
        "schema_hash": schema_hash
    });
    write_json(&bundle_lock_path, &bundle_lock)?;

    let env_path = root.join("environment_fingerprint.json");
    let mut env_payload = serde_json::Map::new();
    env_payload.insert(
        "agentcarousel_version".to_string(),
        json!(run.agentcarousel_version),
    );
    env_payload.insert("os".to_string(), json!(std::env::consts::OS));
    env_payload.insert("arch".to_string(), json!(std::env::consts::ARCH));
    env_payload.insert("rust_version".to_string(), json!("unknown"));
    env_payload.insert("timestamp_utc".to_string(), json!(Utc::now().to_rfc3339()));
    env_payload.insert("git_sha".to_string(), json!(run.git_sha));
    env_payload.insert(
        "fixture_bundle_id".to_string(),
        json!(run.fixture_bundle_id),
    );
    env_payload.insert(
        "fixture_bundle_version".to_string(),
        json!(run.fixture_bundle_version),
    );
    if let Ok(v) = std::env::var("GITHUB_REF") {
        let t = v.trim();
        if !t.is_empty() {
            env_payload.insert("github_ref".to_string(), json!(t));
        }
    }
    if let Ok(v) = std::env::var("GITHUB_RUN_ID") {
        let t = v.trim();
        if !t.is_empty() {
            env_payload.insert("github_run_id".to_string(), json!(t));
        }
    }
    write_json(&env_path, &serde_json::Value::Object(env_payload))?;

    let redaction_path = root.join("REDACTION_POLICY.md");
    let mut file = fs::File::create(&redaction_path).map_err(|err| err.to_string())?;
    file.write_all(b"Redaction policy: trace outputs are scrubbed of common secrets and tokens.\n")
        .map_err(|err| err.to_string())?;

    let manifest_path = root.join("MANIFEST.json");
    let manifest = build_manifest(&root)?;
    write_json(&manifest_path, &manifest)?;

    let out_path = out
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!("agentcarousel-evidence-{run_id}.tar.gz")));
    let archive = fs::File::create(&out_path).map_err(|err| err.to_string())?;
    let encoder = GzEncoder::new(archive, Compression::default());
    let mut tar = Builder::new(encoder);
    tar.append_dir_all(format!("agentcarousel-evidence-{run_id}"), &root)
        .map_err(|err| err.to_string())?;
    tar.finish().map_err(|err| err.to_string())?;
    fs::remove_dir_all(&root).ok();
    Ok(out_path)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    let mut file = fs::File::create(path).map_err(|err| err.to_string())?;
    file.write_all(payload.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn fixture_schema_sha256() -> String {
    let mut hasher = Sha256::new();
    hasher.update(SKILL_DEFINITION_SCHEMA.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Integrity manifest over evidence files (excludes `MANIFEST.json` itself).
fn build_manifest(root: &Path) -> Result<serde_json::Value, String> {
    let tracked = [
        "run.json",
        "fixture_bundle.lock",
        "environment_fingerprint.json",
        "REDACTION_POLICY.md",
    ];
    let mut files = Vec::new();
    for name in tracked {
        let path = root.join(name);
        let digest = sha256_file_hex(&path)?;
        files.push(json!({
            "path": name,
            "sha256": digest
        }));
    }
    Ok(json!({
        "manifest_version": 1,
        "files": files
    }))
}
