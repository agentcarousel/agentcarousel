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
        .args(["validate", "fixtures/regex-builder/cases.yaml"])
        .assert()
        .success();
}

#[test]
fn validate_invalid_examples() {
    let root = workspace_root();
    let tmp = tempfile::NamedTempFile::with_suffix(".yaml").expect("tmp file");
    std::fs::write(
        tmp.path(),
        "schema_version: 99\nskill_or_agent: ''\ncases:\n  - id: bad case\n",
    )
    .expect("write invalid fixture");
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["validate", tmp.path().to_str().unwrap()])
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
            "fixtures/regex-builder/cases.yaml",
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
