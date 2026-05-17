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
fn eval_json_includes_run_id_in_envelope() {
    let root = workspace_root();
    // stdout is piped (not a TTY) → globals.json is auto-true → JSON envelope on stdout
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "eval",
            "examples/example-skill.yaml",
            "--execution-mode",
            "mock",
            "--filter",
            "example-skill/positive",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("expected valid JSON envelope on stdout");
    assert_eq!(parsed["ok"], true, "expected ok:true, got: {stdout:?}");
    // run ID lives in data.id
    assert!(
        parsed["data"]["id"].is_string(),
        "expected data.id in envelope, got: {stdout:?}"
    );
}
