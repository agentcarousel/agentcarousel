//! Assurance spec v0.1 documents in evidence exports: evidence.json and the in-toto statement.
use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const CASES: &str = r#"schema_version: 1
skill_or_agent: evidence-demo
bundle_id: test/evidence-demo
bundle_version: 1.0.0
defaults:
  evaluator: rules
cases:
  - id: evidence-demo/tool-case
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
          weight: 1.0
          critical: true
          auto_check: { kind: contains, value: "sunny" }
  - id: evidence-demo/filtered
    input:
      messages:
        - role: user
          content: "And in Rome?"
    expected:
      output:
        - kind: contains
          value: "sunny"
"#;

const MOCKS: &str = r#"[
  {"tool": "weather_lookup", "args_match": {"city": "Paris"}, "response": {"forecast": "sunny"}},
  {"tool": "agent_response", "args_match": {"case_id": "evidence-demo/tool-case"}, "response": "It is sunny in Paris."}
]"#;

const PROMPT: &str = "You report the weather.\n";

fn bin() -> Command {
    Command::cargo_bin("agentcarousel").unwrap()
}

/// Mock eval (filtered to one of two cases) then export; returns (workdir, extracted bundle dir).
fn eval_and_export(extra_export_args: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("fixtures/evidence-demo")).unwrap();
    fs::create_dir_all(root.join("mocks")).unwrap();
    fs::write(root.join("fixtures/evidence-demo/cases.yaml"), CASES).unwrap();
    fs::write(root.join("fixtures/evidence-demo/prompt.md"), PROMPT).unwrap();
    fs::write(root.join("mocks/evidence-demo.json"), MOCKS).unwrap();
    let history = root.join("history.db");

    let out = bin()
        .current_dir(root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args([
            "eval",
            "fixtures/evidence-demo/cases.yaml",
            "--execution-mode",
            "mock",
            "--filter",
            "evidence-demo/tool-case",
            "--json",
        ])
        .output()
        .unwrap();
    let envelope: Value = serde_json::from_slice(&out.stdout).expect("eval json");
    let run_id = envelope["data"]["id"].as_str().expect("run id").to_string();

    let tarball = root.join("evidence.tar.gz");
    bin()
        .current_dir(root)
        .env("AGENTCAROUSEL_HISTORY_DB", &history)
        .args(["export", &run_id, "--out", tarball.to_str().unwrap()])
        .args(extra_export_args)
        .assert()
        .success();
    let extract = root.join("extracted");
    fs::create_dir_all(&extract).unwrap();
    let status = std::process::Command::new("tar")
        .args([
            "-xzf",
            tarball.to_str().unwrap(),
            "-C",
            extract.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let bundle = fs::read_dir(&extract)
        .unwrap()
        .next()
        .expect("top-level dir")
        .unwrap()
        .path();
    (dir, bundle)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// Validate against the vendored spec schemas with local-only reference resolution.
fn schema_errors(schema_file: &str, doc: &Value) -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/assurance/0.1");
    let mut registry = jsonschema::Registry::new();
    let mut target = None;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let schema = read_json(&path);
        let id = schema["$id"].as_str().unwrap().to_string();
        if path.file_name().unwrap() == schema_file {
            target = Some(schema.clone());
        }
        registry = registry.add(id, schema).unwrap();
    }
    let registry = registry.prepare().unwrap();
    let validator = jsonschema::options()
        .with_registry(&registry)
        .offline()
        .build(&target.expect("schema file"))
        .unwrap();
    validator.iter_errors(doc).map(|e| e.to_string()).collect()
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path).unwrap());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn export_emits_conformant_evidence_and_statement() {
    let (_dir, bundle) = eval_and_export(&[]);
    let evidence = read_json(&bundle.join("evidence.json"));
    let statement = read_json(&bundle.join("attestation.intoto.json"));
    let manifest = read_json(&bundle.join("MANIFEST.json"));

    assert_eq!(
        schema_errors("evidence.schema.json", &evidence),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_errors("agent-evaluation-statement.schema.json", &statement),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_errors("evidence-manifest.schema.json", &manifest),
        Vec::<String>::new()
    );

    // INV-1 / INV-2: the filtered case is visible and the critical case is counted.
    let summary = &evidence["summary"];
    assert_eq!(summary["total"], 2);
    assert_eq!(summary["not_tested"], 1);
    assert_eq!(summary["passed"], 1);
    assert_eq!(
        evidence["subject"]["configuration"]["test_suite"]["case_count"],
        2
    );
    let codes: Vec<&str> = evidence["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["code"].as_str().unwrap())
        .collect();
    for code in ["mock_execution", "tool_calls_not_observed", "partial_suite"] {
        assert!(codes.contains(&code), "missing {code}: {codes:?}");
    }
    let tool_case = &evidence["cases"][0];
    assert_eq!(
        tool_case["trace_sources"]["tool_calls"],
        "fixture_expectation"
    );
    assert_eq!(
        tool_case["assertions"][0]["evaluator_class"],
        "deterministic"
    );
    assert_eq!(tool_case["critical"], true);

    // INV-7: prompt withheld by default, identified by digest.
    assert_eq!(evidence["disclosure"]["system_prompt"], "digest_only");
    assert!(!bundle.join("prompt.md").exists());

    // Statement: subjects are the tested artifacts; documents exclude MANIFEST.json.
    let subjects: Vec<&str> = statement["subject"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        subjects,
        vec![
            "fixtures/evidence-demo/cases.yaml",
            "fixtures/evidence-demo/prompt.md"
        ]
    );
    let documents = statement["predicate"]["evidence_documents"]
        .as_array()
        .unwrap();
    for doc in documents {
        let path = doc["path"].as_str().unwrap();
        assert_ne!(path, "MANIFEST.json");
        assert_eq!(
            doc["digest"].as_str().unwrap(),
            format!("sha256:{}", sha256(&bundle.join(path)))
        );
    }
    assert_eq!(statement["predicate"]["summary"], evidence["summary"]);

    // Both documents are covered by the manifest (and so by its signature).
    let listed: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"evidence.json") && listed.contains(&"attestation.intoto.json"));
}

#[test]
fn export_with_include_prompt_discloses_prompt_file() {
    let (_dir, bundle) = eval_and_export(&["--include-prompt"]);
    let evidence = read_json(&bundle.join("evidence.json"));
    assert_eq!(evidence["disclosure"]["system_prompt"], "included");
    assert_eq!(evidence["disclosure"]["test_inputs"], "included");
    assert_eq!(
        fs::read_to_string(bundle.join("prompt.md")).unwrap(),
        PROMPT
    );
    let manifest = read_json(&bundle.join("MANIFEST.json"));
    assert!(manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == "prompt.md"));
}
