use assert_cmd::Command;
use mockito::Matcher;
use std::fs;

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(std::path::PathBuf::from)
        .expect("workspace root")
}

#[test]
fn bundle_pull_fetches_manifest_and_files() {
    let mut server = mockito::Server::new();
    let base = server.url();

    let manifest = serde_json::json!({
        "bundle_id": "test/pull-fixture",
        "bundle_version": "1.0.0",
        "fixtures": [{
            "path": "skills/hello.yaml",
            "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        }],
        "mocks": []
    });

    let m_manifest = server
        .mock("GET", "/v1/bundles/pull-fixture-1.0.0/manifest")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(manifest.to_string())
        .create();

    let m_file = server
        .mock(
            "GET",
            Matcher::Regex(r"/v1/bundles/pull-fixture-1\.0\.0/file\?path=.*".into()),
        )
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body("")
        .create();

    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("pulled");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "bundle",
            "pull",
            "pull-fixture-1.0.0",
            "--url",
            base.trim_end_matches('/'),
            "-o",
        ])
        .arg(&out)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("bundle pull: wrote"),
        "expected success banner, got: {stdout:?}"
    );

    m_manifest.assert();
    m_file.assert();

    assert!(out.join("bundle.manifest.json").exists());
    assert!(out.join("skills").join("hello.yaml").exists());
    let written = fs::read_to_string(out.join("skills").join("hello.yaml")).expect("read");
    assert_eq!(written, "");
}

#[test]
fn bundle_pull_verify_checks_hashes() {
    let mut server = mockito::Server::new();
    let base = server.url();

    let manifest = serde_json::json!({
        "bundle_id": "test/pull-verify",
        "bundle_version": "1.0.0",
        "fixtures": [{
            "path": "x.txt",
            "sha256": "deadbeef"
        }],
        "mocks": []
    });

    server
        .mock("GET", "/v1/bundles/pull-verify-1.0.0/manifest")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(manifest.to_string())
        .create();

    server
        .mock(
            "GET",
            Matcher::Regex(r"/v1/bundles/pull-verify-1\.0\.0/file\?path=.*".into()),
        )
        .with_status(200)
        .with_body("wrong-bytes")
        .create();

    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("bad");

    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "bundle",
            "pull",
            "pull-verify-1.0.0",
            "--url",
            base.trim_end_matches('/'),
            "-o",
        ])
        .arg(&out)
        .arg("--verify")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("hash mismatch") || stderr.contains("error:"),
        "expected verify/hash error, got: {stderr:?}"
    );
}
