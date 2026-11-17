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
        input: vec![],
        discrimination_score: None,
        discrimination_label: None,
        tags: Vec::new(),
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
        fixture_bundle_id: Some("test/bundle".to_string()),
        fixture_bundle_version: Some("1.0.0".to_string()),
        skill_or_agent: Some("example-skill".to_string()),
        runner_offline: false,
        runner_mock_strict: false,
        runner_mock_only: true,
        prompt_audit: None,
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
        .args(["export", run_id, "--out", out_path.to_str().expect("out")])
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

fn export_for_verify(run_id: &str, extra: &[&str]) -> (PathBuf, PathBuf, tempfile::TempDir) {
    let (history_path, _config_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(run_id, Utc::now())).expect("persist run");
    let dir = tempfile::tempdir().expect("out dir");
    let out_path = dir.path().join(format!("{run_id}.tar.gz"));
    let mut args = vec!["export", run_id, "--out", out_path.to_str().unwrap()];
    args.extend_from_slice(extra);
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(workspace_root())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args(&args)
        .assert()
        .success();
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
    (out_path, history_path, dir)
}

fn verify_json(args: &[&str]) -> (bool, Value) {
    let output = Command::cargo_bin("agentcarousel")
        .unwrap()
        .arg("--json")
        .arg("verify")
        .args(args)
        .output()
        .expect("run verify");
    let v: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (output.status.success(), v)
}

#[test]
fn verify_unsigned_export_reports_integrity_only_and_detects_tampering() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let run_id = "verify-unsigned-run";
    let (tarball, _db, dir) = export_for_verify(run_id, &[]);

    let (ok, v) = verify_json(&[tarball.to_str().unwrap()]);
    assert!(ok, "{v}");
    assert_eq!(v["data"]["status"], "integrity_only", "{v}");
    assert_eq!(v["data"]["signature"], "absent", "{v}");

    // A public key demands a signature; an unsigned bundle must fail.
    let fake_pub = dir.path().join("nokey.pub");
    fs::write(&fake_pub, "untrusted comment: x\nRWQ\n").unwrap();
    let (ok, _) = verify_json(&[
        tarball.to_str().unwrap(),
        "--minisign-pubkey",
        fake_pub.to_str().unwrap(),
    ]);
    assert!(!ok, "unsigned bundle must fail when a pubkey is required");

    // Extract, tamper with one file, and add an unlisted one.
    let extract = dir.path().join("x");
    fs::create_dir_all(&extract).unwrap();
    let status = std::process::Command::new("tar")
        .args([
            "-xzf",
            tarball.to_str().unwrap(),
            "-C",
            extract.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let bundle = extract.join(format!("agentcarousel-evidence-{run_id}"));
    let (ok, _) = verify_json(&[bundle.to_str().unwrap()]);
    assert!(ok, "pristine extracted dir should verify");

    fs::write(bundle.join("report.md"), "edited").unwrap();
    fs::write(bundle.join("extra.txt"), "smuggled").unwrap();
    let (ok, v) = verify_json(&[bundle.to_str().unwrap()]);
    assert!(!ok);
    let msg = v["error"]["message"].as_str().unwrap_or_default();
    assert!(msg.contains("hash mismatch: report.md"), "{v}");
    assert!(msg.contains("not listed in manifest: extra.txt"), "{v}");
}

#[test]
fn signed_export_verifies_with_matching_key_only() {
    if std::process::Command::new("minisign")
        .arg("-v")
        .output()
        .is_err()
    {
        eprintln!("skipping: minisign not installed");
        return;
    }
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let keys = tempfile::tempdir().unwrap();
    let gen = |name: &str| {
        let (sk, pk) = (
            keys.path().join(format!("{name}.key")),
            keys.path().join(format!("{name}.pub")),
        );
        let st = std::process::Command::new("minisign")
            .args([
                "-G",
                "-W",
                "-f",
                "-s",
                sk.to_str().unwrap(),
                "-p",
                pk.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            st.status.success(),
            "{}",
            String::from_utf8_lossy(&st.stderr)
        );
        (sk, pk)
    };
    let (sk, pk) = gen("good");
    let (_other_sk, other_pk) = gen("other");

    let (tarball, _db, _dir) =
        export_for_verify("verify-signed-run", &["--sign-key", sk.to_str().unwrap()]);

    let (ok, v) = verify_json(&[
        tarball.to_str().unwrap(),
        "--minisign-pubkey",
        pk.to_str().unwrap(),
    ]);
    assert!(ok, "{v}");
    assert_eq!(v["data"]["status"], "verified", "{v}");

    let (ok, v) = verify_json(&[tarball.to_str().unwrap()]);
    assert!(ok);
    assert_eq!(v["data"]["signature"], "present_unverified", "{v}");

    let (ok, v) = verify_json(&[
        tarball.to_str().unwrap(),
        "--minisign-pubkey",
        other_pk.to_str().unwrap(),
    ]);
    assert!(!ok, "wrong key must fail: {v}");
}

#[test]
fn export_oscal_ar_uses_import_ap_and_schema_shapes() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let run_id = "oscal-ar-shape-run";
    let href = "https://grc.example/plans/sap-1.json";
    let (tarball, _db, _dir) = export_for_verify(run_id, &["--import-ap", href]);

    let out = std::process::Command::new("tar")
        .args([
            "-xOzf",
            tarball.to_str().unwrap(),
            &format!("agentcarousel-evidence-{run_id}/assessment-results.oscal.json"),
        ])
        .output()
        .expect("tar");
    assert!(out.status.success());
    let ar: Value = serde_json::from_slice(&out.stdout).expect("AR json");
    let doc = &ar["assessment-results"];
    assert_eq!(doc["import-ap"]["href"], href);

    let result = &doc["results"][0];
    let selections = result["reviewed-controls"]["control-selections"]
        .as_array()
        .expect("control-selections array (OSCAL 1.1.2 group-as name)");
    assert!(!selections.is_empty());
    assert!(result["reviewed-controls"]
        .get("control-selection")
        .is_none());
    // Only assessed controls appear: no blanket include-all, one finding each.
    assert!(selections.iter().all(|s| s.get("include-all").is_none()));
    let findings = result["findings"].as_array().map_or(0, Vec::len);
    let included: usize = selections
        .iter()
        .filter_map(|s| s["include-controls"].as_array())
        .map(Vec::len)
        .sum();
    assert_eq!(findings, included, "one finding per assessed control");

    for obs in result["observations"].as_array().into_iter().flatten() {
        assert!(obs.get("method").is_none(), "{obs}");
        assert!(
            obs["methods"].as_array().is_some_and(|m| !m.is_empty()),
            "{obs}"
        );
    }
    for f in result["findings"].as_array().into_iter().flatten() {
        let state = f["target"]["status"]["state"].as_str().unwrap();
        assert!(
            state == "satisfied" || state == "not-satisfied",
            "invalid OSCAL finding state {state}"
        );
    }
}
