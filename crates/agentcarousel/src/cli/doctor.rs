use clap::Parser;
use console::style;
use std::path::Path;

use super::config::{resolve_schema_path, ResolvedConfig};
use super::exit_codes::ExitCode;

/// Check environment, configuration, and fixture setup for common issues.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc doctor                    # full environment check\n  agc doctor --json             # machine-readable output"
)]
pub struct DoctorArgs {
    #[arg(short = 'j', long)]
    pub json: bool,
}

enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

struct Check {
    label: &'static str,
    status: CheckStatus,
    detail: String,
}

pub fn run_doctor(args: DoctorArgs, config: &ResolvedConfig) -> i32 {
    let checks = vec![
        check_api_keys(),
        check_config_file(),
        check_history_db(config),
        check_fixtures_dir(),
        check_schema_file(config),
    ];

    if args.json {
        return output_json(&checks);
    }

    println!("🎠 AgentCarousel v{} · doctor", env!("CARGO_PKG_VERSION"));
    println!();

    let mut any_fail = false;
    let mut any_warn = false;

    for check in &checks {
        match check.status {
            CheckStatus::Ok => println!(
                "    ✅  {}  — {}",
                style(check.label).green(),
                style(check.detail.as_str()).dim()
            ),
            CheckStatus::Warn => {
                any_warn = true;
                println!(
                    "    {}  {}  — {}",
                    style("⚠").yellow(),
                    style(check.label).yellow(),
                    style(check.detail.as_str()).dim()
                );
            }
            CheckStatus::Fail => {
                any_fail = true;
                println!(
                    "    ❌  {}  — {}",
                    style(check.label).red(),
                    style(check.detail.as_str()).dim()
                );
            }
        }
    }

    println!();
    println!("  ──────────────────────────────────────────────────────");
    if any_fail {
        println!(
            "  {}",
            style("Doctor: issues found — fix errors above").red()
        );
        println!("  ──────────────────────────────────────────────────────");
        ExitCode::Failed.as_i32()
    } else if any_warn {
        println!(
            "  {}",
            style("Doctor: warnings — live eval may be limited").yellow()
        );
        println!("  ──────────────────────────────────────────────────────");
        ExitCode::Ok.as_i32()
    } else {
        println!("  {}", style("Doctor: all checks passed").green());
        println!("  ──────────────────────────────────────────────────────");
        ExitCode::Ok.as_i32()
    }
}

fn output_json(checks: &[Check]) -> i32 {
    let mut any_fail = false;
    let items: Vec<serde_json::Value> = checks
        .iter()
        .map(|c| {
            let status = match c.status {
                CheckStatus::Ok => "ok",
                CheckStatus::Warn => "warn",
                CheckStatus::Fail => {
                    any_fail = true;
                    "fail"
                }
            };
            serde_json::json!({
                "check": c.label,
                "status": status,
                "detail": c.detail,
            })
        })
        .collect();
    let payload = serde_json::json!({ "checks": items });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );
    if any_fail {
        ExitCode::Failed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn check_api_keys() -> Check {
    const PROVIDERS: &[(&str, &str)] = &[
        ("ANTHROPIC_API_KEY", "Anthropic"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Gemini"),
        ("OPENROUTER_API_KEY", "OpenRouter"),
        ("AGENTCAROUSEL_GENERATOR_KEY", "agentcarousel-generator"),
        ("AGENTCAROUSEL_JUDGE_KEY", "agentcarousel-judge"),
    ];

    let found: Vec<&str> = PROVIDERS
        .iter()
        .filter(|(env, _)| std::env::var(env).is_ok_and(|v| !v.is_empty()))
        .map(|(_, name)| *name)
        .collect();

    if found.is_empty() {
        Check {
            label: "API keys",
            status: CheckStatus::Warn,
            detail: "no provider keys found — offline/mock mode only (set ANTHROPIC_API_KEY etc for live eval)".to_string(),
        }
    } else {
        Check {
            label: "API keys",
            status: CheckStatus::Ok,
            detail: format!("{} configured", found.join(", ")),
        }
    }
}

fn check_config_file() -> Check {
    if Path::new("agentcarousel.toml").exists() {
        Check {
            label: "agentcarousel.toml",
            status: CheckStatus::Ok,
            detail: "found".to_string(),
        }
    } else {
        Check {
            label: "agentcarousel.toml",
            status: CheckStatus::Warn,
            detail: "not found — using defaults (copy agentcarousel.example.toml to customize)"
                .to_string(),
        }
    }
}

fn check_history_db(config: &ResolvedConfig) -> Check {
    let db_path = history_db_path(config);
    if let Some(parent) = db_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return Check {
                label: "History DB",
                status: CheckStatus::Fail,
                detail: format!("cannot create parent dir {}: {err}", parent.display()),
            };
        }
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&db_path)
    {
        Ok(_) => Check {
            label: "History DB",
            status: CheckStatus::Ok,
            detail: db_path.display().to_string(),
        },
        Err(err) => Check {
            label: "History DB",
            status: CheckStatus::Fail,
            detail: format!("{}: {err}", db_path.display()),
        },
    }
}

fn check_fixtures_dir() -> Check {
    let fixtures = Path::new("fixtures");
    if fixtures.is_dir() {
        let count = std::fs::read_dir(fixtures)
            .map(|iter| iter.count())
            .unwrap_or(0);
        Check {
            label: "Fixtures directory",
            status: CheckStatus::Ok,
            detail: format!("fixtures/ ({count} entries)"),
        }
    } else {
        Check {
            label: "Fixtures directory",
            status: CheckStatus::Warn,
            detail: "fixtures/ not found — run `agc init --skill my-skill` to create one"
                .to_string(),
        }
    }
}

fn check_schema_file(config: &ResolvedConfig) -> Check {
    let schema_path = resolve_schema_path(config);
    if schema_path.exists() {
        Check {
            label: "JSON Schema",
            status: CheckStatus::Ok,
            detail: schema_path.display().to_string(),
        }
    } else {
        Check {
            label: "JSON Schema",
            status: CheckStatus::Fail,
            detail: format!(
                "{} not found — reinstall or set validate.schema_dir in agentcarousel.toml",
                schema_path.display()
            ),
        }
    }
}

fn history_db_path(config: &ResolvedConfig) -> std::path::PathBuf {
    if let Ok(path) = std::env::var("AGENTCAROUSEL_HISTORY_DB") {
        return std::path::PathBuf::from(path);
    }
    if let Some(ref path) = config.report.history_db {
        return super::config::expand_tilde(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    if cfg!(target_os = "macos") {
        std::path::PathBuf::from(home).join("Library/Application Support/agentcarousel/history.db")
    } else {
        std::path::PathBuf::from(home).join(".local/share/agentcarousel/history.db")
    }
}
