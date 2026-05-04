use agentcarousel::{
    persist_run, CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, OverallStatus,
    ProviderErrorMetrics, Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::Utc;
use std::fs;
use std::path::PathBuf;

fn temp_paths() -> (PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.keep();
    (base.join("history.db"), base.join("agentcarousel.toml"))
}

fn run_with_latency(id: &str, latency_ms: u64) -> Run {
    let case = CaseResult {
        case_id: CaseId("example-skill/positive".to_string()),
        status: CaseStatus::Passed,
        error: None,
        trace: ExecutionTrace {
            steps: Vec::new(),
            final_output: Some("ok".to_string()),
            redacted: false,
        },
        metrics: Metrics {
            total_latency_ms: latency_ms,
            ..Metrics::default()
        },
        eval_scores: None,
    };
    Run {
        id: RunId(id.to_string()),
        schema_version: 1,
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
        command: "test".to_string(),
        git_sha: None,
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: "none".to_string(),
        cases: vec![case],
        summary: RunSummary {
            total: 1,
            passed: 1,
            failed: 0,
            skipped: 0,
            flaky: 0,
            errored: 0,
            timed_out: 0,
            pass_rate: 1.0,
            mean_latency_ms: latency_ms as f64,
            mean_effectiveness_score: None,
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: OverallStatus::Pass,
        },
        fixture_bundle_id: None,
        fixture_bundle_version: None,
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
    }
}

#[test]
fn report_diff_uses_config_threshold() {
    let (history_path, config_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let run_a = run_with_latency("run-a", 100);
    let run_b = run_with_latency("run-b", 103);
    persist_run(&run_a).expect("persist run a");
    persist_run(&run_b).expect("persist run b");

    fs::write(
        &config_path,
        format!(
            r#"[report]
history_db = "{}"
regression_threshold = 0.0
"#,
            history_path.display()
        ),
    )
    .expect("write config");

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "report",
            "diff",
            "run-a",
            "run-b",
        ])
        .assert()
        .failure()
        .code(1);
}
