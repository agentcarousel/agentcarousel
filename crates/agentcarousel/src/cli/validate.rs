use agentcarousel_fixtures::{load_fixture_value, validate_fixture_value, SchemaLocation};
use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::config::{resolve_schema_path, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{collect_fixture_paths_with_ignore, is_kebab_case};
use super::GlobalOptions;

const AGENTCAROUSEL_IGNORE: &str = ".agentcarousel-ignore";

/// Validate fixtures: JSON Schema, kebab-case ids, case id prefixes, safe relative paths.
#[derive(Debug, Parser)]
pub struct ValidateArgs {
    /// Files or dirs to scan (default: `.` if omitted).
    #[arg(value_name = "PATHS")]
    paths: Vec<PathBuf>,
    /// JSON Schema file (default from config).
    #[arg(short = 's', long)]
    schema: Option<PathBuf>,
    /// Fail on warnings too (also if validate.strict in config).
    #[arg(short = 'x', long)]
    strict: bool,
    /// human | json (default from config).
    #[arg(short = 'f', long)]
    format: Option<String>,
}

#[derive(Debug, Serialize)]
struct ValidationReport {
    path: String,
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct OutputMessage {
    file: String,
    line: u32,
    col: u32,
    level: String,
    message: String,
}

/// Per-file signals derived from fixture YAML (heuristic “coverage map”, not ATF certification).
#[derive(Debug, Default, Serialize)]
struct AtfFileHints {
    cases: usize,
    cases_with_negative_tag: usize,
    risk_tier: Option<String>,
    data_handling: Option<String>,
    certification_track: Option<String>,
    declares_bundle_id: bool,
}

#[derive(Debug, Default, Serialize)]
struct AtfSummary {
    /// Files successfully loaded as YAML/JSON fixture values.
    fixture_files_loaded: usize,
    total_cases: usize,
    cases_with_negative_tag: usize,
    fixtures_declaring_bundle_id: usize,
    risk_tier: BTreeMap<String, usize>,
    data_handling: BTreeMap<String, usize>,
    certification_track: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
struct ValidateJsonBody<'a> {
    messages: Vec<OutputMessage>,
    /// Heuristic map from existing fixture fields only (schema + optional bundle metadata).
    atf_summary: &'a AtfSummary,
}

pub fn run_validate(args: ValidateArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let mut reports = Vec::new();
    let mut has_errors = false;
    let mut has_warnings = false;
    let format = resolve_format(args.format.as_deref(), &config.output.format);
    let strict = args.strict || config.validate.strict;
    let inputs = fixture_scan_roots(&args.paths);
    let ignore_file = Path::new(AGENTCAROUSEL_IGNORE)
        .exists()
        .then_some(Path::new(AGENTCAROUSEL_IGNORE));

    let mut atf_rows = Vec::new();
    for path in collect_fixture_paths_with_ignore(&inputs, ignore_file) {
        match validate_path(&path, args.schema.as_deref(), config) {
            Ok((report, hints)) => {
                has_errors |= !report.errors.is_empty();
                has_warnings |= !report.warnings.is_empty();
                atf_rows.push(hints);
                reports.push(report);
            }
            Err(err) => {
                has_errors = true;
                reports.push(ValidationReport {
                    path: path.display().to_string(),
                    errors: vec![err],
                    warnings: vec![],
                });
            }
        }
    }

    let atf_summary = summarize_atf(&atf_rows);
    if !globals.quiet {
        output_reports(&format, &reports, &atf_summary);
    }

    if has_errors || (strict && has_warnings) {
        return ExitCode::ValidationFailed.as_i32();
    }

    ExitCode::Ok.as_i32()
}

fn fixture_scan_roots(paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths.to_vec()
    }
}

fn validate_path(
    path: &Path,
    schema_path: Option<&Path>,
    config: &ResolvedConfig,
) -> Result<(ValidationReport, AtfFileHints), String> {
    let value = load_fixture_value(path).map_err(|err| err.to_string())?;

    let schema_location = schema_path
        .map(|path| SchemaLocation::Path(path.to_path_buf()))
        .unwrap_or_else(|| SchemaLocation::Path(resolve_schema_path(config)));

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    warn_pem_material(&value, &mut warnings);

    let schema_errors =
        validate_fixture_value(&value, schema_location).map_err(|err| err.to_string())?;
    for issue in schema_errors {
        errors.push(issue.to_string());
    }

    if let Some(object) = value.as_object() {
        let known_keys: HashSet<&str> = [
            "schema_version",
            "skill_or_agent",
            "defaults",
            "cases",
            "bundle_id",
            "bundle_version",
            "certification_track",
            "risk_tier",
            "data_handling",
        ]
        .into_iter()
        .collect();
        for key in object.keys() {
            if !known_keys.contains(key.as_str()) {
                warnings.push(format!("unknown top-level key: {key}"));
            }
        }
    }

    if let Some(skill_or_agent) = value.get("skill_or_agent").and_then(|value| value.as_str()) {
        if !is_kebab_case(skill_or_agent) {
            errors.push("skill_or_agent must be kebab-case".to_string());
        }
        if let Some(cases) = value.get("cases").and_then(|value| value.as_array()) {
            for case in cases {
                if let Some(case_id) = case.get("id").and_then(|id| id.as_str()) {
                    let prefix = format!("{skill_or_agent}/");
                    if !case_id.starts_with(&prefix) {
                        errors.push(format!("case id must start with \"{prefix}\": {case_id}"));
                    }
                } else {
                    errors.push("case id is required".to_string());
                }
                if let Some(config) = case
                    .get("evaluator_config")
                    .and_then(|value| value.as_object())
                {
                    if let Some(path) = config.get("golden_path").and_then(|value| value.as_str()) {
                        if let Err(message) = ensure_safe_relative("golden_path", path) {
                            errors.push(message);
                        }
                    }
                    if let Some(cmds) = config.get("process_cmd").and_then(|value| value.as_array())
                    {
                        for cmd in cmds {
                            if let Some(cmd_str) = cmd.as_str() {
                                if let Err(message) = ensure_safe_relative("process_cmd", cmd_str) {
                                    errors.push(message);
                                }
                            }
                        }
                    }
                }
            }
        } else {
            warnings.push("cases array is empty".to_string());
        }
    } else {
        errors.push("skill_or_agent is required".to_string());
    }

    let hints = atf_hints_from_value(&value);
    Ok((
        ValidationReport {
            path: path.display().to_string(),
            errors,
            warnings,
        },
        hints,
    ))
}

fn warn_pem_material(value: &Value, warnings: &mut Vec<String>) {
    let mut found = false;
    walk_string_values(value, &mut |s: &str| {
        if found {
            return;
        }
        if s.contains("BEGIN ") && s.contains("PRIVATE KEY") {
            warnings.push(
                "possible PEM private key material in fixture strings (warning only; remove secrets from fixtures)"
                    .to_string(),
            );
            found = true;
        }
    });
}

fn walk_string_values(value: &Value, f: &mut impl FnMut(&str)) {
    match value {
        Value::Object(map) => {
            for v in map.values() {
                walk_string_values(v, f);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                walk_string_values(v, f);
            }
        }
        Value::String(s) => f(s),
        _ => {}
    }
}

fn atf_hints_from_value(value: &Value) -> AtfFileHints {
    let mut hints = AtfFileHints::default();
    let Some(obj) = value.as_object() else {
        return hints;
    };
    hints.declares_bundle_id = obj
        .get("bundle_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    hints.risk_tier = obj
        .get("risk_tier")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    hints.data_handling = obj
        .get("data_handling")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    hints.certification_track = obj
        .get("certification_track")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let Some(cases) = obj.get("cases").and_then(|c| c.as_array()) else {
        return hints;
    };
    hints.cases = cases.len();
    for case in cases {
        let Some(case_obj) = case.as_object() else {
            continue;
        };
        let tags = case_obj
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_lowercase)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if tags.iter().any(|t| t == "negative" || t == "smoke") {
            hints.cases_with_negative_tag += 1;
        }
    }
    hints
}

fn summarize_atf(rows: &[AtfFileHints]) -> AtfSummary {
    let mut summary = AtfSummary {
        fixture_files_loaded: rows.len(),
        ..AtfSummary::default()
    };
    for row in rows {
        summary.total_cases += row.cases;
        summary.cases_with_negative_tag += row.cases_with_negative_tag;
        if row.declares_bundle_id {
            summary.fixtures_declaring_bundle_id += 1;
        }
        if let Some(ref tier) = row.risk_tier {
            *summary.risk_tier.entry(tier.clone()).or_insert(0) += 1;
        } else {
            *summary.risk_tier.entry("unset".to_string()).or_insert(0) += 1;
        }
        if let Some(ref dh) = row.data_handling {
            *summary.data_handling.entry(dh.clone()).or_insert(0) += 1;
        } else {
            *summary
                .data_handling
                .entry("unset".to_string())
                .or_insert(0) += 1;
        }
        if let Some(ref ct) = row.certification_track {
            *summary.certification_track.entry(ct.clone()).or_insert(0) += 1;
        } else {
            *summary
                .certification_track
                .entry("unset".to_string())
                .or_insert(0) += 1;
        }
    }
    summary
}

fn resolve_format(value: Option<&str>, default_format: &str) -> String {
    value.unwrap_or(default_format).to_string()
}

fn ensure_safe_relative(label: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(format!("{label} must be a relative path: {value}"));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("{label} must not contain '..': {value}"));
    }
    Ok(())
}

fn output_reports(format: &str, reports: &[ValidationReport], atf_summary: &AtfSummary) {
    let messages = collect_messages(reports);
    match format {
        "json" => {
            let body = ValidateJsonBody {
                messages,
                atf_summary,
            };
            let payload =
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{\"messages\":[]}".into());
            println!("{payload}");
        }
        _ => print_human_validation_lines(&messages),
    }
}

fn print_human_validation_lines(messages: &[OutputMessage]) {
    for message in messages {
        println!(
            "{}:{}:{}: [{}] {}",
            message.file, message.line, message.col, message.level, message.message
        );
    }
}

fn collect_messages(reports: &[ValidationReport]) -> Vec<OutputMessage> {
    let mut messages = Vec::new();
    for report in reports {
        for error in &report.errors {
            messages.push(OutputMessage {
                file: report.path.clone(),
                line: 1,
                col: 1,
                level: "ERROR".to_string(),
                message: error.clone(),
            });
        }
        for warning in &report.warnings {
            messages.push(OutputMessage {
                file: report.path.clone(),
                line: 1,
                col: 1,
                level: "WARN".to_string(),
                message: warning.clone(),
            });
        }
    }
    messages
}
