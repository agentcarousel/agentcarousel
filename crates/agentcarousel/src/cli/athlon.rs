use clap::{Parser, Subcommand};
use console::style;
use std::collections::HashSet;
use std::path::PathBuf;

use super::athlon_format::{
    athlon_path, framework_key, load_athlon, materialize, materialized_path, save_athlon,
    write_materialized, AthlonDefinition, AthlonError, Block, Event, EvidenceResult, ExternalEvent,
    Goals, LifecycleStage, NativeEvent, SCHEMA_VERSION,
};
use super::athlon_validate::{validate_athlon, Severity};
use super::compliance_mappings::{
    collapse_scores, compute_control_scores_with_registry, controls_for_framework,
    load_framework_registry, ControlCoverageStatus, ControlScore,
};
use super::exit_codes::ExitCode;
use super::fixture_utils::collect_fixture_paths;
use super::metrics::serialize_assessment_results;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;
use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::list_full_runs;

/// Author, validate, run, and report on a TEVV-Athlon assessment (NIST AI 200-2).
///
/// See docs/plans/2026-08-08-tevv-athlon-independent-baseline.md for the design.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc athlon init --slug demo --objective \"...\" --lifecycle-stage deploy\n  agc athlon add-block --slug demo --block-id helpfulness --definition \"...\" --tc \"Valid & Reliable\"\n  agc athlon add-event --slug demo --block-id helpfulness --event-id user-testing --native --evaluator judge\n  agc athlon report --slug demo"
)]
pub struct AthlonArgs {
    #[command(subcommand)]
    command: AthlonCommand,
}

#[derive(Debug, Subcommand)]
enum AthlonCommand {
    /// Stage 1 (Articulate & Organize) — scaffold a new athlon.yaml.
    Init(InitArgs),
    /// Stage 2 (Define & Construct) — declare a Metrology Block.
    AddBlock(AddBlockArgs),
    /// Stage 3 (Apply & Measure) — declare a native or external Event under a Block.
    AddEvent(AddEventArgs),
    /// Referential-integrity check across goals/blocks/events/fixture tags.
    Validate(ValidateArgs),
    /// Execute this athlon's native Events (thin wrapper over `agc eval`).
    Run(RunArgs),
    /// Stage 4 (Synthesize & Interrogate) — render the TEVV-Athlon report.
    Report(ReportArgs),
}

pub fn run_athlon(args: AthlonArgs, globals: &GlobalOptions) -> i32 {
    match args.command {
        AthlonCommand::Init(a) => run_init(a, globals),
        AthlonCommand::AddBlock(a) => run_add_block(a, globals),
        AthlonCommand::AddEvent(a) => run_add_event(a, globals),
        AthlonCommand::Validate(a) => run_validate(a, globals),
        AthlonCommand::Run(a) => run_run(a, globals),
        AthlonCommand::Report(a) => run_report(a, globals),
    }
}

// ── init ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct InitArgs {
    /// Short identifier for this athlon (used in file paths and framework tags).
    #[arg(long)]
    slug: String,
    /// The assessment's objective, in plain language (Stage 1, Question 1).
    #[arg(long)]
    objective: String,
    /// Who cares about the results (Stage 1, Question 2). Repeatable.
    #[arg(long = "stakeholder")]
    stakeholders: Vec<String>,
    /// Where the AI system is in its lifecycle (NIST AI 200-2 Fig. 1).
    #[arg(long, value_enum)]
    lifecycle_stage: LifecycleStageArg,
    /// Approximate timeline/budget (Stage 1, Question 3).
    #[arg(long)]
    cost_and_duration: Option<String>,
    /// What current TEVV techniques this builds on (Stage 1, Question 5).
    #[arg(long)]
    builds_on: Option<String>,
    /// What a successful assessment looks like (Stage 1, Question 6).
    #[arg(long)]
    success_criteria: Option<String>,
    /// Known challenges in this approach (Stage 1, Question 7).
    #[arg(long)]
    challenges: Option<String>,
    /// Overwrite an existing athlon.yaml with the same slug.
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum LifecycleStageArg {
    PlanAndDesign,
    CollectAndProcessData,
    BuildAndUse,
    Deploy,
    OperateAndMonitor,
}

impl From<LifecycleStageArg> for LifecycleStage {
    fn from(v: LifecycleStageArg) -> Self {
        match v {
            LifecycleStageArg::PlanAndDesign => LifecycleStage::PlanAndDesign,
            LifecycleStageArg::CollectAndProcessData => LifecycleStage::CollectAndProcessData,
            LifecycleStageArg::BuildAndUse => LifecycleStage::BuildAndUse,
            LifecycleStageArg::Deploy => LifecycleStage::Deploy,
            LifecycleStageArg::OperateAndMonitor => LifecycleStage::OperateAndMonitor,
        }
    }
}

fn run_init(args: InitArgs, globals: &GlobalOptions) -> i32 {
    let path = athlon_path(&args.slug);
    if path.exists() && !args.force {
        let msg = format!(
            "athlon '{}' already exists at {} (pass --force to overwrite)",
            args.slug,
            path.display()
        );
        return fail(globals, "already_exists", &msg, ExitCode::RuntimeError);
    }

    let def = AthlonDefinition {
        schema_version: SCHEMA_VERSION,
        slug: args.slug.clone(),
        goals: Goals {
            objective: args.objective,
            stakeholders: args.stakeholders,
            lifecycle_stage: args.lifecycle_stage.into(),
            cost_and_duration: args.cost_and_duration,
            builds_on: args.builds_on,
            success_criteria: args.success_criteria,
            challenges: args.challenges,
        },
        blocks: Vec::new(),
    };

    if let Err(e) = save_athlon(&def) {
        return fail(
            globals,
            "write_error",
            &e.to_string(),
            ExitCode::RuntimeError,
        );
    }

    if globals.json {
        JsonOutput::ok(
            "athlon",
            serde_json::json!({ "slug": args.slug, "path": path.display().to_string() }),
        )
        .print();
    } else {
        println!(
            "{} wrote {}",
            style("✓").green(),
            style(path.display()).cyan()
        );
        println!(
            "  next: {}",
            style(format!(
                "agc athlon add-block --slug {} --block-id <id> --definition <text> --tc <characteristic>",
                args.slug
            ))
            .dim()
        );
    }
    ExitCode::Ok.as_i32()
}

// ── add-block ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct AddBlockArgs {
    #[arg(long)]
    slug: String,
    #[arg(long)]
    block_id: String,
    #[arg(long)]
    definition: String,
    /// Trustworthiness Characteristic this Block addresses. Repeatable.
    /// Soft-validated only — see Adversarial #6 in the design plan.
    #[arg(long = "tc")]
    trustworthiness_characteristics: Vec<String>,
}

fn run_add_block(args: AddBlockArgs, globals: &GlobalOptions) -> i32 {
    let mut def = match load_athlon(&args.slug) {
        Ok(d) => d,
        Err(e) => return fail(globals, "not_found", &e.to_string(), ExitCode::NotFound),
    };

    if def.blocks.iter().any(|b| b.id == args.block_id) {
        let err = AthlonError::AlreadyExists(args.block_id.clone(), athlon_path(&args.slug));
        return fail(
            globals,
            "already_exists",
            &err.to_string(),
            ExitCode::RuntimeError,
        );
    }

    def.blocks.push(Block {
        id: args.block_id.clone(),
        trustworthiness_characteristics: args.trustworthiness_characteristics,
        definition: args.definition,
        events: Vec::new(),
    });

    if let Err(e) = save_athlon(&def) {
        return fail(
            globals,
            "write_error",
            &e.to_string(),
            ExitCode::RuntimeError,
        );
    }
    // Re-materialize so `add-block` alone (no Events yet) keeps the registry in
    // sync — a Block with zero native Events materializes to zero controls,
    // which is correct: nothing to score until an Event is added.
    if let Err(e) = write_materialized(&def) {
        return fail(
            globals,
            "materialize_error",
            &e.to_string(),
            ExitCode::RuntimeError,
        );
    }

    if globals.json {
        JsonOutput::ok(
            "athlon",
            serde_json::json!({ "slug": args.slug, "block_id": args.block_id }),
        )
        .print();
    } else {
        println!(
            "{} added block {} to {}",
            style("✓").green(),
            style(&args.block_id).bold(),
            style(athlon_path(&args.slug).display()).cyan()
        );
    }
    ExitCode::Ok.as_i32()
}

// ── add-event ────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(group(
    clap::ArgGroup::new("kind").args(["native", "external"]).required(true)
))]
struct AddEventArgs {
    #[arg(long)]
    slug: String,
    #[arg(long)]
    block_id: String,
    #[arg(long)]
    event_id: String,

    /// Declare this Event as native (agc executes it).
    #[arg(long)]
    native: bool,
    /// Which evaluator this native Event uses.
    #[arg(long, requires = "native")]
    evaluator: Option<String>,

    /// Declare this Event as external (evidence attached by reference).
    #[arg(long)]
    external: bool,
    /// External tool name (e.g. "garak"). Required with --external.
    #[arg(long, requires = "external")]
    tool: Option<String>,
    /// Human-readable description of the external Event.
    #[arg(long, requires = "external")]
    description: Option<String>,
    /// Path to the evidence artifact.
    #[arg(long, requires = "external")]
    evidence_path: Option<String>,
    /// Free-text summary of the external evidence.
    #[arg(long, requires = "external")]
    summary: Option<String>,
    /// pass | fail | inconclusive — advisory only, see Adversarial #5.
    #[arg(long, requires = "external", value_enum)]
    result: Option<EvidenceResultArg>,
    #[arg(long, requires = "external")]
    assessed_by: Option<String>,
    #[arg(long, requires = "external")]
    date: Option<String>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum EvidenceResultArg {
    Pass,
    Fail,
    Inconclusive,
}

impl From<EvidenceResultArg> for EvidenceResult {
    fn from(v: EvidenceResultArg) -> Self {
        match v {
            EvidenceResultArg::Pass => EvidenceResult::Pass,
            EvidenceResultArg::Fail => EvidenceResult::Fail,
            EvidenceResultArg::Inconclusive => EvidenceResult::Inconclusive,
        }
    }
}

fn run_add_event(args: AddEventArgs, globals: &GlobalOptions) -> i32 {
    let mut def = match load_athlon(&args.slug) {
        Ok(d) => d,
        Err(e) => return fail(globals, "not_found", &e.to_string(), ExitCode::NotFound),
    };

    let Some(block) = def.blocks.iter_mut().find(|b| b.id == args.block_id) else {
        let err = AthlonError::BlockNotFound(args.block_id.clone(), args.slug.clone());
        let msg = format!(
            "{err}. Run `agc athlon add-block --slug {} --block-id {}` first.",
            args.slug, args.block_id
        );
        return fail(globals, "not_found", &msg, ExitCode::NotFound);
    };

    let event = if args.native {
        let Some(evaluator) = args.evaluator else {
            return fail(
                globals,
                "invalid_args",
                "--native requires --evaluator",
                ExitCode::RuntimeError,
            );
        };
        Event {
            id: args.event_id.clone(),
            native: Some(NativeEvent {
                evaluator,
                fixture_tag: format!(
                    "tevv-athlon:{}:{}:{}",
                    args.slug, args.block_id, args.event_id
                ),
            }),
            external: None,
        }
    } else {
        let (Some(tool), Some(description), Some(evidence_path), Some(result)) =
            (args.tool, args.description, args.evidence_path, args.result)
        else {
            return fail(
                globals,
                "invalid_args",
                "--external requires --tool, --description, --evidence-path, and --result",
                ExitCode::RuntimeError,
            );
        };
        Event {
            id: args.event_id.clone(),
            native: None,
            external: Some(ExternalEvent {
                tool,
                description,
                evidence_path,
                summary: args.summary,
                assessed_by: args.assessed_by,
                date: args.date,
                result: result.into(),
            }),
        }
    };

    // Upsert on --event-id, per plan §2.4.
    if let Some(existing) = block.events.iter_mut().find(|e| e.id == args.event_id) {
        *existing = event;
    } else {
        block.events.push(event);
    }

    if let Err(e) = save_athlon(&def) {
        return fail(
            globals,
            "write_error",
            &e.to_string(),
            ExitCode::RuntimeError,
        );
    }
    if let Err(e) = write_materialized(&def) {
        return fail(
            globals,
            "materialize_error",
            &e.to_string(),
            ExitCode::RuntimeError,
        );
    }

    if globals.json {
        JsonOutput::ok(
            "athlon",
            serde_json::json!({
                "slug": args.slug,
                "block_id": args.block_id,
                "event_id": args.event_id,
                "materialized_path": materialized_path(&args.slug).display().to_string(),
            }),
        )
        .print();
    } else {
        println!(
            "{} added event {} to block {} in {}",
            style("✓").green(),
            style(&args.event_id).bold(),
            style(&args.block_id).bold(),
            style(athlon_path(&args.slug).display()).cyan()
        );
    }
    ExitCode::Ok.as_i32()
}

// ── validate ─────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct ValidateArgs {
    #[arg(long)]
    slug: String,
}

/// Scan every fixture file in the project (mirroring `agc validate`'s own
/// discovery) and collect each case's tag list. Cases that fail to parse are
/// silently skipped here — `agc validate`/`agc test` are the tools responsible
/// for surfacing malformed fixtures; this is a read-only tag inventory.
fn collect_all_case_tags() -> Vec<Vec<String>> {
    let mut all_tags = Vec::new();
    for path in collect_fixture_paths(&[PathBuf::from(".")]) {
        if let Ok(fixture) = load_fixture(&path) {
            for case in fixture.cases {
                all_tags.push(case.tags);
            }
        }
    }
    all_tags
}

fn run_validate(args: ValidateArgs, globals: &GlobalOptions) -> i32 {
    let def = match load_athlon(&args.slug) {
        Ok(d) => d,
        Err(e) => return fail(globals, "not_found", &e.to_string(), ExitCode::NotFound),
    };

    let all_case_tags = collect_all_case_tags();
    let violations = validate_athlon(&def, &all_case_tags);
    let has_errors = violations.iter().any(|v| v.severity == Severity::Error);

    if globals.json {
        let ok = !has_errors;
        let data = serde_json::json!({ "slug": args.slug, "violations": violations });
        if ok {
            JsonOutput::ok("athlon", data).print();
        } else {
            JsonOutput {
                ok: false,
                command: "athlon",
                data: Some(data),
                error: Some(JsonError::new(
                    "validation_failed",
                    format!("{} violation(s) found", violations.len()),
                )),
            }
            .print();
        }
    } else if violations.is_empty() {
        println!(
            "{} athlon '{}' — no violations found",
            style("✓").green(),
            args.slug
        );
    } else {
        println!(
            "  {}",
            style(format!("Athlon Validation — {}", args.slug)).bold()
        );
        println!("  {}", "─".repeat(70));
        for v in &violations {
            let marker = match v.severity {
                Severity::Error => style("❌").red(),
                Severity::Warning => style("⚠").yellow(),
            };
            println!("  {marker} [check {}] {}", v.check, v.message);
        }
        println!();
    }

    if has_errors {
        ExitCode::ValidationFailed.as_i32()
    } else {
        ExitCode::Ok.as_i32()
    }
}

// ── run ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct RunArgs {
    #[arg(long)]
    slug: String,
    /// Only run this Event's fixture tag (default: every native Event).
    #[arg(long)]
    event: Option<String>,
    /// Extra arguments forwarded verbatim to `agc eval` (e.g. --model, --judge, -x live).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    eval_args: Vec<String>,
}

/// Adds zero new scoring/execution logic — collects the native Events' fixture
/// tags and delegates to the existing `agc eval` execution path via
/// `--filter-tags`, exactly as the design plan's Core Principle #6 requires.
fn run_run(args: RunArgs, globals: &GlobalOptions) -> i32 {
    let def = match load_athlon(&args.slug) {
        Ok(d) => d,
        Err(e) => return fail(globals, "not_found", &e.to_string(), ExitCode::NotFound),
    };

    let all_case_tags = collect_all_case_tags();
    let violations = validate_athlon(&def, &all_case_tags);
    let error_count = violations
        .iter()
        .filter(|v| v.severity == Severity::Error)
        .count();
    if error_count > 0 {
        let msg = format!(
            "athlon '{}' has {error_count} validation error(s) — run \
             `agc athlon validate --slug {}` for details",
            args.slug, args.slug
        );
        return fail(
            globals,
            "validation_failed",
            &msg,
            ExitCode::ValidationFailed,
        );
    }

    let mut tags = Vec::new();
    let mut external_count = 0usize;
    for block in &def.blocks {
        for event in &block.events {
            if let Some(filter) = &args.event {
                if &event.id != filter {
                    continue;
                }
            }
            if let Some(native) = &event.native {
                tags.push(native.fixture_tag.clone());
            } else if event.external.is_some() {
                external_count += 1;
            }
        }
    }

    if tags.is_empty() {
        return fail(
            globals,
            "nothing_to_run",
            "no native Events to run — external Events are never executed by agc; \
             attach their evidence via `agc athlon add-event --external`",
            ExitCode::RuntimeError,
        );
    }

    if external_count > 0 && !globals.quiet && !globals.json {
        eprintln!("note: skipping {external_count} external Event(s) — agc never executes them.");
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("could not locate current executable: {e}");
            return fail(globals, "runtime_error", &msg, ExitCode::RuntimeError);
        }
    };

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("eval").arg("--filter-tags").arg(tags.join(","));
    if globals.json {
        cmd.arg("--json");
    }
    if globals.quiet {
        cmd.arg("--quiet");
    }
    cmd.args(&args.eval_args);

    match cmd.status() {
        Ok(status) => status.code().unwrap_or(ExitCode::RuntimeError.as_i32()),
        Err(e) => {
            let msg = format!("failed to invoke `agc eval`: {e}");
            fail(globals, "runtime_error", &msg, ExitCode::RuntimeError)
        }
    }
}

// ── report ───────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct ReportArgs {
    #[arg(long)]
    slug: String,
    /// Emit OSCAL Assessment Results JSON instead of Markdown.
    #[arg(long)]
    oscal: bool,
    /// Write output to this path instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Number of historical runs to analyze.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

/// A Block/native-Event is "stale" if `athlon.yaml` declares it but the
/// materialized registry file doesn't have a matching control — e.g. the
/// materialized JSON was hand-edited or deleted after the fact. Reporting
/// against a stale registry would silently under-report instead of failing
/// loudly, which the design plan's Operability section explicitly forbids.
fn stale_materialization(def: &AthlonDefinition) -> Vec<String> {
    let expected = materialize(def);
    let registry = load_framework_registry();
    let framework = framework_key(&def.slug);
    let actual: HashSet<&str> = controls_for_framework(&registry, &framework)
        .iter()
        .map(|c| c.control_id.as_str())
        .collect();
    expected
        .iter()
        .map(|c| c.control_id.as_str())
        .filter(|id| !actual.contains(id))
        .map(str::to_string)
        .collect()
}

fn lifecycle_label(stage: LifecycleStage) -> &'static str {
    match stage {
        LifecycleStage::PlanAndDesign => "Plan & Design",
        LifecycleStage::CollectAndProcessData => "Collect & Process Data",
        LifecycleStage::BuildAndUse => "Build & Use",
        LifecycleStage::Deploy => "Deploy",
        LifecycleStage::OperateAndMonitor => "Operate & Monitor",
    }
}

fn score_and_status_cells(score: Option<&ControlScore>) -> (String, String) {
    match score {
        None => ("—".to_string(), "❌ Gap".to_string()),
        Some(s) => {
            let score_str = format!("{:.0}%", s.effectiveness_mean * 100.0);
            let status_str = match s.status {
                ControlCoverageStatus::Satisfied => "✅ Satisfied",
                ControlCoverageStatus::PartialEvidence => "⚠ Partial",
                ControlCoverageStatus::Failed => "❌ Failed",
                ControlCoverageStatus::Gap => "❌ Gap",
                ControlCoverageStatus::Procedural => "📋 Procedural",
            }
            .to_string();
            (score_str, status_str)
        }
    }
}

/// Renders the TEVV-Athlon-shaped report (design plan §2.5): per-Event rows
/// first (native Events scored individually, external Events shown with a
/// dash and a citations pointer), then a bolded joint per-Block row using the
/// two-level pooled `ControlScore` — matching NIST's own Table 2 / Fig. 4.
fn render_athlon_report_markdown(def: &AthlonDefinition, scores: &[ControlScore]) -> String {
    use std::fmt::Write as _;
    let collapsed = collapse_scores(scores);
    let mut md = String::new();

    let _ = writeln!(md, "## TEVV-Athlon Report — {}", def.slug);
    let _ = writeln!(md);
    let _ = writeln!(md, "**Goal:** {}", def.goals.objective);
    let stakeholders = if def.goals.stakeholders.is_empty() {
        "—".to_string()
    } else {
        def.goals.stakeholders.join(", ")
    };
    let _ = writeln!(
        md,
        "**Lifecycle stage:** {} · **Stakeholders:** {stakeholders}",
        lifecycle_label(def.goals.lifecycle_stage)
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "| TC | Block | Event | Toolbox | Score | Status |");
    let _ = writeln!(md, "|----|-------|-------|---------|-------|--------|");

    let mut evidence_lines: Vec<String> = Vec::new();

    for block in &def.blocks {
        let tc_label = if block.trustworthiness_characteristics.is_empty() {
            "—".to_string()
        } else {
            block.trustworthiness_characteristics.join(", ")
        };

        for event in &block.events {
            if let Some(native) = &event.native {
                let score = collapsed
                    .iter()
                    .find(|s| s.control.tag == native.fixture_tag);
                let (score_str, status_str) = score_and_status_cells(score);
                let _ = writeln!(
                    md,
                    "| {tc_label} | {} | {} | {} (native) | {score_str} | {status_str} |",
                    block.id, event.id, native.evaluator
                );
            } else if let Some(external) = &event.external {
                let disputed = matches!(external.result, EvidenceResult::Fail);
                let status_cell = if disputed {
                    "⚠ external evidence disputes this score"
                } else {
                    "—"
                };
                let _ = writeln!(
                    md,
                    "| {tc_label} | {} | {} | {} (external, see evidence) | — | {status_cell} |",
                    block.id, event.id, external.tool
                );
                let date = external
                    .date
                    .as_deref()
                    .map(|d| format!(", {d}"))
                    .unwrap_or_default();
                let assessed_by = external
                    .assessed_by
                    .as_deref()
                    .map(|a| format!(", assessed by {a}"))
                    .unwrap_or_default();
                let summary = external.summary.as_deref().unwrap_or(&external.description);
                let result_label = match external.result {
                    EvidenceResult::Pass => "pass",
                    EvidenceResult::Fail => "fail",
                    EvidenceResult::Inconclusive => "inconclusive",
                };
                evidence_lines.push(format!(
                    "- **{}** / {} — {}{date}: {summary}{assessed_by} — *{result_label}* (advisory)",
                    block.id, event.id, external.tool
                ));
            }
        }

        let two_level_tag = format!("tevv-athlon:{}:{}", def.slug, block.id);
        let joint_score = collapsed.iter().find(|s| s.control.tag == two_level_tag);
        if joint_score.is_some() {
            let (score_str, status_str) = score_and_status_cells(joint_score);
            let _ = writeln!(
                md,
                "| {tc_label} | **{} (joint, all Events)** | — | — | {score_str} | {status_str} |",
                block.id
            );
        }
    }

    if !evidence_lines.is_empty() {
        let _ = writeln!(md);
        let _ = writeln!(md, "### External evidence");
        for line in evidence_lines {
            let _ = writeln!(md, "{line}");
        }
    }

    md
}

fn run_report(args: ReportArgs, globals: &GlobalOptions) -> i32 {
    let def = match load_athlon(&args.slug) {
        Ok(d) => d,
        Err(e) => return fail(globals, "not_found", &e.to_string(), ExitCode::NotFound),
    };

    let missing = stale_materialization(&def);
    if !missing.is_empty() {
        let msg = format!(
            "materialized registry ({}) is missing control(s) {:?} that athlons/{}.yaml \
             declares — re-run `agc athlon add-event` to repair, or check for a corrupted \
             or manually-deleted materialized file",
            materialized_path(&args.slug).display(),
            missing,
            args.slug
        );
        return fail(
            globals,
            "stale_materialization",
            &msg,
            ExitCode::ValidationFailed,
        );
    }

    let runs = match list_full_runs(args.limit) {
        Ok(r) => r,
        Err(e) => {
            return fail(
                globals,
                "history_error",
                &e.to_string(),
                ExitCode::RuntimeError,
            )
        }
    };

    let registry = load_framework_registry();
    let framework = framework_key(&args.slug);
    let scores = compute_control_scores_with_registry(&registry, &runs, &framework, None, None);

    if args.oscal {
        let run_id = runs
            .first()
            .map(|r| r.id.0.clone())
            .unwrap_or_else(|| "latest".to_string());
        let content = serialize_assessment_results(&scores, &framework, None, &run_id, &runs);
        return write_report_output(args.out.as_deref(), &content);
    }

    // `--out` is an explicit artifact request and wins over the JSON envelope,
    // matching `agc compliance report`'s convention (compliance.rs).
    if globals.json && args.out.is_none() {
        JsonOutput::ok(
            "athlon",
            serde_json::json!({ "slug": args.slug, "control_scores": scores }),
        )
        .print();
        return ExitCode::Ok.as_i32();
    }

    let md = render_athlon_report_markdown(&def, &scores);
    write_report_output(args.out.as_deref(), &md)
}

fn write_report_output(out: Option<&std::path::Path>, content: &str) -> i32 {
    match out {
        Some(path) => match std::fs::write(path, content) {
            Ok(()) => {
                println!("wrote {}", path.display());
                ExitCode::Ok.as_i32()
            }
            Err(e) => {
                eprintln!("error writing {}: {e}", path.display());
                ExitCode::RuntimeError.as_i32()
            }
        },
        None => {
            print!("{content}");
            ExitCode::Ok.as_i32()
        }
    }
}

// ── shared helpers ───────────────────────────────────────────────────────────

fn fail(globals: &GlobalOptions, code: &'static str, msg: &str, exit: ExitCode) -> i32 {
    if globals.json {
        JsonOutput::err("athlon", JsonError::new(code, msg)).print();
    } else {
        eprintln!("error: {msg}");
    }
    exit.as_i32()
}
