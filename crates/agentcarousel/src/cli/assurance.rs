//! AgentCarousel Specification v0.1 documents: build `evidence.json` and its in-toto statement
//! from a single [`Run`], and check documents against the embedded schemas, the semantic rules
//! (SEM-001..SEM-008) and the cross-document rules (XDOC-001..XDOC-003).
//!
//! Every value in the evidence is derived from the run record alone (spec INV-5); nothing is
//! read from local history or the environment at export time.
use agentcarousel_core::{CaseResult, CaseStatus, Run, ScoreSource, StepKind, TraceSource};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub(crate) const SPEC_NAME: &str = "agentcarousel-spec/evidence";
pub(crate) const SPEC_VERSION: &str = "0.1.0";
pub(crate) const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";
pub(crate) const PREDICATE_TYPE: &str =
    "https://agentcarousel.com/spec/predicates/agent-evaluation/v0.1";
const SCHEMA_BASE: &str = "https://agentcarousel.com/spec/schemas/0.1/";
/// Provisional identifiers emitted only by agc 0.9.0, before the spec was named. The spec
/// retired them (VERSIONING.md): documents that use them are an unsupported draft.
const LEGACY_SPEC_NAME: &str = "assurance-spec/evidence";
const LEGACY_BASE: &str = "https://assurance-spec.invalid/";
const IMPLEMENTATION_NAME: &str = "agentcarousel";
const IMPLEMENTATION_URI: &str = "https://github.com/agentcarousel/agentcarousel";
/// Naming authority for fallback agent ids when a fixture declares no `agent` block.
const FALLBACK_AGENT_ID_PREFIX: &str = "tag:agentcarousel.com,2026:skill/";
/// A rubric score at or above this is an assertion `pass` (the ✅ threshold in report.md).
pub(crate) const RUBRIC_PASS_SCORE: f32 = 0.9;

pub(crate) const EVIDENCE_FILE: &str = "evidence.json";
pub(crate) const STATEMENT_FILE: &str = "attestation.intoto.json";

macro_rules! schema {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/schemas/assurance/0.1/",
                $name
            )),
        )
    };
}

/// Embedded copies of the spec schemas (see `schemas/assurance/0.1/SHA256SUMS`).
pub(crate) const SCHEMAS: [(&str, &str); 5] = [
    schema!("agent.schema.json"),
    schema!("evidence.schema.json"),
    schema!("evidence-manifest.schema.json"),
    schema!("agent-evaluation-predicate.schema.json"),
    schema!("agent-evaluation-statement.schema.json"),
];

/// A failed conformance rule, reported with its spec rule id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Violation {
    pub rule: &'static str,
    pub message: String,
}

impl Violation {
    fn new(rule: &'static str, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.rule, self.message)
    }
}

/// What sensitive material the bundle carries (spec INV-7).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Disclosure {
    /// `digest_only`, `included`, or `not_applicable`.
    pub system_prompt: &'static str,
    /// `included`, `redacted`, or `excluded`.
    pub test_inputs: &'static str,
}

// ── Building ────────────────────────────────────────────────────────────────

/// Build `evidence.json` for one run. Fails for runs recorded before run subjects existed,
/// because their configuration identity cannot be reconstructed.
pub(crate) fn build_evidence(run: &Run, disclosure: Disclosure) -> Result<Value, String> {
    let subject = run.subject.as_ref().ok_or_else(|| {
        "run predates assurance evidence (no recorded subject); re-run eval to produce evidence.json"
            .to_string()
    })?;
    let mode = subject
        .generation_mode
        .as_deref()
        .unwrap_or(if run.runner_mock_only { "mock" } else { "live" });

    let cases: Vec<Value> = run.cases.iter().map(case_json).collect();
    let summary = summary_json(&cases);

    let mut configuration = Map::new();
    if mode != "mock" {
        configuration.insert("model".into(), model_json(run));
    }
    if let Some(prompt) = subject.system_prompt.as_ref() {
        configuration.insert(
            "system_prompt".into(),
            json!({ "path": prompt.path, "digest": prompt.sha256 }),
        );
    }
    let suite_id = run
        .fixture_bundle_id
        .clone()
        .or_else(|| run.skill_or_agent.clone())
        .unwrap_or_else(|| "unnamed-suite".to_string());
    let mut test_suite = json!({
        "id": suite_id,
        "case_count": cases.len(),
        "files": subject
            .fixture_files
            .iter()
            .map(|f| json!({ "path": f.path, "digest": f.sha256 }))
            .collect::<Vec<_>>(),
    });
    if let Some(version) = run.fixture_bundle_version.as_ref() {
        test_suite["version"] = json!(version);
    }
    configuration.insert("test_suite".into(), test_suite);
    if !subject.golden_files.is_empty() {
        configuration.insert(
            "golden_files".into(),
            json!(subject
                .golden_files
                .iter()
                .map(|f| json!({ "path": f.path, "digest": f.sha256 }))
                .collect::<Vec<_>>()),
        );
    }
    if let Some(judge) = subject.judge.as_ref() {
        configuration.insert(
            "judge".into(),
            json!({ "model": judge.model, "temperature": judge.temperature }),
        );
    }

    let mut execution = json!({
        "run_id": run.id.0,
        "mode": mode,
        "implementation": implementation_json(run),
        "started_at": rfc3339(&run.started_at),
    });
    if let Some(finished) = run.finished_at.as_ref() {
        execution["finished_at"] = json!(rfc3339(finished));
    }
    let repetitions = run
        .cases
        .iter()
        .map(|c| c.metrics.runs_attempted)
        .max()
        .unwrap_or(0);
    if repetitions > 1 {
        execution["repetitions"] = json!(repetitions);
    }
    if let Some(sha) = run.git_sha.as_ref() {
        execution["environment"] = json!({ "source_revision": sha });
    }

    let mut limitations = derive_limitations(mode, &cases);
    if subject.suite_case_count != 0 && subject.suite_case_count as usize != cases.len() {
        limitations.push(json!({
            "code": "x-suite_count_mismatch",
            "detail": format!(
                "The suite defined {} cases when the run started, but the run records {}.",
                subject.suite_case_count,
                cases.len()
            ),
        }));
    }

    let mut evidence = json!({
        "spec": { "name": SPEC_NAME, "version": SPEC_VERSION },
        "subject": {
            "agent": agent_json(run),
            "configuration": Value::Object(configuration),
        },
        "execution": execution,
        "disclosure": {
            "system_prompt": disclosure.system_prompt,
            "test_inputs": disclosure.test_inputs,
        },
        "cases": cases,
        "summary": summary,
        "limitations": limitations,
    });
    // Severity is not part of spec 0.1 cases; carry the tally as a top-level
    // extension, only when the suite assigns severities.
    if let Some(tally) = super::severity::failed_tally(&run.cases) {
        evidence[SEVERITY_EXTENSION] = json!({
            "scale": "agentcarousel",
            "failed": {
                "P0": tally.p0,
                "P1": tally.p1,
                "P2": tally.p2,
                "P3": tally.p3,
                "unrated": tally.unrated,
            },
            "open_p0_p1": tally.open_p0_p1(),
        });
    }
    Ok(evidence)
}

/// Top-level `evidence.json` extension key carrying failed-case counts by severity.
pub(crate) const SEVERITY_EXTENSION: &str = "x-agentcarousel-severity";

/// Build the in-toto statement. `documents` are `(path, "sha256:<hex>")` pairs for the
/// evidence documents it covers, which MUST include `evidence.json` and MUST NOT include
/// `MANIFEST.json` (spec §4.2).
pub(crate) fn build_statement(evidence: &Value, documents: &[(String, String)]) -> Value {
    let configuration = &evidence["subject"]["configuration"];
    let subjects: Vec<Value> = expected_subjects(configuration)
        .into_iter()
        .map(|(name, hex)| json!({ "name": name, "digest": { "sha256": hex } }))
        .collect();
    let evaluated_at = evidence["execution"]
        .get("finished_at")
        .unwrap_or(&evidence["execution"]["started_at"])
        .clone();
    json!({
        "_type": STATEMENT_TYPE,
        "subject": subjects,
        "predicateType": PREDICATE_TYPE,
        "predicate": {
            "run_id": evidence["execution"]["run_id"],
            "evaluated_at": evaluated_at,
            "producer": evidence["execution"]["implementation"],
            "agent": evidence["subject"]["agent"],
            "configuration": configuration,
            "summary": evidence["summary"],
            "limitations": evidence["limitations"],
            "evidence_documents": documents
                .iter()
                .map(|(path, digest)| json!({ "path": path, "digest": digest }))
                .collect::<Vec<_>>(),
        },
    })
}

fn implementation_json(run: &Run) -> Value {
    json!({
        "name": IMPLEMENTATION_NAME,
        "version": run.agentcarousel_version,
        "uri": IMPLEMENTATION_URI,
    })
}

fn rfc3339(ts: &chrono::DateTime<chrono::Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Declared agent, or a documented fallback derived from the suite name.
fn agent_json(run: &Run) -> Value {
    if let Some(agent) = run.subject.as_ref().and_then(|s| s.agent.as_ref()) {
        return serde_json::to_value(agent).unwrap_or(Value::Null);
    }
    let slug: String = run
        .skill_or_agent
        .as_deref()
        .unwrap_or("unnamed-agent")
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    let mut agent = json!({ "id": format!("{FALLBACK_AGENT_ID_PREFIX}{slug}") });
    if let Some(version) = run.fixture_bundle_version.as_ref() {
        agent["version"] = json!(version);
    }
    agent
}

/// Model under test, taken from the run's own observed generation steps when present.
fn model_json(run: &Run) -> Value {
    let observed = run
        .cases
        .iter()
        .flat_map(|c| c.trace.steps.iter())
        .filter(|step| matches!(step.kind, StepKind::LlmCall))
        .find_map(|step| step.result.as_ref());
    let provider = observed
        .and_then(|r| r.get("provider"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let name = observed
        .and_then(|r| r.get("model"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| run.summary.generator_model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    json!({ "provider": provider, "name": name })
}

fn status_str(status: &CaseStatus) -> &'static str {
    match status {
        CaseStatus::Passed => "passed",
        CaseStatus::Failed => "failed",
        CaseStatus::Error => "error",
        CaseStatus::TimedOut => "timed_out",
        CaseStatus::Flaky => "flaky",
        CaseStatus::Skipped => "not_tested",
    }
}

fn case_json(case: &CaseResult) -> Value {
    let status = status_str(&case.status);
    let mut value = json!({
        "id": case.case_id.0,
        "status": status,
        "critical": case.critical,
    });
    if !case.tags.is_empty() {
        value["tags"] = json!(case.tags);
    }
    if case.status == CaseStatus::Skipped {
        value["not_tested_reason"] = json!(case.skip_reason.as_deref().unwrap_or("not_executed"));
        value["trace_sources"] = json!({ "model_output": "none", "tool_calls": "none" });
        value["assertions"] = json!([]);
        return value;
    }
    value["trace_sources"] = json!({
        "model_output": model_output_source(case),
        "tool_calls": tool_call_source(case),
    });
    let assertions: Vec<Value> = case
        .eval_scores
        .as_ref()
        .map(|scores| {
            scores
                .rubric_scores
                .iter()
                .map(|rs| assertion_json(rs, &scores.evaluator, scores.passed))
                .collect()
        })
        .unwrap_or_default();
    value["assertions"] = json!(assertions);
    if let Some(scores) = case.eval_scores.as_ref() {
        value["effectiveness"] = json!(scores.effectiveness_score.clamp(0.0, 1.0));
    }
    value
}

fn model_output_source(case: &CaseResult) -> &'static str {
    let llm_steps: Vec<_> = case
        .trace
        .steps
        .iter()
        .filter(|s| matches!(s.kind, StepKind::LlmCall))
        .collect();
    if llm_steps
        .iter()
        .any(|s| matches!(s.source, Some(TraceSource::Observed) | None))
    {
        "observed"
    } else if case.trace.final_output.is_some() {
        "mock"
    } else {
        "none"
    }
}

fn tool_call_source(case: &CaseResult) -> &'static str {
    let calls: Vec<_> = case
        .trace
        .steps
        .iter()
        .filter(|s| matches!(s.kind, StepKind::ToolCall))
        .collect();
    if calls.is_empty() {
        "none"
    } else if calls
        .iter()
        .all(|s| s.source == Some(TraceSource::Observed))
    {
        "observed"
    } else {
        // Unmarked (legacy) tool steps were also synthesized: the runner has never observed calls.
        "fixture_expectation"
    }
}

fn evaluator_class(source: Option<ScoreSource>, evaluator: &str) -> &'static str {
    match source {
        Some(ScoreSource::Deterministic) => "deterministic",
        Some(ScoreSource::Golden) => "golden",
        Some(ScoreSource::Process) => "process",
        Some(ScoreSource::LlmJudge) => "llm_judge",
        Some(ScoreSource::NotEvaluated) | None => match evaluator {
            "judge" => "llm_judge",
            "golden" => "golden",
            "process" => "process",
            _ => "deterministic",
        },
    }
}

fn assertion_json(rs: &agentcarousel_core::RubricScore, evaluator: &str, passed: bool) -> Value {
    let class = evaluator_class(rs.source, evaluator);
    let kind = match (rs.rubric_id.as_str(), class) {
        ("golden", "golden") => "golden_diff",
        (_, "process") => "process",
        ("rules", "deterministic") => "x-rule-set",
        _ => "rubric",
    };
    let outcome = if rs.source == Some(ScoreSource::NotEvaluated) {
        "not_evaluated"
    } else if kind == "golden_diff" {
        if passed {
            "pass"
        } else {
            "fail"
        }
    } else if rs.score >= RUBRIC_PASS_SCORE {
        "pass"
    } else {
        "fail"
    };
    let mut value = json!({
        "id": rs.rubric_id,
        "kind": kind,
        "evaluator_class": class,
        "outcome": outcome,
        "score": rs.score.clamp(0.0, 1.0),
        "weight": rs.weight.clamp(0.0, 1.0),
    });
    if rs.critical {
        value["critical"] = json!(true);
    }
    value
}

fn summary_json(cases: &[Value]) -> Value {
    let count = |status: &str| cases.iter().filter(|c| c["status"] == status).count();
    let critical_failures = cases
        .iter()
        .filter(|c| c["critical"] == true && is_failure_status(c["status"].as_str()))
        .count();
    json!({
        "total": cases.len(),
        "passed": count("passed"),
        "failed": count("failed"),
        "error": count("error"),
        "timed_out": count("timed_out"),
        "flaky": count("flaky"),
        "not_tested": count("not_tested"),
        "critical_failures": critical_failures,
    })
}

fn is_failure_status(status: Option<&str>) -> bool {
    matches!(status, Some("failed" | "error" | "timed_out" | "flaky"))
}

fn case_ids_where(cases: &[Value], pred: impl Fn(&Value) -> bool) -> Vec<String> {
    cases
        .iter()
        .filter(|c| pred(c))
        .filter_map(|c| c["id"].as_str().map(str::to_string))
        .collect()
}

/// Limitations required by SEM-003..SEM-006, derived from the case records.
fn derive_limitations(mode: &str, cases: &[Value]) -> Vec<Value> {
    let mut limitations = Vec::new();
    if mode == "mock" {
        limitations.push(json!({
            "code": "mock_execution",
            "detail": "No model was called; agent outputs came from fixture mocks.",
        }));
    }
    let synthesized = case_ids_where(cases, |c| {
        c["trace_sources"]["tool_calls"] == "fixture_expectation"
    });
    if !synthesized.is_empty() {
        limitations.push(json!({
            "code": "tool_calls_not_observed",
            "detail": "Tool-call steps were synthesized from each case's expected tool sequence, not emitted by the agent; they show nothing about the agent's tool use.",
            "cases": synthesized,
        }));
    }
    let judge_only = case_ids_where(cases, is_judge_only_critical);
    if !judge_only.is_empty() {
        limitations.push(json!({
            "code": "judge_only_assertion",
            "detail": "Critical cases decided solely by an LLM judge, with no deterministic check.",
            "cases": judge_only,
        }));
    }
    let not_tested = case_ids_where(cases, |c| c["status"] == "not_tested");
    if !not_tested.is_empty() {
        limitations.push(json!({
            "code": "partial_suite",
            "detail": format!("{} suite case(s) were not executed.", not_tested.len()),
            "cases": not_tested,
        }));
    }
    limitations
}

fn is_judge_only_critical(case: &Value) -> bool {
    let assertions = case["assertions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    case["critical"] == true
        && !assertions.is_empty()
        && assertions
            .iter()
            .all(|a| a["evaluator_class"] == "llm_judge")
}

/// `(subject name, hex digest)` pairs the statement must carry (XDOC-003).
fn expected_subjects(configuration: &Value) -> Vec<(String, String)> {
    let mut subjects = Vec::new();
    for file in configuration["test_suite"]["files"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if let (Some(path), Some(digest)) = (file["path"].as_str(), file["digest"].as_str()) {
            subjects.push((path.to_string(), hex_of(digest).to_string()));
        }
    }
    if let Some(prompt) = configuration.get("system_prompt") {
        if let Some(digest) = prompt["digest"].as_str() {
            let name = prompt["path"].as_str().unwrap_or("system-prompt");
            subjects.push((name.to_string(), hex_of(digest).to_string()));
        }
    }
    subjects
}

fn hex_of(digest: &str) -> &str {
    digest.strip_prefix("sha256:").unwrap_or(digest)
}

// ── Checking ────────────────────────────────────────────────────────────────

/// Which spec schema to validate a document against.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DocKind {
    Evidence,
    Statement,
    Manifest,
}

impl DocKind {
    fn schema_file(self) -> &'static str {
        match self {
            DocKind::Evidence => "evidence.schema.json",
            DocKind::Statement => "agent-evaluation-statement.schema.json",
            DocKind::Manifest => "evidence-manifest.schema.json",
        }
    }
}

/// SCH-001: validate against the embedded schemas. References resolve only against the
/// embedded set; nothing is fetched (spec INV-6).
pub(crate) fn schema_violations(kind: DocKind, doc: &Value) -> Vec<Violation> {
    let mut registry = jsonschema::Registry::new();
    let mut target = None;
    for (name, text) in SCHEMAS {
        let schema: Value = match serde_json::from_str(text) {
            Ok(schema) => schema,
            Err(err) => return vec![Violation::new("SCH-001", format!("embedded {name}: {err}"))],
        };
        if name == kind.schema_file() {
            target = Some(schema.clone());
        }
        registry = match registry.add(format!("{SCHEMA_BASE}{name}"), schema) {
            Ok(registry) => registry,
            Err(err) => return vec![Violation::new("SCH-001", format!("registry {name}: {err}"))],
        };
    }
    let Some(target) = target else {
        return vec![Violation::new("SCH-001", "schema not embedded")];
    };
    let registry = match registry.prepare() {
        Ok(registry) => registry,
        Err(err) => return vec![Violation::new("SCH-001", format!("registry: {err}"))],
    };
    let validator = match jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .with_registry(&registry)
        .offline()
        .build(&target)
    {
        Ok(validator) => validator,
        Err(err) => return vec![Violation::new("SCH-001", format!("schema build: {err}"))],
    };
    validator
        .iter_errors(doc)
        .map(|err| {
            Violation::new(
                "SCH-001",
                format!(
                    "{} ({}) at {}",
                    kind.schema_file(),
                    err,
                    err.instance_path()
                ),
            )
        })
        .collect()
}

fn limitation_codes(evidence: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut codes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for limitation in evidence["limitations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
    {
        if let Some(code) = limitation["code"].as_str() {
            let entry = codes.entry(code.to_string()).or_default();
            for case in limitation["cases"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                if let Some(id) = case.as_str() {
                    entry.insert(id.to_string());
                }
            }
        }
    }
    codes
}

/// SEM-001..SEM-008 (spec §3.6). Assumes the document already passed the schema.
pub(crate) fn semantic_violations(evidence: &Value) -> Vec<Violation> {
    let mut out = Vec::new();
    let cases = evidence["cases"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let summary = &evidence["summary"];
    let suite_count = evidence["subject"]["configuration"]["test_suite"]["case_count"].as_u64();

    let total = summary["total"].as_u64();
    if total != Some(cases.len() as u64) || suite_count != Some(cases.len() as u64) {
        out.push(Violation::new(
            "SEM-001",
            format!(
                "summary.total {:?}, cases {}, test_suite.case_count {:?} must be equal",
                total,
                cases.len(),
                suite_count
            ),
        ));
    }
    for (field, status) in [
        ("passed", "passed"),
        ("failed", "failed"),
        ("error", "error"),
        ("timed_out", "timed_out"),
        ("flaky", "flaky"),
        ("not_tested", "not_tested"),
    ] {
        let actual = cases.iter().filter(|c| c["status"] == status).count() as u64;
        if summary[field].as_u64() != Some(actual) {
            out.push(Violation::new(
                "SEM-001",
                format!(
                    "summary.{field} is {} but {actual} case(s) have status {status}",
                    summary[field]
                ),
            ));
        }
    }

    let critical_failures = cases
        .iter()
        .filter(|c| c["critical"] == true && is_failure_status(c["status"].as_str()))
        .count() as u64;
    if summary["critical_failures"].as_u64() != Some(critical_failures) {
        out.push(Violation::new(
            "SEM-002",
            format!(
                "summary.critical_failures is {} but {critical_failures} critical case(s) failed",
                summary["critical_failures"]
            ),
        ));
    }

    let codes = limitation_codes(evidence);
    if cases
        .iter()
        .any(|c| c["trace_sources"]["tool_calls"] == "fixture_expectation")
        && !codes.contains_key("tool_calls_not_observed")
    {
        out.push(Violation::new(
            "SEM-003",
            "synthesized tool-call traces without a tool_calls_not_observed limitation",
        ));
    }
    let judge_only: Vec<String> = case_ids_where(cases, is_judge_only_critical);
    let listed = codes.get("judge_only_assertion");
    for id in &judge_only {
        if !listed.is_some_and(|ids| ids.contains(id)) {
            out.push(Violation::new(
                "SEM-004",
                format!(
                    "critical case {id} is judge-only but not listed under judge_only_assertion"
                ),
            ));
        }
    }
    if evidence["execution"]["mode"] == "mock" && !codes.contains_key("mock_execution") {
        out.push(Violation::new(
            "SEM-005",
            "mock execution without a mock_execution limitation",
        ));
    }
    if summary["not_tested"].as_u64().unwrap_or(0) > 0 && !codes.contains_key("partial_suite") {
        out.push(Violation::new(
            "SEM-006",
            "not_tested cases without a partial_suite limitation",
        ));
    }
    let uses_judge = cases.iter().any(|c| {
        c["assertions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|a| a["evaluator_class"] == "llm_judge"))
    });
    if uses_judge && evidence["subject"]["configuration"].get("judge").is_none() {
        out.push(Violation::new(
            "SEM-007",
            "llm_judge assertions without configuration.judge",
        ));
    }
    let mut seen = HashSet::new();
    for case in cases {
        if let Some(id) = case["id"].as_str() {
            if !seen.insert(id) {
                out.push(Violation::new("SEM-008", format!("duplicate case id {id}")));
            }
        }
    }
    out
}

/// XDOC-001..XDOC-003 (spec §5 step 9). `file_digests` maps bundle-relative paths to the
/// `sha256:<hex>` digest of each file's actual bytes.
pub(crate) fn cross_document_violations(
    evidence: &Value,
    statement: &Value,
    file_digests: &BTreeMap<String, String>,
) -> Vec<Violation> {
    let mut out = Vec::new();
    let predicate = &statement["predicate"];

    let documents = predicate["evidence_documents"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !documents.iter().any(|d| d["path"] == EVIDENCE_FILE) {
        out.push(Violation::new(
            "XDOC-001",
            "predicate.evidence_documents does not list evidence.json",
        ));
    }
    for document in documents {
        let path = document["path"].as_str().unwrap_or_default();
        if path == "MANIFEST.json" {
            out.push(Violation::new(
                "XDOC-001",
                "predicate must not reference MANIFEST.json (circular digest)",
            ));
            continue;
        }
        match file_digests.get(path) {
            Some(actual) if document["digest"].as_str() == Some(actual.as_str()) => {}
            Some(actual) => out.push(Violation::new(
                "XDOC-001",
                format!(
                    "{path}: statement digest {} != actual {actual}",
                    document["digest"]
                ),
            )),
            None => out.push(Violation::new(
                "XDOC-001",
                format!("{path} is referenced by the statement but not in the bundle"),
            )),
        }
    }

    for field in ["agent", "configuration", "summary", "limitations"] {
        let in_evidence = match field {
            "agent" | "configuration" => &evidence["subject"][field],
            _ => &evidence[field],
        };
        if &predicate[field] != in_evidence {
            out.push(Violation::new(
                "XDOC-002",
                format!("predicate.{field} differs from evidence.json"),
            ));
        }
    }
    if predicate["run_id"] != evidence["execution"]["run_id"] {
        out.push(Violation::new(
            "XDOC-002",
            "predicate.run_id differs from execution.run_id",
        ));
    }

    let expected: BTreeSet<(String, String)> =
        expected_subjects(&evidence["subject"]["configuration"])
            .into_iter()
            .collect();
    let actual: BTreeSet<(String, String)> = statement["subject"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|s| {
            Some((
                s["name"].as_str()?.to_string(),
                s["digest"]["sha256"].as_str()?.to_string(),
            ))
        })
        .collect();
    if expected != actual {
        out.push(Violation::new(
            "XDOC-003",
            "statement subjects must be exactly the suite files plus the system prompt",
        ));
    }
    out
}

/// Every check a producer or verifier applies to the pair of documents.
pub(crate) fn conformance_violations(
    evidence: &Value,
    statement: &Value,
    file_digests: &BTreeMap<String, String>,
) -> Vec<Violation> {
    if uses_legacy_identifiers(evidence, statement) {
        return vec![Violation::new(
            "SCH-001",
            format!(
                "bundle uses the provisional identifiers of agc 0.9.0 ({LEGACY_BASE}), an \
                 unsupported draft; re-export the run with agc 0.9.1 or later \
                 (`agc export <run-id>`)"
            ),
        )];
    }
    let mut out = schema_violations(DocKind::Evidence, evidence);
    out.extend(schema_violations(DocKind::Statement, statement));
    if out.is_empty() {
        out.extend(semantic_violations(evidence));
        out.extend(cross_document_violations(evidence, statement, file_digests));
    }
    out
}

fn uses_legacy_identifiers(evidence: &Value, statement: &Value) -> bool {
    evidence.pointer("/spec/name").and_then(Value::as_str) == Some(LEGACY_SPEC_NAME)
        || statement
            .get("predicateType")
            .and_then(Value::as_str)
            .is_some_and(|t| t.starts_with(LEGACY_BASE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn embedded_schemas_match_committed_checksums() {
        let sums = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/schemas/assurance/0.1/SHA256SUMS"
        ));
        for (name, text) in SCHEMAS {
            let mut hasher = Sha256::new();
            hasher.update(text.as_bytes());
            let hex: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            assert!(
                sums.lines().any(|l| l == format!("{hex}  {name}")),
                "{name} does not match SHA256SUMS; run scripts/sync-assurance-schemas.sh"
            );
        }
    }

    #[test]
    fn every_embedded_schema_compiles_offline() {
        let doc = json!({});
        for kind in [DocKind::Evidence, DocKind::Statement, DocKind::Manifest] {
            let violations = schema_violations(kind, &doc);
            assert!(
                violations.iter().all(|v| !v.message.contains("schema build")
                    && !v.message.contains("registry")),
                "{kind:?}: {violations:?}"
            );
            assert!(
                !violations.is_empty(),
                "{kind:?} should reject an empty object"
            );
        }
    }

    #[test]
    fn semantic_rules_catch_folded_not_tested() {
        let evidence = json!({
            "subject": { "configuration": { "test_suite": { "case_count": 2 } } },
            "execution": { "mode": "live" },
            "cases": [
                { "id": "a", "status": "passed", "critical": false,
                  "trace_sources": { "model_output": "observed", "tool_calls": "none" },
                  "assertions": [] },
                { "id": "b", "status": "not_tested", "critical": true,
                  "trace_sources": { "model_output": "none", "tool_calls": "none" },
                  "assertions": [] }
            ],
            "summary": { "total": 2, "passed": 2, "failed": 0, "error": 0, "timed_out": 0,
                         "flaky": 0, "not_tested": 0, "critical_failures": 0 },
            "limitations": []
        });
        let rules: Vec<&str> = semantic_violations(&evidence)
            .iter()
            .map(|v| v.rule)
            .collect();
        assert!(rules.contains(&"SEM-001"), "{rules:?}");
    }
}
