use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn temp_workspace() -> TempDir {
    tempfile::Builder::new()
        .prefix("agentcarousel-init-negative-")
        .tempdir()
        .expect("create temp workspace")
}

fn write_template(root: &Path) {
    let templates_dir = root.join("templates");
    fs::create_dir_all(&templates_dir).expect("create templates dir");
    fs::write(
        templates_dir.join("fixture-skeleton.yaml"),
        "schema_version: 1\nskill_or_agent: <skill-or-agent-id>\ncases: []\n",
    )
    .expect("write fixture template");
    fs::write(
        templates_dir.join("bundle-manifest-skeleton.json"),
        "{\"bundle_id\":\"<org>/<skill-or-agent-id>\",\"fixtures\":[]}\n",
    )
    .expect("write bundle manifest template");
}

#[test]
fn init_rejects_path_separator_names() {
    let workspace = temp_workspace();
    write_template(workspace.path());

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(workspace.path())
        .args(["init", "--skill", "../escape"])
        .assert()
        .failure()
        .code(4);
}

#[test]
fn init_rejects_non_kebab_names() {
    let workspace = temp_workspace();
    write_template(workspace.path());

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(workspace.path())
        .args(["init", "--agent", "Not-Kebab"])
        .assert()
        .failure()
        .code(4);
}

#[test]
fn init_creates_fixture_with_sanitized_name() {
    let workspace = temp_workspace();
    write_template(workspace.path());

    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(workspace.path())
        .args(["init", "--skill", "sample-agent"])
        .assert()
        .success();

    let cases_path = workspace.path().join("fixtures/sample-agent/cases.yaml");
    assert!(cases_path.exists(), "expected fixtures/sample-agent/cases.yaml to exist");
    let contents = fs::read_to_string(&cases_path).expect("read generated fixture");
    assert!(contents.contains("skill_or_agent: sample-agent"));

    assert!(workspace.path().join("fixtures/sample-agent/prompt.md").exists());
    assert!(workspace.path().join("fixtures/sample-agent/bundle.manifest.json").exists());
    assert!(workspace.path().join("fixtures/sample-agent/golden").is_dir());
}
