//! Integration tests for `agc audit suggest` and `agc audit run`.
//!
//! `audit suggest` is fully offline (reads stored audit data, no LLM call), so all
//! paths are tested.  `audit run` requires a live LLM; only the fast-fail error
//! paths (run not found, API key missing) are covered here.

use agentcarousel::{
    AuditFinding, CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, OverallStatus,
    PromptAudit, PromptAuditFailureMode, ProviderErrorMetrics, Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serialize tests that mutate process-global env vars (API keys).
static AUDIT_ENV_LOCK: Mutex<()> = Mutex::new(());

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

fn make_run(id: &str, skill: &str, audit: Option<PromptAudit>) -> Run {
    Run {
        id: RunId(id.to_string()),
        schema_version: 1,
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
        command: "eval".to_string(),
        git_sha: None,
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: "none".to_string(),
        cases: vec![CaseResult {
            case_id: CaseId(format!("{skill}/positive")),
            status: CaseStatus::Failed,
            error: Some("expected X, got Y".to_string()),
            trace: ExecutionTrace {
                steps: vec![],
                final_output: None,
                redacted: false,
            },
            metrics: Metrics {
                total_latency_ms: 10,
                ..Metrics::default()
            },
            eval_scores: None,
            input: vec![],
            discrimination_score: None,
            discrimination_label: None,
        }],
        summary: RunSummary {
            total: 1,
            passed: 0,
            failed: 1,
            skipped: 0,
            flaky: 0,
            errored: 0,
            timed_out: 0,
            pass_rate: 0.0,
            mean_latency_ms: 10.0,
            mean_effectiveness_score: None,
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: OverallStatus::Fail,
            tokens_in: None,
            tokens_out: None,
            mean_tokens_per_judged_case: None,
            latency_p50_ms: None,
            latency_p95_ms: None,
            latency_p99_ms: None,
            judge_tokens_in: None,
            judge_tokens_out: None,
            gen_cost_usd: None,
            judge_cost_usd: None,
            total_cost_usd: None,
            generator_model: None,
            judge_model: None,
            command_line: None,
        },
        fixture_bundle_id: None,
        fixture_bundle_version: None,
        skill_or_agent: Some(skill.to_string()),
        runner_offline: true,
        runner_mock_strict: false,
        runner_mock_only: true,
        prompt_audit: audit,
    }
}

fn sample_audit(suggestions: &[&str]) -> PromptAudit {
    PromptAudit {
        failure_mode: PromptAuditFailureMode::Prompt,
        confidence: 0.9,
        findings: vec![AuditFinding {
            pattern: "Missing citations in 2/3 cases".to_string(),
            affected_case_count: 2,
            root_cause: "Prompt does not instruct the model to cite sources".to_string(),
        }],
        suggested_fixes: suggestions.iter().map(|s| s.to_string()).collect(),
        suggested_implementations: vec![],
        suggested_locations: vec![],
        overall_rationale: "The prompt lacks explicit citation requirements.".to_string(),
        judge_tokens_in: None,
        judge_tokens_out: None,
    }
}

fn write_run_json(dir: &Path, run: &Run) {
    fs::write(
        dir.join("run.json"),
        serde_json::to_string_pretty(run).expect("serialize run"),
    )
    .expect("write run.json");
}

// ─── audit suggest ──────────────────────────────────────────────────────────

/// Passing a directory that contains no run.json should fail with a not-found error.
#[test]
fn audit_suggest_empty_dir_exits_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = workspace_root();

    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["audit", "suggest", dir.path().to_str().expect("utf8")])
        .assert()
        .failure()
        .get_output()
        .clone();

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("not_found")
            || combined.contains("run.json")
            || combined.contains("error"),
        "expected not-found error, got: {combined:?}"
    );
}

/// A run.json with no `prompt_audit` should yield a clear "run has no stored audit" error.
#[test]
fn audit_suggest_no_stored_audit_exits_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_run_json(
        dir.path(),
        &make_run("audit-no-audit-run", "demo-skill", None),
    );

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["audit", "suggest", dir.path().to_str().expect("utf8")])
        .assert()
        .failure()
        .get_output()
        .clone();

    // In test env stdout is piped, so JSON mode is auto-enabled.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], false, "expected ok:false, got {stdout}");
    assert_eq!(
        parsed["error"]["code"].as_str(),
        Some("not_found"),
        "expected not_found code, got {stdout}"
    );
    let msg = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("no stored audit"),
        "expected 'no stored audit' in message, got: {msg}"
    );
}

/// A run with suggestions should output them as a JSON array (auto-JSON in tests).
#[test]
fn audit_suggest_with_suggestions_returns_json_array() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = sample_audit(&["Add citation instructions", "Be more specific about format"]);
    write_run_json(
        dir.path(),
        &make_run("audit-suggest-run", "demo-skill", Some(audit)),
    );

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["audit", "suggest", dir.path().to_str().expect("utf8")])
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], true, "expected ok:true, got {stdout}");
    let suggestions = parsed["data"]["suggestions"]
        .as_array()
        .expect("suggestions array");
    assert_eq!(suggestions.len(), 2, "expected 2 suggestions, got {stdout}");
    assert_eq!(
        suggestions[0]["title"].as_str(),
        Some("Add citation instructions"),
        "suggestion[0].title mismatch, got {stdout}"
    );
    assert_eq!(
        suggestions[1]["title"].as_str(),
        Some("Be more specific about format"),
        "suggestion[1].title mismatch, got {stdout}"
    );
}

/// A stored audit with no `suggested_fixes` should return an empty array, not an error.
#[test]
fn audit_suggest_empty_suggestions_returns_empty_array() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = sample_audit(&[]);
    write_run_json(
        dir.path(),
        &make_run("audit-suggest-empty-run", "demo-skill", Some(audit)),
    );

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["audit", "suggest", dir.path().to_str().expect("utf8")])
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], true, "expected ok:true, got {stdout}");
    let suggestions = parsed["data"]["suggestions"]
        .as_array()
        .expect("suggestions array");
    assert!(
        suggestions.is_empty(),
        "expected empty suggestions, got {stdout}"
    );
}

/// `--apply` appends a commented HTML-style block to the prompt file and reports how many were applied.
#[test]
fn audit_suggest_apply_appends_block_to_prompt_md() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = sample_audit(&["Fix #1", "Fix #2"]);
    write_run_json(
        dir.path(),
        &make_run("audit-suggest-apply-run", "demo-skill", Some(audit)),
    );

    let prompt_path = dir.path().join("prompt.md");
    fs::write(&prompt_path, "# My Skill\n\nDo something.\n").expect("write prompt.md");

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "audit",
            "suggest",
            dir.path().to_str().expect("utf8"),
            "--apply",
            "--prompt",
            prompt_path.to_str().expect("prompt path utf8"),
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    // JSON output should report how many suggestions were applied.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], true, "expected ok:true, got {stdout}");
    assert_eq!(
        parsed["data"]["applied"].as_u64(),
        Some(2),
        "expected applied:2, got {stdout}"
    );

    // The prompt file should contain the suggestions block.
    let contents = fs::read_to_string(&prompt_path).expect("re-read prompt.md");
    assert!(
        contents.contains("<!-- audit:suggestions"),
        "expected audit:suggestions block in prompt, got:\n{contents}"
    );
    assert!(
        contents.contains("Fix #1"),
        "expected Fix #1 in prompt, got:\n{contents}"
    );
    assert!(
        contents.contains("Fix #2"),
        "expected Fix #2 in prompt, got:\n{contents}"
    );
    assert!(
        contents.contains("<!-- /audit:suggestions -->"),
        "expected closing tag in prompt, got:\n{contents}"
    );
    assert!(
        contents.contains("# My Skill"),
        "original content should be preserved, got:\n{contents}"
    );
}

/// `--apply` without `--prompt`, when the run's skill has no prompt.md in fixtures/, should fail.
#[test]
fn audit_suggest_apply_missing_prompt_md_exits_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    let audit = sample_audit(&["Some fix"]);
    // Use a skill name that has no fixtures/<skill>/prompt.md in the workspace.
    write_run_json(
        dir.path(),
        &make_run(
            "audit-suggest-apply-noprompt-run",
            "nonexistent-test-skill-xyz",
            Some(audit),
        ),
    );

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "audit",
            "suggest",
            dir.path().to_str().expect("utf8"),
            "--apply",
        ])
        .assert()
        .failure()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], false, "expected ok:false, got {stdout}");
    assert_eq!(
        parsed["error"]["code"].as_str(),
        Some("not_found"),
        "expected not_found code, got {stdout}"
    );
    let msg = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("prompt.md"),
        "expected prompt.md in error message, got: {msg}"
    );
}

// ─── audit run ──────────────────────────────────────────────────────────────

/// Passing a directory with no run.json to `audit run` should fail immediately.
#[test]
fn audit_run_empty_dir_exits_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = workspace_root();

    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["audit", "run", dir.path().to_str().expect("utf8")])
        .assert()
        .failure()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], false, "expected ok:false, got {stdout}");
    assert_eq!(
        parsed["error"]["code"].as_str(),
        Some("not_found"),
        "expected not_found code, got {stdout}"
    );
}

/// When the judge model's API key is absent, `audit run` should fail before making any network
/// call, reporting an auth_error with the missing key name.
#[test]
fn audit_run_no_api_key_exits_config_error() {
    let _guard = AUDIT_ENV_LOCK.lock().expect("audit env lock");

    let dir = tempfile::tempdir().expect("tempdir");
    write_run_json(
        dir.path(),
        &make_run("audit-run-no-key", "demo-skill", None),
    );

    let prompt_path = dir.path().join("prompt.md");
    fs::write(&prompt_path, "Do something useful.\n").expect("write prompt.md");

    let root = workspace_root();
    // Use gemini-2.5-flash (default judge model) and clear all its key candidates.
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env_remove("AGENTCAROUSEL_JUDGE_KEY")
        .env_remove("agentcarousel_JUDGE_KEY")
        .env_remove("GEMINI_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .args([
            "audit",
            "run",
            dir.path().to_str().expect("utf8"),
            "--prompt",
            prompt_path.to_str().expect("prompt path utf8"),
            "--model",
            "gemini-2.5-flash",
        ])
        .assert()
        .failure()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], false, "expected ok:false, got {stdout}");
    assert_eq!(
        parsed["error"]["code"].as_str(),
        Some("auth_error"),
        "expected auth_error code, got {stdout}"
    );
    let msg = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("GEMINI_API_KEY") || msg.contains("AGENTCAROUSEL_JUDGE_KEY"),
        "expected key name in error message, got: {msg}"
    );
}

/// `audit run --no-save` with a valid run.json but missing prompt.md exits with not_found.
#[test]
fn audit_run_missing_prompt_exits_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Use a skill with no prompt.md in fixtures/.
    write_run_json(
        dir.path(),
        &make_run("audit-run-no-prompt", "nonexistent-test-skill-xyz", None),
    );

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "audit",
            "run",
            dir.path().to_str().expect("utf8"),
            "--no-save",
        ])
        .assert()
        .failure()
        .get_output()
        .clone();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("JSON output");
    assert_eq!(parsed["ok"], false, "expected ok:false, got {stdout}");
    assert_eq!(
        parsed["error"]["code"].as_str(),
        Some("not_found"),
        "expected not_found code, got {stdout}"
    );
    let msg = parsed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("prompt.md"),
        "expected prompt.md in error message, got: {msg}"
    );
}
