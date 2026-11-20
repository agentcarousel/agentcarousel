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

/// Write a fixture with one case that always fails (its output never contains the
/// expected text), optionally carrying a severity.
fn failing_fixture(dir: &std::path::Path, severity: Option<&str>) -> std::path::PathBuf {
    let severity_line = severity
        .map(|s| format!("    severity: {s}\n"))
        .unwrap_or_default();
    let path = dir.join("severity-fixture.yaml");
    fs::write(
        &path,
        format!(
            r#"schema_version: 1
skill_or_agent: example-skill
cases:
  - id: example-skill/sev
{severity_line}    input:
      messages:
        - role: user
          content: "hello"
    expected:
      output:
        - kind: contains
          value: "does-not-exist"
"#
        ),
    )
    .expect("write fixture");
    path
}

fn agc_test(fixture: &std::path::Path, history: &std::path::Path, extra: &[&str]) -> Command {
    let mut cmd = Command::cargo_bin("agentcarousel").unwrap();
    cmd.env("AGENTCAROUSEL_HISTORY_DB", history.join("history.db"))
        .args(["test", fixture.to_str().unwrap(), "--mock-dir", "mocks"])
        .args(extra);
    cmd
}

#[test]
fn fail_on_severity_fails_at_or_above_threshold() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = failing_fixture(dir.path(), Some("P1"));
    agc_test(&fixture, dir.path(), &["--fail-on-severity", "P1"])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn fail_on_severity_passes_when_failures_are_below_threshold() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = failing_fixture(dir.path(), Some("P3"));
    let out = agc_test(&fixture, dir.path(), &["--fail-on-severity", "P1"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("P3 1"), "{stderr}");
}

#[test]
fn fail_on_severity_counts_unrated_failures_as_failing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = failing_fixture(dir.path(), None);
    let out = agc_test(&fixture, dir.path(), &["--fail-on-severity", "P0"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(stderr.contains("without severity"), "{stderr}");
}

#[test]
fn without_the_flag_any_failure_still_fails() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = failing_fixture(dir.path(), Some("P3"));
    agc_test(&fixture, dir.path(), &[])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn severity_must_be_uppercase_p0_to_p3() {
    let dir = tempfile::tempdir().expect("temp dir");
    let fixture = failing_fixture(dir.path(), Some("p1"));
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .args(["validate", fixture.to_str().unwrap()])
        .assert()
        .failure();
}
