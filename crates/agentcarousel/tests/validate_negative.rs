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
fn validate_examples() {
    let root = workspace_root();
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["validate", "fixtures/examples/example-skill.yaml"])
        .assert()
        .success();
}

#[test]
fn validate_invalid_examples() {
    let root = workspace_root();
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["validate", "fixtures/examples/invalid-skill.yaml"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn validate_json_includes_atf_summary() {
    let root = workspace_root();
    let out = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "validate",
            "fixtures/examples/example-skill.yaml",
            "--format",
            "json",
        ])
        .output()
        .expect("run validate");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(body.trim()).expect("json");
    assert!(v.get("messages").is_some());
    let summary = v.get("atf_summary").expect("atf_summary");
    assert!(
        summary
            .get("fixture_files_loaded")
            .and_then(|n| n.as_u64())
            .unwrap_or(0)
            >= 1
    );
}
