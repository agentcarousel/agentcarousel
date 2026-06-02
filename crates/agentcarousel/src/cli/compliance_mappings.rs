use agentcarousel_core::{CaseStatus, Run};
use oscal::catalog::{load_catalog, CatalogSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Minimum fixture case count required before a control can be marked Satisfied.
/// A single case is not statistically meaningful evidence for any compliance framework.
pub const MIN_CASES: u32 = 3;

/// Effectiveness mean threshold above which a control is considered Satisfied (0.0–1.0).
/// Applies uniformly across all frameworks unless overridden by per-framework config.
pub const SATISFACTION_THRESHOLD_DEFAULT: f32 = 0.80;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkControl {
    pub framework: String,
    pub control_id: String,
    pub requirement: String,
    /// Fixture tag used to map cases to this control (e.g. `"nist-800-171:3.1.1"`).
    pub tag: String,
    /// Whether this control is primarily validated by behavioral test cases.
    pub behavioral: bool,
    /// Relative importance weight for scoring (default 1.0).
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlScore {
    pub control: FrameworkControl,
    pub model_version: String,
    pub effectiveness_mean: f32,
    pub pass_rate: f32,
    pub case_count: u32,
    pub run_count: u32,
    pub last_run_date: Option<String>,
    pub status: ControlCoverageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlCoverageStatus {
    /// All mapped cases pass above threshold with sufficient evidence (≥ MIN_CASES).
    Satisfied,
    /// Some cases pass but below the required count or threshold.
    PartialEvidence,
    /// Cases exist but all fail (pass_rate == 0.0).
    Failed,
    /// No mapped fixture cases found.
    Gap,
    /// Control is documented but validated by procedure, not automated tests.
    Procedural,
}

/// Merged view of all known framework controls, keyed by framework name.
pub type FrameworkRegistry = HashMap<String, Vec<FrameworkControl>>;

/// Build the framework registry from three sources in priority order:
/// 1. Embedded OSCAL catalogs from the `oscal` crate
/// 2. `./frameworks/*.json` in the working directory
/// 3. `~/.agentcarousel/frameworks/*.json`
///
/// Later sources can extend (but not remove) controls from earlier ones.
pub fn load_framework_registry() -> FrameworkRegistry {
    let mut registry: FrameworkRegistry = HashMap::new();

    load_embedded_catalogs(&mut registry);
    load_directory_catalogs(&mut registry, std::path::Path::new("frameworks"));
    if let Some(home) = home_frameworks_dir() {
        load_directory_catalogs(&mut registry, &home);
    }

    registry
}

/// Return all `FrameworkControl` entries for the named framework, or an empty slice.
pub fn controls_for_framework<'a>(
    registry: &'a FrameworkRegistry,
    name: &str,
) -> &'a [FrameworkControl] {
    registry.get(name).map(|v| v.as_slice()).unwrap_or(&[])
}

/// Aggregate `CaseResult.tags` from `runs` into per-control effectiveness scores
/// for the named `framework`, optionally scoped to a single `skill` and/or a
/// specific generator `model`. When `model_filter` is `Some`, only runs whose
/// `generator_model` matches exactly are included.
///
/// Loads the framework registry on each call. Use `compute_control_scores_with_registry`
/// when running multiple frameworks in a loop to avoid repeated registry I/O.
pub fn compute_control_scores(
    runs: &[Run],
    framework: &str,
    skill: Option<&str>,
    model_filter: Option<&str>,
) -> Vec<ControlScore> {
    let registry = load_framework_registry();
    compute_control_scores_with_registry(&registry, runs, framework, skill, model_filter)
}

/// Like `compute_control_scores` but reuses a pre-loaded `FrameworkRegistry`.
/// Prefer this when scoring multiple frameworks in a single command invocation.
pub fn compute_control_scores_with_registry(
    registry: &FrameworkRegistry,
    runs: &[Run],
    framework: &str,
    skill: Option<&str>,
    model_filter: Option<&str>,
) -> Vec<ControlScore> {
    let controls = controls_for_framework(registry, framework);
    if controls.is_empty() {
        return vec![];
    }

    // Collect cases and run metadata keyed by model version.
    let mut model_cases: HashMap<String, Vec<&agentcarousel_core::CaseResult>> = HashMap::new();
    let mut model_run_counts: HashMap<String, u32> = HashMap::new();
    let mut model_last_date: HashMap<String, String> = HashMap::new();

    for run in runs {
        if let Some(sk) = skill {
            if run.skill_or_agent.as_deref() != Some(sk) {
                continue;
            }
        }
        let model = run
            .summary
            .generator_model
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        if let Some(mf) = model_filter {
            if model != mf {
                continue;
            }
        }

        *model_run_counts.entry(model.clone()).or_insert(0) += 1;
        let run_end = run.finished_at.unwrap_or(run.started_at).to_rfc3339();
        model_last_date
            .entry(model.clone())
            .and_modify(|d| {
                if run_end > *d {
                    *d = run_end.clone();
                }
            })
            .or_insert(run_end);
        model_cases
            .entry(model)
            .or_default()
            .extend(run.cases.iter());
    }

    let mut scores = Vec::new();
    for control in controls {
        for (model, cases) in &model_cases {
            let matching: Vec<_> = cases
                .iter()
                .filter(|c| c.tags.contains(&control.tag))
                .collect();
            let case_count = matching.len() as u32;

            let (effectiveness_mean, pass_rate, status) = if !control.behavioral {
                (0.0_f32, 0.0_f32, ControlCoverageStatus::Procedural)
            } else if case_count == 0 {
                (0.0, 0.0, ControlCoverageStatus::Gap)
            } else {
                let passed = matching
                    .iter()
                    .filter(|c| c.status == CaseStatus::Passed)
                    .count();
                let pr = passed as f32 / case_count as f32;

                let judge_scores: Vec<f32> = matching
                    .iter()
                    .filter_map(|c| {
                        c.eval_scores
                            .as_ref()
                            .filter(|es| es.evaluator != "rules")
                            .map(|es| es.effectiveness_score)
                    })
                    .collect();

                let em = if !judge_scores.is_empty() {
                    judge_scores.iter().sum::<f32>() / judge_scores.len() as f32
                } else {
                    pr
                };

                let st = if pr == 0.0 {
                    ControlCoverageStatus::Failed
                } else if case_count < MIN_CASES || em < SATISFACTION_THRESHOLD_DEFAULT {
                    ControlCoverageStatus::PartialEvidence
                } else {
                    ControlCoverageStatus::Satisfied
                };
                (em, pr, st)
            };

            scores.push(ControlScore {
                control: control.clone(),
                model_version: model.clone(),
                effectiveness_mean,
                pass_rate,
                case_count,
                run_count: model_run_counts.get(model).copied().unwrap_or(0),
                last_run_date: model_last_date.get(model).cloned(),
                status,
            });
        }
    }
    scores
}

/// Collapse a `Vec<ControlScore>` (which may have one entry per model) into one
/// entry per control, keeping the entry with the highest-priority status.
///
/// Priority: Satisfied > PartialEvidence > Gap > Procedural.
/// Tie-breaks on effectiveness_mean (higher wins).
pub fn collapse_scores(scores: &[ControlScore]) -> Vec<ControlScore> {
    let mut best: HashMap<String, &ControlScore> = HashMap::new();
    for s in scores {
        let key = s.control.control_id.clone();
        let winner = best.entry(key).or_insert(s);
        if status_rank(&s.status) > status_rank(&winner.status)
            || (status_rank(&s.status) == status_rank(&winner.status)
                && s.effectiveness_mean > winner.effectiveness_mean)
        {
            *winner = s;
        }
    }
    // Preserve original order by iterating controls in insertion order.
    let mut seen = std::collections::HashSet::new();
    scores
        .iter()
        .filter(|s| seen.insert(s.control.control_id.clone()))
        .map(|s| best[&s.control.control_id].clone())
        .collect()
}

fn status_rank(s: &ControlCoverageStatus) -> u8 {
    match s {
        ControlCoverageStatus::Satisfied => 4,
        ControlCoverageStatus::PartialEvidence => 3,
        ControlCoverageStatus::Procedural => 2,
        ControlCoverageStatus::Failed => 1,
        ControlCoverageStatus::Gap => 0,
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn load_embedded_catalogs(registry: &mut FrameworkRegistry) {
    for name in oscal::catalogs::EMBEDDED_CATALOG_NAMES {
        let catalog = match load_catalog(CatalogSource::Embedded(name)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let controls = registry.entry(name.to_string()).or_default();
        for control in catalog.all_controls() {
            controls.push(FrameworkControl {
                framework: name.to_string(),
                control_id: control.id.clone(),
                requirement: control.statement().unwrap_or(&control.title).to_string(),
                tag: format!("{name}:{}", control.id),
                behavioral: true,
                importance: 1.0,
            });
        }
    }
}

fn load_directory_catalogs(registry: &mut FrameworkRegistry, dir: &std::path::Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let controls: Vec<FrameworkControl> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(framework) = controls.first().map(|c| c.framework.clone()) {
            registry.entry(framework).or_default().extend(controls);
        }
    }
}

fn home_frameworks_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".agentcarousel").join("frameworks"))
}
