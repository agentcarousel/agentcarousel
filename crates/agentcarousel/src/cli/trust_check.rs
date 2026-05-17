use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::registry_client::{resolve_registry_url, RegistryClient};
use super::GlobalOptions;

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

/// Check a bundle's trust state in the registry and optionally verify its attestation.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc trust-check customer-support-1.0.0 --url https://registry.example.com\n  agc trust-check customer-support-1.0.0 --min-trust stable\n  agc trust-check customer-support-1.0.0 --attestation ./attestation.json --minisign-pubkey ./key.pub"
)]
pub struct TrustCheckArgs {
    /// Bundle selector as `<bundle-id>` or `<bundle-id>@<version>`.
    #[arg(value_name = "BUNDLE[@VERSION]")]
    target: String,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
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

pub fn run_trust_check(
    args: TrustCheckArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> i32 {
    match trust_check(&args, config, globals.json) {
        Ok(payload) => {
            if globals.json {
                JsonOutput::ok("trust-check", payload).print();
            }
            ExitCode::Ok.as_i32()
        }
        Err(TrustCheckError::BelowThreshold { current, required }) => {
            if globals.json {
                JsonOutput::err(
                    "trust-check",
                    JsonError::new(
                        "below_threshold",
                        format!(
                            "trust state below required threshold (current: {:?}, required: {:?})",
                            current, required
                        ),
                    ),
                )
                .print();
            } else {
                eprintln!(
                    "error: trust state below required threshold (current: {:?}, required: {:?})",
                    current, required
                );
            }
            ExitCode::Failed.as_i32()
        }
        Err(TrustCheckError::SignatureInvalid(msg)) => {
            if globals.json {
                JsonOutput::err("trust-check", JsonError::new("signature_invalid", msg)).print();
            } else {
                eprintln!("error: attestation signature invalid: {msg}");
            }
            ExitCode::Failed.as_i32()
        }
        Err(TrustCheckError::Runtime(msg)) => {
            if globals.json {
                JsonOutput::err("trust-check", JsonError::new("runtime_error", msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
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

fn trust_check(
    args: &TrustCheckArgs,
    config: &ResolvedConfig,
    json: bool,
) -> Result<Value, TrustCheckError> {
    let endpoint = resolve_registry_url(args.url.as_deref(), config)
        .map_err(|err| TrustCheckError::Runtime(err.to_string()))?;
    let registry_bundle_id = compute_registry_bundle_id(&args.target)
        .map_err(|err| TrustCheckError::Runtime(err.to_string()))?;

    let client = RegistryClient::new(&endpoint, "").map_err(TrustCheckError::Runtime)?;
    let payload = client
        .get_trust_state(&registry_bundle_id)
        .map_err(TrustCheckError::Runtime)?;
    let current = trust_tier_from_payload(&payload)?;

    if !json {
        print_summary(&payload, &registry_bundle_id, current);
    }

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
        verify_attestation(&args.minisign_bin, attestation, pubkey.path())?;
    }

    Ok(payload)
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

enum PubkeyHandle {
    UserPath(PathBuf),
    TempFile(NamedTempFile),
}

impl PubkeyHandle {
    fn path(&self) -> &Path {
        match self {
            PubkeyHandle::UserPath(p) => p.as_path(),
            PubkeyHandle::TempFile(f) => f.path(),
        }
    }
}

fn resolve_pubkey_path(input: &str) -> Result<PubkeyHandle, TrustCheckError> {
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
        let mut tmp = tempfile::Builder::new()
            .prefix("agentcarousel-minisign-")
            .suffix(".pub")
            .tempfile()
            .map_err(|err| {
                TrustCheckError::Runtime(format!("failed to create temp pubkey: {err}"))
            })?;
        std::io::Write::write_all(&mut tmp, body.as_bytes()).map_err(|err| {
            TrustCheckError::Runtime(format!("failed to write temp pubkey: {err}"))
        })?;
        return Ok(PubkeyHandle::TempFile(tmp));
    }
    Ok(PubkeyHandle::UserPath(PathBuf::from(trimmed)))
}

fn verify_attestation(
    minisign_bin: &str,
    attestation: &Path,
    pubkey: &Path,
) -> Result<(), TrustCheckError> {
    if std::path::Path::new(minisign_bin)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(TrustCheckError::Runtime(
            "--minisign-bin path must not contain '..'".to_string(),
        ));
    }
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
