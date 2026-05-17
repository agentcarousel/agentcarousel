use clap::Parser;
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use ulid::Ulid;

use super::exit_codes::ExitCode;

const REPO: &str = "agentcarousel/agentcarousel";
const API_BASE: &str = "https://api.github.com";
const DL_BASE: &str = "https://github.com";

#[derive(Debug, Parser)]
/// Check for and install updates to the agentcarousel CLI.
pub struct UpdateArgs {
    /// Print whether an update is available without installing it.
    #[arg(long)]
    pub check: bool,
    /// Skip the confirmation prompt (useful for CI/non-interactive shells).
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run_update(args: UpdateArgs) -> i32 {
    match update(args) {
        Ok(()) => ExitCode::Ok.as_i32(),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn update(args: UpdateArgs) -> Result<(), String> {
    let current = env!("CARGO_PKG_VERSION");

    let tag = fetch_latest_tag()?;
    let latest = tag.trim_start_matches('v');

    if current == latest {
        println!("agentcarousel {current} is already up to date.");
        return Ok(());
    }

    if args.check {
        println!("update available: {current} → {latest}  (run `agentcarousel update` to install)");
        return Ok(());
    }

    println!("update available: {current} → {latest}");

    if !args.yes && !confirm_prompt()? {
        println!("aborted.");
        return Ok(());
    }

    let triple = target_triple()?;
    let asset = format!("agentcarousel-{tag}-{triple}.tar.gz");
    let tarball_url = format!("{DL_BASE}/{REPO}/releases/download/{tag}/{asset}");
    let sums_url = format!("{DL_BASE}/{REPO}/releases/download/{tag}/SHA256SUMS");

    println!("downloading {asset}...");
    let tarball = http_get_bytes(&tarball_url)?;

    if let Ok(sums) = http_get_string(&sums_url) {
        verify_checksum(&tarball, &asset, &sums)?;
    }

    let binary = extract_binary_from_tarball(&tarball)?;

    let current_exe =
        std::env::current_exe().map_err(|e| format!("could not locate current executable: {e}"))?;

    atomic_replace(&current_exe, &binary)?;

    println!("updated to agentcarousel {latest}.");
    Ok(())
}

fn fetch_latest_tag() -> Result<String, String> {
    let url = format!("{API_BASE}/repos/{REPO}/releases/latest");
    let body = http_get_string(&url)?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("failed to parse GitHub API response: {e}"))?;
    json.get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "GitHub API response missing tag_name".to_string())
}

fn target_triple() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        (os, arch) => Err(format!("unsupported platform: {os}/{arch}")),
    }
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent("agentcarousel-cli")
        .build()
        .expect("failed to build HTTP client")
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = http_client()
        .get(url)
        .send()
        .map_err(|e| format!("request failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP error ({url}): {e}"))?;
    let bytes = resp
        .bytes()
        .map_err(|e| format!("failed to read response body: {e}"))?;
    Ok(bytes.to_vec())
}

fn http_get_string(url: &str) -> Result<String, String> {
    http_client()
        .get(url)
        .send()
        .map_err(|e| format!("request failed ({url}): {e}"))?
        .error_for_status()
        .map_err(|e| format!("HTTP error ({url}): {e}"))?
        .text()
        .map_err(|e| format!("failed to read response body: {e}"))
}

fn verify_checksum(data: &[u8], asset_name: &str, sums: &str) -> Result<(), String> {
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches([' ', '*']);
            (name == asset_name).then(|| hash.to_string())
        })
        .ok_or_else(|| format!("no checksum entry found for {asset_name} in SHA256SUMS"))?;

    let mut hasher = Sha256::new();
    hasher.update(data);
    let got = crate::core::hex_util::hex_lower(hasher.finalize().as_ref());

    if got != expected {
        return Err(format!(
            "checksum mismatch for {asset_name}\n  expected: {expected}\n  got:      {got}"
        ));
    }
    Ok(())
}

fn extract_binary_from_tarball(tarball: &[u8]) -> Result<Vec<u8>, String> {
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut archive = tar::Archive::new(gz);

    for entry in archive
        .entries()
        .map_err(|e| format!("failed to read tarball entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("failed to read tarball entry: {e}"))?;
        let is_binary = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == "agentcarousel"))
            .unwrap_or(false);
        if is_binary {
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .map_err(|e| format!("failed to read binary from tarball: {e}"))?;
            return Ok(bytes);
        }
    }

    Err("archive did not contain an `agentcarousel` binary at its root".to_string())
}

fn atomic_replace(current_exe: &std::path::Path, new_binary: &[u8]) -> Result<(), String> {
    let parent = current_exe.parent().ok_or_else(|| {
        "could not determine parent directory of the current executable".to_string()
    })?;

    let tmp = parent.join(format!(".agentcarousel-update.{}", Ulid::new()));

    std::fs::write(&tmp, new_binary)
        .map_err(|e| format!("failed to write update to {}: {e}", tmp.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&tmp, perms)
            .map_err(|e| format!("failed to set permissions on update binary: {e}"))?;
    }

    std::fs::rename(&tmp, current_exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("failed to replace binary at {}: {e}", current_exe.display())
    })?;

    Ok(())
}

fn confirm_prompt() -> Result<bool, String> {
    print!("install update? [y/N] ");
    io::stdout()
        .flush()
        .map_err(|e| format!("stdout flush error: {e}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read input: {e}"))?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
