//! `report show` uses the same `print_terminal` path as eval/test, including humanized error lines.
//! File paths (run.json or evidence directory) are supported for offline review.

use agentcarousel::{
    CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, OverallStatus, ProviderErrorMetrics,
    Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

fn run_with_embedded_api_error() -> Run {
    let err = r#"provider: {
  "error": {
    "code": 400,
    "message": "API key not valid. Please pass a valid API key.",
    "status": "INVALID_ARGUMENT"
  }
}"#;
    let case = CaseResult {
        case_id: CaseId("demo/case-err".to_string()),
        status: CaseStatus::Error,
        error: Some(err.to_string()),
        trace: ExecutionTrace {
            steps: Vec::new(),
            final_output: None,
            redacted: false,
        },
        metrics: Metrics {
            total_latency_ms: 0,
            ..Metrics::default()
        },
        eval_scores: None,
    };
    Run {
        id: RunId("test-report-show-humanize".to_string()),
        schema_version: 1,
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
        command: "eval".to_string(),
        git_sha: None,
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: "test".to_string(),
        cases: vec![case],
        summary: RunSummary {
            total: 1,
            passed: 0,
            failed: 0,
            skipped: 0,
            flaky: 0,
            errored: 1,
            timed_out: 0,
            pass_rate: 0.0,
            mean_latency_ms: 0.0,
            mean_effectiveness_score: None,
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: OverallStatus::Fail,
            tokens_in: None,
            tokens_out: None,
            mean_tokens_per_judged_case: None,
            latency_p50_ms: None,
            latency_p95_ms: None,
            latency_p99_ms: None,
        },
        fixture_bundle_id: None,
        fixture_bundle_version: None,
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
        skill_or_agent: Some("demo-skill".to_string()),
        runner_offline: true,
        runner_mock_strict: false,
        runner_mock_only: true,
    }
}

#[test]
fn report_show_file_path_prints_humanized_error() {
    let root = workspace_root();
    let dir = tempfile::tempdir().expect("tempdir");
    let run_path = dir.path().join("run.json");
    let run = run_with_embedded_api_error();
    fs::write(
        &run_path,
        serde_json::to_string_pretty(&run).expect("serialize run"),
    )
    .expect("write run.json");

    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["report", "show", run_path.to_str().expect("utf8 path")])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(
        s.contains("API key not valid"),
        "expected humanized message in terminal output, got:\n{s}"
    );
    assert!(
        !s.contains("INVALID_ARGUMENT"),
        "expected JSON noise removed from terminal, got:\n{s}"
    );
}

#[test]
fn report_show_evidence_dir_prints_humanized_error() {
    let root = workspace_root();
    let dir = tempfile::tempdir().expect("tempdir");
    let run = run_with_embedded_api_error();
    fs::write(
        dir.path().join("run.json"),
        serde_json::to_string_pretty(&run).expect("serialize run"),
    )
    .expect("write run.json");

    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["report", "show", dir.path().to_str().expect("utf8")])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("API key not valid"), "got:\n{s}");
}
