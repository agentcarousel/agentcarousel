//! Thin wrapper around the external `minisign` binary.
//!
//! Used to sign evidence manifests (`agc export --sign-key`) and to verify
//! signatures (`agc verify`, `agc trust-check --attestation`). Shelling out keeps
//! key handling (including password-protected secret keys) inside minisign.

use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// Extension minisign uses for detached signatures.
pub const SIG_SUFFIX: &str = ".minisig";

#[derive(Debug)]
pub enum MinisignError {
    /// minisign ran and reported the signature as invalid.
    Invalid(String),
    /// minisign could not run, or inputs were missing.
    Runtime(String),
}

impl std::fmt::Display for MinisignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MinisignError::Invalid(msg) | MinisignError::Runtime(msg) => f.write_str(msg),
        }
    }
}

/// Path of the detached signature minisign writes for `file` by default.
pub fn default_sig_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(SIG_SUFFIX);
    PathBuf::from(s)
}

fn check_bin(bin: &str) -> Result<(), MinisignError> {
    if Path::new(bin)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(MinisignError::Runtime(
            "--minisign-bin path must not contain '..'".to_string(),
        ));
    }
    Ok(())
}

fn spawn_err(bin: &str, err: std::io::Error) -> MinisignError {
    MinisignError::Runtime(format!(
        "failed to run `{bin}`: {err}. Install minisign or pass --minisign-bin"
    ))
}

fn output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Sign `file` with the minisign secret key at `secret_key`, writing
/// `<file>.minisig`. Password-protected keys prompt on the terminal; for CI use
/// a key generated with `minisign -G -W`.
pub fn sign_file(
    bin: &str,
    secret_key: &Path,
    file: &Path,
    trusted_comment: &str,
) -> Result<PathBuf, MinisignError> {
    check_bin(bin)?;
    if !secret_key.exists() {
        return Err(MinisignError::Runtime(format!(
            "minisign secret key file not found: {}",
            secret_key.display()
        )));
    }
    let sig = default_sig_path(file);
    let output = Command::new(bin)
        .arg("-S")
        .arg("-s")
        .arg(secret_key)
        .arg("-m")
        .arg(file)
        .arg("-x")
        .arg(&sig)
        .arg("-t")
        .arg(trusted_comment)
        .stdin(Stdio::inherit())
        .output()
        .map_err(|err| spawn_err(bin, err))?;
    if output.status.success() && sig.exists() {
        return Ok(sig);
    }
    Err(MinisignError::Runtime(format!(
        "minisign signing failed: {}",
        output_detail(&output)
    )))
}

/// Verify `file` against a detached signature (`sig`, or `<file>.minisig` when
/// `None`) using the public key at `pubkey`.
pub fn verify_file(
    bin: &str,
    file: &Path,
    sig: Option<&Path>,
    pubkey: &Path,
) -> Result<(), MinisignError> {
    check_bin(bin)?;
    if !file.exists() {
        return Err(MinisignError::Runtime(format!(
            "file not found: {}",
            file.display()
        )));
    }
    if !pubkey.exists() {
        return Err(MinisignError::Runtime(format!(
            "minisign public key file not found: {}",
            pubkey.display()
        )));
    }
    let mut cmd = Command::new(bin);
    cmd.arg("-V").arg("-m").arg(file);
    if let Some(sig) = sig {
        cmd.arg("-x").arg(sig);
    }
    cmd.arg("-p").arg(pubkey);
    let output = cmd.output().map_err(|err| spawn_err(bin, err))?;
    if output.status.success() {
        return Ok(());
    }
    Err(MinisignError::Invalid(output_detail(&output)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sig_path_appends_suffix() {
        assert_eq!(
            default_sig_path(Path::new("/x/MANIFEST.json")),
            PathBuf::from("/x/MANIFEST.json.minisig")
        );
    }

    #[test]
    fn rejects_parent_dir_bin() {
        let err = verify_file("../minisign", Path::new("a"), None, Path::new("b")).unwrap_err();
        assert!(matches!(err, MinisignError::Runtime(_)));
    }
}
