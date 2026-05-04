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
fn bundle_verify_ok_prints_for_bundle_directory() {
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["bundle", "verify", "fixtures/bundles/cmmc-assessor"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("bundle verify: OK"),
        "expected success banner on stdout, got: {stdout:?}"
    );
}

#[test]
fn bundle_verify_ok_when_passing_bundle_manifest_json() {
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "bundle",
            "verify",
            "fixtures/bundles/cmmc-assessor/bundle.manifest.json",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("bundle verify: OK"),
        "expected success banner on stdout, got: {stdout:?}"
    );
}

#[test]
fn agc_bundle_verify_matches_agentcarousel_binary() {
    let root = workspace_root();
    Command::cargo_bin("agc")
        .unwrap()
        .current_dir(&root)
        .args(["bundle", "verify", "fixtures/bundles/cmmc-assessor"])
        .assert()
        .success();
}
