use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use ulid::Ulid;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid fixture: {0}")]
    InvalidFixture(String),
}

pub fn new_run_id() -> RunId {
    // Skip the 10-char timestamp prefix; take 10 chars from the random portion of the ULID.
    RunId(Ulid::new().to_string()[10..20].to_string())
}

/// Opaque identifier for a single **run** (persisted history, exports, registry).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(pub String);

/// Identifier for one **case** inside a fixture file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct CaseId(pub String);

/// One fixture file: metadata, optional defaults, and a list of [`Case`] definitions.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FixtureFile {
    pub schema_version: u32,
    pub skill_or_agent: String,
    pub defaults: Option<CaseDefaults>,
    pub cases: Vec<Case>,
    pub bundle_id: Option<String>,
    pub bundle_version: Option<String>,
    pub certification_track: Option<CertificationTrack>,
    pub risk_tier: Option<RiskTier>,
    pub data_handling: Option<DataHandling>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseDefaults {
    pub timeout_secs: Option<u64>,
    pub tags: Option<Vec<String>>,
    pub evaluator: Option<String>,
}

/// Executable example: input messages, expected tool/output assertions, tags, and timeouts.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Case {
    pub id: CaseId,
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub input: CaseInput,
    pub expected: Expected,
    pub evaluator_config: Option<EvaluatorConfig>,
    pub timeout_secs: Option<u64>,
    pub seed: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseInput {
    pub messages: Vec<Message>,
    pub context: Option<Value>,
    pub env_overrides: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    // "model" is Gemini's name for the assistant turn — accept it as an alias.
    #[serde(alias = "model")]
    Assistant,
    System,
    Tool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Expected {
    #[serde(default)]
    pub tool_sequence: Option<Vec<ToolCallExpectation>>,
    pub output: Option<Vec<OutputAssertion>>,
    pub rubric: Option<Vec<RubricItem>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolCallExpectation {
    // Accept "name" and "tool_name" as aliases — LLMs frequently emit those variants.
    #[serde(alias = "name", alias = "tool_name")]
    pub tool: String,
    pub args_match: Option<Value>,
    #[serde(default = "default_tool_order")]
    pub order: ToolOrder,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolOrder {
    Strict,
    Subsequence,
    Unordered,
}

fn default_tool_order() -> ToolOrder {
    ToolOrder::Subsequence
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OutputAssertion {
    pub kind: AssertionKind,
    pub value: String,
    pub field: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    Contains,
    NotContains,
    Equals,
    Regex,
    JsonPath,
    GoldenDiff,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RubricItem {
    pub id: String,
    pub description: String,
    pub weight: f32,
    pub auto_check: Option<OutputAssertion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EvaluatorConfig {
    pub evaluator: String,
    pub golden_path: Option<PathBuf>,
    pub golden_threshold: Option<f32>,
    pub process_cmd: Option<Vec<String>>,
    pub judge_prompt: Option<String>,
    pub effectiveness_threshold: Option<f32>,
}

/// Result of executing one or more fixtures: case outcomes, [`RunSummary`], provenance fields.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub schema_version: u32,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_sha: Option<String>,
    pub agentcarousel_version: String,
    pub config_hash: String,
    pub cases: Vec<CaseResult>,
    pub summary: RunSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixture_bundle_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_or_agent: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runner_offline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runner_mock_strict: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runner_mock_only: bool,
    /// Second-pass run-level judge analysis: classifies whether failures are due to prompt
    /// design, model capability, or fixture miscalibration, and emits actionable fixes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_audit: Option<PromptAudit>,
}

/// Root-cause classification produced by the prompt-audit judge pass.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PromptAuditFailureMode {
    /// Prompt is underspecified; fixing the prompt is likely to fix the failures.
    Prompt,
    /// Model cannot follow these instructions at this capability level regardless of prompt wording.
    Model,
    /// Rubric thresholds or expectations are miscalibrated; the model output is actually reasonable.
    Fixture,
    /// Multiple factors are contributing.
    Mixed,
}

/// One systematic failure pattern found across the run.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuditFinding {
    /// Human-readable description of the systematic failure pattern (e.g. "7/7 cases missing [T####] citations").
    pub pattern: String,
    /// Number of cases exhibiting this pattern.
    pub affected_case_count: u32,
    /// What in the prompt (or model / fixture) caused this pattern.
    pub root_cause: String,
}

/// Result of the run-level prompt-audit judge pass.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptAudit {
    pub failure_mode: PromptAuditFailureMode,
    /// Judge's confidence in the failure_mode classification (0.0–1.0).
    pub confidence: f32,
    pub findings: Vec<AuditFinding>,
    /// Concrete, actionable fixes the prompt author should apply.
    pub suggested_fixes: Vec<String>,
    /// Fully worked prompt text for each fix — parallel to suggested_fixes.
    /// Each element is the actual markdown content to paste into prompt.md.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_implementations: Vec<String>,
    pub overall_rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_out: Option<u64>,
}

/// Outcome for a single [`Case`]: status, optional error string, [`ExecutionTrace`], [`Metrics`],
/// and optional [`EvalScores`] after evaluation.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseResult {
    pub case_id: CaseId,
    pub status: CaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub trace: ExecutionTrace,
    pub metrics: Metrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_scores: Option<EvalScores>,
    /// Input messages from the fixture case; stored for human review in reports and dashboard.
    #[serde(default)]
    pub input: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discrimination_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discrimination_label: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Passed,
    Failed,
    Skipped,
    Flaky,
    TimedOut,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExecutionTrace {
    pub steps: Vec<TraceStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_output: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraceStep {
    pub index: u32,
    pub kind: StepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    LlmCall,
    ToolCall,
    ToolResult,
    AgentDecision,
    Error,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Metrics {
    pub total_latency_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub llm_calls: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tool_calls: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub total_steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_variance_ms2: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_stddev_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effectiveness_variance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effectiveness_stddev: Option<f32>,
    #[serde(default)]
    pub runs_attempted: u32,
    #[serde(default)]
    pub runs_succeeded: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consistency_score: Option<f32>,
    #[serde(default, skip_serializing_if = "ProviderErrorMetrics::is_empty")]
    pub provider_errors: ProviderErrorMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ProviderErrorMetrics {
    pub status_429: u32,
    pub status_500: u32,
    pub status_503: u32,
    pub status_504: u32,
}

impl ProviderErrorMetrics {
    pub fn is_empty(&self) -> bool {
        self.status_429 == 0 && self.status_500 == 0 && self.status_503 == 0 && self.status_504 == 0
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EvalScores {
    pub evaluator: String,
    pub rubric_scores: Vec<RubricScore>,
    pub effectiveness_score: f32,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_out: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RubricScore {
    pub rubric_id: String,
    pub score: f32,
    pub weight: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub skipped: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub flaky: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub errored: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub timed_out: u32,
    pub pass_rate: f32,
    pub mean_latency_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_effectiveness_score: Option<f32>,
    #[serde(default, skip_serializing_if = "ProviderErrorMetrics::is_empty")]
    pub provider_errors: ProviderErrorMetrics,
    pub overall_status: OverallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_tokens_per_judged_case: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_p99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_tokens_out: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gen_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_line: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OverallStatus {
    Pass,
    Fail,
    ValidationError,
    ConfigError,
    RuntimeError,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CertificationTrack {
    None,
    Candidate,
    Stable,
    Trusted,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RiskTier {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DataHandling {
    SyntheticOnly,
    NoPii,
    PiiReviewed,
}
