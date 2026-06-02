use agentcarousel_reporters::{list_full_runs, list_full_runs_by_skill};
use clap::{Parser, Subcommand};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use crate::fixtures::{validate_fixture_value, SchemaLocation};
use crate::runner::call_llm;

use super::compliance_mappings::{
    collapse_scores, compute_control_scores, compute_control_scores_with_registry,
    load_framework_registry, ControlCoverageStatus,
};
use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::llm_output::{normalize_expected_block, prepare_llm_yaml};
use super::metrics::{render_framework_compliance_report, serialize_assessment_results};
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const COMPLIANCE_GENERATE_PROMPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/compliance-generate-prompt.md"
));

const ALL_FRAMEWORKS: &[&str] = &[
    "nist-ai-rmf",
    "eu-ai-act",
    "iso-42001",
    "hipaa",
    "fda-samd",
    "nist-800-171",
    "nist-800-172",
    "nist-800-207",
];

/// Generate compliance attestation reports and gap advisories from run history.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc compliance report --framework nist-ai-rmf\n  agc compliance report --framework all --skill my-agent --out ./reports/\n  agc compliance report --framework hipaa --oscal > hipaa.oscal.json\n  agc compliance gaps --framework eu-ai-act"
)]
pub struct ComplianceArgs {
    #[command(subcommand)]
    command: ComplianceCommand,
}

#[derive(Debug, Subcommand)]
enum ComplianceCommand {
    /// Render a per-control compliance attestation report (Markdown or OSCAL JSON).
    Report(ReportArgs),
    /// List controls with no fixture coverage and print remediation advisories.
    Gaps(GapsArgs),
    /// Generate fixture cases for a skill that provide behavioral evidence for a compliance control.
    Generate(GenerateComplianceArgs),
}

#[derive(Debug, Parser)]
struct GenerateComplianceArgs {
    /// Skill name. Cases are written to fixtures/<skill>/cases.yaml.
    #[arg(long)]
    skill: String,

    /// Compliance tag(s) to generate cases for (e.g. nist-800-171:3.1.1, hipaa:164.308.a.1). Repeatable.
    #[arg(long, required = true)]
    tag: Vec<String>,

    /// Cases to generate per tag.
    #[arg(long, default_value_t = 3)]
    count: u32,

    /// LLM model to use for generation (default: configured generator model).
    #[arg(long)]
    model: Option<String>,

    /// Base URL for a custom/Ollama generator endpoint.
    #[arg(long, value_name = "URL")]
    generator_endpoint: Option<String>,

    /// Write output to this path instead of fixtures/<skill>/cases.yaml.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Print YAML to stdout instead of writing to disk.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Parser)]
struct ReportArgs {
    /// Framework to report on. Use "all" to run all embedded frameworks.
    /// Available: nist-ai-rmf, eu-ai-act, iso-42001, hipaa, fda-samd,
    ///            nist-800-171, nist-800-172, nist-800-207
    #[arg(long, default_value = "nist-ai-rmf")]
    framework: String,

    /// Filter run history to a specific skill or agent name.
    #[arg(long)]
    skill: Option<String>,

    /// Number of historical runs to analyze (default: 20).
    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Write output to this path instead of stdout.
    /// When --framework all, treated as a directory and one file is written per framework.
    #[arg(long)]
    out: Option<PathBuf>,

    /// Emit OSCAL Assessment Results JSON instead of Markdown.
    /// Not valid with --framework all.
    #[arg(long)]
    oscal: bool,

    /// Run ID to use as the evidence anchor in OSCAL output (default: "latest").
    #[arg(long, default_value = "latest")]
    run_id: String,

    /// Skip per-model filtering and include all models in history.
    /// Useful for historical audit reviews when you want the full multi-model picture.
    #[arg(long)]
    all_models: bool,
}

#[derive(Debug, Parser)]
struct GapsArgs {
    /// Framework to check for gaps.
    #[arg(long, default_value = "nist-ai-rmf")]
    framework: String,

    /// Filter run history to a specific skill or agent name.
    #[arg(long)]
    skill: Option<String>,

    /// Number of historical runs to analyze (default: 20).
    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Skip per-model filtering and include all models in history.
    #[arg(long)]
    all_models: bool,
}

pub fn run_compliance(
    args: ComplianceArgs,
    globals: &GlobalOptions,
    config: &ResolvedConfig,
) -> i32 {
    let configured_model = config.generator.model.as_str();
    match args.command {
        ComplianceCommand::Report(a) => {
            let mf = (!a.all_models).then_some(configured_model);
            run_report(a, globals, mf)
        }
        ComplianceCommand::Gaps(a) => {
            let mf = (!a.all_models).then_some(configured_model);
            run_gaps(a, globals, mf)
        }
        ComplianceCommand::Generate(a) => run_compliance_generate(a, globals, config),
    }
}

fn load_runs(
    skill: Option<&str>,
    limit: usize,
    globals: &GlobalOptions,
) -> Result<Vec<agentcarousel_core::Run>, i32> {
    let result = match skill {
        Some(s) => list_full_runs_by_skill(s, limit),
        None => list_full_runs(limit),
    };
    result.map_err(|e| {
        if globals.json {
            JsonOutput::err("compliance", JsonError::new("history_error", e.to_string())).print();
        } else {
            eprintln!("error reading run history: {e}");
        }
        ExitCode::RuntimeError.as_i32()
    })
}

fn filter_label(model_filter: Option<&str>, skill: Option<&str>) -> String {
    match (skill, model_filter) {
        (Some(s), Some(m)) => format!("{s} · {m}"),
        (Some(s), None) => s.to_string(),
        (None, Some(m)) => m.to_string(),
        (None, None) => "all skills".to_string(),
    }
}

/// When a model filter is active, verify that at least one loaded run matches it.
/// Returns `Err(exit_code)` with a human-readable error (or JSON error) if not.
fn check_model_coverage(
    runs: &[agentcarousel_core::Run],
    model_filter: Option<&str>,
    globals: &GlobalOptions,
) -> Result<(), i32> {
    let Some(model) = model_filter else {
        return Ok(());
    };

    let has_match = runs.iter().any(|r| {
        r.summary
            .generator_model
            .as_deref()
            .is_some_and(|m| m == model)
    });

    if has_match {
        return Ok(());
    }

    // Collect what models ARE present so we can suggest them (sorted, deduplicated).
    let present: Vec<String> = runs
        .iter()
        .filter_map(|r| r.summary.generator_model.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    if globals.json {
        let hint = if runs.is_empty() {
            format!("Run `agc eval --model {model}` to generate run data.")
        } else {
            format!(
                "Run `agc eval --model {model}` or use one of the models already in history: {}",
                present.join(", ")
            )
        };
        let data = serde_json::json!({
            "model": model,
            "models_in_history": present,
            "hint": hint,
        });
        JsonOutput {
            ok: false,
            command: "compliance",
            data: Some(data),
            error: Some(JsonError::new(
                "no_runs_for_model",
                format!("no runs found for model '{model}'"),
            )),
        }
        .print();
    } else if runs.is_empty() {
        eprintln!("error: no run history found. Run `agc eval --model {model}` to generate data.");
    } else {
        eprintln!("error: no runs found for model '{model}'.");
        if present.is_empty() {
            eprintln!("       Run `agc eval --model {model}` to generate data for this model.");
        } else {
            eprintln!("       Models in history: {}", present.join(", "));
            eprintln!(
                "       Run `agc eval --model {model}` or set a matching model in your config."
            );
        }
    }
    Err(ExitCode::NotFound.as_i32())
}

fn run_report(args: ReportArgs, globals: &GlobalOptions, model_filter: Option<&str>) -> i32 {
    if args.oscal && args.framework == "all" {
        if globals.json {
            JsonOutput::err(
                "compliance",
                JsonError::new("invalid_args", "--oscal is not valid with --framework all"),
            )
            .print();
        } else {
            eprintln!("error: --oscal is not valid with --framework all");
        }
        return ExitCode::RuntimeError.as_i32();
    }

    if args.framework == "all" && args.out.is_none() && !globals.json {
        eprintln!(
            "error: --framework all writes one file per framework. \
             Pass --out <dir> to specify the output directory."
        );
        return ExitCode::RuntimeError.as_i32();
    }

    let runs = match load_runs(args.skill.as_deref(), args.limit, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };
    if let Err(code) = check_model_coverage(&runs, model_filter, globals) {
        return code;
    }

    let frameworks: Vec<&str> = if args.framework == "all" {
        ALL_FRAMEWORKS.to_vec()
    } else {
        vec![args.framework.as_str()]
    };

    let registry = load_framework_registry();

    if globals.json {
        let mut results = Vec::new();
        for fw in &frameworks {
            let scores = compute_control_scores_with_registry(
                &registry,
                &runs,
                fw,
                args.skill.as_deref(),
                model_filter,
            );
            results.push(serde_json::json!({
                "framework": fw,
                "skill": args.skill,
                "model": model_filter,
                "control_scores": scores,
            }));
        }
        JsonOutput::ok("compliance", serde_json::json!({ "reports": results })).print();
        return ExitCode::Ok.as_i32();
    }

    if args.framework == "all" {
        let out_dir = args.out.clone().unwrap();
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("error creating output directory: {e}");
            return ExitCode::RuntimeError.as_i32();
        }
        for fw in &frameworks {
            let scores = compute_control_scores_with_registry(
                &registry,
                &runs,
                fw,
                args.skill.as_deref(),
                model_filter,
            );
            let md = render_framework_compliance_report(&scores, fw, args.skill.as_deref());
            let path = out_dir.join(format!("compliance_{fw}.md"));
            if let Err(e) = std::fs::write(&path, &md) {
                eprintln!("error writing {}: {e}", path.display());
                return ExitCode::RuntimeError.as_i32();
            }
            println!("wrote {}", path.display());
        }
        return ExitCode::Ok.as_i32();
    }

    let fw = args.framework.as_str();
    let scores = compute_control_scores_with_registry(
        &registry,
        &runs,
        fw,
        args.skill.as_deref(),
        model_filter,
    );

    if args.oscal {
        let resolved_run_id: String = if args.run_id == "latest" {
            runs.first()
                .map(|r| r.id.0.clone())
                .unwrap_or_else(|| "latest".to_string())
        } else {
            args.run_id.clone()
        };
        let content = serialize_assessment_results(
            &scores,
            fw,
            args.skill.as_deref(),
            &resolved_run_id,
            &runs,
        );
        return match args.out {
            Some(path) => write_output(&path, &content),
            None => {
                print!("{content}");
                ExitCode::Ok.as_i32()
            }
        };
    }

    match args.out {
        Some(path) => {
            let md = render_framework_compliance_report(&scores, fw, args.skill.as_deref());
            write_output(&path, &md)
        }
        None => {
            print_compliance_terminal(&scores, fw, args.skill.as_deref(), model_filter);
            ExitCode::Ok.as_i32()
        }
    }
}

fn write_output(path: &std::path::Path, content: &str) -> i32 {
    if let Err(e) = std::fs::write(path, content) {
        eprintln!("error writing {}: {e}", path.display());
        ExitCode::RuntimeError.as_i32()
    } else {
        println!("wrote {}", path.display());
        ExitCode::Ok.as_i32()
    }
}

fn print_compliance_terminal(
    scores: &[super::compliance_mappings::ControlScore],
    framework: &str,
    skill: Option<&str>,
    model_filter: Option<&str>,
) {
    let collapsed = collapse_scores(scores);
    let skill_label = filter_label(model_filter, skill);

    let mut satisfied = 0usize;
    let mut partial = 0usize;
    let mut failed = 0usize;
    let mut gap = 0usize;
    let mut procedural = 0usize;
    for s in &collapsed {
        match s.status {
            ControlCoverageStatus::Satisfied => satisfied += 1,
            ControlCoverageStatus::PartialEvidence => partial += 1,
            ControlCoverageStatus::Failed => failed += 1,
            ControlCoverageStatus::Gap => gap += 1,
            ControlCoverageStatus::Procedural => procedural += 1,
        }
    }
    let total = collapsed.len();

    println!();
    println!(
        "  {}",
        style(format!("Compliance Report — {framework}")).bold()
    );
    println!("  {}", "─".repeat(70));
    println!(
        "  Skill: {}  ·  {} controls",
        style(skill_label).cyan(),
        total
    );
    println!(
        "  {}  {}  {}  {}  {}",
        style(format!("✅ {satisfied} satisfied")).green(),
        style(format!("⚠  {partial} partial")).yellow(),
        style(format!("❌ {failed} failed")).red(),
        style(format!("❌ {gap} gap")).red(),
        style(format!("📋 {procedural} procedural")).dim(),
    );
    println!();

    let covered: Vec<&super::compliance_mappings::ControlScore> = collapsed
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                ControlCoverageStatus::Satisfied
                    | ControlCoverageStatus::PartialEvidence
                    | ControlCoverageStatus::Failed
            )
        })
        .collect();

    let procedural_list: Vec<&super::compliance_mappings::ControlScore> = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Procedural)
        .collect();

    if covered.is_empty() && procedural_list.is_empty() {
        println!("  {}", style("No behavioral evidence yet.").yellow().bold());
        println!(
            "  Tag fixture cases with  {}  to link test results to controls.",
            style(format!("{framework}:<control-id>")).cyan()
        );
        println!(
            "  Then run  {}  to generate cases for specific controls.",
            style("agc compliance generate --skill <skill> --tag <tag>").dim()
        );
    } else {
        println!(
            "  {:<32} {:<8} {:<6} {}",
            style("CONTROL").dim().bold(),
            style("SCORE").dim().bold(),
            style("CASES").dim().bold(),
            style("STATUS").dim().bold(),
        );
        println!("  {}", "─".repeat(70));

        for s in &covered {
            let score_str = format!("{:.0}%", s.effectiveness_mean * 100.0);
            let status_str = match s.status {
                ControlCoverageStatus::Satisfied => style("✅ Satisfied").green().to_string(),
                ControlCoverageStatus::Failed => style("❌ Failed").red().to_string(),
                _ => style("⚠  Partial").yellow().to_string(),
            };
            println!(
                "  {:<32} {:<8} {:<6} {}",
                truncate_chars(&s.control.control_id, 31),
                score_str,
                s.case_count,
                status_str,
            );
            for line in wrap_text(&s.control.requirement, 62) {
                println!("     {}", style(line).dim());
            }
        }

        for s in &procedural_list {
            println!(
                "  {:<32} {:<8} {:<6} {}",
                truncate_chars(&s.control.control_id, 31),
                "n/a",
                "—",
                style("📋 Procedural").dim(),
            );
        }

        println!("  {}", "─".repeat(70));
    }

    if gap > 0 {
        println!();
        println!(
            "  {}  controls have no fixture coverage.",
            style(format!("❌ {gap}")).red().bold()
        );
        println!(
            "  Run  {}  to see them and get remediation hints.",
            style(format!("agc compliance gaps --framework {framework}")).dim()
        );
    }
    println!();
}

fn run_gaps(args: GapsArgs, globals: &GlobalOptions, model_filter: Option<&str>) -> i32 {
    let runs = match load_runs(args.skill.as_deref(), args.limit, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };
    if let Err(code) = check_model_coverage(&runs, model_filter, globals) {
        return code;
    }

    let fw = args.framework.as_str();
    let scores = compute_control_scores(&runs, fw, args.skill.as_deref(), model_filter);
    let collapsed = collapse_scores(&scores);
    let gaps: Vec<_> = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Gap)
        .collect();

    if globals.json {
        JsonOutput::ok(
            "compliance",
            serde_json::json!({
                "framework": fw,
                "skill": args.skill,
                "model": model_filter,
                "gap_count": gaps.len(),
                "gaps": gaps,
            }),
        )
        .print();
        return ExitCode::Ok.as_i32();
    }

    let label = filter_label(model_filter, args.skill.as_deref());
    println!();
    println!(
        "  {}",
        style(format!("Compliance Gaps — {fw} · {label}")).bold()
    );
    println!("  {}", "─".repeat(70));

    if gaps.is_empty() {
        println!(
            "  {} No gaps — all controls have fixture coverage.",
            style("✅").green()
        );
        println!();
        return ExitCode::Ok.as_i32();
    }

    println!(
        "  {} controls need fixture cases. Run the scaffold command shown to generate them.",
        style(format!("❌ {}", gaps.len())).red().bold()
    );
    println!();

    for s in &gaps {
        println!(
            "  {} {}",
            style("❌").red(),
            style(&s.control.control_id).bold()
        );
        // Print the full requirement, word-wrapped at 70 chars
        for line in wrap_text(&s.control.requirement, 66) {
            println!("     {line}");
        }
        println!(
            "     {}",
            style(format!(
                "agc compliance generate --skill <skill> --tag {}",
                s.control.tag
            ))
            .dim()
        );
        println!();
    }

    ExitCode::Ok.as_i32()
}

/// Truncate `s` to at most `max` Unicode scalar values, staying on char boundaries.
fn truncate_chars(s: &str, max: usize) -> &str {
    s.char_indices().nth(max).map_or(s, |(i, _)| &s[..i])
}

/// Wrap `text` to lines of at most `width` chars, breaking at word boundaries.
/// Paragraph breaks (`\n\n`) are preserved as blank lines in the output.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, para) in text.split("\n\n").enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        let mut current = String::new();
        for word in para.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current.clone());
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}

// ── agc compliance generate ───────────────────────────────────────────────────

fn run_compliance_generate(
    args: GenerateComplianceArgs,
    globals: &GlobalOptions,
    config: &ResolvedConfig,
) -> i32 {
    match run_compliance_generate_inner(args, globals, config) {
        Ok(code) => code,
        Err((code, msg)) => {
            if globals.json {
                JsonOutput::err("compliance", JsonError::new("runtime_error", msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            code
        }
    }
}

fn run_compliance_generate_inner(
    args: GenerateComplianceArgs,
    globals: &GlobalOptions,
    config: &ResolvedConfig,
) -> Result<i32, (i32, String)> {
    let registry = load_framework_registry();

    let skill = &args.skill;
    let output_path = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("fixtures").join(skill).join("cases.yaml"));
    let model = args
        .model
        .as_deref()
        .unwrap_or(config.generator.model.as_str());
    let endpoint = args.generator_endpoint.as_deref();

    // Load the compliance-specific generation prompt (embedded fallback).
    let prompt_template = {
        let disk = std::path::Path::new("templates/compliance-generate-prompt.md");
        if disk.exists() {
            std::fs::read_to_string(disk).unwrap_or_else(|_| COMPLIANCE_GENERATE_PROMPT.to_string())
        } else {
            COMPLIANCE_GENERATE_PROMPT.to_string()
        }
    };

    // Load skill description from fixtures/<skill>/prompt.md if present.
    let skill_description = {
        let prompt_md = PathBuf::from("fixtures").join(skill).join("prompt.md");
        if prompt_md.exists() {
            std::fs::read_to_string(&prompt_md).unwrap_or_else(|_| skill.clone())
        } else {
            skill.clone()
        }
    };

    let mut existing_ids = cg_read_existing_ids(&output_path);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let total_cases = args.tag.len() * args.count as usize;
    let show_progress = !globals.quiet && !globals.json && std::io::stderr().is_terminal();
    let pb: Option<ProgressBar> = if show_progress {
        let bar = ProgressBar::new(total_cases as u64);
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} cases {msg}",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        bar.enable_steady_tick(Duration::from_millis(120));
        Some(bar)
    } else {
        None
    };

    let mut all_cases: Vec<serde_json::Value> = Vec::new();
    let mut tag_slot = 0usize;
    let total_tags = args.tag.len();

    for tag in &args.tag {
        tag_slot += 1;

        // Resolve tag to a FrameworkControl across all frameworks in the registry.
        let control = registry
            .values()
            .flat_map(|controls| controls.iter())
            .find(|c| c.tag == *tag)
            .ok_or_else(|| {
                (
                    ExitCode::NotFound.as_i32(),
                    format!(
                        "unknown compliance tag '{tag}'. \
                         Run `agc compliance gaps` to see controls without coverage."
                    ),
                )
            })?;

        if !globals.quiet && !globals.json {
            if let Some(ref bar) = pb {
                bar.suspend(|| {
                    eprintln!(
                        "  {} {} — {}",
                        style("→").cyan(),
                        style(tag).bold(),
                        &control.requirement[..control.requirement.len().min(72)]
                    );
                });
            } else {
                eprintln!(
                    "  {} {} — {}",
                    style("→").cyan(),
                    style(tag).bold(),
                    &control.requirement[..control.requirement.len().min(72)]
                );
            }
        }

        if let Some(ref bar) = pb {
            bar.set_message(format!(
                "generating {args_count} case(s) for {tag} ({tag_slot}/{total_tags})...",
                args_count = args.count,
            ));
        }

        let prompt = cg_build_prompt(
            &prompt_template,
            skill,
            &skill_description,
            &control.framework,
            &control.control_id,
            &control.requirement,
            tag,
            args.count,
            &existing_ids,
        );

        let raw = runtime
            .block_on(call_llm(model, &prompt, Some(8192), endpoint))
            .map_err(|e| {
                if let Some(ref bar) = pb {
                    bar.finish_and_clear();
                }
                (
                    ExitCode::RuntimeError.as_i32(),
                    format!("tag {tag_slot}/{total_tags} ({tag}) — LLM call failed: {e}"),
                )
            })?
            .output;

        let verbose = globals.verbose;
        // Sanitize once here so the retry prompt can echo back exactly what the model
        // produced (post-cleanup) — giving it concrete context to fix rather than just
        // an abstract error message.
        let sanitized = prepare_llm_yaml(&raw);

        // Validate; retry once with error feedback on failure.
        let case_value = cg_parse_validate(&sanitized, skill, verbose).or_else(|errors| {
            if !globals.quiet && !globals.json {
                let msg = format!(
                    "tag {tag_slot}/{total_tags} ({tag}): validation failed, retrying...\n{errors}"
                );
                if let Some(ref bar) = pb {
                    bar.suspend(|| eprintln!("{msg}"));
                } else {
                    eprintln!("{msg}");
                }
            }
            // Echo the sanitized YAML so the model can fix specific lines, not guess.
            let retry_prompt = format!(
                "{prompt}\n\n\
                 Your previous output (after automatic whitespace and quoting cleanup):\n\
                 ```yaml\n{sanitized}\n```\n\n\
                 Validation errors:\n{errors}\n\n\
                 Fix all errors and return only the corrected `cases:` YAML block."
            );
            let raw2 = runtime
                .block_on(call_llm(model, &retry_prompt, Some(8192), endpoint))
                .map_err(|e| format!("retry LLM call failed: {e}"))?
                .output;
            let sanitized2 = prepare_llm_yaml(&raw2);
            cg_parse_validate(&sanitized2, skill, verbose)
        });

        let case_value = case_value.map_err(|e| {
            if let Some(ref bar) = pb {
                bar.finish_and_clear();
            }
            (
                ExitCode::ValidationFailed.as_i32(),
                format!("tag {tag_slot}/{total_tags} ({tag}) failed validation after retry:\n{e}"),
            )
        })?;

        let mut new_cases = case_value
            .get("cases")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        // Ensure the compliance tag is present in every generated case's tags array,
        // and register every new ID so subsequent tag calls cannot reuse them.
        for case in new_cases.iter_mut() {
            cg_inject_tag(case, tag);
            if let Some(id) = case.get("id").and_then(|v| v.as_str()) {
                existing_ids.push(id.to_string());
            }
        }

        let generated = new_cases.len();
        if let Some(ref bar) = pb {
            bar.inc(generated as u64);
            bar.suspend(|| {
                for case in &new_cases {
                    let id = case
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(unknown)");
                    println!("  ✓ {tag} — {id}");
                }
            });
        } else if !globals.quiet && !globals.json {
            for case in &new_cases {
                let id = case
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(unknown)");
                println!("  ✓ {tag} — {id}");
            }
        }

        all_cases.extend(new_cases);
    }

    if let Some(bar) = pb {
        bar.finish_and_clear();
    }

    let case_count = all_cases.len();
    let cases_yaml = serde_yaml::to_string(&all_cases).unwrap_or_default();

    if args.dry_run {
        println!("{cases_yaml}");
        if globals.json {
            JsonOutput::ok(
                "compliance",
                serde_json::json!({ "cases_generated": case_count, "dry_run": true }),
            )
            .print();
        }
        return Ok(ExitCode::Ok.as_i32());
    }

    super::fixture_utils::append_cases_to_fixture(&output_path, &cases_yaml, skill)
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e))?;

    if globals.json {
        JsonOutput::ok(
            "compliance",
            serde_json::json!({
                "cases_generated": case_count,
                "output_path": output_path.display().to_string(),
                "dry_run": false,
            }),
        )
        .print();
    } else {
        println!("wrote {} case(s) to {}", case_count, output_path.display());
    }

    Ok(ExitCode::Ok.as_i32())
}

// ── compliance generate helpers ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn cg_build_prompt(
    template: &str,
    skill_name: &str,
    skill_description: &str,
    framework: &str,
    control_id: &str,
    control_requirement: &str,
    compliance_tag: &str,
    count: u32,
    existing_ids: &[String],
) -> String {
    let existing = if existing_ids.is_empty() {
        "(none)".to_string()
    } else {
        existing_ids.join("\n")
    };

    let mut coverage = format!(
        "Generate exactly {count} case(s).\n\n\
         1. **Nominal** — the agent is given a realistic, in-scope input and demonstrates \
         compliant behavior. The expected output confirms the control is satisfied under \
         normal conditions."
    );
    if count >= 2 {
        coverage.push_str(
            "\n\n2. **Boundary** — an edge-case input relevant to this control: limit values, \
             unusual but valid requests, inputs that stress the boundary of the control's scope.",
        );
    }
    if count >= 3 {
        coverage.push_str(
            "\n\n3. **Adversarial** — an input specifically designed to cause a control violation \
             (e.g. prompt injection attempting to bypass the control, a social-engineering request, \
             an ambiguous instruction that could be misread). The expected output confirms the agent \
             **resists** and maintains compliance.",
        );
    }

    template
        .replace("{{COUNT}}", &count.to_string())
        .replace("{{SKILL_NAME}}", skill_name)
        .replace("{{SKILL_DESCRIPTION}}", skill_description)
        .replace("{{FRAMEWORK}}", framework)
        .replace("{{CONTROL_ID}}", control_id)
        .replace("{{CONTROL_REQUIREMENT}}", control_requirement)
        .replace("{{COMPLIANCE_TAG}}", compliance_tag)
        .replace("{{EXISTING_IDS}}", &existing)
        .replace("{{COVERAGE_REQUIREMENTS}}", &coverage)
}

/// Parse and schema-validate already-sanitized YAML from an LLM response.
/// Caller must pass output from [`prepare_llm_yaml`].
fn cg_parse_validate(
    sanitized_yaml: &str,
    skill_name: &str,
    verbose: u8,
) -> Result<serde_json::Value, String> {
    let mut value: serde_json::Value = serde_yaml::from_str(sanitized_yaml).map_err(|e| {
        if verbose >= 1 {
            eprintln!(
                "\n── sanitized YAML that failed to parse ──\n{sanitized_yaml}\n────────────────────────────────────────"
            );
        }
        format!("YAML parse error: {e}")
    })?;

    if value.get("cases").and_then(|c| c.as_array()).is_none() {
        return Err("LLM output missing top-level 'cases:' key".to_string());
    }

    normalize_expected_block(&mut value);

    let cases_array = value
        .get("cases")
        .and_then(|c| c.as_array())
        .expect("verified above");

    let mut errors: Vec<String> = Vec::new();
    for (i, case) in cases_array.iter().enumerate() {
        let doc = serde_json::json!({
            "schema_version": 1,
            "skill_or_agent": skill_name,
            "cases": [case]
        });
        match validate_fixture_value(&doc, SchemaLocation::Default) {
            Ok(issues) if !issues.is_empty() => {
                for issue in issues {
                    errors.push(format!("case[{i}]: {issue}"));
                }
            }
            Err(e) => errors.push(format!("case[{i}]: schema error: {e}")),
            _ => {}
        }
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    Ok(value)
}

fn cg_inject_tag(case: &mut serde_json::Value, tag: &str) {
    let Some(obj) = case.as_object_mut() else {
        return;
    };
    let tags = obj.entry("tags").or_insert_with(|| serde_json::json!([]));
    let Some(arr) = tags.as_array_mut() else {
        return;
    };
    if !arr.iter().any(|t| t.as_str() == Some(tag)) {
        arr.insert(0, serde_json::json!(tag));
    }
}

fn cg_read_existing_ids(path: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return vec![];
    };
    let Ok(value) = serde_yaml::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };
    value
        .get("cases")
        .and_then(|c| c.as_array())
        .map(|cases| {
            cases
                .iter()
                .filter_map(|c| c.get("id").and_then(|id| id.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
