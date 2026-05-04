use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

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
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EvaluatorConfig {
    pub evaluator: String,
    pub golden_path: Option<PathBuf>,
    pub golden_threshold: Option<f32>,
    pub process_cmd: Option<Vec<String>>,
    pub judge_prompt: Option<String>,
}

/// Result of executing one or more fixtures: case outcomes, [`RunSummary`], provenance fields.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub schema_version: u32,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub command: String,
    pub git_sha: Option<String>,
    pub agentcarousel_version: String,
    pub config_hash: String,
    pub cases: Vec<CaseResult>,
    pub summary: RunSummary,
    pub fixture_bundle_id: Option<String>,
    pub fixture_bundle_version: Option<String>,
    pub carousel_iteration: Option<u32>,
    pub certification_context: Option<CertificationContext>,
    pub policy_version: Option<String>,
}

/// Outcome for a single [`Case`]: status, optional error string, [`ExecutionTrace`], [`Metrics`],
/// and optional [`EvalScores`] after evaluation.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseResult {
    pub case_id: CaseId,
    pub status: CaseStatus,
    pub error: Option<String>,
    pub trace: ExecutionTrace,
    pub metrics: Metrics,
    pub eval_scores: Option<EvalScores>,
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
    pub final_output: Option<String>,
    pub redacted: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TraceStep {
    pub index: u32,
    pub kind: StepKind,
    pub tool: Option<String>,
    pub args: Option<Value>,
    pub result: Option<Value>,
    pub latency_ms: u64,
    pub tokens_in: Option<u32>,
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
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub total_steps: u32,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
    pub latency_variance_ms2: Option<f64>,
    pub latency_stddev_ms: Option<f64>,
    pub effectiveness_variance: Option<f32>,
    pub effectiveness_stddev: Option<f32>,
    pub runs_attempted: u32,
    pub runs_succeeded: u32,
    pub error_rate: Option<f32>,
    pub consistency_score: Option<f32>,
    pub provider_errors: ProviderErrorMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ProviderErrorMetrics {
    pub status_429: u32,
    pub status_500: u32,
    pub status_503: u32,
    pub status_504: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EvalScores {
    pub evaluator: String,
    pub rubric_scores: Vec<RubricScore>,
    pub effectiveness_score: f32,
    pub passed: bool,
    pub judge_rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RubricScore {
    pub rubric_id: String,
    pub score: f32,
    pub weight: f32,
    pub rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub flaky: u32,
    pub errored: u32,
    pub timed_out: u32,
    pub pass_rate: f32,
    pub mean_latency_ms: f64,
    pub mean_effectiveness_score: Option<f32>,
    pub provider_errors: ProviderErrorMetrics,
    pub overall_status: OverallStatus,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunDiff {
    pub run_a: RunId,
    pub run_b: RunId,
    pub regressions: Vec<CaseRegression>,
    pub improvements: Vec<Value>,
    pub has_regressions: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CaseRegression {
    pub case_id: CaseId,
    pub kind: RegressionKind,
    pub before: Value,
    pub after: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RegressionKind {
    StatusChange,
    LatencyIncrease,
    EffectivenessDropped,
    ErrorRateIncreased,
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CertificationContext {
    Local,
    Msp,
    Ci,
}
