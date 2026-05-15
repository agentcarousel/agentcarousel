use agentcarousel_fixtures::load_fixture_value;
use clap::Parser;
use console::style;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::exit_codes::ExitCode;
use super::fixture_utils::collect_fixture_paths_with_ignore;
use super::GlobalOptions;

const AGENTCAROUSEL_IGNORE: &str = ".agentcarousel-ignore";

/// Check fixture quality beyond schema: smoke coverage, rubric weights, descriptions.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc lint fixtures/                     # check all fixtures\n  agc lint fixtures/skills/my-skill.yaml # check one file\n  agc lint --error-on-warn               # fail on any warning"
)]
pub struct LintArgs {
    /// Fixture files or dirs (default: fixtures/).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Exit with a non-zero code on warnings (default: only fail on errors).
    #[arg(short = 'x', long)]
    error_on_warn: bool,
    /// `human` or `json`.
    #[arg(short = 'f', long, default_value = "human")]
    format: String,
}

#[derive(Debug)]
struct LintIssue {
    level: Level,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Error,
    Warn,
}

struct FileLint {
    path: String,
    issues: Vec<LintIssue>,
}

pub fn run_lint(args: LintArgs, globals: &GlobalOptions) -> i32 {
    let inputs: Vec<PathBuf> = if args.paths.is_empty() {
        vec![PathBuf::from("fixtures")]
    } else {
        args.paths.clone()
    };

    let ignore_file = Path::new(AGENTCAROUSEL_IGNORE)
        .exists()
        .then_some(Path::new(AGENTCAROUSEL_IGNORE));

    let mut results: Vec<FileLint> = Vec::new();
    let mut any_error = false;
    let mut any_warn = false;

    for path in collect_fixture_paths_with_ignore(&inputs, ignore_file) {
        let file_lint = lint_path(&path);
        for issue in &file_lint.issues {
            match issue.level {
                Level::Error => any_error = true,
                Level::Warn => any_warn = true,
            }
        }
        results.push(file_lint);
    }

    if !globals.quiet {
        match args.format.as_str() {
            "json" => output_json(&results),
            _ => output_human(&results),
        }
    }

    if any_error || (args.error_on_warn && any_warn) {
        ExitCode::ValidationFailed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

fn lint_path(path: &Path) -> FileLint {
    let path_str = path.display().to_string();
    let value = match load_fixture_value(path) {
        Ok(v) => v,
        Err(err) => {
            return FileLint {
                path: path_str,
                issues: vec![LintIssue {
                    level: Level::Error,
                    message: format!("failed to load fixture: {err}"),
                }],
            };
        }
    };

    let mut issues = Vec::new();
    lint_fixture(&value, &mut issues);
    FileLint {
        path: path_str,
        issues,
    }
}

fn lint_fixture(value: &Value, issues: &mut Vec<LintIssue>) {
    let Some(obj) = value.as_object() else {
        return;
    };

    let skill = obj
        .get("skill_or_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let has_bundle_id = obj
        .get("bundle_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());

    if has_bundle_id {
        if obj
            .get("risk_tier")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.is_empty())
        {
            issues.push(LintIssue {
                level: Level::Warn,
                message: "bundle fixture is missing risk_tier — required for ATF compliance"
                    .to_string(),
            });
        }
        if obj
            .get("certification_track")
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.is_empty())
        {
            issues.push(LintIssue {
                level: Level::Warn,
                message: "bundle fixture is missing certification_track".to_string(),
            });
        }
    }

    let Some(cases) = obj.get("cases").and_then(|c| c.as_array()) else {
        return;
    };

    let has_smoke = cases.iter().any(|case| {
        case.get("tags")
            .and_then(|t| t.as_array())
            .is_some_and(|tags| {
                tags.iter()
                    .any(|t| t.as_str().is_some_and(|s| s.eq_ignore_ascii_case("smoke")))
            })
    });
    if !has_smoke {
        issues.push(LintIssue {
            level: Level::Warn,
            message: format!(
                "'{skill}' has no smoke-tagged case — add tags: [smoke] to at least one case for fast CI gating"
            ),
        });
    }

    for case in cases {
        let Some(case_obj) = case.as_object() else {
            continue;
        };
        let case_id = case_obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let evaluator = case_obj
            .get("evaluator")
            .and_then(|v| v.as_str())
            .unwrap_or("rules");

        let description = case_obj.get("description").and_then(|v| v.as_str());

        if evaluator == "judge" && description.is_none_or(|d| d.trim().is_empty()) {
            issues.push(LintIssue {
                level: Level::Warn,
                message: format!("case '{case_id}' uses judge evaluator but has no description — descriptions improve judge scoring consistency"),
            });
        }

        if let Some(rubric) = case_obj
            .get("expected")
            .and_then(|e| e.as_object())
            .and_then(|e| e.get("rubric"))
            .and_then(|r| r.as_array())
        {
            check_rubric_weights(case_id, rubric, issues);
        }
    }
}

fn check_rubric_weights(case_id: &str, rubric: &[Value], issues: &mut Vec<LintIssue>) {
    let weights: Vec<f64> = rubric
        .iter()
        .filter_map(|item| item.get("weight").and_then(|w| w.as_f64()))
        .collect();

    if weights.is_empty() || weights.len() != rubric.len() {
        return;
    }

    let sum: f64 = weights.iter().sum();
    if (sum - 1.0).abs() > 0.05 {
        issues.push(LintIssue {
            level: Level::Warn,
            message: format!(
                "case '{case_id}' rubric weights sum to {sum:.3} (expected ~1.0) — judge scoring may be skewed"
            ),
        });
    }
}

fn output_human(results: &[FileLint]) {
    let n = results.len();
    let plural = if n == 1 { "fixture" } else { "fixtures" };
    println!(
        "🎠 AgentCarousel v{} · lint · {} {}",
        env!("CARGO_PKG_VERSION"),
        n,
        plural
    );
    println!();
    println!(
        "{}",
        style("Checking smoke coverage, rubric weights, descriptions, and bundle metadata").dim()
    );
    println!();

    let mut total_errors = 0usize;
    let mut total_warnings = 0usize;

    for result in results {
        total_errors += result
            .issues
            .iter()
            .filter(|i| i.level == Level::Error)
            .count();
        total_warnings += result
            .issues
            .iter()
            .filter(|i| i.level == Level::Warn)
            .count();

        if result.issues.is_empty() {
            println!("    ✅  PASS  {}", style(result.path.as_str()).green());
            continue;
        }

        let has_errors = result.issues.iter().any(|i| i.level == Level::Error);
        if has_errors {
            println!("    ❌  FAIL  {}", style(result.path.as_str()).red());
        } else {
            println!(
                "    {}  WARN  {}",
                style("⚠").yellow(),
                style(result.path.as_str()).yellow()
            );
        }

        for issue in &result.issues {
            let prefix = match issue.level {
                Level::Error => style("error").red(),
                Level::Warn => style("warn ").yellow(),
            };
            println!(
                "             › {} {}",
                prefix,
                style(issue.message.as_str()).dim()
            );
        }
    }

    println!();
    println!("  ──────────────────────────────────────────────────────");
    let err_word = if total_errors == 1 { "error" } else { "errors" };
    let warn_word = if total_warnings == 1 {
        "warning"
    } else {
        "warnings"
    };
    println!(
        "  Results   {} {} · {} {}",
        total_errors, err_word, total_warnings, warn_word
    );

    if total_errors == 0 && total_warnings == 0 {
        println!(
            "  {}",
            style("Lint: OK — fixtures pass quality checks").green()
        );
    } else if total_errors == 0 {
        println!(
            "  {}",
            style("Lint: passed with warnings (use --error-on-warn to fail)").yellow()
        );
    } else {
        println!("  {}", style("Lint: failed — fix errors above").red());
    }
    println!("  ──────────────────────────────────────────────────────");
}

fn output_json(results: &[FileLint]) {
    let items: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            let issues: Vec<serde_json::Value> = r
                .issues
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "level": match i.level { Level::Error => "error", Level::Warn => "warn" },
                        "message": i.message,
                    })
                })
                .collect();
            serde_json::json!({ "path": r.path, "issues": issues })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({ "files": items })).unwrap_or_default()
    );
}
