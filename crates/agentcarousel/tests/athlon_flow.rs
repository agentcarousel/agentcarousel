//! Integration tests for `agc athlon` — the full P1 flow from the design plan
//! (docs/plans/2026-08-08-tevv-athlon-independent-baseline.md): init -> add-block
//! -> add-event (native + external) -> validate -> report, plus the failure
//! modes called out in Iteration 3 (stale materialization) and the refusal
//! behavior of `run` when validation fails.
//!
//! `report`'s scoring reads directly from persisted run history
//! (`agentcarousel::persist_run`), never from real fixture execution — so
//! these tests never invoke a real (or mock) generator and stay fast and
//! deterministic. `validate` does scan real fixture files on disk, so those
//! tests write a minimal `fixtures/<skill>/cases.yaml`.

use agentcarousel::{
    persist_run, CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, OverallStatus,
    ProviderErrorMetrics, Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `persist_run`/`agc athlon report` read `AGENTCAROUSEL_HISTORY_DB` from the
/// process environment; serialize tests that set it (same convention as
/// export_evidence_pack.rs and bundle_registry_flow.rs).
static ATHLON_HISTORY_LOCK: Mutex<()> = Mutex::new(());

fn agc() -> Command {
    Command::cargo_bin("agentcarousel").unwrap()
}

fn temp_history_db() -> PathBuf {
    tempfile::tempdir()
        .expect("tempdir")
        .keep()
        .join("history.db")
}

fn make_run(id: &str, tags: &[&str], status: CaseStatus) -> Run {
    let is_passed = status == CaseStatus::Passed;
    let case = CaseResult {
        case_id: CaseId("demo/case-1".to_string()),
        status,
        error: None,
        trace: ExecutionTrace {
            steps: Vec::new(),
            final_output: Some("ok".to_string()),
            redacted: false,
        },
        metrics: Metrics {
            total_latency_ms: 1,
            ..Metrics::default()
        },
        eval_scores: None,
        input: vec![],
        discrimination_score: None,
        discrimination_label: None,
        tags: tags.iter().map(|t| t.to_string()).collect(),
    };
    Run {
        id: RunId(id.to_string()),
        schema_version: 1,
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
        command: "eval".to_string(),
        git_sha: None,
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: "none".to_string(),
        cases: vec![case],
        summary: RunSummary {
            total: 1,
            passed: is_passed as u32,
            failed: (!is_passed) as u32,
            skipped: 0,
            flaky: 0,
            errored: 0,
            timed_out: 0,
            pass_rate: is_passed as u32 as f32,
            mean_latency_ms: 1.0,
            mean_effectiveness_score: None,
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: if is_passed {
                OverallStatus::Pass
            } else {
                OverallStatus::Fail
            },
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
            generator_model: Some("test-model".to_string()),
            judge_model: None,
            command_line: None,
        },
        fixture_bundle_id: None,
        fixture_bundle_version: None,
        skill_or_agent: Some("demo".to_string()),
        runner_offline: false,
        runner_mock_strict: false,
        runner_mock_only: true,
        prompt_audit: None,
    }
}

/// Persist `count` Passed runs carrying `tags` — enough to clear `MIN_CASES`
/// (3) so the control reaches `Satisfied`, not `PartialEvidence`.
fn persist_passing_runs(id_prefix: &str, tags: &[&str], count: usize) {
    for i in 0..count {
        persist_run(&make_run(
            &format!("{id_prefix}-{i}"),
            tags,
            CaseStatus::Passed,
        ))
        .expect("persist run");
    }
}

fn write_fixture_with_tags(dir: &Path, tags: &[&str]) {
    let fixtures_dir = dir.join("fixtures").join("demo");
    fs::create_dir_all(&fixtures_dir).expect("create fixtures dir");
    let tags_yaml: String = tags
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let yaml = format!(
        "schema_version: 1\nskill_or_agent: demo\ncases:\n  - id: c1\n    tags: [{tags_yaml}]\n    input:\n      messages: [{{role: user, content: \"hi\"}}]\n    expected:\n      contains: [\"hi\"]\n"
    );
    fs::write(fixtures_dir.join("cases.yaml"), yaml).expect("write fixture");
}

// ─── init ───────────────────────────────────────────────────────────────────

#[test]
fn athlon_init_creates_yaml_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "init",
            "--slug",
            "demo",
            "--objective",
            "test objective",
            "--lifecycle-stage",
            "deploy",
        ])
        .assert()
        .success();

    let contents = fs::read_to_string(dir.path().join("athlons/demo.yaml")).expect("read yaml");
    assert!(contents.contains("test objective"));
    assert!(contents.contains("deploy"));
}

#[test]
fn athlon_init_refuses_overwrite_without_force() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base_args = [
        "athlon",
        "init",
        "--slug",
        "demo",
        "--objective",
        "obj",
        "--lifecycle-stage",
        "deploy",
    ];
    agc()
        .current_dir(dir.path())
        .args(base_args)
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args(base_args)
        .assert()
        .failure();
}

// ─── add-block / add-event / materialization ───────────────────────────────

fn scaffold_athlon_with_native_event(dir: &Path) {
    agc()
        .current_dir(dir)
        .args([
            "athlon",
            "init",
            "--slug",
            "demo",
            "--objective",
            "obj",
            "--lifecycle-stage",
            "deploy",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir)
        .args([
            "athlon",
            "add-block",
            "--slug",
            "demo",
            "--block-id",
            "helpfulness",
            "--definition",
            "Answers usefully.",
            "--tc",
            "Valid & Reliable",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir)
        .args([
            "athlon",
            "add-event",
            "--slug",
            "demo",
            "--block-id",
            "helpfulness",
            "--event-id",
            "user-testing",
            "--native",
            "--evaluator",
            "rules",
        ])
        .assert()
        .success();
}

#[test]
fn add_event_native_materializes_three_level_and_two_level_controls() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());

    let materialized = fs::read_to_string(dir.path().join("frameworks/tevv-athlon-demo.json"))
        .expect("read materialized json");
    let controls: Value = serde_json::from_str(&materialized).expect("valid json");
    let control_ids: Vec<&str> = controls
        .as_array()
        .expect("array")
        .iter()
        .map(|c| c["control_id"].as_str().unwrap())
        .collect();
    assert!(control_ids.contains(&"helpfulness:user-testing"));
    assert!(control_ids.contains(&"helpfulness"));
    assert_eq!(control_ids.len(), 2);
}

#[test]
fn add_event_external_does_not_materialize() {
    let dir = tempfile::tempdir().expect("tempdir");
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "init",
            "--slug",
            "demo",
            "--objective",
            "obj",
            "--lifecycle-stage",
            "deploy",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "add-block",
            "--slug",
            "demo",
            "--block-id",
            "violation-frequency",
            "--definition",
            "d",
            "--tc",
            "Safe",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "add-event",
            "--slug",
            "demo",
            "--block-id",
            "violation-frequency",
            "--event-id",
            "red-teaming",
            "--external",
            "--tool",
            "garak",
            "--description",
            "probe suite",
            "--evidence-path",
            "./evidence.jsonl",
            "--result",
            "pass",
        ])
        .assert()
        .success();

    let materialized = fs::read_to_string(dir.path().join("frameworks/tevv-athlon-demo.json"))
        .expect("read materialized json");
    let controls: Value = serde_json::from_str(&materialized).expect("valid json");
    assert_eq!(
        controls.as_array().unwrap().len(),
        0,
        "external-only Block must materialize zero controls"
    );
}

// ─── validate ───────────────────────────────────────────────────────────────

#[test]
fn validate_passes_on_well_formed_athlon() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());
    write_fixture_with_tags(
        dir.path(),
        &[
            "tevv-athlon:demo:helpfulness:user-testing",
            "tevv-athlon:demo:helpfulness",
        ],
    );

    agc()
        .current_dir(dir.path())
        .args(["athlon", "validate", "--slug", "demo"])
        .assert()
        .success();
}

#[test]
fn validate_fails_check4_when_no_fixture_matches_native_event_tag() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());
    // No fixtures written at all.

    let out = agc()
        .current_dir(dir.path())
        .args(["athlon", "validate", "--slug", "demo"])
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
        combined.contains("check\":4") || combined.contains("matches zero cases"),
        "expected a check-4 violation, got: {combined}"
    );
}

#[test]
fn validate_fails_check2_on_case_tagged_to_undeclared_block() {
    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());
    write_fixture_with_tags(dir.path(), &["tevv-athlon:demo:ghost-block"]);

    let out = agc()
        .current_dir(dir.path())
        .args(["athlon", "validate", "--slug", "demo"])
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
        combined.contains("undeclared block"),
        "expected an undeclared-block violation, got: {combined}"
    );
}

// ─── report ─────────────────────────────────────────────────────────────────

#[test]
fn report_shows_satisfied_when_enough_passing_history_exists() {
    let _lock = ATHLON_HISTORY_LOCK.lock().expect("athlon history lock");
    let history_path = temp_history_db();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());
    persist_passing_runs(
        "report-satisfied",
        &[
            "tevv-athlon:demo:helpfulness:user-testing",
            "tevv-athlon:demo:helpfulness",
        ],
        3,
    );

    let out_path = dir.path().join("report.md");
    agc()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "athlon",
            "report",
            "--slug",
            "demo",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let report = fs::read_to_string(&out_path).expect("read report");
    assert!(report.contains("TEVV-Athlon Report — demo"));
    assert!(report.contains("helpfulness"));
    assert!(
        report.contains("Satisfied"),
        "expected Satisfied, got:\n{report}"
    );
    assert!(
        report.contains("joint, all Events"),
        "expected a joint row, got:\n{report}"
    );

    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn report_external_evidence_renders_citation_and_dispute_marker() {
    let _lock = ATHLON_HISTORY_LOCK.lock().expect("athlon history lock");
    let history_path = temp_history_db();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("evidence.jsonl"), "{}").expect("write evidence file");
    scaffold_athlon_with_native_event(dir.path());
    agc()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "athlon",
            "add-event",
            "--slug",
            "demo",
            "--block-id",
            "helpfulness",
            "--event-id",
            "red-teaming",
            "--external",
            "--tool",
            "garak",
            "--description",
            "probe suite",
            "--evidence-path",
            "evidence.jsonl",
            "--summary",
            "2 violations",
            "--result",
            "fail",
        ])
        .assert()
        .success();

    let out_path = dir.path().join("report.md");
    agc()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "athlon",
            "report",
            "--slug",
            "demo",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let report = fs::read_to_string(&out_path).expect("read report");
    assert!(report.contains("external evidence disputes this score"));
    assert!(report.contains("### External evidence"));
    assert!(report.contains("garak"));

    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn report_oscal_output_is_valid_json_for_the_athlon_framework() {
    let _lock = ATHLON_HISTORY_LOCK.lock().expect("athlon history lock");
    let history_path = temp_history_db();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());
    persist_passing_runs(
        "report-oscal",
        &[
            "tevv-athlon:demo:helpfulness:user-testing",
            "tevv-athlon:demo:helpfulness",
        ],
        3,
    );

    let out = agc()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args(["athlon", "report", "--slug", "demo", "--oscal"])
        .assert()
        .success()
        .get_output()
        .clone();

    let json: Value = serde_json::from_slice(&out.stdout).expect("valid OSCAL JSON");
    assert!(json.get("assessment-results").is_some());

    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn report_fails_loudly_on_stale_materialization() {
    let _lock = ATHLON_HISTORY_LOCK.lock().expect("athlon history lock");
    let history_path = temp_history_db();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let dir = tempfile::tempdir().expect("tempdir");
    scaffold_athlon_with_native_event(dir.path());

    // Simulate a corrupted/hand-deleted materialized registry file.
    fs::write(dir.path().join("frameworks/tevv-athlon-demo.json"), "[]").expect("truncate");

    agc()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args(["athlon", "report", "--slug", "demo"])
        .assert()
        .failure();

    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

// ─── run ────────────────────────────────────────────────────────────────────

#[test]
fn run_refuses_when_athlon_has_validation_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "init",
            "--slug",
            "demo",
            "--objective",
            "obj",
            "--lifecycle-stage",
            "deploy",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "add-block",
            "--slug",
            "demo",
            "--block-id",
            "empty-block",
            "--definition",
            "d",
        ])
        .assert()
        .success();
    // `empty-block` has zero Events — check 1 must fire, and `run` must refuse
    // to shell out to `agc eval` at all.

    let out = agc()
        .current_dir(dir.path())
        .args(["athlon", "run", "--slug", "demo"])
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
        combined.contains("validation error"),
        "expected run to refuse due to validation errors, got: {combined}"
    );
}

#[test]
fn run_reports_nothing_to_run_when_all_events_are_external() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A real file the external Event's evidence_path can point at, so
    // `validate` check 7 passes and doesn't mask the scenario under test.
    fs::write(dir.path().join("evidence.jsonl"), "{}").expect("write evidence file");
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "init",
            "--slug",
            "demo",
            "--objective",
            "obj",
            "--lifecycle-stage",
            "deploy",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "add-block",
            "--slug",
            "demo",
            "--block-id",
            "b1",
            "--definition",
            "d",
        ])
        .assert()
        .success();
    agc()
        .current_dir(dir.path())
        .args([
            "athlon",
            "add-event",
            "--slug",
            "demo",
            "--block-id",
            "b1",
            "--event-id",
            "e1",
            "--external",
            "--tool",
            "garak",
            "--description",
            "d",
            "--evidence-path",
            "evidence.jsonl",
            "--result",
            "pass",
        ])
        .assert()
        .success();

    let out = agc()
        .current_dir(dir.path())
        .args(["athlon", "run", "--slug", "demo"])
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
        combined.contains("nothing to run") || combined.contains("never executed"),
        "expected a nothing-to-run message, got: {combined}"
    );
}
