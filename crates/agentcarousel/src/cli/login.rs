// agc-a0ru: agc login / logout with credential store
use clap::{Parser, Subcommand};

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const KEYCHAIN_SERVICE: &str = "agentcarousel";
const CREDENTIALS_FILE_NAME: &str = "credentials.toml";

#[derive(Debug, Parser)]
#[command(
    about = "Save registry credentials so agc can publish and pull bundles.",
    long_about = "Save registry credentials so agc can publish and pull bundles.\n\nYour token is stored securely in the OS credential store (macOS Keychain on Mac) and is read automatically by agc publish, agc bundle pull, and agc compare --registry. Use agc logout to remove stored credentials.",
    after_help = "Examples:\n  agc login --token agct_abc123\n  agc login --token agct_abc123 --url https://registry.agentcarousel.com\n  agc logout    # remove stored credentials"
)]
pub struct LoginArgs {
    #[command(subcommand)]
    command: Option<LoginCommand>,

    /// Personal access token (starts with agct_).
    #[arg(long)]
    token: Option<String>,

    /// Registry URL to store alongside the token.
    #[arg(long)]
    url: Option<String>,
}

#[derive(Debug, Subcommand)]
enum LoginCommand {
    /// Remove stored credentials.
    Logout,
}

pub fn run_login(args: LoginArgs, globals: &GlobalOptions) -> i32 {
    if let Some(LoginCommand::Logout) = args.command {
        return run_logout(globals);
    }
    let token = match args.token {
        Some(t) => t,
        None => {
            if globals.json {
                JsonOutput::err(
                    "login",
                    JsonError::new("missing_token", "Provide a token with --token <token>"),
                )
                .print();
            } else {
                eprintln!("error: provide a token with --token <token>");
                eprintln!("  hint: agc login --token agct_...");
            }
            return ExitCode::ValidationFailed.as_i32();
        }
    };

    let token = token.trim().to_string();
    if token.is_empty() {
        if globals.json {
            JsonOutput::err(
                "login",
                JsonError::new("empty_token", "Token must not be empty"),
            )
            .print();
        } else {
            eprintln!("error: token must not be empty");
        }
        return ExitCode::ValidationFailed.as_i32();
    }

    // Optionally verify the token against /v1/tokens/me before storing
    let user_ref = if let Some(ref url) = args.url {
        verify_token_against_registry(url, &token)
    } else if let Ok(env_url) = std::env::var("AGENTCAROUSEL_REGISTRY_URL")
        .or_else(|_| std::env::var("REGISTRY_API_BASE_URL"))
    {
        if !env_url.trim().is_empty() {
            verify_token_against_registry(env_url.trim(), &token)
        } else {
            None
        }
    } else {
        None
    };

    match store_token(&token) {
        Ok(()) => {
            let identity = user_ref.as_deref().unwrap_or("(unverified)");
            if globals.json {
                JsonOutput::ok(
                    "login",
                    serde_json::json!({ "status": "ok", "user_ref": identity }),
                )
                .print();
            } else {
                println!("logged in as {identity}");
                println!("  token stored in credential store");
            }
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            if globals.json {
                JsonOutput::err("login", JsonError::new("store_error", err)).print();
            } else {
                eprintln!("error: {err}");
            }
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn run_logout(globals: &GlobalOptions) -> i32 {
    match delete_stored_token() {
        Ok(()) => {
            if globals.json {
                JsonOutput::ok("logout", serde_json::json!({ "status": "ok" })).print();
            } else {
                println!("logged out — credentials removed");
            }
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            if globals.json {
                JsonOutput::err("logout", JsonError::new("delete_error", err)).print();
            } else {
                eprintln!("error: {err}");
            }
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn verify_token_against_registry(url: &str, token: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let me_url = format!("{}/v1/tokens/me", url.trim_end_matches('/'));
    let res = client
        .get(&me_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let body: serde_json::Value = res.json().ok()?;
    body.get("user_ref")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ── Credential store ─────────────────────────────────────────────────────────

/// Store token in OS credential store (macOS Keychain) or fallback file.
pub fn store_token(token: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if keychain_store(token).is_ok() {
            return Ok(());
        }
    }
    file_store(token)
}

/// Load token from credential store.
pub fn load_stored_token() -> Option<String> {
    #[cfg(target_os = "macos")]
    if let Some(t) = keychain_load() {
        return Some(t);
    }
    file_load()
}

/// Delete token from credential store.
pub fn delete_stored_token() -> Result<(), String> {
    let mut any_ok = false;
    #[cfg(target_os = "macos")]
    {
        if keychain_delete().is_ok() {
            any_ok = true;
        }
    }
    if file_delete().is_ok() {
        any_ok = true;
    }
    if any_ok {
        Ok(())
    } else {
        Err("no stored credentials found".to_string())
    }
}

// ── macOS Keychain ────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn keychain_store(token: &str) -> Result<(), String> {
    let out = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_SERVICE,
            "-w",
            token,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(target_os = "macos")]
fn keychain_load() -> Option<String> {
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_SERVICE,
            "-w",
        ])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn keychain_delete() -> Result<(), String> {
    let out = std::process::Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_SERVICE,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ── File-based fallback ───────────────────────────────────────────────────────

fn credentials_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("agentcarousel")
        .join(CREDENTIALS_FILE_NAME)
}

fn file_store(token: &str) -> Result<(), String> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create credentials directory: {e}"))?;
        // Mode 700 on the directory
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let content = format!("token = \"{token}\"\n");
    std::fs::write(&path, content)
        .map_err(|e| format!("cannot write credentials file {}: {e}", path.display()))?;
    // Mode 600 on the file
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        // Warn if secrets dir is world-readable
        if let Ok(meta) = std::fs::metadata(path.parent().unwrap()) {
            if meta.permissions().mode() & 0o007 != 0 {
                eprintln!("warning: credentials directory is world-readable — consider `chmod 700 ~/.config/agentcarousel`");
            }
        }
    }
    Ok(())
}

fn file_load() -> Option<String> {
    let path = credentials_path();
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("token") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let token = rest.trim().trim_matches('"').to_string();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
    }
    None
}

fn file_delete() -> Result<(), String> {
    let path = credentials_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("cannot remove credentials file: {e}"))
    } else {
        Err("credentials file not found".to_string())
    }
}
