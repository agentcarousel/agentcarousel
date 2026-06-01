use agentcarousel_reporters::{list_full_runs, list_full_runs_by_skill};
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use super::compliance_mappings::{collapse_scores, compute_control_scores, ControlCoverageStatus};
use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::metrics::{render_framework_compliance_report, serialize_assessment_results};
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const ALL_FRAMEWORKS: &[&str] = &[
    "nist-ai-rmf",
    "eu-ai-act",
    "iso-42001",
    "hipaa",
    "fda-samd",
    "nist-800-53",
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
}

#[derive(Debug, Parser)]
struct ReportArgs {
    /// Framework to report on. Use "all" to run all embedded frameworks.
    /// Available: nist-ai-rmf, eu-ai-act, iso-42001, hipaa, fda-samd,
    ///            nist-800-53, nist-800-171, nist-800-172, nist-800-207
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
}

pub fn run_compliance(args: ComplianceArgs, globals: &GlobalOptions, config: &ResolvedConfig) -> i32 {
    let model_filter = Some(config.generator.model.as_str());
    match args.command {
        ComplianceCommand::Report(a) => run_report(a, globals, model_filter),
        ComplianceCommand::Gaps(a) => run_gaps(a, globals, model_filter),
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

    // Collect what models ARE present so we can suggest them.
    let mut present: Vec<String> = runs
        .iter()
        .filter_map(|r| r.summary.generator_model.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    present.sort();

    if globals.json {
        let mut val = serde_json::json!({
            "model": model,
            "models_in_history": present,
        });
        if runs.is_empty() {
            val["hint"] = serde_json::json!(
                format!("Run `agc eval --model {model}` to generate run data.")
            );
        } else {
            val["hint"] = serde_json::json!(
                format!(
                    "Run `agc eval --model {model}` or use one of the models already in history: {}",
                    present.join(", ")
                )
            );
        }
        JsonOutput::err("compliance", JsonError::new("no_runs_for_model", val.to_string()))
            .print();
    } else if runs.is_empty() {
        eprintln!(
            "error: no run history found. Run `agc eval --model {model}` to generate data."
        );
    } else {
        eprintln!("error: no runs found for model '{model}'.");
        if present.is_empty() {
            eprintln!("       Run `agc eval --model {model}` to generate data for this model.");
        } else {
            eprintln!(
                "       Models in history: {}",
                present.join(", ")
            );
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

    if globals.json {
        let mut results = Vec::new();
        for fw in &frameworks {
            let scores =
                compute_control_scores(&runs, fw, args.skill.as_deref(), model_filter);
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
        let out_dir = args.out.clone().unwrap_or_else(|| PathBuf::from("."));
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("error creating output directory: {e}");
            return ExitCode::RuntimeError.as_i32();
        }
        for fw in &frameworks {
            let scores =
                compute_control_scores(&runs, fw, args.skill.as_deref(), model_filter);
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
    let scores = compute_control_scores(&runs, fw, args.skill.as_deref(), model_filter);

    if args.oscal {
        let content =
            serialize_assessment_results(&scores, fw, args.skill.as_deref(), &args.run_id);
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

    let satisfied = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Satisfied)
        .count();
    let partial = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::PartialEvidence)
        .count();
    let gap = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Gap)
        .count();
    let procedural = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Procedural)
        .count();
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
        "  {}  {}  {}  {}",
        style(format!("✅ {satisfied} satisfied")).green(),
        style(format!("⚠  {partial} partial")).yellow(),
        style(format!("❌ {gap} gap")).red(),
        style(format!("📋 {procedural} procedural")).dim(),
    );
    println!();

    let covered: Vec<&super::compliance_mappings::ControlScore> = collapsed
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                ControlCoverageStatus::Satisfied | ControlCoverageStatus::PartialEvidence
            )
        })
        .collect();

    let procedural_list: Vec<&super::compliance_mappings::ControlScore> = collapsed
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Procedural)
        .collect();

    if covered.is_empty() && procedural_list.is_empty() {
        println!(
            "  {}",
            style("No behavioral evidence yet.").yellow().bold()
        );
        println!(
            "  Tag fixture cases with  {}  to link test results to controls.",
            style(format!("{framework}:<control-id>")).cyan()
        );
        println!(
            "  Then run  {}  to scaffold cases for specific controls.",
            style("agc compliance scaffold --tag <tag>").dim()
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
            let status_str = if s.status == ControlCoverageStatus::Satisfied {
                style("✅ Satisfied").green().to_string()
            } else {
                style("⚠  Partial").yellow().to_string()
            };
            println!(
                "  {:<32} {:<8} {:<6} {}",
                &s.control.control_id[..s.control.control_id.len().min(31)],
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
                &s.control.control_id[..s.control.control_id.len().min(31)],
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

    let filter_label = filter_label(model_filter, args.skill.as_deref());
    println!();
    println!(
        "  {}",
        style(format!("Compliance Gaps — {fw} · {filter_label}")).bold()
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
            style(format!("agc compliance scaffold --tag {}", s.control.tag)).dim()
        );
        println!();
    }

    ExitCode::Ok.as_i32()
}

/// Wrap `text` to lines of at most `width` chars, breaking at word boundaries.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
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
    lines
}
