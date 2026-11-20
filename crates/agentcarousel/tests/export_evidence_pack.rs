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
        skip_reason: None,
        critical: false,
        severity: None,
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
        subject: None,
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

#[test]
fn export_signs_with_password_protected_key_and_keeps_json_stdout_clean() {
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
    let (sk, pk) = (keys.path().join("enc.key"), keys.path().join("enc.pub"));
    let mut gen = std::process::Command::new("minisign")
        .args([
            "-G",
            "-f",
            "-s",
            sk.to_str().unwrap(),
            "-p",
            pk.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::io::Write::write_all(gen.stdin.as_mut().unwrap(), b"pw\npw\n").unwrap();
    assert!(gen.wait().unwrap().success());

    let (history_path, _config) = temp_paths();
    let run_id = "verify-encrypted-key-run";
    persist_run_with_db(&history_path, run_id);
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("enc.tar.gz");
    let output = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(workspace_root())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "--json",
            "export",
            run_id,
            "--out",
            out_path.to_str().unwrap(),
            "--sign-key",
            sk.to_str().unwrap(),
        ])
        .write_stdin("pw\n")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is pure JSON");
    assert_eq!(stdout["ok"], true);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Password"),
        "password prompt should reach the user's terminal (stderr)"
    );

    let (ok, v) = verify_json(&[
        out_path.to_str().unwrap(),
        "--minisign-pubkey",
        pk.to_str().unwrap(),
    ]);
    assert!(ok, "{v}");
    assert_eq!(v["data"]["status"], "verified");
}

fn persist_run_with_db(history_path: &std::path::Path, run_id: &str) {
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", history_path);
    persist_run(&minimal_run(run_id, Utc::now())).expect("persist run");
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

const PROMPT_SENTINEL: &str = "SENTINEL-SYSTEM-PROMPT-7f3a";

fn run_with_prompt_material(id: &str) -> Run {
    let mut run = minimal_run(id, Utc::now());
    run.cases[0].input = vec![
        agentcarousel::Message {
            role: agentcarousel::Role::System,
            content: format!("You are a careful agent. {PROMPT_SENTINEL}"),
        },
        agentcarousel::Message {
            role: agentcarousel::Role::User,
            content: "hello".to_string(),
        },
    ];
    run.prompt_audit = Some(agentcarousel::PromptAudit {
        failure_mode: agentcarousel::PromptAuditFailureMode::Prompt,
        confidence: 0.5,
        findings: Vec::new(),
        suggested_fixes: vec!["Tighten the instructions.".to_string()],
        suggested_implementations: vec![format!("## Instructions\n{PROMPT_SENTINEL}")],
        suggested_locations: vec![format!("## Rules {PROMPT_SENTINEL}")],
        overall_rationale: "test".to_string(),
        judge_tokens_in: None,
        judge_tokens_out: None,
    });
    run
}

/// Extracts the tarball and returns the concatenated contents of every file in it.
fn tarball_text(root: &std::path::Path, tarball: &std::path::Path) -> String {
    let dir = tempfile::tempdir().expect("extract dir");
    let status = Command::new("tar")
        .current_dir(root)
        .args(["-xzf", tarball.to_str().unwrap(), "-C"])
        .arg(dir.path())
        .output()
        .expect("tar extract");
    assert!(status.status.success());
    let mut text = String::new();
    let mut stack = vec![dir.path().to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("read dir") {
            let entry = entry.expect("entry");
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                text.push_str(&String::from_utf8_lossy(&fs::read(&p).expect("read file")));
            }
        }
    }
    text
}

#[test]
fn export_withholds_prompt_material_by_default_and_discloses_on_request() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let (history_path, _config_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    let run_id = "export-prompt-disclosure";
    persist_run(&run_with_prompt_material(run_id)).expect("persist run");

    let root = workspace_root();
    let default_out = root.join("target/export-prompt-default.tar.gz");
    let disclosed_out = root.join("target/export-prompt-disclosed.tar.gz");
    for (out, extra) in [
        (&default_out, None),
        (&disclosed_out, Some("--include-prompt")),
    ] {
        let _ = fs::remove_file(out);
        let mut cmd = Command::cargo_bin("agentcarousel").unwrap();
        cmd.current_dir(&root)
            .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
            .args(["export", run_id, "--out", out.to_str().expect("out")]);
        if let Some(flag) = extra {
            cmd.arg(flag);
        }
        cmd.assert().success();
    }

    let withheld = tarball_text(&root, &default_out);
    assert!(
        !withheld.contains(PROMPT_SENTINEL),
        "default export leaked prompt text"
    );
    assert!(
        withheld.contains("[system prompt withheld; sha256:"),
        "default export should mark the withheld system prompt"
    );
    assert!(withheld.contains("Tighten the instructions."));

    let disclosed = tarball_text(&root, &disclosed_out);
    assert!(
        disclosed.contains(PROMPT_SENTINEL),
        "--include-prompt export should carry the prompt"
    );

    let _ = fs::remove_file(&default_out);
    let _ = fs::remove_file(&disclosed_out);
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

/// Export a run and return the list of entries in the tarball, plus the tarball path.
fn export_and_list(run: &Run, out_name: &str) -> (Vec<String>, PathBuf) {
    let (history_path, _config_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(run).expect("persist run");

    let root = workspace_root();
    let out_path = root.join(format!("target/{out_name}.tar.gz"));
    let _ = fs::remove_file(&out_path);
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .args([
            "export",
            &run.id.0,
            "--out",
            out_path.to_str().expect("out"),
        ])
        .assert()
        .success();
    let listing = Command::new("tar")
        .args(["-tzf", out_path.to_str().unwrap()])
        .output()
        .expect("tar list");
    assert!(listing.status.success());
    let entries = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (entries, out_path)
}

#[test]
fn export_writes_aiuc1_crosswalk_only_when_cases_carry_aiuc1_tags() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");

    let untagged = minimal_run("export-aiuc1-untagged", Utc::now());
    let (entries, out_path) = export_and_list(&untagged, "export-aiuc1-untagged");
    assert!(
        !entries.iter().any(|e| e.ends_with("compliance_aiuc-1.md")),
        "untagged runs must not mention AIUC-1: {entries:?}"
    );
    let _ = fs::remove_file(&out_path);

    let mut tagged = minimal_run("export-aiuc1-tagged", Utc::now());
    tagged.cases[0].tags = vec!["aiuc-1:D001".to_string()];
    let (entries, out_path) = export_and_list(&tagged, "export-aiuc1-tagged");
    assert!(
        entries.iter().any(|e| e.ends_with("compliance_aiuc-1.md")),
        "tagged runs get the AIUC-1 crosswalk report: {entries:?}"
    );

    let md = Command::new("tar")
        .args([
            "-xOzf",
            out_path.to_str().unwrap(),
            "agentcarousel-evidence-export-aiuc1-tagged/compliance_aiuc-1.md",
        ])
        .output()
        .expect("tar read report");
    let md = String::from_utf8_lossy(&md.stdout);
    assert!(
        md.contains("not affiliated with or endorsed by AIUC"),
        "{md}"
    );
    assert!(md.contains("https://standard.aiuc-1.com/"), "{md}");

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args(["verify", out_path.to_str().unwrap()])
        .assert()
        .success();

    let _ = fs::remove_file(&out_path);
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
}

#[test]
fn export_carries_severity_extension_when_cases_have_severity() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let dir = tempfile::tempdir().expect("temp dir");
    let history = dir.path().join("history.db");
    let fixture = dir.path().join("sev.yaml");
    fs::write(
        &fixture,
        r#"schema_version: 1
skill_or_agent: sev-skill
cases:
  - id: sev-skill/major
    severity: P1
    input:
      messages:
        - role: user
          content: "hello"
    expected:
      output:
        - kind: contains
          value: "does-not-exist"
"#,
    )
    .expect("write fixture");

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args([
            "test",
            fixture.to_str().unwrap(),
            "--mock-dir",
            "mocks",
            "--json",
        ])
        .output()
        .expect("agc test");
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("test json");
    let run_id = envelope["data"]["id"].as_str().expect("run id").to_string();

    let out_path = root.join("target/export-severity-test.tar.gz");
    let _ = fs::remove_file(&out_path);
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args(["export", &run_id, "--out", out_path.to_str().unwrap()])
        .assert()
        .success();

    let evidence = Command::new("tar")
        .args([
            "-xOzf",
            out_path.to_str().unwrap(),
            &format!("agentcarousel-evidence-{run_id}/evidence.json"),
        ])
        .output()
        .expect("tar read evidence");
    let evidence: Value = serde_json::from_slice(&evidence.stdout).expect("evidence json");
    let ext = &evidence["x-agentcarousel-severity"];
    assert_eq!(ext["failed"]["P1"], 1, "{evidence}");
    assert_eq!(ext["open_p0_p1"], true, "{evidence}");

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args(["verify", out_path.to_str().unwrap()])
        .assert()
        .success();
    let _ = fs::remove_file(&out_path);
}

#[test]
fn export_includes_attested_scope_profile_and_verify_detects_tampering() {
    let _lock = EXPORT_HISTORY_LOCK.lock().expect("export tests lock");
    let dir = tempfile::tempdir().expect("temp dir");
    let history = dir.path().join("history.db");
    let fixture_dir = dir.path().join("scoped");
    fs::create_dir_all(&fixture_dir).expect("fixture dir");
    fs::write(
        fixture_dir.join("cases.yaml"),
        r#"schema_version: 1
skill_or_agent: scoped
cases:
  - id: scoped/one
    tags: [grounding]
    input:
      messages:
        - role: user
          content: "hello"
    expected:
      output:
        - kind: contains
          value: "does-not-exist"
"#,
    )
    .expect("write fixture");
    fs::write(
        fixture_dir.join("scope.yaml"),
        r#"schema_version: 1
skill_or_agent: scoped
use_cases: [answer questions]
instance_under_test:
  environment: test harness
  matches_production: false
  data: synthetic
risk_distribution:
  - category: Grounding
    share_pct: 100
    rationale: only risk tested
    tags: [grounding]
tool_calls_observed: false
"#,
    )
    .expect("write scope");

    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args([
            "test",
            fixture_dir.to_str().unwrap(),
            "--mock-dir",
            "mocks",
            "--json",
        ])
        .output()
        .expect("agc test");
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("test json");
    let run_id = envelope["data"]["id"].as_str().expect("run id").to_string();
    assert_eq!(
        envelope["data"]["summary"]["total"], 1,
        "scope.yaml is not a fixture"
    );

    let out_path = root.join("target/export-scope-test.tar.gz");
    let _ = fs::remove_file(&out_path);
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args(["export", &run_id, "--out", out_path.to_str().unwrap()])
        .assert()
        .success();

    let extract = dir.path().join("extract");
    fs::create_dir_all(&extract).expect("extract dir");
    let status = Command::new("tar")
        .args([
            "-xzf",
            out_path.to_str().unwrap(),
            "-C",
            extract.to_str().unwrap(),
        ])
        .output()
        .expect("tar extract");
    assert!(status.status.success());
    let bundle = extract.join(format!("agentcarousel-evidence-{run_id}"));
    let scope: Value =
        serde_json::from_str(&fs::read_to_string(bundle.join("scope.json")).expect("scope.json"))
            .expect("scope json");
    assert_eq!(scope["risk_distribution"][0]["category"], "Grounding");
    let statement = fs::read_to_string(bundle.join("attestation.intoto.json")).expect("statement");
    assert!(statement.contains("scope.json"), "scope.json is attested");

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args(["verify", bundle.to_str().unwrap()])
        .assert()
        .success();

    fs::write(bundle.join("scope.json"), "{\"tampered\": true}").expect("tamper");
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args(["verify", bundle.to_str().unwrap()])
        .assert()
        .failure();
    let _ = fs::remove_file(&out_path);
}
