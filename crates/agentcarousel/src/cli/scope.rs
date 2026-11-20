//! Agent-under-test scope profile (`scope.yaml`) and `agc scope init|check`.
//!
//! A scope profile sits next to a fixture's `cases.yaml` and records what the
//! evaluation covers: the agent's use cases, tools, modalities, data, users, the
//! instance under test, what is out of scope, and how the test effort is meant to
//! be split across risk categories. `agc export` includes it in the evidence
//! bundle as `scope.json`.

use std::path::{Path, PathBuf};

use agentcarousel_core::FixtureFile;
use agentcarousel_fixtures::load_fixture;
use clap::{Parser, Subcommand};
use console::style;
use serde::{Deserialize, Serialize};

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

/// File names reserved for scope profiles inside a fixture directory.
pub(crate) const SCOPE_FILE_NAMES: &[&str] = &["scope.yaml", "scope.yml"];

/// Name of the scope profile inside an exported evidence bundle.
pub(crate) const SCOPE_BUNDLE_FILE: &str = "scope.json";

const SCOPE_SKELETON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/scope-skeleton.yaml"
));

/// Allowed deviation of the summed `share_pct` values from 100.
const SHARE_TOLERANCE: f64 = 0.5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScopeProfile {
    pub schema_version: u32,
    pub skill_or_agent: String,
    #[serde(default)]
    pub use_cases: Vec<String>,
    #[serde(default)]
    pub tools: Vec<ScopeTool>,
    #[serde(default)]
    pub modalities: Vec<String>,
    #[serde(default)]
    pub data_sensitivity: Vec<String>,
    #[serde(default)]
    pub users: Vec<ScopeUser>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_under_test: Option<InstanceUnderTest>,
    #[serde(default)]
    pub out_of_scope: Vec<OutOfScope>,
    #[serde(default)]
    pub risk_distribution: Vec<RiskShare>,
    /// Whether the evaluation observed the agent's real tool calls. agc cannot yet
    /// (tool calls are synthesized from fixture expectations), so this must be false.
    #[serde(default)]
    pub tool_calls_observed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScopeTool {
    pub name: String,
    pub access: ToolAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScopeUser {
    #[serde(rename = "type")]
    pub user_type: String,
    #[serde(default)]
    pub vulnerable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstanceUnderTest {
    pub environment: String,
    pub matches_production: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<String>,
    pub data: TestData,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TestData {
    Synthetic,
    Seeded,
    Real,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutOfScope {
    pub item: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RiskShare {
    pub category: String,
    pub share_pct: f64,
    pub rationale: String,
    /// Case tags that count toward this category.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Problems found in a scope profile.
#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct ScopeIssues {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Planned vs actual share of cases for one risk category.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CategoryCoverage {
    pub category: String,
    pub planned_pct: f64,
    pub actual_pct: f64,
    pub cases: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ScopeCoverage {
    pub total_cases: usize,
    pub categories: Vec<CategoryCoverage>,
    /// Case IDs whose tags match no risk category.
    pub uncategorized_cases: Vec<String>,
}

/// Path of the scope profile in `dir`, if one exists.
pub(crate) fn find_scope_file(dir: &Path) -> Option<PathBuf> {
    SCOPE_FILE_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// True when `path` is a reserved scope profile file name (not a fixture).
pub(crate) fn is_scope_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| SCOPE_FILE_NAMES.contains(&n))
}

pub(crate) fn load_scope(path: &Path) -> Result<ScopeProfile, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|err| format!("{}: {err}", path.display()))
}

/// Check a parsed profile. `fixture` (when given) is cross-checked for the subject
/// name and declared agent tools.
pub fn check_profile(profile: &ScopeProfile, fixture: Option<&FixtureFile>) -> ScopeIssues {
    let mut issues = ScopeIssues::default();
    if profile.schema_version != 1 {
        issues.errors.push(format!(
            "schema_version must be 1, found {}",
            profile.schema_version
        ));
    }
    if profile.tool_calls_observed {
        issues.errors.push(
            "tool_calls_observed must be false: agc does not observe the agent's real tool calls yet"
                .to_string(),
        );
    }
    if profile.use_cases.is_empty() {
        issues
            .warnings
            .push("use_cases is empty: say what the agent is for".to_string());
    }
    if profile.instance_under_test.is_none() {
        issues
            .warnings
            .push("instance_under_test is missing: say what was tested".to_string());
    }
    if profile.risk_distribution.is_empty() {
        issues
            .warnings
            .push("risk_distribution is empty: no planned split of test effort".to_string());
    } else {
        let total: f64 = profile.risk_distribution.iter().map(|r| r.share_pct).sum();
        if (total - 100.0).abs() > SHARE_TOLERANCE {
            issues.errors.push(format!(
                "risk_distribution share_pct values sum to {total:.1}, not 100"
            ));
        }
        for share in &profile.risk_distribution {
            if share.share_pct <= 0.0 {
                issues.errors.push(format!(
                    "risk_distribution `{}`: share_pct must be greater than 0",
                    share.category
                ));
            }
            if share.tags.is_empty() {
                issues.warnings.push(format!(
                    "risk_distribution `{}` lists no tags, so no cases can count toward it",
                    share.category
                ));
            }
        }
    }
    if let Some(fixture) = fixture {
        if fixture.skill_or_agent != profile.skill_or_agent {
            issues.errors.push(format!(
                "skill_or_agent `{}` does not match the fixture's `{}`",
                profile.skill_or_agent, fixture.skill_or_agent
            ));
        }
        if let Some(agent) = fixture.agent.as_ref() {
            for declared in &agent.tools {
                if !profile.tools.iter().any(|t| t.name == declared.name) {
                    issues.warnings.push(format!(
                        "fixture agent declares tool `{}` but the scope profile does not list it",
                        declared.name
                    ));
                }
            }
            for tool in &profile.tools {
                if !agent.tools.iter().any(|t| t.name == tool.name) {
                    issues.warnings.push(format!(
                        "scope profile lists tool `{}` that the fixture agent does not declare",
                        tool.name
                    ));
                }
            }
        }
    }
    issues
}

/// Planned vs actual share of the fixture's cases per risk category.
pub fn coverage(profile: &ScopeProfile, fixture: &FixtureFile) -> ScopeCoverage {
    let total = fixture.cases.len();
    let pct = |n: usize| {
        if total == 0 {
            0.0
        } else {
            n as f64 * 100.0 / total as f64
        }
    };
    let categories = profile
        .risk_distribution
        .iter()
        .map(|share| {
            let n = fixture
                .cases
                .iter()
                .filter(|case| case.tags.iter().any(|t| share.tags.contains(t)))
                .count();
            CategoryCoverage {
                category: share.category.clone(),
                planned_pct: share.share_pct,
                actual_pct: pct(n),
                cases: n,
            }
        })
        .collect();
    let uncategorized_cases = fixture
        .cases
        .iter()
        .filter(|case| {
            !profile
                .risk_distribution
                .iter()
                .any(|share| case.tags.iter().any(|t| share.tags.contains(t)))
        })
        .map(|case| case.id.0.clone())
        .collect();
    ScopeCoverage {
        total_cases: total,
        categories,
        uncategorized_cases,
    }
}

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc scope init --skill customer-support\n  agc scope check --skill customer-support\n  agc scope check fixtures/customer-support"
)]
pub struct ScopeArgs {
    #[command(subcommand)]
    command: ScopeCommand,
}

#[derive(Debug, Subcommand)]
enum ScopeCommand {
    /// Write a commented scope.yaml skeleton into a fixture directory.
    Init(ScopeTarget),
    /// Validate scope.yaml and compare planned vs actual case coverage per risk category.
    Check(ScopeTarget),
}

#[derive(Debug, Parser)]
struct ScopeTarget {
    /// Fixture directory containing cases.yaml (default: fixtures/<skill>).
    #[arg(value_name = "DIR")]
    dir: Option<PathBuf>,
    /// Skill or agent name; resolves to fixtures/<skill>.
    #[arg(long)]
    skill: Option<String>,
}

impl ScopeTarget {
    fn resolve(&self) -> Result<PathBuf, String> {
        match (&self.dir, &self.skill) {
            (Some(dir), _) => Ok(dir.clone()),
            (None, Some(skill)) => Ok(PathBuf::from("fixtures").join(skill)),
            (None, None) => Err("pass a fixture directory or --skill <name>".to_string()),
        }
    }
}

pub fn run_scope(args: ScopeArgs, globals: &GlobalOptions) -> i32 {
    match args.command {
        ScopeCommand::Init(target) => run_init(target, globals),
        ScopeCommand::Check(target) => run_check(target, globals),
    }
}

fn fail(globals: &GlobalOptions, code: ExitCode, kind: &'static str, msg: String) -> i32 {
    if globals.json {
        JsonOutput::err("scope", JsonError::new(kind, msg)).print();
    } else {
        eprintln!("error: {msg}");
    }
    code.as_i32()
}

/// The fixture file in `dir`: `cases.yaml` or `cases.yml`.
pub(crate) fn fixture_in(dir: &Path) -> Option<PathBuf> {
    ["cases.yaml", "cases.yml"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

fn run_init(target: ScopeTarget, globals: &GlobalOptions) -> i32 {
    let dir = match target.resolve() {
        Ok(d) => d,
        Err(msg) => return fail(globals, ExitCode::ConfigError, "usage_error", msg),
    };
    if !dir.is_dir() {
        return fail(
            globals,
            ExitCode::NotFound,
            "not_found",
            format!("fixture directory not found: {}", dir.display()),
        );
    }
    if let Some(existing) = find_scope_file(&dir) {
        return fail(
            globals,
            ExitCode::ValidationFailed,
            "exists",
            format!("{} already exists", existing.display()),
        );
    }
    let skill = fixture_in(&dir)
        .and_then(|p| load_fixture(&p).ok())
        .map(|f| f.skill_or_agent)
        .or(target.skill.clone())
        .unwrap_or_else(|| "my-agent".to_string());
    let body = SCOPE_SKELETON.replace("__SKILL__", &skill);
    let path = dir.join(SCOPE_FILE_NAMES[0]);
    if let Err(err) = std::fs::write(&path, body) {
        return fail(
            globals,
            ExitCode::RuntimeError,
            "runtime_error",
            format!("cannot write {}: {err}", path.display()),
        );
    }
    if globals.json {
        JsonOutput::ok("scope", serde_json::json!({ "written": path })).print();
    } else {
        println!("wrote {}", path.display());
        println!(
            "next: edit it, then run `agc scope check {}`",
            dir.display()
        );
    }
    ExitCode::Ok.as_i32()
}

fn run_check(target: ScopeTarget, globals: &GlobalOptions) -> i32 {
    let dir = match target.resolve() {
        Ok(d) => d,
        Err(msg) => return fail(globals, ExitCode::ConfigError, "usage_error", msg),
    };
    let Some(scope_path) = find_scope_file(&dir) else {
        return fail(
            globals,
            ExitCode::NotFound,
            "not_found",
            format!(
                "no scope.yaml in {} (create one with `agc scope init {}`)",
                dir.display(),
                dir.display()
            ),
        );
    };
    let profile = match load_scope(&scope_path) {
        Ok(p) => p,
        Err(msg) => {
            return fail(
                globals,
                ExitCode::ValidationFailed,
                "validation_failed",
                msg,
            )
        }
    };
    let fixture = fixture_in(&dir).and_then(|p| load_fixture(&p).ok());
    let issues = check_profile(&profile, fixture.as_ref());
    let cov = fixture.as_ref().map(|f| coverage(&profile, f));

    if globals.json {
        let body = serde_json::json!({
            "path": scope_path,
            "errors": issues.errors,
            "warnings": issues.warnings,
            "coverage": cov,
        });
        if issues.errors.is_empty() {
            JsonOutput::ok("scope", body).print();
        } else {
            JsonOutput::err(
                "scope",
                JsonError::new("validation_failed", issues.errors.join("; ")),
            )
            .print();
        }
    } else {
        println!("{}", style(scope_path.display()).bold());
        for e in &issues.errors {
            println!("  {} {e}", style("error:").red().bold());
        }
        for w in &issues.warnings {
            println!("  {} {w}", style("warning:").yellow().bold());
        }
        match &cov {
            Some(cov) => print_coverage(cov),
            None => println!("  no cases.yaml next to the profile; coverage not computed"),
        }
    }
    if issues.errors.is_empty() {
        ExitCode::Ok.as_i32()
    } else {
        ExitCode::ValidationFailed.as_i32()
    }
}

fn print_coverage(cov: &ScopeCoverage) {
    println!();
    println!(
        "  {:<32} {:>8} {:>8} {:>6}",
        "risk category", "planned", "actual", "cases"
    );
    for c in &cov.categories {
        println!(
            "  {:<32} {:>7.1}% {:>7.1}% {:>6}",
            c.category, c.planned_pct, c.actual_pct, c.cases
        );
    }
    println!("  {} cases in total", cov.total_cases);
    if !cov.uncategorized_cases.is_empty() {
        println!(
            "  {} case(s) match no risk category:",
            cov.uncategorized_cases.len()
        );
        for id in &cov.uncategorized_cases {
            println!("    - {id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(yaml: &str) -> ScopeProfile {
        serde_yaml::from_str(yaml).expect("profile parses")
    }

    const BASE: &str = "schema_version: 1\nskill_or_agent: demo\nuse_cases: [triage]\ninstance_under_test:\n  environment: staging\n  matches_production: true\n  data: synthetic\n";

    #[test]
    fn shares_must_sum_to_100() {
        let ok = profile(&format!(
            "{BASE}risk_distribution:\n  - {{category: a, share_pct: 60, rationale: r, tags: [x]}}\n  - {{category: b, share_pct: 40, rationale: r, tags: [y]}}\n"
        ));
        assert!(check_profile(&ok, None).errors.is_empty());
        let bad = profile(&format!(
            "{BASE}risk_distribution:\n  - {{category: a, share_pct: 60, rationale: r, tags: [x]}}\n  - {{category: b, share_pct: 30, rationale: r, tags: [y]}}\n"
        ));
        assert!(check_profile(&bad, None).errors[0].contains("sum to 90.0"));
    }

    #[test]
    fn tool_calls_observed_cannot_be_claimed() {
        let p = profile(&format!("{BASE}tool_calls_observed: true\n"));
        assert!(check_profile(&p, None)
            .errors
            .iter()
            .any(|e| e.contains("tool_calls_observed")));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let res: Result<ScopeProfile, _> = serde_yaml::from_str(&format!("{BASE}bogus: 1\n"));
        assert!(res.is_err());
    }

    #[test]
    fn coverage_counts_cases_by_category_tags() {
        let p = profile(&format!(
            "{BASE}risk_distribution:\n  - {{category: grounding, share_pct: 50, rationale: r, tags: [aiuc-1:D001]}}\n  - {{category: privacy, share_pct: 50, rationale: r, tags: [aiuc-1:A006]}}\n"
        ));
        let fixture: FixtureFile = serde_yaml::from_str(
            "schema_version: 1\nskill_or_agent: demo\ncases:\n  - id: demo/a\n    tags: [aiuc-1:D001]\n    input: {messages: [{role: user, content: hi}]}\n    expected: {}\n  - id: demo/b\n    tags: [smoke]\n    input: {messages: [{role: user, content: hi}]}\n    expected: {}\n",
        )
        .expect("fixture parses");
        let cov = coverage(&p, &fixture);
        assert_eq!(cov.total_cases, 2);
        assert_eq!(cov.categories[0].cases, 1);
        assert!((cov.categories[0].actual_pct - 50.0).abs() < 1e-9);
        assert_eq!(cov.categories[1].cases, 0);
        assert_eq!(cov.uncategorized_cases, vec!["demo/b".to_string()]);
    }

    #[test]
    fn skeleton_parses_and_has_no_errors_after_naming() {
        let body = SCOPE_SKELETON.replace("__SKILL__", "demo");
        let p: ScopeProfile = serde_yaml::from_str(&body).expect("skeleton parses");
        assert!(check_profile(&p, None).errors.is_empty(), "{p:?}");
        assert!(!p.tool_calls_observed);
    }
}
