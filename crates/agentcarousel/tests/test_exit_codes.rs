use assert_cmd::Command;
use std::fs;

#[test]
fn test_command_returns_failure_on_bad_output() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture_path = dir.path().join("example-fail.yaml");
    fs::write(
        &fixture_path,
        r#"schema_version: 1
skill_or_agent: example-skill
cases:
  - id: example-skill/positive
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

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args([
            "test",
            fixture_path.to_str().expect("fixture path"),
            "--mock-dir",
            "mocks",
        ])
        .assert()
        .failure()
        .code(1);
}
