use assert_cmd::Command;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

#[test]
fn eval_json_writes_run_id_hint_to_stderr() {
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "eval",
            "fixtures/examples/example-skill.yaml",
            "--execution-mode",
            "mock",
            "--format",
            "json",
            "--filter",
            "example-skill/positive",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("run id:"),
        "expected run id hint on stderr for json format, got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("report show"),
        "expected report command hint on stderr, got stderr={stderr:?}"
    );
}
