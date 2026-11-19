//! Provenance recorded for the assurance spec: trace-step sources (INV-3) and per-score
//! evaluator class (INV-4).
use agentcarousel::{fetch_run, ScoreSource, StepKind, TraceSource};
use assert_cmd::Command;
use std::fs;
use std::path::Path;

const CASES: &str = r#"schema_version: 1
skill_or_agent: provenance-demo
bundle_id: test/provenance-demo
bundle_version: 1.0.0
defaults:
  evaluator: rules
cases:
  - id: provenance-demo/tool-case
    input:
      messages:
        - role: user
          content: "What is the weather in Paris?"
    expected:
      tool_sequence:
        - tool: weather_lookup
          args_match: { city: "Paris" }
          order: strict
      rubric:
        - id: mentions_sunny
          description: Reports the forecast
          weight: 0.5
          auto_check: { kind: contains, value: "sunny" }
        - id: friendly_tone
          description: Friendly tone (needs a judge)
          weight: 0.5
"#;

const MOCKS: &str = r#"[
  {"tool": "weather_lookup", "args_match": {"city": "Paris"}, "response": {"forecast": "sunny"}},
  {"tool": "agent_response", "args_match": {"case_id": "provenance-demo/tool-case"}, "response": "It is sunny in Paris."}
]"#;

fn write_demo_fixture(dir: &Path) {
    fs::create_dir_all(dir.join("fixtures/provenance-demo")).unwrap();
    fs::create_dir_all(dir.join("mocks")).unwrap();
    fs::write(dir.join("fixtures/provenance-demo/cases.yaml"), CASES).unwrap();
    fs::write(dir.join("mocks/provenance-demo.json"), MOCKS).unwrap();
}

#[test]
fn mock_eval_records_trace_and_score_sources() {
    let dir = tempfile::tempdir().unwrap();
    write_demo_fixture(dir.path());
    let history = dir.path().join("history.db");

    let assert = Command::cargo_bin("agentcarousel")
        .unwrap()
        .current_dir(dir.path())
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args([
            "eval",
            "fixtures/provenance-demo/cases.yaml",
            "--execution-mode",
            "mock",
            "--json",
        ])
        .assert();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let envelope: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("eval JSON envelope ({e}): {stdout}"));
    let run_id = envelope["data"]["id"].as_str().expect("run id").to_string();

    std::env::set_var("AGENTCAROUSEL_HISTORY_DB", &history);
    let run = fetch_run(&run_id).expect("fetch run");
    std::env::remove_var("AGENTCAROUSEL_HISTORY_DB");

    let case = &run.cases[0];
    let tool_call = case
        .trace
        .steps
        .iter()
        .find(|s| matches!(s.kind, StepKind::ToolCall))
        .expect("tool call step");
    assert_eq!(tool_call.source, Some(TraceSource::FixtureExpectation));
    let tool_result = case
        .trace
        .steps
        .iter()
        .find(|s| matches!(s.kind, StepKind::ToolResult))
        .expect("tool result step");
    assert_eq!(tool_result.source, Some(TraceSource::Mock));

    let scores = case.eval_scores.as_ref().expect("eval scores");
    let source_of = |id: &str| {
        scores
            .rubric_scores
            .iter()
            .find(|r| r.rubric_id == id)
            .and_then(|r| r.source)
    };
    assert_eq!(
        source_of("mentions_sunny"),
        Some(ScoreSource::Deterministic)
    );
    assert_eq!(source_of("friendly_tone"), Some(ScoreSource::NotEvaluated));
}
