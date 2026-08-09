//! `athlon.yaml` format: the single authoring file for a TEVV-Athlon assessment
//! (NIST AI 200-2), plus write-through materialization into the compliance
//! registry's `FrameworkControl` shape.
//!
//! See docs/plans/2026-08-08-tevv-athlon-independent-baseline.md, Iteration 2,
//! for the design this module implements.

use crate::cli::compliance_mappings::FrameworkControl;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Error)]
pub enum AthlonError {
    #[error("athlon '{0}' not found at {1} — run `agc athlon init --slug {0}` first")]
    NotFound(String, PathBuf),
    #[error("failed to read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("failed to parse {0}: {1}")]
    Parse(PathBuf, String),
    #[error("failed to write {0}: {1}")]
    Write(PathBuf, std::io::Error),
    #[error("athlon '{0}' already exists at {1} (pass --force to overwrite)")]
    AlreadyExists(String, PathBuf),
    #[error("block '{0}' not found in athlon '{1}'")]
    BlockNotFound(String, String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AthlonDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub slug: String,
    pub goals: Goals,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goals {
    pub objective: String,
    #[serde(default)]
    pub stakeholders: Vec<String>,
    pub lifecycle_stage: LifecycleStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_and_duration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builds_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenges: Option<String>,
}

/// The AI lifecycle stages from NIST AI 200-2 Fig. 1 (OECD taxonomy).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleStage {
    PlanAndDesign,
    CollectAndProcessData,
    BuildAndUse,
    Deploy,
    OperateAndMonitor,
}

/// A Metrology Block (Stage 2: Define & Construct).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: String,
    #[serde(default)]
    pub trustworthiness_characteristics: Vec<String>,
    pub definition: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// An Event (Stage 3: Apply & Measure) — either native (agc executes it) or
/// external (evidence attached by reference). Exactly one of the two must be
/// present; enforced by `agc athlon validate` (design plan, Iteration 3, check 6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeEvent {
    /// One of "rules", "golden", "process", "judge" — matches `EvaluatorKind`.
    pub evaluator: String,
    pub fixture_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalEvent {
    pub tool: String,
    pub description: String,
    pub evidence_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub result: EvidenceResult,
}

/// Advisory only — never blended into `ControlScore`. See Adversarial #5.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceResult {
    Pass,
    Fail,
    Inconclusive,
}

// ── Paths ───────────────────────────────────────────────────────────────────

pub fn athlon_path(slug: &str) -> PathBuf {
    Path::new("athlons").join(format!("{slug}.yaml"))
}

pub fn materialized_path(slug: &str) -> PathBuf {
    Path::new("frameworks").join(format!("tevv-athlon-{slug}.json"))
}

/// The `framework` registry key for an athlon — see plan §2.3's namespacing (D4).
pub fn framework_key(slug: &str) -> String {
    format!("tevv-athlon:{slug}")
}

// ── Load / Save ─────────────────────────────────────────────────────────────

pub fn load_athlon(slug: &str) -> Result<AthlonDefinition, AthlonError> {
    let path = athlon_path(slug);
    if !path.exists() {
        return Err(AthlonError::NotFound(slug.to_string(), path));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| AthlonError::Read(path.clone(), e))?;
    serde_yaml::from_str(&raw).map_err(|e| AthlonError::Parse(path, e.to_string()))
}

pub fn save_athlon(def: &AthlonDefinition) -> Result<(), AthlonError> {
    let path = athlon_path(&def.slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AthlonError::Write(path.clone(), e))?;
    }
    let yaml = serde_yaml::to_string(def)
        .map_err(|e| AthlonError::Write(path.clone(), std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, yaml).map_err(|e| AthlonError::Write(path, e))
}

// ── Materialization (two-tag scheme, plan §2.3) ─────────────────────────────

/// Convert every native Event across all Blocks into the two-level tag scheme:
/// one three-level `FrameworkControl` per native Event (its own contribution in
/// isolation), plus one two-level `FrameworkControl` per Block (the joint,
/// pooled view across every native Event feeding it). External Events never
/// produce a `FrameworkControl` row — there's no fixture evidence to score.
pub fn materialize(def: &AthlonDefinition) -> Vec<FrameworkControl> {
    let framework = framework_key(&def.slug);
    let mut controls = Vec::new();

    for block in &def.blocks {
        let native_events: Vec<&Event> =
            block.events.iter().filter(|e| e.native.is_some()).collect();

        for event in &native_events {
            let native = event.native.as_ref().expect("filtered above");
            controls.push(FrameworkControl {
                framework: framework.clone(),
                control_id: format!("{}:{}", block.id, event.id),
                requirement: format!("{} ({} Event).", block.definition, event.id),
                tag: native.fixture_tag.clone(),
                behavioral: true,
                importance: 1.0,
                trustworthiness_characteristics: block.trustworthiness_characteristics.clone(),
            });
        }

        if !native_events.is_empty() {
            controls.push(FrameworkControl {
                framework: framework.clone(),
                control_id: block.id.clone(),
                requirement: format!("{} (joint, all Events).", block.definition),
                tag: format!("{framework}:{}", block.id),
                behavioral: true,
                importance: 1.0,
                trustworthiness_characteristics: block.trustworthiness_characteristics.clone(),
            });
        }
    }

    controls
}

/// Write `materialize(def)` to `frameworks/tevv-athlon-<slug>.json`, then read
/// it back and confirm it round-trips through `serde_json` before considering
/// the write successful (Adversarial #3 — never leave a partially-written or
/// unparseable materialized file behind).
pub fn write_materialized(def: &AthlonDefinition) -> Result<(), AthlonError> {
    let controls = materialize(def);
    let path = materialized_path(&def.slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AthlonError::Write(path.clone(), e))?;
    }
    let json = serde_json::to_string_pretty(&controls)
        .map_err(|e| AthlonError::Write(path.clone(), std::io::Error::other(e.to_string())))?;
    std::fs::write(&path, &json).map_err(|e| AthlonError::Write(path.clone(), e))?;

    // Round-trip check: re-read and re-parse what was just written.
    let reread = std::fs::read_to_string(&path).map_err(|e| AthlonError::Write(path.clone(), e))?;
    let _: Vec<FrameworkControl> = serde_json::from_str(&reread)
        .map_err(|e| AthlonError::Write(path, std::io::Error::other(e.to_string())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_athlon() -> AthlonDefinition {
        AthlonDefinition {
            schema_version: SCHEMA_VERSION,
            slug: "query-violation-chatbot".to_string(),
            goals: Goals {
                objective: "To what extent do our support chatbots answer queries without giving away violations?".to_string(),
                stakeholders: vec!["support-eng".to_string(), "trust-and-safety".to_string()],
                lifecycle_stage: LifecycleStage::OperateAndMonitor,
                cost_and_duration: Some("2 sprints, existing eval infra".to_string()),
                builds_on: None,
                success_criteria: None,
                challenges: None,
            },
            blocks: vec![
                Block {
                    id: "helpfulness".to_string(),
                    trustworthiness_characteristics: vec!["Valid & Reliable".to_string()],
                    definition: "Extent to which the system answers users' queries usefully."
                        .to_string(),
                    events: vec![Event {
                        id: "user-testing".to_string(),
                        native: Some(NativeEvent {
                            evaluator: "judge".to_string(),
                            fixture_tag: "tevv-athlon:query-violation-chatbot:helpfulness:user-testing".to_string(),
                        }),
                        external: None,
                    }],
                },
                Block {
                    id: "violation-frequency".to_string(),
                    trustworthiness_characteristics: vec!["Safe".to_string()],
                    definition: "Rate at which the system discloses a prohibited answer type."
                        .to_string(),
                    events: vec![
                        Event {
                            id: "user-testing".to_string(),
                            native: Some(NativeEvent {
                                evaluator: "rules".to_string(),
                                fixture_tag: "tevv-athlon:query-violation-chatbot:violation-frequency:user-testing".to_string(),
                            }),
                            external: None,
                        },
                        Event {
                            id: "red-teaming".to_string(),
                            native: None,
                            external: Some(ExternalEvent {
                                tool: "garak".to_string(),
                                description: "Automated LLM red-team probe suite".to_string(),
                                evidence_path: "./evidence/garak-run.jsonl".to_string(),
                                summary: Some("4/312 violations".to_string()),
                                assessed_by: Some("trust-and-safety@acme.example".to_string()),
                                date: Some("2026-09-12".to_string()),
                                result: EvidenceResult::Pass,
                            }),
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn athlon_yaml_round_trips() {
        let def = sample_athlon();
        let yaml = serde_yaml::to_string(&def).unwrap();
        let parsed: AthlonDefinition = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.slug, def.slug);
        assert_eq!(parsed.blocks.len(), 2);
        assert_eq!(
            parsed.goals.lifecycle_stage,
            LifecycleStage::OperateAndMonitor
        );
    }

    #[test]
    fn lifecycle_stage_serializes_kebab_case() {
        let yaml = serde_yaml::to_string(&LifecycleStage::OperateAndMonitor).unwrap();
        assert_eq!(yaml.trim(), "operate-and-monitor");
    }

    // The "exactly one of native/external" shape invariant is enforced by
    // `agc athlon validate` (check 6) — see agc-g2qe.5, which owns its tests.

    #[test]
    fn materialize_produces_three_level_and_two_level_controls() {
        let def = sample_athlon();
        let controls = materialize(&def);

        // helpfulness: 1 native Event -> 1 three-level + 1 two-level = 2 controls.
        // violation-frequency: 1 native Event (red-teaming is external, skipped)
        //   -> 1 three-level + 1 two-level = 2 controls.
        assert_eq!(controls.len(), 4);

        let three_level = controls
            .iter()
            .find(|c| c.control_id == "helpfulness:user-testing")
            .expect("three-level helpfulness control missing");
        assert_eq!(
            three_level.tag,
            "tevv-athlon:query-violation-chatbot:helpfulness:user-testing"
        );
        assert_eq!(
            three_level.trustworthiness_characteristics,
            vec!["Valid & Reliable"]
        );

        let two_level = controls
            .iter()
            .find(|c| c.control_id == "helpfulness")
            .expect("two-level (joint) helpfulness control missing");
        assert_eq!(
            two_level.tag,
            "tevv-athlon:query-violation-chatbot:helpfulness"
        );

        // violation-frequency's external red-teaming Event must never materialize.
        assert!(!controls
            .iter()
            .any(|c| c.control_id.contains("red-teaming")));

        let vf_three_level = controls
            .iter()
            .find(|c| c.control_id == "violation-frequency:user-testing")
            .expect("three-level violation-frequency control missing");
        assert_eq!(vf_three_level.trustworthiness_characteristics, vec!["Safe"]);
    }

    #[test]
    fn framework_key_matches_namespacing_scheme() {
        assert_eq!(framework_key("demo"), "tevv-athlon:demo");
    }

    #[test]
    fn load_athlon_missing_file_returns_not_found() {
        let err = load_athlon("does-not-exist-anywhere-xyz").unwrap_err();
        assert!(matches!(err, AthlonError::NotFound(_, _)));
    }
}
