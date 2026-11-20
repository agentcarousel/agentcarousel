use agentcarousel_core::{CaseStatus, Run};
use oscal::load_catalog;
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
    /// Link to the control's definition at its publisher, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Version of the framework this control entry is pinned to (e.g. `"2026-Q4"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework_version: Option<String>,
    /// Why the control is (or is not) treated as behavioral, and evidence limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Framework crosswalks bundled with the binary as `Vec<FrameworkControl>` JSON.
///
/// These are frameworks whose control text is not redistributable, so the
/// bundle carries only control IDs, AgentCarousel's own short labels, and
/// links to the publisher. See `docs/frameworks/`.
const BUNDLED_FRAMEWORKS: &[(&str, &str)] = &[(
    "aiuc-1",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/frameworks/aiuc-1.json"
    )),
)];

/// Attribution line shown under a framework's compliance report, when the
/// framework's control labels are AgentCarousel's own wording.
pub fn framework_attribution(framework: &str) -> Option<&'static str> {
    match framework {
        "aiuc-1" => Some(
            "AIUC-1 control IDs and links refer to the standard published by the Artificial \
             Intelligence Underwriting Company (AIUC). Control labels are AgentCarousel's own \
             wording. AgentCarousel is not affiliated with or endorsed by AIUC.",
        ),
        _ => None,
    }
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
/// Bundled crosswalks (see [`BUNDLED_FRAMEWORKS`]) load after the embedded
/// catalogs. Later sources can add controls; a control whose `tag` already
/// exists replaces the earlier entry instead of being counted twice.
pub fn load_framework_registry() -> FrameworkRegistry {
    let mut registry: FrameworkRegistry = HashMap::new();

    load_embedded_catalogs(&mut registry);
    load_bundled_frameworks(&mut registry);
    load_directory_catalogs(&mut registry, std::path::Path::new("frameworks"));
    if let Some(home) = home_frameworks_dir() {
        load_directory_catalogs(&mut registry, &home);
    }

    registry
}

/// Add `controls` to `registry`, replacing any existing control with the same tag.
fn merge_controls(registry: &mut FrameworkRegistry, controls: Vec<FrameworkControl>) {
    for control in controls {
        let entry = registry.entry(control.framework.clone()).or_default();
        match entry.iter_mut().find(|c| c.tag == control.tag) {
            Some(existing) => *existing = control,
            None => entry.push(control),
        }
    }
}

fn load_bundled_frameworks(registry: &mut FrameworkRegistry) {
    for (name, raw) in BUNDLED_FRAMEWORKS {
        match serde_json::from_str::<Vec<FrameworkControl>>(raw) {
            Ok(controls) => merge_controls(registry, controls),
            Err(err) => debug_assert!(false, "bundled framework {name} is invalid: {err}"),
        }
    }
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

    // Build a per-model tag → cases index to avoid O(controls × cases) filtering.
    let mut model_tag_index: HashMap<
        String,
        HashMap<String, Vec<&agentcarousel_core::CaseResult>>,
    > = HashMap::new();
    for (model, cases) in &model_cases {
        let index = model_tag_index.entry(model.clone()).or_default();
        // Not-tested cases are not evidence for any control (assurance spec INV-1).
        for case in cases
            .iter()
            .copied()
            .filter(|case| case.status != CaseStatus::Skipped)
        {
            for tag in &case.tags {
                index.entry(tag.clone()).or_default().push(case);
            }
        }
    }

    let mut scores = Vec::new();
    for control in controls {
        for model in model_cases.keys() {
            let empty: Vec<&agentcarousel_core::CaseResult> = vec![];
            let matching: &[&agentcarousel_core::CaseResult] = model_tag_index
                .get(model)
                .and_then(|idx| idx.get(&control.tag))
                .map(Vec::as_slice)
                .unwrap_or(&empty);
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
/// Priority: Satisfied > PartialEvidence > Procedural > Failed > Gap.
/// Note: Gap and Procedural are mutually exclusive per control (non-behavioral
/// controls always produce Procedural, never Gap), so their relative rank only
/// affects edge-case comparisons across heterogeneous score sets.
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
        let catalog = match load_catalog(name) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let controls = registry.entry(name.to_string()).or_default();
        for control in catalog.all_controls() {
            controls.push(FrameworkControl {
                framework: name.to_string(),
                control_id: control.id.clone(),
                requirement: control
                    .statement()
                    .map(str::to_string)
                    .unwrap_or_else(|| control.title.to_string()),
                tag: format!("{name}:{}", control.id),
                // OSCAL catalogs mark governance/process controls with
                // `evidence-type: procedural`; those are validated by procedure,
                // not by behavioral test cases.
                behavioral: !control
                    .props
                    .iter()
                    .any(|p| p.name == "evidence-type" && p.value == "procedural"),
                importance: 1.0,
                url: None,
                framework_version: None,
                notes: None,
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
        merge_controls(registry, controls);
    }
}

fn home_frameworks_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".agentcarousel").join("frameworks"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentcarousel_core::{
        CaseId, CaseResult, ExecutionTrace, Metrics, OverallStatus, ProviderErrorMetrics, RunId,
        RunSummary,
    };
    use chrono::Utc;

    fn case(id: &str, status: CaseStatus, tags: &[&str]) -> CaseResult {
        CaseResult {
            case_id: CaseId(format!("skill/{id}")),
            status,
            error: None,
            trace: ExecutionTrace {
                steps: Vec::new(),
                final_output: Some("ok".to_string()),
                redacted: false,
            },
            metrics: Metrics::default(),
            eval_scores: None,
            input: vec![],
            discrimination_score: None,
            discrimination_label: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            skip_reason: None,
            critical: false,
        }
    }

    fn run_with(cases: Vec<CaseResult>, model: &str) -> Run {
        let now = Utc::now();
        Run {
            id: RunId(format!("run-{model}")),
            schema_version: 1,
            started_at: now,
            finished_at: Some(now),
            command: "test".to_string(),
            git_sha: None,
            agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
            config_hash: "none".to_string(),
            cases,
            summary: RunSummary {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                flaky: 0,
                errored: 0,
                timed_out: 0,
                pass_rate: 0.0,
                mean_latency_ms: 0.0,
                mean_effectiveness_score: None,
                provider_errors: ProviderErrorMetrics::default(),
                overall_status: OverallStatus::Pass,
                tokens_in: None,
                tokens_out: None,
                mean_tokens_per_judged_case: None,
                latency_p50_ms: None,
                latency_p95_ms: None,
                latency_p99_ms: None,
                judge_tokens_in: None,
                judge_tokens_out: None,
                gen_cost_usd: None,
                judge_cost_usd: None,
                total_cost_usd: None,
                generator_model: Some(model.to_string()),
                judge_model: None,
                command_line: None,
            },
            fixture_bundle_id: None,
            fixture_bundle_version: None,
            skill_or_agent: Some("skill".to_string()),
            runner_offline: true,
            runner_mock_strict: false,
            runner_mock_only: true,
            prompt_audit: None,
            subject: None,
        }
    }

    fn control(id: &str, behavioral: bool) -> FrameworkControl {
        FrameworkControl {
            framework: "fw".to_string(),
            control_id: id.to_string(),
            requirement: format!("requirement {id}"),
            tag: format!("fw:{id}"),
            behavioral,
            importance: 1.0,
            url: None,
            framework_version: None,
            notes: None,
        }
    }

    fn registry(controls: Vec<FrameworkControl>) -> FrameworkRegistry {
        let mut r = FrameworkRegistry::new();
        r.insert("fw".to_string(), controls);
        r
    }

    fn status_of(scores: &[ControlScore], id: &str) -> ControlCoverageStatus {
        scores
            .iter()
            .find(|s| s.control.control_id == id)
            .map(|s| s.status.clone())
            .expect("control scored")
    }

    #[test]
    fn coverage_statuses_follow_the_satisfaction_rule() {
        let reg = registry(vec![
            control("proc", false),
            control("gap", true),
            control("partial", true),
            control("failed", true),
            control("satisfied", true),
        ]);
        let cases = vec![
            case("p1", CaseStatus::Passed, &["fw:partial"]),
            case("p2", CaseStatus::Passed, &["fw:partial"]),
            case("f1", CaseStatus::Failed, &["fw:failed"]),
            case("f2", CaseStatus::Failed, &["fw:failed"]),
            case("f3", CaseStatus::Failed, &["fw:failed"]),
            case("s1", CaseStatus::Passed, &["fw:satisfied", "fw:proc"]),
            case("s2", CaseStatus::Passed, &["fw:satisfied"]),
            case("s3", CaseStatus::Passed, &["fw:satisfied"]),
        ];
        let runs = vec![run_with(cases, "m")];
        let scores = compute_control_scores_with_registry(&reg, &runs, "fw", None, None);

        // Tagged cases never turn a procedural control into a behavioral one.
        assert_eq!(
            status_of(&scores, "proc"),
            ControlCoverageStatus::Procedural
        );
        assert_eq!(status_of(&scores, "gap"), ControlCoverageStatus::Gap);
        assert_eq!(
            status_of(&scores, "partial"),
            ControlCoverageStatus::PartialEvidence
        );
        assert_eq!(status_of(&scores, "failed"), ControlCoverageStatus::Failed);
        assert_eq!(
            status_of(&scores, "satisfied"),
            ControlCoverageStatus::Satisfied
        );
    }

    #[test]
    fn skipped_cases_are_not_evidence() {
        let reg = registry(vec![control("c", true)]);
        let cases = vec![
            case("a", CaseStatus::Skipped, &["fw:c"]),
            case("b", CaseStatus::Skipped, &["fw:c"]),
            case("c", CaseStatus::Skipped, &["fw:c"]),
        ];
        let runs = vec![run_with(cases, "m")];
        let scores = compute_control_scores_with_registry(&reg, &runs, "fw", None, None);
        assert_eq!(status_of(&scores, "c"), ControlCoverageStatus::Gap);
    }

    #[test]
    fn collapse_keeps_best_status_across_models() {
        let reg = registry(vec![control("c", true)]);
        let runs = vec![
            run_with(vec![case("x", CaseStatus::Failed, &["fw:c"])], "weak"),
            run_with(
                vec![
                    case("a", CaseStatus::Passed, &["fw:c"]),
                    case("b", CaseStatus::Passed, &["fw:c"]),
                    case("c", CaseStatus::Passed, &["fw:c"]),
                ],
                "strong",
            ),
        ];
        let scores = compute_control_scores_with_registry(&reg, &runs, "fw", None, None);
        assert_eq!(scores.len(), 2, "one score per model");
        let collapsed = collapse_scores(&scores);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].status, ControlCoverageStatus::Satisfied);
        assert_eq!(collapsed[0].model_version, "strong");
    }

    #[test]
    fn embedded_catalogs_honour_procedural_evidence_type() {
        let mut reg = FrameworkRegistry::new();
        load_embedded_catalogs(&mut reg);
        for (framework, expected_procedural) in [("nist-ai-rmf", 6usize), ("iso-42001", 6usize)] {
            let controls = controls_for_framework(&reg, framework);
            assert!(!controls.is_empty(), "{framework} catalog loaded");
            let procedural = controls.iter().filter(|c| !c.behavioral).count();
            assert_eq!(
                procedural, expected_procedural,
                "{framework}: controls marked evidence-type=procedural"
            );
        }
    }

    #[test]
    fn bundled_aiuc1_crosswalk_is_complete_and_well_formed() {
        let mut reg = FrameworkRegistry::new();
        load_bundled_frameworks(&mut reg);
        let controls = controls_for_framework(&reg, "aiuc-1");
        assert_eq!(
            controls.len(),
            50,
            "AIUC-1 2026-Q4 has 50 active requirements"
        );
        let tags: std::collections::HashSet<_> = controls.iter().map(|c| &c.tag).collect();
        assert_eq!(tags.len(), 50, "tags are unique");
        for c in controls {
            assert_eq!(c.tag, format!("aiuc-1:{}", c.control_id));
            assert!(
                c.url
                    .as_deref()
                    .is_some_and(|u| u.starts_with("https://standard.aiuc-1.com/")),
                "{} has a standard.aiuc-1.com link",
                c.control_id
            );
            assert_eq!(c.framework_version.as_deref(), Some("2026-Q4"));
            assert!(!c.requirement.is_empty(), "{} has a label", c.control_id);
        }
        let behavioral = controls.iter().filter(|c| c.behavioral).count();
        assert_eq!(behavioral, 15, "behavioral/procedural split is deliberate");
        for tool_dependent in ["aiuc-1:B006", "aiuc-1:D003"] {
            let c = controls.iter().find(|c| c.tag == tool_dependent).unwrap();
            assert!(!c.behavioral, "{tool_dependent} needs observed tool calls");
        }
    }

    #[test]
    fn aiuc1_docs_table_matches_the_bundled_crosswalk() {
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/frameworks/aiuc-1.md");
        let docs = std::fs::read_to_string(docs).expect("read docs/frameworks/aiuc-1.md");
        // Rows look like: | [A001](url) | label | behavioral | notes |
        let documented: std::collections::BTreeMap<String, bool> = docs
            .lines()
            .filter_map(|line| line.strip_prefix("| ["))
            .map(|row| {
                let id = row.split(']').next().unwrap().to_string();
                let evidence = row.split('|').nth(2).unwrap().trim();
                (id, evidence == "behavioral")
            })
            .collect();
        let mut reg = FrameworkRegistry::new();
        load_bundled_frameworks(&mut reg);
        let bundled: std::collections::BTreeMap<String, bool> =
            controls_for_framework(&reg, "aiuc-1")
                .iter()
                .map(|c| (c.control_id.clone(), c.behavioral))
                .collect();
        assert_eq!(documented, bundled, "docs table and aiuc-1.json disagree");
    }

    #[test]
    fn later_sources_replace_controls_with_the_same_tag() {
        let mut reg = FrameworkRegistry::new();
        load_bundled_frameworks(&mut reg);
        let mut override_d001 = controls_for_framework(&reg, "aiuc-1")
            .iter()
            .find(|c| c.control_id == "D001")
            .cloned()
            .unwrap();
        override_d001.requirement = "custom label".to_string();
        merge_controls(&mut reg, vec![override_d001]);
        let controls = controls_for_framework(&reg, "aiuc-1");
        assert_eq!(controls.len(), 50, "no duplicate after override");
        assert_eq!(
            controls
                .iter()
                .find(|c| c.control_id == "D001")
                .unwrap()
                .requirement,
            "custom label"
        );
    }

    #[test]
    fn aiuc1_tagged_cases_score_against_the_crosswalk() {
        let mut reg = FrameworkRegistry::new();
        load_bundled_frameworks(&mut reg);
        let cases = vec![
            case("a", CaseStatus::Passed, &["aiuc-1:D001"]),
            case("b", CaseStatus::Passed, &["aiuc-1:D001"]),
            case("c", CaseStatus::Passed, &["aiuc-1:D001", "aiuc-1:B006"]),
        ];
        let runs = vec![run_with(cases, "m")];
        let scores = compute_control_scores_with_registry(&reg, &runs, "aiuc-1", None, None);
        assert_eq!(status_of(&scores, "D001"), ControlCoverageStatus::Satisfied);
        assert_eq!(
            status_of(&scores, "B006"),
            ControlCoverageStatus::Procedural
        );
        assert_eq!(status_of(&scores, "A006"), ControlCoverageStatus::Gap);
    }
}
