use crate::core::hex_util::hex_lower;
use agentcarousel_core::{CaseStatus, Role, Run};
use agentcarousel_reporters::{fetch_run, list_full_runs, list_full_runs_by_skill, list_runs};
use chrono::Utc;
use clap::Parser;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tar::Builder;

const SKILL_DEFINITION_SCHEMA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/schemas/skill-definition.schema.json"
));

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;
/// Export run results as an evidence archive for audits or sharing.
///
/// agc export packages one or more run records (including results, traces, and cryptographic fingerprints) into a .tar.gz archive with a SHA-256 MANIFEST.json. Pass --sign-key to also sign the manifest with minisign so recipients can check it with `agc verify --minisign-pubkey`.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc export --last 1\n  agc export <run-id> --sign-key ~/.minisign/agc.key\n  agc verify agentcarousel-evidence-<run-id>.tar.gz --minisign-pubkey agc.pub"
)]
pub struct ExportArgs {
    /// Run id to export (from `report list` or eval/test hint lines; omit with `--last`).
    #[arg(value_name = "RUN_ID")]
    run_id_positional: Option<String>,
    /// Export the N most recent runs (newest first). Typical: `1`, `5`, or `10` (max 50).
    #[arg(short = 'l', long, value_name = "N")]
    last: Option<usize>,
    /// Output path for a single run (default: `./agentcarousel-evidence-<run_id>.tar.gz`).
    #[arg(short = 'o', long)]
    out: Option<PathBuf>,
    /// With `--last`, write each tarball under this directory (created if missing; default: cwd).
    #[arg(short = 'd', long, value_name = "DIR")]
    out_dir: Option<PathBuf>,
    /// minisign secret key used to sign MANIFEST.json (writes MANIFEST.json.minisig).
    /// For non-interactive use (CI), generate the key with `minisign -G -W`.
    #[arg(long, value_name = "PATH")]
    sign_key: Option<PathBuf>,
    /// minisign binary name/path (default: minisign).
    #[arg(long, default_value = "minisign")]
    minisign_bin: String,
    /// href of the OSCAL Assessment Plan (SAP) the assessment results answer,
    /// e.g. the plan exported from your GRC platform. Written to `import-ap`.
    #[arg(long, value_name = "HREF")]
    import_ap: Option<String>,
    /// Include system prompts verbatim in run.json and report.md. By default, system-prompt
    /// messages are replaced by their SHA-256 digest and prompt-audit rewrites are omitted.
    #[arg(long, default_value_t = false)]
    include_prompt: bool,
}

/// Optional settings for an export beyond run selection.
#[derive(Default)]
pub(crate) struct ExportOptions<'a> {
    /// Sign MANIFEST.json with this minisign key.
    pub sign: Option<SignOptions<'a>>,
    /// OSCAL `import-ap` href for assessment-results.oscal.json.
    pub import_ap: Option<&'a str>,
    /// Disclose system prompts verbatim instead of their digests (spec INV-7).
    pub include_prompt: bool,
}

/// Manifest-signing settings for an export.
pub(crate) struct SignOptions<'a> {
    pub secret_key: &'a Path,
    pub minisign_bin: &'a str,
}

const EXPORT_LAST_MAX: usize = 50;

pub fn run_export(args: ExportArgs, globals: &GlobalOptions) -> i32 {
    let json = globals.json;
    let opts = ExportOptions {
        sign: args.sign_key.as_deref().map(|secret_key| SignOptions {
            secret_key,
            minisign_bin: args.minisign_bin.as_str(),
        }),
        import_ap: args.import_ap.as_deref(),
        include_prompt: args.include_prompt,
    };
    let run_id = args.run_id_positional.as_ref();
    match (run_id, args.last) {
        (Some(_), Some(_)) => {
            if json {
                JsonOutput::err(
                    "export",
                    JsonError::new(
                        "invalid_args",
                        "specify either RUN_ID or --last N, not both",
                    ),
                )
                .print();
            } else {
                eprintln!("error: specify either RUN_ID or --last N, not both");
            }
            ExitCode::RuntimeError.as_i32()
        }
        (None, None) => {
            if json {
                JsonOutput::err(
                    "export",
                    JsonError::new("invalid_args", "specify RUN_ID or --last N").with_suggestions(
                        vec!["Example: agc export --last 5 --out-dir ./evidence".to_string()],
                    ),
                )
                .print();
            } else {
                eprintln!(
                    "error: specify RUN_ID or --last N (e.g. export --last 5 --out-dir ./evidence)"
                );
            }
            ExitCode::RuntimeError.as_i32()
        }
        (Some(run_id), None) => {
            if args.out_dir.is_some() {
                if json {
                    JsonOutput::err(
                        "export",
                        JsonError::new("invalid_args", "--out-dir is only valid with --last"),
                    )
                    .print();
                } else {
                    eprintln!("error: --out-dir is only valid with --last");
                }
                return ExitCode::RuntimeError.as_i32();
            }
            match export_run_artifact_with(run_id, args.out.as_deref(), &opts) {
                Ok(path) => {
                    if json {
                        JsonOutput::ok(
                            "export",
                            serde_json::json!({ "paths": [path.display().to_string()] }),
                        )
                        .print();
                    } else {
                        println!("created {}", path.display());
                    }
                    ExitCode::Ok.as_i32()
                }
                Err(err) => {
                    if json {
                        JsonOutput::err("export", JsonError::new("runtime_error", err)).print();
                    } else {
                        eprintln!("error: {err}");
                    }
                    ExitCode::RuntimeError.as_i32()
                }
            }
        }
        (None, Some(n)) => {
            if args.out.is_some() {
                if json {
                    JsonOutput::err(
                        "export",
                        JsonError::new(
                            "invalid_args",
                            "with --last, use --out-dir for the output directory (not --out)",
                        ),
                    )
                    .print();
                } else {
                    eprintln!(
                        "error: with --last, use --out-dir for the output directory (not --out)"
                    );
                }
                return ExitCode::RuntimeError.as_i32();
            }
            match export_last_n(n, args.out_dir.as_deref(), &opts) {
                Ok(paths) => {
                    if json {
                        let strs: Vec<String> =
                            paths.iter().map(|p| p.display().to_string()).collect();
                        JsonOutput::ok("export", serde_json::json!({ "paths": strs })).print();
                    } else {
                        for path in paths {
                            println!("created {}", path.display());
                        }
                    }
                    ExitCode::Ok.as_i32()
                }
                Err(err) => {
                    if json {
                        JsonOutput::err("export", JsonError::new("runtime_error", err)).print();
                    } else {
                        eprintln!("error: {err}");
                    }
                    ExitCode::RuntimeError.as_i32()
                }
            }
        }
    }
}

fn infer_domain_from_skill(skill: Option<&str>) -> Option<&'static str> {
    let lo = skill?.to_lowercase();
    if lo.contains("healthcare")
        || lo.contains("hipaa")
        || lo.contains("phi")
        || lo.contains("clinical")
        || lo.contains("medical")
    {
        Some("healthcare")
    } else {
        None
    }
}

fn export_last_n(
    n: usize,
    out_dir: Option<&Path>,
    opts: &ExportOptions<'_>,
) -> Result<Vec<PathBuf>, String> {
    if n == 0 || n > EXPORT_LAST_MAX {
        return Err(format!(
            "--last N must be between 1 and {EXPORT_LAST_MAX} (got {n})"
        ));
    }
    let listings = list_runs(n).map_err(|e| e.to_string())?;
    if listings.is_empty() {
        println!("no runs recorded");
        return Ok(Vec::new());
    }
    if listings.len() < n {
        eprintln!(
            "note: only {} run(s) in history (requested {})",
            listings.len(),
            n
        );
    }
    let base = out_dir.unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    for listing in listings {
        let path = base.join(format!("agentcarousel-evidence-{}.tar.gz", listing.id));
        paths.push(export_run_artifact_with(&listing.id, Some(&path), opts)?);
    }
    Ok(paths)
}

pub(crate) fn export_run_artifact(run_id: &str, out: Option<&Path>) -> Result<PathBuf, String> {
    export_run_artifact_with(run_id, out, &ExportOptions::default())
}

pub(crate) fn export_run_artifact_with(
    run_id: &str,
    out: Option<&Path>,
    opts: &ExportOptions<'_>,
) -> Result<PathBuf, String> {
    let run = fetch_run(run_id).map_err(|err| err.to_string())?;
    let run = if opts.include_prompt {
        run
    } else {
        redact_prompt_material(run)
    };
    let root = std::env::temp_dir().join(format!("agentcarousel-evidence-{run_id}"));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|err| err.to_string())?;
    }
    fs::create_dir_all(&root).map_err(|err| err.to_string())?;

    let run_json_path = root.join("run.json");
    write_json(&run_json_path, &run)?;
    write_assurance_documents(&root, &run, opts.include_prompt)?;

    let bundle_lock_path = root.join("fixture_bundle.lock");
    let schema_hash = fixture_schema_sha256();
    let bundle_lock = json!({
        "fixture_bundle_id": run.fixture_bundle_id,
        "fixture_bundle_version": run.fixture_bundle_version,
        "schema_hash": schema_hash
    });
    write_json(&bundle_lock_path, &bundle_lock)?;

    let env_path = root.join("environment_fingerprint.json");
    let mut env_payload = serde_json::Map::new();
    env_payload.insert(
        "agentcarousel_version".to_string(),
        json!(run.agentcarousel_version),
    );
    env_payload.insert("os".to_string(), json!(std::env::consts::OS));
    env_payload.insert("arch".to_string(), json!(std::env::consts::ARCH));
    env_payload.insert("rust_version".to_string(), json!("unknown"));
    env_payload.insert("timestamp_utc".to_string(), json!(Utc::now().to_rfc3339()));
    env_payload.insert("git_sha".to_string(), json!(run.git_sha));
    env_payload.insert(
        "fixture_bundle_id".to_string(),
        json!(run.fixture_bundle_id),
    );
    env_payload.insert(
        "fixture_bundle_version".to_string(),
        json!(run.fixture_bundle_version),
    );
    if let Ok(v) = std::env::var("GITHUB_REF") {
        let t = v.trim();
        if !t.is_empty() {
            env_payload.insert("github_ref".to_string(), json!(t));
        }
    }
    if let Ok(v) = std::env::var("GITHUB_RUN_ID") {
        let t = v.trim();
        if !t.is_empty() {
            env_payload.insert("github_run_id".to_string(), json!(t));
        }
    }
    // 90-day re-attestation deadline per NIST AI RMF MANAGE 4.1 / MEASURE 2.4.
    let next_attestation_due = (Utc::now() + chrono::Duration::days(90)).to_rfc3339();
    env_payload.insert(
        "next_attestation_due".to_string(),
        json!(next_attestation_due),
    );
    write_json(&env_path, &serde_json::Value::Object(env_payload))?;

    let redaction_path = root.join("REDACTION_POLICY.md");
    let mut file = fs::File::create(&redaction_path).map_err(|err| err.to_string())?;
    file.write_all(b"Redaction policy: trace outputs are scrubbed of common secrets and tokens.\n")
        .map_err(|err| err.to_string())?;

    // Compute compliance metrics for this run's skill and embed in the tarball.
    let skill = run.skill_or_agent.as_deref();
    let (effective_skill, metrics_results, runs_analyzed) =
        super::metrics::compute_metrics_for_export(skill, 20, infer_domain_from_skill(skill));
    let metrics_json_path = root.join("metrics.json");
    let metrics_json_payload = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "skill": effective_skill.as_deref().unwrap_or("all skills"),
        "analysis_window_runs": runs_analyzed,
        "metrics": metrics_results
            .iter()
            .map(|m| json!({
                "id": m.id,
                "title": m.title,
                "domain": m.domain,
                "score_0_to_100": m.score_0_to_100,
                "grade": m.grade,
                "finding": m.finding,
                "sample_size": m.sample_size,
                "detail": m.detail,
                "framework_controls": m.framework_controls,
            }))
            .collect::<Vec<_>>()
    });
    write_json(&metrics_json_path, &metrics_json_payload)?;

    let metrics_section = super::metrics::render_metrics_to_markdown(
        &metrics_results,
        effective_skill.as_deref(),
        runs_analyzed,
    );

    let report_md_path = root.join("report.md");
    let report_md = render_markdown_report(&run, &metrics_section);
    let mut file = fs::File::create(&report_md_path).map_err(|err| err.to_string())?;
    file.write_all(report_md.as_bytes())
        .map_err(|err| err.to_string())?;

    // Compliance artifacts: per-framework Markdown reports + OSCAL Assessment Results.
    // Load run history for scoring (same window as metrics).
    let compliance_runs = match skill {
        Some(s) => list_full_runs_by_skill(s, 20).unwrap_or_default(),
        None => list_full_runs(20).unwrap_or_default(),
    };
    const COMPLIANCE_FRAMEWORKS: &[&str] = &["nist-ai-rmf", "eu-ai-act", "iso-42001"];
    const OPT_IN_COMPLIANCE_FRAMEWORKS: &[&str] = &["aiuc-1"];
    // Load registry once for all framework scoring in this loop.
    let registry = super::compliance_mappings::load_framework_registry();
    // Opt-in crosswalks are only written when the scored runs carry the
    // framework's tags, so bundles that never use a crosswalk don't mention it.
    let uses_framework = |fw: &str| {
        let prefix = format!("{fw}:");
        compliance_runs
            .iter()
            .flat_map(|r| &r.cases)
            .any(|c| c.tags.iter().any(|t| t.starts_with(&prefix)))
    };
    let frameworks = COMPLIANCE_FRAMEWORKS.iter().chain(
        OPT_IN_COMPLIANCE_FRAMEWORKS
            .iter()
            .filter(|fw| uses_framework(fw)),
    );
    for fw in frameworks {
        let scores = super::compliance_mappings::compute_control_scores_with_registry(
            &registry,
            &compliance_runs,
            fw,
            skill,
            None,
        );
        let md = super::metrics::render_framework_compliance_report(&scores, fw, skill);
        fs::write(root.join(format!("compliance_{fw}.md")), md).map_err(|err| err.to_string())?;
    }
    // OSCAL Assessment Results — use the first framework's scores as the primary artifact.
    let oscal_scores = super::compliance_mappings::compute_control_scores_with_registry(
        &registry,
        &compliance_runs,
        COMPLIANCE_FRAMEWORKS[0],
        skill,
        None,
    );
    let oscal_json = super::metrics::serialize_assessment_results(
        &oscal_scores,
        COMPLIANCE_FRAMEWORKS[0],
        skill,
        run_id,
        &compliance_runs,
        opts.import_ap,
    );
    let oscal_path = root.join("assessment-results.oscal.json");
    let mut f = fs::File::create(&oscal_path).map_err(|err| err.to_string())?;
    f.write_all(oscal_json.as_bytes())
        .map_err(|err| err.to_string())?;

    let manifest_path = root.join("MANIFEST.json");
    let manifest = build_manifest(&root)?;
    let manifest_violations =
        super::assurance::schema_violations(super::assurance::DocKind::Manifest, &manifest);
    if let Some(first) = manifest_violations.first() {
        return Err(format!(
            "internal error: MANIFEST.json is nonconformant: {first}"
        ));
    }
    write_json(&manifest_path, &manifest)?;
    if let Some(sign) = opts.sign.as_ref() {
        if let Err(err) = super::minisign::sign_file(
            sign.minisign_bin,
            sign.secret_key,
            &manifest_path,
            &format!("agentcarousel evidence manifest run_id:{run_id}"),
        ) {
            fs::remove_dir_all(&root).ok();
            return Err(err.to_string());
        }
    }

    let out_path = out
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!("agentcarousel-evidence-{run_id}.tar.gz")));
    let archive = fs::File::create(&out_path).map_err(|err| err.to_string())?;
    let encoder = GzEncoder::new(archive, Compression::default());
    let mut tar = Builder::new(encoder);
    tar.append_dir_all(format!("agentcarousel-evidence-{run_id}"), &root)
        .map_err(|err| err.to_string())?;
    tar.finish().map_err(|err| err.to_string())?;
    fs::remove_dir_all(&root).ok();
    Ok(out_path)
}

/// Withhold prompt text from an exported run (spec INV-7): replace every system-role input
/// message with a digest marker and drop prompt-audit rewrites, which reproduce prompt.md text.
/// The local history copy is untouched.
pub(crate) fn redact_prompt_material(mut run: Run) -> Run {
    for case in &mut run.cases {
        for msg in &mut case.input {
            if msg.role == Role::System {
                msg.content = system_prompt_marker(&msg.content);
            }
        }
    }
    if let Some(audit) = run.prompt_audit.as_mut() {
        audit.suggested_implementations.clear();
        audit.suggested_locations.clear();
    }
    run
}

/// Marker substituted for withheld system-prompt content.
pub(crate) fn system_prompt_marker(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!(
        "[system prompt withheld; sha256:{}]",
        hex_lower(hasher.finalize().as_ref())
    )
}

fn render_markdown_report(run: &Run, metrics_section: &str) -> String {
    use std::fmt::Write as _;
    let mut md = String::new();

    let skill = run.skill_or_agent.as_deref().unwrap_or("unknown");
    let _ = writeln!(md, "# agentcarousel Evidence Report — {skill}");
    let _ = writeln!(md);
    let _ = writeln!(md, "**Run ID:** {}", run.id.0);
    if let Some(cmd_line) = run.summary.command_line.as_ref() {
        let _ = writeln!(md, "**Command:** `{cmd_line}`");
    } else {
        let _ = writeln!(md, "**Command:** {}", run.command);
    }
    let _ = writeln!(
        md,
        "**Started:** {}",
        run.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    if let Some(finished) = run.finished_at {
        let _ = writeln!(
            md,
            "**Finished:** {}",
            finished.format("%Y-%m-%d %H:%M:%S UTC")
        );
    }
    let _ = writeln!(
        md,
        "**agentcarousel version:** {}",
        run.agentcarousel_version
    );
    let _ = writeln!(md);

    let s = &run.summary;
    let _ = writeln!(md, "## Summary");
    let _ = writeln!(md);

    let result_icon = if s.failed == 0 && s.errored == 0 && s.timed_out == 0 {
        "✅"
    } else {
        "❌"
    };
    let pass_rate_pct = s.pass_rate * 100.0;
    let _ = writeln!(
        md,
        "{result_icon} **{}/{} passed** ({pass_rate_pct:.1}%)",
        s.passed, s.total
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "| Metric | Value |");
    let _ = writeln!(md, "|--------|-------|");
    let _ = writeln!(md, "| Total | {} |", s.total);
    let _ = writeln!(md, "| Passed | {} |", s.passed);
    let _ = writeln!(md, "| Failed | {} |", s.failed);
    if s.errored > 0 {
        let _ = writeln!(md, "| Errored | {} |", s.errored);
    }
    if s.timed_out > 0 {
        let _ = writeln!(md, "| Timed out | {} |", s.timed_out);
    }
    if let Some(eff) = s.mean_effectiveness_score {
        let _ = writeln!(md, "| Effectiveness score | {eff:.2} / 1.00 |");
    }
    let _ = writeln!(md, "| Mean latency | {:.0}ms |", s.mean_latency_ms);
    if let (Some(p50), Some(p95), Some(p99)) =
        (s.latency_p50_ms, s.latency_p95_ms, s.latency_p99_ms)
    {
        let _ = writeln!(
            md,
            "| Latency p50 / p95 / p99 | {p50:.0}ms / {p95:.0}ms / {p99:.0}ms |"
        );
    }
    let _ = writeln!(md);

    // Models section
    if s.generator_model.is_some() || s.judge_model.is_some() {
        let _ = writeln!(md, "### Models");
        let _ = writeln!(md);
        if let Some(gen_model) = s.generator_model.as_ref() {
            let gen_tokens = match (s.tokens_in, s.tokens_out) {
                (Some(i), Some(o)) => format!("  ({i} in / {o} out tokens)"),
                _ => String::new(),
            };
            let gen_cost = s
                .gen_cost_usd
                .map(|c| format!("  · ${c:.4}"))
                .unwrap_or_default();
            let _ = writeln!(md, "- **Generator:** `{gen_model}`{gen_tokens}{gen_cost}");
        }
        if let Some(judge_model) = s.judge_model.as_ref() {
            let judge_tokens = match (s.judge_tokens_in, s.judge_tokens_out) {
                (Some(i), Some(o)) => format!("  ({i} in / {o} out tokens)"),
                _ => String::new(),
            };
            let judge_cost = s
                .judge_cost_usd
                .map(|c| format!("  · ${c:.4}"))
                .unwrap_or_default();
            let _ = writeln!(md, "- **Judge:** `{judge_model}`{judge_tokens}{judge_cost}");
        }
        if let Some(total_cost) = s.total_cost_usd {
            let _ = writeln!(md, "- **Total cost:** ${total_cost:.4}");
        }
        let _ = writeln!(md);
    } else {
        if let (Some(gin), Some(gout)) = (s.tokens_in, s.tokens_out) {
            let _ = writeln!(md, "- Generator tokens: {gin} in / {gout} out");
        }
        if let (Some(jin), Some(jout)) = (s.judge_tokens_in, s.judge_tokens_out) {
            let _ = writeln!(md, "- Judge tokens: {jin} in / {jout} out");
        }
        if let Some(cost) = s.gen_cost_usd {
            let _ = writeln!(md, "- Generator cost: ${cost:.4}");
        }
        if let Some(cost) = s.judge_cost_usd {
            let _ = writeln!(md, "- Judge cost: ${cost:.4}");
        }
        if let Some(cost) = s.total_cost_usd {
            let _ = writeln!(md, "- Total cost: ${cost:.4}");
        }
        let _ = writeln!(md);
    }

    let _ = writeln!(md, "---");
    let _ = writeln!(md);
    let _ = writeln!(md, "## Certification Scope and Limitations");
    let _ = writeln!(md);
    let _ = writeln!(md, "This certification covers the agent system prompt, model version, and workflow configuration described herein, as evaluated on the run date above.");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "Behavioral certification does **not** automatically extend to:"
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "- Subsequent model versions or provider-issued model updates to the tested model"
    );
    let _ = writeln!(md, "- Revisions to the agent's system prompt or workflow configuration after the certification date");
    let _ = writeln!(md, "- Production deployments that differ materially from the configuration described in this report");
    let _ = writeln!(
        md,
        "- Future agent behavior in scenarios not represented in the test case suite"
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "**Re-attestation:** When the tested model version is deprecated, updated, or replaced by the LLM provider, re-attestation against the same fixture suite is required to maintain a current behavioral certification record. A 90-day grace period applies from the date the provider announces a model version change. Re-attestation is available at reduced cost for existing customers.");
    let _ = writeln!(md);
    let _ = writeln!(md, "*This report is issued under the AgentCarousel Behavioral Certification Program. The domain expert attestation constitutes a professional opinion that the test cases are appropriate and material for the stated use case as of the certification date — not a guarantee of agent safety or regulatory compliance in all production scenarios or future model versions.*");
    let _ = writeln!(md);
    let _ = writeln!(md, "---");
    let _ = writeln!(md);
    md.push_str(metrics_section);
    let _ = writeln!(md, "---");
    let _ = writeln!(md);
    let _ = writeln!(md, "## Cases");
    let _ = writeln!(md);

    for case in &run.cases {
        let id = &case.case_id.0;
        let (status_icon, status_label) = match case.status {
            CaseStatus::Passed => ("✅", "Passed"),
            CaseStatus::Failed => ("❌", "Failed"),
            CaseStatus::Error => ("⚠️", "Error"),
            CaseStatus::TimedOut => ("⏱️", "Timed Out"),
            CaseStatus::Skipped => ("⏭️", "Skipped"),
            CaseStatus::Flaky => ("⚠️", "Flaky"),
        };
        let latency_ms = case.metrics.total_latency_ms;
        let eff_str = case
            .eval_scores
            .as_ref()
            .map(|s| format!(" · effectiveness {:.2}", s.effectiveness_score))
            .unwrap_or_default();
        let _ = writeln!(
            md,
            "### {status_icon} {id}  <sup>{status_label} · {latency_ms}ms{eff_str}</sup>"
        );
        let _ = writeln!(md);

        if !case.input.is_empty() {
            let _ = writeln!(md, "**Input:**");
            let _ = writeln!(md);
            for msg in &case.input {
                let role = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "system",
                    Role::Tool => "tool",
                };
                let _ = writeln!(md, "**[{role}]**");
                for line in msg.content.trim().lines() {
                    let _ = writeln!(md, "> {line}");
                }
                let _ = writeln!(md);
            }
        }

        if let Some(reply) = case.trace.final_output.as_ref() {
            let reply = reply.trim();
            if !reply.is_empty() {
                let _ = writeln!(md, "**Agent replied:**");
                let _ = writeln!(md);
                for line in reply.lines() {
                    let _ = writeln!(md, "> {line}");
                }
                let _ = writeln!(md);
            }
        }

        if let Some(scores) = case.eval_scores.as_ref() {
            if !scores.rubric_scores.is_empty() {
                let _ = writeln!(md, "**Rubric:**");
                let _ = writeln!(md);
                let _ = writeln!(md, "|  | Criterion | Score | Weight |");
                let _ = writeln!(md, "|--|-----------|-------|--------|");
                for rs in &scores.rubric_scores {
                    let icon = if rs.score >= 0.9 {
                        "✅"
                    } else if rs.score >= 0.5 {
                        "⚠️"
                    } else {
                        "❌"
                    };
                    let _ = writeln!(
                        md,
                        "| {icon} | {} | {:.2} | {:.2} |",
                        rs.rubric_id, rs.score, rs.weight
                    );
                }
                let _ = writeln!(md);
            }
            if let Some(rationale) = scores.judge_rationale.as_ref() {
                let rationale = rationale.trim();
                if !rationale.is_empty() {
                    let _ = writeln!(md, "**Judge:** {rationale}");
                    let _ = writeln!(md);
                }
            }
            let case_result_icon = match case.status {
                CaseStatus::Passed => "✅",
                CaseStatus::Failed => "❌",
                _ => "⚠️",
            };
            let _ = writeln!(
                md,
                "{case_result_icon} **Effectiveness: {:.2}**",
                scores.effectiveness_score
            );
            let _ = writeln!(md);
        }

        if let Some(err) = case.error.as_ref() {
            if !err.is_empty() {
                let _ = writeln!(md, "**Error:** {err}");
                let _ = writeln!(md);
            }
        }

        let _ = writeln!(md, "---");
        let _ = writeln!(md);
    }

    md
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    let mut file = fs::File::create(path).map_err(|err| err.to_string())?;
    file.write_all(payload.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn fixture_schema_sha256() -> String {
    let mut hasher = Sha256::new();
    hasher.update(SKILL_DEFINITION_SCHEMA.as_bytes());
    format!("sha256:{}", hex_lower(hasher.finalize().as_ref()))
}

pub(super) fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("sha256:{}", hex_lower(hasher.finalize().as_ref())))
}

/// Write `evidence.json` and `attestation.intoto.json` (assurance spec v0.1), plus
/// `prompt.md` when disclosure was requested and the recorded digest still matches.
/// Runs recorded before run subjects existed get neither document; a note says why.
fn write_assurance_documents(root: &Path, run: &Run, include_prompt: bool) -> Result<(), String> {
    use super::assurance;
    let recorded_prompt = run.subject.as_ref().and_then(|s| s.system_prompt.as_ref());
    let system_prompt = match recorded_prompt {
        None => "not_applicable",
        Some(prompt) if include_prompt => {
            let source = Path::new(&prompt.path);
            match sha256_file_hex(source) {
                Ok(digest) if digest == prompt.sha256 => {
                    fs::copy(source, root.join("prompt.md")).map_err(|err| err.to_string())?;
                    "included"
                }
                _ => {
                    eprintln!(
                        "note: {} changed or is missing since the run; prompt not included",
                        prompt.path
                    );
                    "digest_only"
                }
            }
        }
        Some(_) => "digest_only",
    };
    let disclosure = assurance::Disclosure {
        system_prompt,
        test_inputs: if include_prompt {
            "included"
        } else {
            "redacted"
        },
    };
    let evidence = match assurance::build_evidence(run, disclosure) {
        Ok(evidence) => evidence,
        Err(reason) => {
            eprintln!("note: {reason}");
            return Ok(());
        }
    };
    let evidence_path = root.join(assurance::EVIDENCE_FILE);
    write_json(&evidence_path, &evidence)?;
    let mut digests = std::collections::BTreeMap::new();
    for name in [assurance::EVIDENCE_FILE, "run.json"] {
        digests.insert(name.to_string(), sha256_file_hex(&root.join(name))?);
    }
    let documents: Vec<(String, String)> = digests
        .iter()
        .map(|(path, digest)| (path.clone(), digest.clone()))
        .collect();
    let statement = assurance::build_statement(&evidence, &documents);
    let violations = assurance::conformance_violations(&evidence, &statement, &digests);
    if !violations.is_empty() {
        let list: Vec<String> = violations.iter().map(ToString::to_string).collect();
        return Err(format!(
            "internal error: generated assurance evidence is nonconformant: {}",
            list.join("; ")
        ));
    }
    write_json(&root.join(assurance::STATEMENT_FILE), &statement)
}

/// Integrity manifest over evidence files (excludes `MANIFEST.json` itself).
fn build_manifest(root: &Path) -> Result<serde_json::Value, String> {
    let optional = [
        "evidence.json",
        "attestation.intoto.json",
        "prompt.md",
        "compliance_aiuc-1.md",
    ];
    let tracked = [
        "run.json",
        "fixture_bundle.lock",
        "environment_fingerprint.json",
        "REDACTION_POLICY.md",
        "metrics.json",
        "report.md",
        "compliance_nist-ai-rmf.md",
        "compliance_eu-ai-act.md",
        "compliance_iso-42001.md",
        "assessment-results.oscal.json",
    ];
    let mut files = Vec::new();
    let present_optional = optional.into_iter().filter(|name| root.join(name).exists());
    for name in tracked.into_iter().chain(present_optional) {
        let path = root.join(name);
        let digest = sha256_file_hex(&path)?;
        files.push(json!({
            "path": name,
            "sha256": digest
        }));
    }
    Ok(json!({
        "manifest_version": 1,
        "files": files
    }))
}
