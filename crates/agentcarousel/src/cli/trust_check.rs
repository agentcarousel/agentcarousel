use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::registry_client::RegistryClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum TrustTier {
    Experimental,
    CarouselCandidate,
    Stable,
    Trusted,
}

impl TrustTier {
    fn from_registry_value(value: &str) -> Option<Self> {
        match value {
            "Experimental" => Some(Self::Experimental),
            "CarouselCandidate" => Some(Self::CarouselCandidate),
            "Stable" => Some(Self::Stable),
            "Trusted" => Some(Self::Trusted),
            _ => None,
        }
    }
}

/// Check trust state for a bundle and optionally verify attestation signature offline.
#[derive(Debug, Parser)]
pub struct TrustCheckArgs {
    /// Bundle selector as `<bundle-id>` or `<bundle-id>@<version>`.
    #[arg(value_name = "BUNDLE[@VERSION]")]
    target: String,
    /// Registry API URL (alias: --registry-url). Falls back to config/env.
    #[arg(long = "url", alias = "registry-url")]
    url: Option<String>,
    /// Minimum required trust state (default: trusted).
    #[arg(long, default_value = "trusted")]
    min_trust: TrustTier,
    /// Optional local attestation JSON file to verify with minisign.
    #[arg(long)]
    attestation: Option<PathBuf>,
    /// minisign public key path (local file or URL) used when --attestation is provided.
    #[arg(long = "minisign-pubkey")]
    minisign_pubkey: Option<String>,
    /// minisign binary name/path (default: minisign).
    #[arg(long, default_value = "minisign")]
    minisign_bin: String,
}

pub fn run_trust_check(args: TrustCheckArgs, config: &ResolvedConfig) -> i32 {
    match trust_check(args, config) {
        Ok(()) => ExitCode::Ok.as_i32(),
        Err(TrustCheckError::BelowThreshold { current, required }) => {
            eprintln!(
                "error: trust state below required threshold (current: {:?}, required: {:?})",
                current, required
            );
            ExitCode::Failed.as_i32()
        }
        Err(TrustCheckError::SignatureInvalid(msg)) => {
            eprintln!("error: attestation signature invalid: {msg}");
            ExitCode::Failed.as_i32()
        }
        Err(TrustCheckError::Runtime(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

#[derive(Debug)]
enum TrustCheckError {
    BelowThreshold {
        current: TrustTier,
        required: TrustTier,
    },
    SignatureInvalid(String),
    Runtime(String),
}

fn trust_check(args: TrustCheckArgs, config: &ResolvedConfig) -> Result<(), TrustCheckError> {
    let endpoint = resolve_registry_url(args.url.as_deref(), config)
        .map_err(|err| TrustCheckError::Runtime(err.to_string()))?;
    let registry_bundle_id = compute_registry_bundle_id(&args.target)
        .map_err(|err| TrustCheckError::Runtime(err.to_string()))?;

    let client = RegistryClient::new(&endpoint, "").map_err(TrustCheckError::Runtime)?;
    let payload = client
        .get_trust_state(&registry_bundle_id)
        .map_err(TrustCheckError::Runtime)?;
    let current = trust_tier_from_payload(&payload)?;

    print_summary(&payload, &registry_bundle_id, current);

    if current < args.min_trust {
        return Err(TrustCheckError::BelowThreshold {
            current,
            required: args.min_trust,
        });
    }

    if let Some(attestation) = args.attestation.as_deref() {
        let pubkey = args
            .minisign_pubkey
            .as_deref()
            .ok_or_else(|| {
                TrustCheckError::Runtime(
                    "--minisign-pubkey is required when --attestation is provided".to_string(),
                )
            })
            .and_then(resolve_pubkey_path)?;
        verify_attestation(&args.minisign_bin, attestation, &pubkey)?;
    }

    Ok(())
}

fn resolve_registry_url(
    registry_url: Option<&str>,
    config: &ResolvedConfig,
) -> Result<String, String> {
    if let Some(url) = registry_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(url) = &config.msp.registry_endpoint {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    for key in ["REGISTRY_API_BASE_URL", "REGISTRY_URL"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    Err("registry URL is required: pass --url/--registry-url, set msp.registry_endpoint in config, or export REGISTRY_API_BASE_URL".to_string())
}

fn compute_registry_bundle_id(target: &str) -> Result<String, String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err("bundle target is required".to_string());
    }
    let Some((bundle, version)) = trimmed.split_once('@') else {
        return Ok(trimmed.to_string());
    };
    let bundle = bundle.trim();
    let version = version.trim();
    if bundle.is_empty() || version.is_empty() {
        return Err(format!(
            "invalid target `{trimmed}`; use <bundle-id> or <bundle-id>@<version>"
        ));
    }
    Ok(format!("{bundle}-{version}"))
}

fn trust_tier_from_payload(payload: &Value) -> Result<TrustTier, TrustCheckError> {
    let raw = payload
        .get("trust_state")
        .and_then(Value::as_str)
        .or_else(|| payload.get("state").and_then(Value::as_str))
        .ok_or_else(|| {
            TrustCheckError::Runtime("registry response missing trust_state field".to_string())
        })?;
    TrustTier::from_registry_value(raw).ok_or_else(|| {
        TrustCheckError::Runtime(format!(
            "unsupported trust_state value from registry: {raw}"
        ))
    })
}

fn print_summary(payload: &Value, bundle_id: &str, state: TrustTier) {
    let raw_state = payload
        .get("trust_state")
        .and_then(Value::as_str)
        .unwrap_or(match state {
            TrustTier::Experimental => "Experimental",
            TrustTier::CarouselCandidate => "CarouselCandidate",
            TrustTier::Stable => "Stable",
            TrustTier::Trusted => "Trusted",
        });
    println!("{bundle_id}: {raw_state}");

    if let Some(certified) = payload.get("certified_at").and_then(Value::as_str) {
        if let Some(expires) = payload.get("expires_at").and_then(Value::as_str) {
            println!("  Certified: {certified}  Expires: {expires}");
        } else {
            println!("  Certified: {certified}");
        }
    }
    if let Some(auditor) = payload.get("auditor").and_then(Value::as_str) {
        println!("  Auditor: {auditor}");
    }
    if let Some(url) = payload.get("attestation_url").and_then(Value::as_str) {
        println!("  Attestation: {url}");
    }
}

fn resolve_pubkey_path(input: &str) -> Result<PathBuf, TrustCheckError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(TrustCheckError::Runtime(
            "minisign public key path cannot be empty".to_string(),
        ));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let body = reqwest::blocking::get(trimmed)
            .map_err(|err| TrustCheckError::Runtime(format!("failed to fetch pubkey URL: {err}")))?
            .text()
            .map_err(|err| {
                TrustCheckError::Runtime(format!("failed to read pubkey response body: {err}"))
            })?;
        let tmp_path =
            std::env::temp_dir().join(format!("agentcarousel-minisign-{}.pub", std::process::id()));
        std::fs::write(&tmp_path, body).map_err(|err| {
            TrustCheckError::Runtime(format!("failed to write temp pubkey: {err}"))
        })?;
        return Ok(tmp_path);
    }
    Ok(PathBuf::from(trimmed))
}

fn verify_attestation(
    minisign_bin: &str,
    attestation: &Path,
    pubkey: &Path,
) -> Result<(), TrustCheckError> {
    if !attestation.exists() {
        return Err(TrustCheckError::Runtime(format!(
            "attestation file not found: {}",
            attestation.display()
        )));
    }
    if !pubkey.exists() {
        return Err(TrustCheckError::Runtime(format!(
            "minisign public key file not found: {}",
            pubkey.display()
        )));
    }
    let output = Command::new(minisign_bin)
        .arg("-Vm")
        .arg(attestation)
        .arg("-p")
        .arg(pubkey)
        .output()
        .map_err(|err| {
            TrustCheckError::Runtime(format!(
                "failed to run `{minisign_bin}`: {err}. Install minisign or pass --minisign-bin"
            ))
        })?;
    if output.status.success() {
        println!("  Signature: [SIGNATURE VALID]");
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    Err(TrustCheckError::SignatureInvalid(detail))
}
