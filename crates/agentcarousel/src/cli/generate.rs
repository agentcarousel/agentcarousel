use clap::Parser;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::fixtures::{validate_fixture_value, SchemaLocation};
use crate::runner::call_llm;

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const DEFAULT_COUNT: u8 = 5;
const MAX_TOKENS: u32 = 8192;
const EMBEDDED_PROMPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/generate-prompt.md"
));

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc generate --skill customer-support --description \"handles refunds\"\n  agc generate --from-prompt fixtures/my-skill/prompt.md --count 10\n  agc generate --extend fixtures/my-skill/ --count 5\n  agc generate --skill my-skill --description \"...\" --dry-run --json"
)]
pub struct GenerateArgs {
    /// Skill name to generate cases for. Creates output at fixtures/<skill>/cases.yaml.
    #[arg(long, conflicts_with_all = ["from_prompt", "extend"])]
    skill: Option<String>,

    /// Description of the skill or agent (used to build the generation prompt).
    #[arg(long)]
    description: Option<String>,

    /// Path to an existing system prompt file to use as the skill description.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["skill", "extend"])]
    from_prompt: Option<PathBuf>,

    /// Extend an existing fixture directory with new cases (deduplicates against existing IDs).
    #[arg(long, value_name = "DIR", conflicts_with_all = ["skill", "from_prompt"])]
    extend: Option<PathBuf>,

    /// Number of cases to generate.
    #[arg(long, short = 'n', default_value_t = DEFAULT_COUNT)]
    count: u8,

    /// Print generated YAML to stdout instead of writing to disk.
    #[arg(long)]
    dry_run: bool,

    /// LLM model to use for generation (default: gemini-2.5-flash).
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,
}

#[derive(Debug, Serialize)]
struct GenerateResult {
    cases_generated: usize,
    output_path: Option<String>,
    dry_run: bool,
}

pub fn run_generate(args: GenerateArgs, globals: &GlobalOptions) -> i32 {
    match run_generate_inner(args, globals) {
        Ok(code) => code,
        Err((code, msg)) => {
            if globals.json {
                JsonOutput::err("generate", JsonError::new("runtime_error", &msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            code
        }
    }
}

fn run_generate_inner(args: GenerateArgs, globals: &GlobalOptions) -> Result<i32, (i32, String)> {
    let (skill_name, description, output_path, existing_ids) = resolve_inputs(&args)?;

    let meta_prompt = load_meta_prompt();
    let final_prompt = build_prompt(&meta_prompt, &description, args.count, &existing_ids);

    if !globals.quiet && !globals.json {
        eprintln!(
            "generating {} case(s) for '{}' using {}...",
            args.count, skill_name, args.model
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let yaml_text = runtime
        .block_on(call_llm(&args.model, &final_prompt, Some(MAX_TOKENS)))
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e))?
        .output;

    let yaml_text = strip_markdown_fences(&yaml_text);

    let cases_value = parse_and_validate(&yaml_text, None).or_else(|validation_errors| {
        let retry_prompt = format!(
            "{final_prompt}\n\nThe previous attempt produced invalid YAML. Errors:\n{validation_errors}\n\nFix all errors and try again. Return only the corrected `cases:` YAML."
        );
        if !globals.quiet && !globals.json {
            eprintln!("validation failed, retrying with error feedback...");
        }
        let yaml_text2 = runtime
            .block_on(call_llm(&args.model, &retry_prompt, Some(MAX_TOKENS)))?
            .output;
        let yaml_text2 = strip_markdown_fences(&yaml_text2);
        parse_and_validate(&yaml_text2, Some(&validation_errors))
    });

    let cases_value = cases_value.map_err(|e| (ExitCode::ValidationFailed.as_i32(), e))?;

    let cases_yaml = cases_to_yaml_block(&cases_value);
    let case_count = count_cases(&cases_value);

    if args.dry_run {
        println!("{cases_yaml}");
        let result = GenerateResult {
            cases_generated: case_count,
            output_path: None,
            dry_run: true,
        };
        if globals.json {
            JsonOutput::ok("generate", &result).print();
        }
        return Ok(ExitCode::Ok.as_i32());
    }

    let out_path = output_path.ok_or_else(|| {
        (
            ExitCode::ConfigError.as_i32(),
            "could not determine output path".to_string(),
        )
    })?;

    append_cases_to_file(&out_path, &cases_yaml)
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e))?;

    let result = GenerateResult {
        cases_generated: case_count,
        output_path: Some(out_path.display().to_string()),
        dry_run: false,
    };

    if globals.json {
        JsonOutput::ok("generate", &result).print();
    } else {
        println!("wrote {} case(s) to {}", case_count, out_path.display());
    }

    Ok(ExitCode::Ok.as_i32())
}

#[allow(clippy::type_complexity)]
fn resolve_inputs(
    args: &GenerateArgs,
) -> Result<(String, String, Option<PathBuf>, Vec<String>), (i32, String)> {
    if let Some(ref dir) = args.extend {
        if !dir.exists() {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            return Err((
                ExitCode::NotFound.as_i32(),
                format!("Directory not found. Run 'agc init --skill {name}' first."),
            ));
        }
        let skill_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let cases_path = dir.join("cases.yaml");
        let existing_ids = read_existing_case_ids(&cases_path);
        let prompt_path = dir.join("prompt.md");
        let description = if prompt_path.exists() {
            std::fs::read_to_string(&prompt_path).unwrap_or_else(|_| skill_name.clone())
        } else {
            skill_name.clone()
        };
        return Ok((skill_name, description, Some(cases_path), existing_ids));
    }

    if let Some(ref prompt_path) = args.from_prompt {
        let description = std::fs::read_to_string(prompt_path).map_err(|e| {
            (
                ExitCode::RuntimeError.as_i32(),
                format!("failed to read {}: {e}", prompt_path.display()),
            )
        })?;
        let skill_name = prompt_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("skill")
            .to_string();
        return Ok((skill_name, description, None, vec![]));
    }

    let skill_name = args.skill.clone().ok_or_else(|| {
        (
            ExitCode::ConfigError.as_i32(),
            "one of --skill, --from-prompt, or --extend is required".to_string(),
        )
    })?;

    let description = args.description.clone().ok_or_else(|| {
        (
            ExitCode::ConfigError.as_i32(),
            "--description is required when using --skill".to_string(),
        )
    })?;

    let output_path = Path::new("fixtures").join(&skill_name).join("cases.yaml");
    Ok((skill_name, description, Some(output_path), vec![]))
}

fn read_existing_case_ids(cases_path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(cases_path) else {
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

fn load_meta_prompt() -> String {
    let disk_path = Path::new("templates/generate-prompt.md");
    if disk_path.exists() {
        if let Ok(text) = std::fs::read_to_string(disk_path) {
            return text;
        }
    }
    EMBEDDED_PROMPT.to_string()
}

fn build_prompt(template: &str, description: &str, count: u8, existing_ids: &[String]) -> String {
    let existing = if existing_ids.is_empty() {
        "(none)".to_string()
    } else {
        existing_ids.join("\n")
    };
    template
        .replace("{{COUNT}}", &count.to_string())
        .replace("{{DESCRIPTION}}", description)
        .replace("{{EXISTING_IDS}}", &existing)
}

fn strip_markdown_fences(text: &str) -> String {
    let text = text.trim();
    // Remove ```yaml or ``` fences if present
    if let Some(stripped) = text.strip_prefix("```yaml") {
        if let Some(inner) = stripped.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    if let Some(stripped) = text.strip_prefix("```") {
        if let Some(inner) = stripped.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    text.to_string()
}

fn parse_and_validate(
    yaml_text: &str,
    _prior_errors: Option<&str>,
) -> Result<serde_json::Value, String> {
    let value: serde_json::Value =
        serde_yaml::from_str(yaml_text).map_err(|e| format!("YAML parse error: {e}"))?;

    // The LLM may return just the cases list or a full fixture doc
    let cases_array = value
        .get("cases")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "LLM output missing top-level 'cases:' key".to_string())?;

    // Validate each case by wrapping in a minimal fixture doc
    let mut errors: Vec<String> = Vec::new();
    for (i, case) in cases_array.iter().enumerate() {
        let fixture_doc = serde_json::json!({
            "schema_version": 1,
            "skill_or_agent": "generated",
            "cases": [case]
        });
        match validate_fixture_value(&fixture_doc, SchemaLocation::Default) {
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

fn cases_to_yaml_block(value: &serde_json::Value) -> String {
    let cases = value
        .get("cases")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    // Re-serialise just the cases array so we can append cleanly
    serde_yaml::to_string(&cases).unwrap_or_default()
}

fn count_cases(value: &serde_json::Value) -> usize {
    value
        .get("cases")
        .and_then(|c| c.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn append_cases_to_file(path: &Path, cases_yaml: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    if path.exists() {
        // Append after removing the leading `- ` list wrapper that serde_yaml adds
        // when serializing a Vec. Strip the `cases:` prefix if present since we're
        // appending individual items into an existing `cases:` block.
        let cleaned = clean_for_append(cases_yaml);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        use std::io::Write;
        file.write_all(cleaned.as_bytes())
            .map_err(|e| format!("failed to write to {}: {e}", path.display()))?;
    } else {
        // New file — write a minimal fixture header + cases
        let header =
            format!("schema_version: 1\nskill_or_agent: generated\n\ncases:\n{cases_yaml}");
        std::fs::write(path, header)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn clean_for_append(yaml: &str) -> String {
    // serde_yaml serializes a Vec<Value> as a YAML sequence; we want the raw
    // list items (each starting with `- `) to append into an existing `cases:` block.
    // If the output already starts with `- `, it's already in item form.
    // If it's wrapped in the sequence root, strip the root.
    let text = yaml.trim();
    // Ensure we have a leading newline so appending looks clean
    format!("\n{text}\n")
}
