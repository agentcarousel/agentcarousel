use crate::common::{BackMatter, Link, Metadata, Property};
use serde::{Deserialize, Serialize};

/// Top-level OSCAL Assessment Results document — the primary auditor artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AssessmentResultsDocument {
    pub assessment_results: AssessmentResults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AssessmentResults {
    pub uuid: String,
    pub metadata: Metadata,
    /// Reference to the assessment plan that drove this assessment.
    pub import_ap: ImportAp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<AssessmentResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_matter: Option<BackMatter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImportAp {
    pub href: String,
}

/// One complete assessment result — findings + observations from a single evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AssessmentResult {
    pub uuid: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<Observation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// Open risks for controls with no fixture coverage (Gap status).
    /// Auditors expect explicit documentation of missing coverage rather
    /// than silent absence from the report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<AssessmentRisk>,
    /// Sampling methodology and population rationale per control family.
    /// Required by SOC 2 and ISO 27001 auditors to validate evidence sufficiency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment_log: Option<AssessmentLog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remarks: Option<String>,
}

// ── Sampling / Assessment Log ─────────────────────────────────────────────────

/// Documents the sampling methodology and population rationale for this assessment.
///
/// SOC 2 and ISO 27001 auditors require: total population size, how the sample
/// was selected, and why it is representative.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AssessmentLog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<LogEntry>,
}

/// A single log entry documenting sampling rationale for one control family or run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LogEntry {
    pub uuid: String,
    pub title: String,
    pub start: String,
    /// Plain-English description of what was tested, sample size, and methodology.
    /// Example: "12 adversarial injection fixture cases exhaustively executed against
    /// claude-opus-4-8. Population: all fixture cases in prompt-injection-detector
    /// bundle v1.0.0 (12 total). Sample = population (exhaustive)."
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
}

// ── Risk Records for Gap Controls ─────────────────────────────────────────────

/// An open risk entry for a control with no fixture coverage (Gap status).
///
/// Gap controls must appear as explicit risk records rather than being silently
/// absent from the assessment report. This gives auditors visibility into scope
/// boundaries and enables formal risk acceptance with owner and expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AssessmentRisk {
    pub uuid: String,
    pub title: String,
    pub description: String,
    /// `"open"` | `"investigating"` | `"remediating"` | `"deviation-requested"` | `"accepted"` | `"closed"`
    pub risk_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    /// The control ID this risk is associated with (e.g. `"eu-ai-act-art-15"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_observation: Option<RiskRelatedObservation>,
    /// Response actions — compensating controls, remediation plans, or formal acceptances.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediations: Vec<RiskRemediation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RiskRelatedObservation {
    pub observation_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RiskRemediation {
    pub uuid: String,
    /// `"mitigation"` | `"risk-response"` | `"vendor-check-in"`
    pub lifecycle: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<RemediationTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RemediationTask {
    pub uuid: String,
    pub title: String,
    #[serde(rename = "type")]
    pub task_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Findings ──────────────────────────────────────────────────────────────────

/// Raw evidence collected during the assessment (a run result, a metric score, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Observation {
    pub uuid: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relevant_evidence: Vec<RelevantEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remarks: Option<String>,
}

/// A pointer to the actual evidence artifact that supports an observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RelevantEvidence {
    /// URI reference to the evidence (e.g. a run ID, tarball SHA256, or file path).
    pub href: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
}

/// The outcome for a single control — satisfied, not-satisfied, or other.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Finding {
    pub uuid: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub target: FindingTarget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_observations: Vec<RelatedObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remarks: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FindingTarget {
    #[serde(rename = "type")]
    pub target_type: String,
    pub target_id: String,
    pub status: FindingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FindingStatus {
    /// `"satisfied"` | `"not-satisfied"` | `"other"`
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RelatedObservation {
    pub observation_uuid: String,
}
