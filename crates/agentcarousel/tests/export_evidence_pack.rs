use agentcarousel::{
    persist_run, CaseId, CaseResult, CaseStatus, ExecutionTrace, Metrics, ProviderErrorMetrics,
    Run, RunId, RunSummary,
};
use assert_cmd::Command;
use chrono::{Duration, Utc};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// `persist_run` reads `AGENTCAROUSEL_HISTORY_DB` from the process environment; serialize these tests.
static EXPORT_HISTORY_LOCK: Mutex<()> = Mutex::new(());

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

fn temp_paths() -> (PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let base = dir.keep();
    (base.join("history.db"), base.join("agentcarousel.toml"))
}

fn minimal_run(id: &str, started_at: chrono::DateTime<Utc>) -> Run {
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
    Run {
        id: RunId(id.to_string()),
        schema_version: 1,
        started_at,
        finished_at: Some(started_at),
        command: "test".to_string(),
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
            mean_effectiveness_score: None,
            provider_errors: ProviderErrorMetrics::default(),
            overall_status: agentcarousel::OverallStatus::Pass,
        },
        fixture_bundle_id: Some("test/bundle".to_string()),
        fixture_bundle_version: Some("1.0.0".to_string()),
        carousel_iteration: None,
        certification_context: None,
        policy_version: None,
    }
}

#[test]
fn export_tarball_contains_manifest_and_schema_hash() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let (history_path, config_path) = temp_paths();
    let run_id = "export-manifest-test-run";
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let run = minimal_run(run_id, Utc::now());
    persist_run(&run).expect("persist run");

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
    let out_path = root.join("target/export-manifest-test.tar.gz");
    let _ = fs::remove_file(&out_path);

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "--config",
            config_path.to_str().expect("config"),
            "export",
            run_id,
            "--out",
            out_path.to_str().expect("out"),
        ])
        .assert()
        .success();

    let inner_prefix = format!("agentcarousel-evidence-{run_id}");
    let manifest_json = Command::new("tar")
        .current_dir(&root)
        .args([
            "-xOzf",
            out_path.to_str().unwrap(),
            &format!("{inner_prefix}/MANIFEST.json"),
        ])
        .output()
        .expect("tar read MANIFEST");
    assert!(
        manifest_json.status.success(),
        "tar: {}",
        String::from_utf8_lossy(&manifest_json.stderr)
    );
    let manifest: Value = serde_json::from_slice(&manifest_json.stdout).expect("MANIFEST json");
    let files = manifest
        .get("files")
        .and_then(|f| f.as_array())
        .expect("manifest files");
    assert!(
        files
            .iter()
            .any(|e| e.get("path").and_then(|p| p.as_str()) == Some("run.json")),
        "{manifest}"
    );

    let lock_json = Command::new("tar")
        .current_dir(&root)
        .args([
            "-xOzf",
            out_path.to_str().unwrap(),
            &format!("{inner_prefix}/fixture_bundle.lock"),
        ])
        .output()
        .expect("tar read lock");
    assert!(lock_json.status.success());
    let lock: Value = serde_json::from_slice(&lock_json.stdout).expect("lock json");
    let schema_hash = lock.get("schema_hash").expect("schema_hash key");
    assert!(
        schema_hash
            .as_str()
            .is_some_and(|s| s.starts_with("sha256:")),
        "expected schema_hash string, got {schema_hash}"
    );

    let _ = fs::remove_file(&out_path);
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn export_last_writes_newest_tarball_only() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let (history_path, config_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);

    let older_id = "export-last-older";
    let newer_id = "export-last-newer";
    persist_run(&minimal_run(older_id, Utc::now() - Duration::hours(2))).expect("persist older");
    persist_run(&minimal_run(newer_id, Utc::now() - Duration::hours(1))).expect("persist newer");

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

    let dir = tempfile::tempdir().expect("out dir");
    let out_dir = dir.path();

    let root = workspace_root();
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "--config",
            config_path.to_str().expect("config"),
            "export",
            "--last",
            "1",
            "--out-dir",
            out_dir.to_str().expect("out_dir"),
        ])
        .assert()
        .success();

    let names: Vec<String> = fs::read_dir(out_dir)
        .expect("read out dir")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.ends_with(".tar.gz"))
        .collect();
    assert_eq!(names.len(), 1, "expected one tarball, got {names:?}");
    assert!(
        names[0].contains(newer_id),
        "expected newest run exported, got {}",
        names[0]
    );
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}
