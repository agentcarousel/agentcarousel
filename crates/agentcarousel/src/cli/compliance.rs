use agentcarousel_reporters::{list_full_runs, list_full_runs_by_skill};
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;

use super::compliance_mappings::{compute_control_scores, ControlCoverageStatus};
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

pub fn run_compliance(args: ComplianceArgs, globals: &GlobalOptions) -> i32 {
    match args.command {
        ComplianceCommand::Report(a) => run_report(a, globals),
        ComplianceCommand::Gaps(a) => run_gaps(a, globals),
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

fn run_report(args: ReportArgs, globals: &GlobalOptions) -> i32 {
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

    let frameworks: Vec<&str> = if args.framework == "all" {
        ALL_FRAMEWORKS.to_vec()
    } else {
        vec![args.framework.as_str()]
    };

    if globals.json {
        let mut results = Vec::new();
        for fw in &frameworks {
            let scores = compute_control_scores(&runs, fw, args.skill.as_deref());
            results.push(serde_json::json!({
                "framework": fw,
                "skill": args.skill,
                "control_scores": scores,
            }));
        }
        JsonOutput::ok("compliance", serde_json::json!({ "reports": results })).print();
        return ExitCode::Ok.as_i32();
    }

    if args.framework == "all" {
        // Write one file per framework to the --out directory (or cwd).
        let out_dir = args.out.clone().unwrap_or_else(|| PathBuf::from("."));
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("error creating output directory: {e}");
            return ExitCode::RuntimeError.as_i32();
        }
        for fw in &frameworks {
            let scores = compute_control_scores(&runs, fw, args.skill.as_deref());
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
    let scores = compute_control_scores(&runs, fw, args.skill.as_deref());

    let content = if args.oscal {
        serialize_assessment_results(&scores, fw, args.skill.as_deref(), &args.run_id)
    } else {
        render_framework_compliance_report(&scores, fw, args.skill.as_deref())
    };

    match args.out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &content) {
                eprintln!("error writing {}: {e}", path.display());
                ExitCode::RuntimeError.as_i32()
            } else {
                println!("wrote {}", path.display());
                ExitCode::Ok.as_i32()
            }
        }
        None => {
            print!("{content}");
            ExitCode::Ok.as_i32()
        }
    }
}

fn run_gaps(args: GapsArgs, globals: &GlobalOptions) -> i32 {
    let runs = match load_runs(args.skill.as_deref(), args.limit, globals) {
        Ok(r) => r,
        Err(code) => return code,
    };

    let fw = args.framework.as_str();
    let scores = compute_control_scores(&runs, fw, args.skill.as_deref());
    let gaps: Vec<_> = scores
        .iter()
        .filter(|s| s.status == ControlCoverageStatus::Gap)
        .collect();

    if globals.json {
        JsonOutput::ok(
            "compliance",
            serde_json::json!({
                "framework": fw,
                "skill": args.skill,
                "gap_count": gaps.len(),
                "gaps": gaps,
            }),
        )
        .print();
        return ExitCode::Ok.as_i32();
    }

    let skill_label = args.skill.as_deref().unwrap_or("all skills");
    println!();
    println!(
        "  {}",
        style(format!("Compliance Gaps — {fw} · {skill_label}")).bold()
    );
    println!("  {}", "─".repeat(70));

    if gaps.is_empty() {
        println!(
            "  {} No gaps found — all controls have fixture coverage.",
            style("✅").green()
        );
        println!();
        return ExitCode::Ok.as_i32();
    }

    println!(
        "  {} controls have no fixture coverage. Add cases tagged as shown below.",
        style(gaps.len().to_string()).red().bold()
    );
    println!();

    for s in &gaps {
        println!(
            "  {} {}",
            style("❌").red(),
            style(&s.control.control_id).bold()
        );
        println!("     Tag:         {}", style(&s.control.tag).cyan());
        println!(
            "     Requirement: {}",
            &s.control.requirement[..s.control.requirement.len().min(80)]
        );
        println!(
            "     Remediate:   {}",
            style(format!("agc compliance scaffold --tag {}", s.control.tag)).dim()
        );
        println!();
    }

    println!("  {}", "─".repeat(70));
    println!(
        "  Run {} to generate fixture scaffolds for missing controls.",
        style("agc compliance scaffold --tag <tag>").dim()
    );
    println!();

    ExitCode::Ok.as_i32()
}
