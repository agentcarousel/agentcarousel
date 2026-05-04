use agentcarousel::{
    persist_run, CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, OverallStatus,
    ProviderErrorMetrics, Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// `AGENTCAROUSEL_HISTORY_DB` is process-global; serialize tests that mutate it.
static HISTORY_ENV_LOCK: Mutex<()> = Mutex::new(());

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

fn temp_paths() -> (PathBuf, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.keep();
    (
        base.join("history.db"),
        base.join("agentcarousel.toml"),
        base.join("sample-evidence.tar.gz"),
    )
}

fn minimal_run(id: &str, bundle_id: &str, bundle_version: &str) -> Run {
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
            total_latency_ms: 1,
            ..Metrics::default()
        },
        eval_scores: None,
    };
    let started_at = Utc::now();
    Run {
        id: RunId(id.to_string()),
        schema_version: 1,
        started_at,
        finished_at: Some(started_at),
        command: "eval".to_string(),
        git_sha: Some("a".repeat(40)),
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
            mean_latency_ms: 1.0,
            mean_effectiveness_score: Some(0.9),
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: OverallStatus::Pass,
        },
        fixture_bundle_id: Some(bundle_id.to_string()),
        fixture_bundle_version: Some(bundle_version.to_string()),
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
    }
}

fn insert_legacy_malformed_run(history_path: &PathBuf, id: &str) {
    let conn = Connection::open(history_path).expect("open history db");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS runs (
            id TEXT PRIMARY KEY,
            started_at TEXT NOT NULL,
            run_json TEXT NOT NULL
        )",
        [],
    )
    .expect("create runs table");
    // Intentionally missing modern required fields (e.g., provider_errors).
    let legacy_json = r#"{"id":"legacy-run","schema_version":1}"#;
    conn.execute(
        "INSERT OR REPLACE INTO runs (id, started_at, run_json) VALUES (?1, ?2, ?3)",
        params![id, Utc::now().to_rfc3339(), legacy_json],
    )
    .expect("insert malformed run");
}

#[test]
fn publish_dry_run_auto_selects_latest_matching_run() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, config_path, evidence_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(
        "bundle-registry-run-match",
        "agentcarousel/cmmc-assessor",
        "1.0.0",
    ))
    .expect("persist run");

    fs::write(
        &config_path,
        format!(
            r#"[report]
history_db = "{}"
"#,
            history_path.display()
        ),
    )
    .expect("write config");
    fs::write(&evidence_path, b"fake").expect("write evidence");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
            "--dry-run",
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("bundle-registry-run-match"),
        "expected selected run id in output, got: {stdout:?}"
    );
    assert!(
        stdout.contains("cmmc-assessor-1.0.0"),
        "expected registry bundle id in output, got: {stdout:?}"
    );

    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn publish_dry_run_all_runs_lists_multiple_run_ids() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, config_path, evidence_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(
        "bundle-registry-run-match-a",
        "agentcarousel/cmmc-assessor",
        "1.0.0",
    ))
    .expect("persist run a");
    persist_run(&minimal_run(
        "bundle-registry-run-match-b",
        "agentcarousel/cmmc-assessor",
        "1.0.0",
    ))
    .expect("persist run b");

    fs::write(
        &config_path,
        format!(
            r#"[report]
history_db = "{}"
"#,
            history_path.display()
        ),
    )
    .expect("write config");
    fs::write(&evidence_path, b"fake").expect("write evidence");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
            "--dry-run",
            "--all-runs",
            "--limit",
            "2",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("\"run_count\": 2"),
        "expected run count in output, got: {stdout:?}"
    );
    assert!(
        stdout.contains("bundle-registry-run-match-a")
            || stdout.contains("bundle-registry-run-match-b"),
        "expected matching run ids in output, got: {stdout:?}"
    );
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn publish_dry_run_all_runs_skips_unreadable_history_rows() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, config_path, _) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    insert_legacy_malformed_run(&history_path, "legacy-malformed-run");
    persist_run(&minimal_run(
        "bundle-registry-run-match-good",
        "agentcarousel/cmmc-assessor",
        "1.0.0",
    ))
    .expect("persist valid run");

    fs::write(
        &config_path,
        format!(
            r#"[report]
history_db = "{}"
"#,
            history_path.display()
        ),
    )
    .expect("write config");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
            "--dry-run",
            "--all-runs",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stdout.contains("bundle-registry-run-match-good"),
        "expected valid run id in output, got: {stdout:?}"
    );
    assert!(
        stderr.contains("skipping unreadable run legacy-malformed-run"),
        "expected unreadable run warning, got: {stderr:?}"
    );
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn publish_rejects_all_runs_with_single_evidence_path() {
    let (_, _, evidence_path) = temp_paths();
    fs::write(&evidence_path, b"fake").expect("write evidence");
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
            "--all-runs",
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("cannot combine --all-runs with --evidence"),
        "expected argument guidance, got: {stderr:?}"
    );
}

#[test]
fn publish_fails_fast_when_token_missing() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, _config_path, _) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(
        "bundle-registry-run-token-missing",
        "agentcarousel/cmmc-assessor",
        "1.0.0",
    ))
    .expect("persist run");
    std::env::remove_var("AGENTCAROUSEL_API_TOKEN");
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("registry token missing"),
        "expected missing token guidance, got: {stderr:?}"
    );
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn publish_dry_run_errors_when_no_matching_run_exists() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, config_path, evidence_path) = temp_paths();
    fs::write(
        &config_path,
        format!(
            r#"[report]
history_db = "{}"
"#,
            history_path.display()
        ),
    )
    .expect("write config");
    fs::write(&evidence_path, b"fake").expect("write evidence");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "publish",
            "fixtures/bundles/cmmc-assessor",
            "--url",
            "https://registry.example.test",
            "--dry-run",
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("no run found for bundle"),
        "expected run selection error, got: {stderr:?}"
    );
}
