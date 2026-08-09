//! `agc athlon validate` — the 8-point binding check from the design plan's
//! Iteration 3. Pure logic lives here (testable without touching the
//! filesystem); `athlon.rs` owns the IO (scanning fixtures, checking paths).

use super::athlon_format::AthlonDefinition;
use serde::Serialize;

/// Soft reference list only — per Adversarial #6, the source document doesn't
/// cleanly enumerate a closed set, so an unrecognized value is a warning, not
/// a hard failure, and this list is trivially editable if that changes.
pub const CANONICAL_TRUSTWORTHINESS_CHARACTERISTICS: &[&str] = &[
    "Valid & Reliable",
    "Safe",
    "Secure & Resilient",
    "Accountable & Transparent",
    "Explainable & Interpretable",
    "Privacy-enhanced",
    "Fair (with Harmful Bias Managed)",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// Which of the 8 checks this violation came from (1-indexed, matches the
    /// design plan's Iteration 3 enumeration).
    pub check: u8,
    pub severity: Severity,
    pub message: String,
}

/// `all_case_tags`: one entry per fixture case found anywhere in the project,
/// each entry being that case's full tag list. Passed in (rather than scanned
/// here) so this function stays pure and unit-testable.
pub fn validate_athlon(def: &AthlonDefinition, all_case_tags: &[Vec<String>]) -> Vec<Violation> {
    let mut violations = Vec::new();
    let slug = &def.slug;
    let prefix = format!("tevv-athlon:{slug}:");

    // Flatten (case_index, tag) pairs under our own athlon's tag namespace only.
    let our_tags: Vec<(usize, &str)> = all_case_tags
        .iter()
        .enumerate()
        .flat_map(|(i, tags)| {
            tags.iter()
                .filter(|t| t.starts_with(&prefix))
                .map(move |t| (i, t.as_str()))
        })
        .collect();

    for block in &def.blocks {
        // Check 1: a Block with no Event bound to it.
        if block.events.is_empty() {
            violations.push(Violation {
                check: 1,
                severity: Severity::Error,
                message: format!("block '{}' has no Events", block.id),
            });
        }

        // Check 5: TC not in the canonical reference list (soft warning).
        for tc in &block.trustworthiness_characteristics {
            if !CANONICAL_TRUSTWORTHINESS_CHARACTERISTICS.contains(&tc.as_str()) {
                violations.push(Violation {
                    check: 5,
                    severity: Severity::Warning,
                    message: format!(
                        "block '{}' declares Trustworthiness Characteristic '{tc}', \
                         which isn't in the canonical reference list (this is only a \
                         suggestion — NIST AI 200-2 doesn't enumerate a closed set)",
                        block.id
                    ),
                });
            }
        }

        for event in &block.events {
            // Check 6: an Event declaring both native and external, or neither.
            if event.native.is_some() == event.external.is_some() {
                let msg = if event.native.is_some() {
                    format!(
                        "event '{}' under block '{}' declares both native and external",
                        event.id, block.id
                    )
                } else {
                    format!(
                        "event '{}' under block '{}' declares neither native nor external",
                        event.id, block.id
                    )
                };
                violations.push(Violation {
                    check: 6,
                    severity: Severity::Error,
                    message: msg,
                });
                continue;
            }

            if let Some(native) = &event.native {
                // Check 4: native Event fixture tag with zero matching cases.
                let has_match = all_case_tags
                    .iter()
                    .any(|tags| tags.contains(&native.fixture_tag));
                if !has_match {
                    violations.push(Violation {
                        check: 4,
                        severity: Severity::Error,
                        message: format!(
                            "event '{}' under block '{}': fixture tag '{}' matches zero cases",
                            event.id, block.id, native.fixture_tag
                        ),
                    });
                }

                // Check 8: a case with the three-level tag must also carry the
                // matching two-level tag.
                let two_level = format!("tevv-athlon:{slug}:{}", block.id);
                for (i, tags) in all_case_tags.iter().enumerate() {
                    if tags.contains(&native.fixture_tag) && !tags.contains(&two_level) {
                        violations.push(Violation {
                            check: 8,
                            severity: Severity::Error,
                            message: format!(
                                "case[{i}] carries '{}' but not the matching joint tag '{two_level}'",
                                native.fixture_tag
                            ),
                        });
                    }
                }
            }

            if let Some(external) = &event.external {
                // Check 7: external Event evidence_path missing on disk.
                if !std::path::Path::new(&external.evidence_path).exists() {
                    violations.push(Violation {
                        check: 7,
                        severity: Severity::Error,
                        message: format!(
                            "event '{}' under block '{}': evidence_path '{}' does not exist",
                            event.id, block.id, external.evidence_path
                        ),
                    });
                }
            }
        }

        // Check 8 (vice versa): a case with the two-level tag must carry at
        // least one three-level tag under this Block.
        let two_level = format!("tevv-athlon:{slug}:{}", block.id);
        let three_level_prefix = format!("{two_level}:");
        for (i, tags) in all_case_tags.iter().enumerate() {
            if tags.iter().any(|t| t == &two_level)
                && !tags.iter().any(|t| t.starts_with(&three_level_prefix))
            {
                violations.push(Violation {
                    check: 8,
                    severity: Severity::Error,
                    message: format!(
                        "case[{i}] carries the joint tag '{two_level}' but no matching \
                         per-Event tag under it"
                    ),
                });
            }
        }
    }

    // Checks 2 & 3: a case tagged to a Block/Event ID that isn't declared.
    let known_block_ids: std::collections::HashSet<&str> =
        def.blocks.iter().map(|b| b.id.as_str()).collect();
    for (case_idx, tag) in &our_tags {
        let rest = &tag[prefix.len()..];
        let mut parts = rest.splitn(2, ':');
        let block_id = parts.next().unwrap_or("");
        let event_id = parts.next();

        if !known_block_ids.contains(block_id) {
            violations.push(Violation {
                check: 2,
                severity: Severity::Error,
                message: format!(
                    "case[{case_idx}] tag '{tag}' references undeclared block '{block_id}'"
                ),
            });
            continue;
        }

        if let Some(event_id) = event_id {
            let block = def.blocks.iter().find(|b| b.id == block_id);
            let known = block.is_some_and(|b| b.events.iter().any(|e| e.id == event_id));
            if !known {
                violations.push(Violation {
                    check: 3,
                    severity: Severity::Error,
                    message: format!(
                        "case[{case_idx}] tag '{tag}' references undeclared event '{event_id}' \
                         under block '{block_id}'"
                    ),
                });
            }
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::athlon_format::{
        Block, Event, EvidenceResult, ExternalEvent, Goals, LifecycleStage, NativeEvent,
        SCHEMA_VERSION,
    };

    fn minimal_def() -> AthlonDefinition {
        AthlonDefinition {
            schema_version: SCHEMA_VERSION,
            slug: "demo".to_string(),
            goals: Goals {
                objective: "obj".to_string(),
                stakeholders: vec![],
                lifecycle_stage: LifecycleStage::Deploy,
                cost_and_duration: None,
                builds_on: None,
                success_criteria: None,
                challenges: None,
            },
            blocks: vec![],
        }
    }

    #[test]
    fn check1_block_with_no_events() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![],
        });
        let violations = validate_athlon(&def, &[]);
        assert!(violations.iter().any(|v| v.check == 1));
    }

    #[test]
    fn check2_case_tagged_to_undeclared_block() {
        let def = minimal_def();
        let tags = vec![vec!["tevv-athlon:demo:ghost-block".to_string()]];
        let violations = validate_athlon(&def, &tags);
        assert!(violations.iter().any(|v| v.check == 2));
    }

    #[test]
    fn check3_case_tagged_to_undeclared_event() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                }),
                external: None,
            }],
        });
        let tags = vec![vec!["tevv-athlon:demo:b1:ghost-event".to_string()]];
        let violations = validate_athlon(&def, &tags);
        assert!(violations.iter().any(|v| v.check == 3));
    }

    #[test]
    fn check4_native_event_fixture_tag_has_zero_matches() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                }),
                external: None,
            }],
        });
        let violations = validate_athlon(&def, &[]);
        assert!(violations.iter().any(|v| v.check == 4));
    }

    #[test]
    fn check5_unrecognized_tc_is_a_warning_not_an_error() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec!["Made Up Characteristic".to_string()],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                }),
                external: None,
            }],
        });
        let tags = vec![vec![
            "tevv-athlon:demo:b1:e1".to_string(),
            "tevv-athlon:demo:b1".to_string(),
        ]];
        let violations = validate_athlon(&def, &tags);
        let v = violations
            .iter()
            .find(|v| v.check == 5)
            .expect("expected a check-5 violation");
        assert_eq!(v.severity, Severity::Warning);
    }

    #[test]
    fn check6_event_with_both_kinds() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "x".to_string(),
                }),
                external: Some(ExternalEvent {
                    tool: "garak".to_string(),
                    description: "d".to_string(),
                    evidence_path: "p".to_string(),
                    summary: None,
                    assessed_by: None,
                    date: None,
                    result: EvidenceResult::Inconclusive,
                }),
            }],
        });
        let violations = validate_athlon(&def, &[]);
        assert!(violations.iter().any(|v| v.check == 6));
    }

    #[test]
    fn check6_event_with_neither_kind() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: None,
                external: None,
            }],
        });
        let violations = validate_athlon(&def, &[]);
        assert!(violations.iter().any(|v| v.check == 6));
    }

    #[test]
    fn check7_external_evidence_path_missing() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: None,
                external: Some(ExternalEvent {
                    tool: "garak".to_string(),
                    description: "d".to_string(),
                    evidence_path: "/definitely/does/not/exist/anywhere.jsonl".to_string(),
                    summary: None,
                    assessed_by: None,
                    date: None,
                    result: EvidenceResult::Pass,
                }),
            }],
        });
        let violations = validate_athlon(&def, &[]);
        assert!(violations.iter().any(|v| v.check == 7));
    }

    #[test]
    fn check8_three_level_tag_without_matching_two_level_tag() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                }),
                external: None,
            }],
        });
        // Case carries only the three-level tag, not the two-level joint tag.
        let tags = vec![vec!["tevv-athlon:demo:b1:e1".to_string()]];
        let violations = validate_athlon(&def, &tags);
        assert!(violations.iter().any(|v| v.check == 8));
    }

    #[test]
    fn check8_two_level_tag_without_any_three_level_tag() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec![],
            definition: "d".to_string(),
            events: vec![Event {
                id: "e1".to_string(),
                native: Some(NativeEvent {
                    evaluator: "rules".to_string(),
                    fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                }),
                external: None,
            }],
        });
        // Case carries only the two-level (joint) tag, no per-Event tag.
        let tags = vec![vec!["tevv-athlon:demo:b1".to_string()]];
        let violations = validate_athlon(&def, &tags);
        assert!(violations.iter().any(|v| v.check == 8));
    }

    #[test]
    fn well_formed_athlon_has_no_violations() {
        let mut def = minimal_def();
        def.blocks.push(Block {
            id: "b1".to_string(),
            trustworthiness_characteristics: vec!["Safe".to_string()],
            definition: "d".to_string(),
            events: vec![
                Event {
                    id: "e1".to_string(),
                    native: Some(NativeEvent {
                        evaluator: "rules".to_string(),
                        fixture_tag: "tevv-athlon:demo:b1:e1".to_string(),
                    }),
                    external: None,
                },
                Event {
                    id: "e2".to_string(),
                    native: None,
                    external: Some(ExternalEvent {
                        tool: "garak".to_string(),
                        description: "d".to_string(),
                        // Every source file exists on disk, so use one we know is real.
                        evidence_path: "Cargo.toml".to_string(),
                        summary: None,
                        assessed_by: None,
                        date: None,
                        result: EvidenceResult::Pass,
                    }),
                },
            ],
        });
        let tags = vec![vec![
            "tevv-athlon:demo:b1:e1".to_string(),
            "tevv-athlon:demo:b1".to_string(),
        ]];
        let violations = validate_athlon(&def, &tags);
        assert!(
            violations.is_empty(),
            "expected no violations, got {violations:?}"
        );
    }
}
