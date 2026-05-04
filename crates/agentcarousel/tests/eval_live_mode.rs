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
fn eval_live_requires_generator_key() {
    let root = workspace_root();
    Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(&root)
        .env_remove("AGENTCAROUSEL_GENERATOR_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("AGENTCAROUSEL_JUDGE_KEY")
        .args([
            "eval",
            "fixtures/examples/example-skill.yaml",
            "--execution-mode",
            "live",
            "--model",
            "gpt-4o-mini",
        ])
        .assert()
        .failure()
        .code(3);
}
