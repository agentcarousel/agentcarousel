use assert_cmd::Command;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(std::path::PathBuf::from)
        .expect("workspace root")
}

fn spawn_json_server(status_line: &str, body: &str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = mpsc::channel::<String>();
    let status = status_line.to_string();
    let payload = body.to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut req = [0u8; 2048];
        let n = stream.read(&mut req).expect("read");
        let request = String::from_utf8_lossy(&req[..n]).to_string();
        let first_line = request.lines().next().unwrap_or_default().to_string();
        let _ = tx.send(first_line);

        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    (format!("http://{}", addr), rx)
}

#[test]
fn trust_check_fails_without_registry_url() {
    std::env::remove_var("REGISTRY_API_BASE_URL");
    std::env::remove_var("REGISTRY_URL");
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args(["trust-check", "cmmc-assessor@1.0.0"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("registry URL is required"),
        "expected missing URL guidance, got: {stderr:?}"
    );
}

#[test]
fn trust_check_parses_bundle_version_and_enforces_threshold() {
    let (url, rx) = spawn_json_server(
        "200 OK",
        r#"{"bundle_id":"cmmc-assessor-1.0.0","trust_state":"Experimental"}"#,
    );
    let root = workspace_root();
    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "trust-check",
            "cmmc-assessor@1.0.0",
            "--url",
            &url,
            "--min-trust",
            "trusted",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("below required threshold"),
        "expected threshold failure, got: {stderr:?}"
    );

    let first_line = rx.recv().expect("request first line");
    assert!(
        first_line.contains("GET /v1/bundles/cmmc-assessor-1.0.0/trust-state"),
        "unexpected request path: {first_line:?}"
    );
}

#[test]
fn trust_check_reports_missing_minisign_binary() {
    let (url, _rx) = spawn_json_server("200 OK", r#"{"trust_state":"Trusted"}"#);
    let root = workspace_root();
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let attestation = temp_dir.path().join("attestation.json");
    let pubkey = temp_dir.path().join("minisign.pub");
    std::fs::write(&attestation, "{}").expect("write attestation");
    std::fs::write(&pubkey, "untrusted comment: minisign public key\nRWQ...")
        .expect("write pubkey");

    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .args([
            "trust-check",
            "cmmc-assessor@1.0.0",
            "--url",
            &url,
            "--attestation",
            attestation.to_str().expect("attestation path"),
            "--minisign-pubkey",
            pubkey.to_str().expect("pubkey path"),
            "--minisign-bin",
            "definitely-not-a-real-minisign-binary",
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("failed to run `definitely-not-a-real-minisign-binary`"),
        "expected minisign error, got: {stderr:?}"
    );
}
