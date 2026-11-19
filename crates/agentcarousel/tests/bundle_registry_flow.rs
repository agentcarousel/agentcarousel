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

/// Seeded runs must match the live bundle version `agc publish` reads from the
/// fixture, so derive it from cases.yaml instead of hardcoding it.
fn github_actions_bundle_version() -> String {
    let cases =
        fs::read_to_string(workspace_root().join("fixtures/github-actions-generator/cases.yaml"))
            .expect("read github-actions-generator cases.yaml");
    cases
        .lines()
        .find_map(|line| line.strip_prefix("bundle_version:"))
        .expect("bundle_version in cases.yaml")
        .trim()
        .to_string()
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
        input: vec![],
        discrimination_score: None,
        discrimination_label: None,
        tags: Vec::new(),
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
        fixture_bundle_id: Some(bundle_id.to_string()),
        fixture_bundle_version: Some(bundle_version.to_string()),
        skill_or_agent: Some("example-skill".to_string()),
        runner_offline: false,
        runner_mock_strict: false,
        runner_mock_only: true,
        prompt_audit: None,
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
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
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
            "publish",
            "--config",
            config_path.to_str().expect("config path"),
            "fixtures/github-actions-generator",
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
    let registry_id = format!(
        "github-actions-generator-{}",
        github_actions_bundle_version()
    );
    assert!(
        stdout.contains(&registry_id),
        "expected registry bundle id {registry_id} in output, got: {stdout:?}"
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
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
    ))
    .expect("persist run a");
    persist_run(&minimal_run(
        "bundle-registry-run-match-b",
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
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
            "publish",
            "--config",
            config_path.to_str().expect("config path"),
            "fixtures/github-actions-generator",
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
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
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
            "publish",
            "--config",
            config_path.to_str().expect("config path"),
            "fixtures/github-actions-generator",
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
            "fixtures/github-actions-generator",
            "--url",
            "https://registry.example.test",
            "--all-runs",
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ])
        .assert()
        .failure();
    // In JSON mode (stdout not a TTY in tests), errors go to stdout as JSON envelope
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("cannot combine --all-runs with --evidence"),
        "expected argument guidance, got stdout: {stdout:?}, stderr: {stderr:?}"
    );
}

#[test]
fn publish_fails_fast_when_token_missing() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let (history_path, _config_path, _) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(
        "bundle-registry-run-token-missing",
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
    ))
    .expect("persist run");
    std::env::remove_var("AGENTCAROUSEL_API_TOKEN");
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "publish",
            "fixtures/github-actions-generator",
            "--url",
            "https://registry.example.test",
        ])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("registry token missing"),
        "expected missing token guidance, got stdout: {stdout:?}, stderr: {stderr:?}"
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
            "publish",
            "--config",
            config_path.to_str().expect("config path"),
            "fixtures/github-actions-generator",
            "--url",
            "https://registry.example.test",
            "--dry-run",
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("no run found for bundle"),
        "expected run selection error, got stdout: {stdout:?}, stderr: {stderr:?}"
    );
}

/// Minimal HTTP capture server: answers every request with `{}` and records
/// "<METHOD> <PATH>\n<body>" for each one.
fn spawn_capture_registry() -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind capture registry");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                continue;
            }
            let mut content_length = 0usize;
            let mut chunked = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header line");
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                    chunked = true;
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut body = Vec::new();
            if chunked {
                loop {
                    let mut size_line = String::new();
                    reader.read_line(&mut size_line).expect("chunk size");
                    let size = usize::from_str_radix(size_line.trim(), 16).unwrap_or(0);
                    let mut chunk = vec![0u8; size + 2];
                    reader.read_exact(&mut chunk).expect("chunk");
                    if size == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..size]);
                }
            } else {
                body.resize(content_length, 0);
                reader.read_exact(&mut body).expect("body");
            }
            let path = request_line
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ");
            sink.lock()
                .expect("capture lock")
                .push(format!("{path}\n{}", String::from_utf8_lossy(&body)));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
        }
    });
    (url, captured)
}

fn publish_against_capture(include_prompt: bool) -> (String, Vec<String>) {
    let (history_path, _config_path, evidence_path) = temp_paths();
    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history_path);
    persist_run(&minimal_run(
        "bundle-registry-prompt-disclosure",
        "agentcarousel/github-actions-generator",
        &github_actions_bundle_version(),
    ))
    .expect("persist run");
    fs::write(&evidence_path, b"fake").expect("write evidence");
    let (url, captured) = spawn_capture_registry();
    let mut cmd = Command::cargo_bin("agentcarousel").unwrap();
    cmd.current_dir(workspace_root())
        .env("AGENTCAROUSEL_HISTORY_DB", &history_path)
        .env("AGENTCAROUSEL_API_TOKEN", "test-token")
        .args([
            "publish",
            "fixtures/github-actions-generator",
            "--url",
            &url,
            "--evidence",
            evidence_path.to_str().expect("evidence path"),
        ]);
    if include_prompt {
        cmd.arg("--include-prompt");
    }
    let assert = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");
    let requests = captured.lock().expect("capture lock").clone();
    (stdout, requests)
}

#[test]
fn publish_uploads_prompt_only_with_include_prompt() {
    let _guard = HISTORY_ENV_LOCK.lock().expect("history lock");
    let prompt =
        fs::read_to_string(workspace_root().join("fixtures/github-actions-generator/prompt.md"))
            .expect("read prompt.md");
    let probe = prompt
        .lines()
        .find(|line| line.trim().len() > 20)
        .expect("a distinctive prompt line")
        .trim()
        .to_string();

    let (stdout, requests) = publish_against_capture(false);
    let bundle_push = requests
        .iter()
        .find(|r| r.starts_with("POST /v1/bundles\n"))
        .expect("bundle push request");
    assert!(
        !bundle_push.contains(&probe) && !bundle_push.contains("name=\"prompt\""),
        "prompt uploaded without --include-prompt: {bundle_push}"
    );
    assert!(
        stdout.contains("\"prompt_disclosure\": \"digest_only\""),
        "{stdout}"
    );
    assert!(stdout.contains("\"prompt_sha256\": \"sha256:"), "{stdout}");

    let (stdout, requests) = publish_against_capture(true);
    let bundle_push = requests
        .iter()
        .find(|r| r.starts_with("POST /v1/bundles\n"))
        .expect("bundle push request");
    assert!(
        bundle_push.contains(&probe),
        "--include-prompt should upload prompt.md"
    );
    assert!(
        stdout.contains("\"prompt_disclosure\": \"included\""),
        "{stdout}"
    );
}
