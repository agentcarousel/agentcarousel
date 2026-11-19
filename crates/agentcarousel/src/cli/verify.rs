use clap::Parser;
use flate2::read::GzDecoder;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::exit_codes::ExitCode;
use super::export::sha256_file_hex;
use super::minisign::{self, MinisignError};
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const MANIFEST: &str = "MANIFEST.json";

/// Verify an evidence archive produced by `agc export`.
///
/// Recomputes the SHA-256 of every file listed in MANIFEST.json, rejects missing or
/// unlisted files, and — with --minisign-pubkey — checks the manifest signature.
/// Without a public key only integrity is checked: anyone who edits the files can
/// regenerate an unsigned manifest, so treat that result as "consistent", not "authentic".
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc verify agentcarousel-evidence-<run-id>.tar.gz\n  agc verify agentcarousel-evidence-<run-id>.tar.gz --minisign-pubkey agc.pub\n  agc verify ./extracted-evidence-dir --minisign-pubkey agc.pub"
)]
pub struct VerifyArgs {
    /// Evidence tarball (.tar.gz) or an extracted evidence directory.
    #[arg(value_name = "EVIDENCE")]
    evidence: PathBuf,
    /// minisign public key; when given, a valid MANIFEST.json.minisig is required.
    #[arg(long = "minisign-pubkey", value_name = "PATH")]
    minisign_pubkey: Option<PathBuf>,
    /// minisign binary name/path (default: minisign).
    #[arg(long, default_value = "minisign")]
    minisign_bin: String,
}

#[derive(Debug)]
enum VerifyError {
    /// Evidence was read but failed integrity or signature checks.
    Failed(Vec<String>),
    /// Evidence could not be read or the tool could not run.
    Runtime(String),
}

pub fn run_verify(args: VerifyArgs, globals: &GlobalOptions) -> i32 {
    match verify(&args) {
        Ok(report) => {
            if globals.json {
                JsonOutput::ok("verify", report).print();
            } else {
                let files = report["files_verified"].as_u64().unwrap_or(0);
                match report["status"].as_str() {
                    Some("verified") => {
                        println!("verified: {files} file(s) match MANIFEST.json; signature valid")
                    }
                    _ => {
                        println!("integrity ok: {files} file(s) match MANIFEST.json");
                        if report["signature"] == "present_unverified" {
                            println!("note: MANIFEST.json.minisig present but not checked; pass --minisign-pubkey to verify authenticity");
                        } else {
                            println!("note: manifest is unsigned; integrity alone does not prove the files were not regenerated");
                        }
                    }
                }
            }
            ExitCode::Ok.as_i32()
        }
        Err(VerifyError::Failed(problems)) => {
            if globals.json {
                JsonOutput::err(
                    "verify",
                    JsonError::new("verification_failed", problems.join("; ")),
                )
                .print();
            } else {
                eprintln!("verification FAILED:");
                for p in &problems {
                    eprintln!("  - {p}");
                }
            }
            ExitCode::Failed.as_i32()
        }
        Err(VerifyError::Runtime(msg)) => {
            if globals.json {
                JsonOutput::err("verify", JsonError::new("runtime_error", msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn verify(args: &VerifyArgs) -> Result<Value, VerifyError> {
    // Keep the temp dir alive for the duration of verification.
    let mut _extracted: Option<tempfile::TempDir> = None;
    let search_root = if args.evidence.is_dir() {
        args.evidence.clone()
    } else if args.evidence.is_file() {
        let tmp = tempfile::tempdir().map_err(|e| VerifyError::Runtime(e.to_string()))?;
        extract_tar_gz(&args.evidence, tmp.path())?;
        let p = tmp.path().to_path_buf();
        _extracted = Some(tmp);
        p
    } else {
        return Err(VerifyError::Runtime(format!(
            "evidence not found: {}",
            args.evidence.display()
        )));
    };
    let root = locate_bundle_root(&search_root)?;
    let manifest_path = root.join(MANIFEST);
    let raw = fs::read(&manifest_path).map_err(|e| VerifyError::Runtime(e.to_string()))?;
    let manifest: Value = serde_json::from_slice(&raw)
        .map_err(|e| VerifyError::Failed(vec![format!("MANIFEST.json is not valid JSON: {e}")]))?;

    let mut problems = Vec::new();
    let listed = check_manifest_entries(&root, &manifest, &mut problems);
    check_unlisted_files(&root, &listed, &mut problems)?;

    let sig_path = minisign::default_sig_path(&manifest_path);
    let signature = match args.minisign_pubkey.as_deref() {
        Some(pubkey) => {
            if !sig_path.exists() {
                problems.push(
                    "MANIFEST.json.minisig missing but --minisign-pubkey was given".to_string(),
                );
                "missing"
            } else {
                match minisign::verify_file(
                    &args.minisign_bin,
                    &manifest_path,
                    Some(&sig_path),
                    pubkey,
                ) {
                    Ok(()) => "valid",
                    Err(MinisignError::Invalid(msg)) => {
                        problems.push(format!("manifest signature invalid: {msg}"));
                        "invalid"
                    }
                    Err(MinisignError::Runtime(msg)) => return Err(VerifyError::Runtime(msg)),
                }
            }
        }
        None if sig_path.exists() => "present_unverified",
        None => "absent",
    };

    if !problems.is_empty() {
        return Err(VerifyError::Failed(problems));
    }
    let status = if signature == "valid" {
        "verified"
    } else {
        "integrity_only"
    };
    Ok(json!({
        "evidence": args.evidence.display().to_string(),
        "status": status,
        "signature": signature,
        "files_verified": listed.len(),
    }))
}

/// Extract only regular files and directories; `unpack_in` refuses paths that
/// escape `dest`, and links are rejected outright.
fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), VerifyError> {
    let file = fs::File::open(archive).map_err(|e| VerifyError::Runtime(e.to_string()))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    let entries = tar
        .entries()
        .map_err(|e| VerifyError::Runtime(format!("not a readable .tar.gz: {e}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| VerifyError::Runtime(format!("not a readable .tar.gz: {e}")))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            let name = entry
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            return Err(VerifyError::Failed(vec![format!(
                "archive contains a non-regular entry: {name}"
            )]));
        }
        let ok = entry
            .unpack_in(dest)
            .map_err(|e| VerifyError::Runtime(format!("extract failed: {e}")))?;
        if !ok {
            return Err(VerifyError::Failed(vec![
                "archive contains a path outside the evidence directory".to_string(),
            ]));
        }
    }
    Ok(())
}

/// The bundle root is the directory holding MANIFEST.json: either `dir` itself or
/// its single child directory (the layout `agc export` produces).
fn locate_bundle_root(dir: &Path) -> Result<PathBuf, VerifyError> {
    if dir.join(MANIFEST).is_file() {
        return Ok(dir.to_path_buf());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| VerifyError::Runtime(e.to_string()))? {
        let entry = entry.map_err(|e| VerifyError::Runtime(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() && path.join(MANIFEST).is_file() {
            candidates.push(path);
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(VerifyError::Failed(vec![
            "MANIFEST.json not found in evidence".to_string(),
        ])),
        _ => Err(VerifyError::Failed(vec![
            "multiple MANIFEST.json bundles found; verify one directory at a time".to_string(),
        ])),
    }
}

fn is_plain_relative(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

fn check_manifest_entries(
    root: &Path,
    manifest: &Value,
    problems: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut listed = BTreeSet::new();
    if manifest.get("manifest_version").and_then(Value::as_u64) != Some(1) {
        problems.push("unsupported or missing manifest_version (expected 1)".to_string());
    }
    let Some(files) = manifest.get("files").and_then(Value::as_array) else {
        problems.push("MANIFEST.json has no files array".to_string());
        return listed;
    };
    for entry in files {
        let (Some(path), Some(expected)) = (
            entry.get("path").and_then(Value::as_str),
            entry.get("sha256").and_then(Value::as_str),
        ) else {
            problems.push(format!("malformed manifest entry: {entry}"));
            continue;
        };
        if !is_plain_relative(path) {
            problems.push(format!(
                "manifest path is not a plain relative path: {path}"
            ));
            continue;
        }
        if !listed.insert(path.to_string()) {
            problems.push(format!("duplicate manifest entry: {path}"));
            continue;
        }
        let file = root.join(path);
        if !file.is_file() {
            problems.push(format!("missing file: {path}"));
            continue;
        }
        match sha256_file_hex(&file) {
            Ok(actual) if actual == expected => {}
            Ok(_) => problems.push(format!("hash mismatch: {path}")),
            Err(err) => problems.push(format!("cannot read {path}: {err}")),
        }
    }
    listed
}

fn check_unlisted_files(
    root: &Path,
    listed: &BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Result<(), VerifyError> {
    let sig_name = format!("{MANIFEST}{}", minisign::SIG_SUFFIX);
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|e| VerifyError::Runtime(e.to_string()))?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| VerifyError::Runtime(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if rel == MANIFEST || rel == sig_name || listed.contains(&rel) {
            continue;
        }
        problems.push(format!("file not listed in manifest: {rel}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_relative_paths() {
        assert!(is_plain_relative("run.json"));
        assert!(is_plain_relative("sub/run.json"));
        assert!(!is_plain_relative("../run.json"));
        assert!(!is_plain_relative("/etc/passwd"));
        assert!(!is_plain_relative(""));
    }
}
