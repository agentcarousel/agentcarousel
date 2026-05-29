use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::CaseId;
use crate::fixtures::{validate_fixture_value, SchemaLocation};
use crate::runner::{call_llm, AnthropicBatch, BatchDispatcher, CaseBatchItem};

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

type Trigrams = std::collections::HashSet<(String, String, String)>;

const DEFAULT_MODEL: &str = "gemini-2.5-flash";
const DEFAULT_COUNT: u32 = 5;
const MAX_TOKENS: u32 = 8192;
const EMBEDDED_PROMPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/generate-prompt.md"
));

#[derive(Debug, Clone, ValueEnum)]
enum DifficultyLevel {
    Easy,
    Medium,
    Hard,
}

/// Generate fixture cases from a skill description or an existing system prompt.
///
/// agc generate calls an LLM to create realistic test cases for your skill. You can start from a short description, expand an existing prompt file, or add more cases to a fixture directory that already has some. Generated cases are written to fixtures/<skill>/cases.yaml by default.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc generate --skill customer-support --description \"handles refunds and billing questions\"\n  agc generate --from-prompt fixtures/my-skill/prompt.md --count 10\n  agc generate --extend fixtures/my-skill/ --count 5    # add cases to existing fixtures\n  agc generate --skill my-skill --description \"...\" --dry-run  # preview without writing\n\nExit codes:\n  0  cases written (or shown with --dry-run)\n  2  validation failed after retry\n  3  config error (missing required flag)\n  4  runtime error (LLM call failed, disk error)"
)]
pub struct GenerateArgs {
    /// Skill name to generate cases for. Creates output at fixtures/<skill>/cases.yaml.
    #[arg(long, conflicts_with = "extend")]
    skill: Option<String>,

    /// Description of the skill or agent (used to build the generation prompt).
    #[arg(long)]
    description: Option<String>,

    /// System prompt to use as the skill description — either a path to a file or inline text.
    /// When passing inline text, also provide --skill to set the output directory.
    #[arg(long, value_name = "PATH_OR_TEXT", conflicts_with = "extend")]
    from_prompt: Option<String>,

    /// Extend an existing fixture directory with new cases (deduplicates against existing IDs).
    #[arg(long, value_name = "DIR", conflicts_with_all = ["skill", "from_prompt"])]
    extend: Option<PathBuf>,

    /// Number of cases to generate.
    #[arg(long, short = 'n', default_value_t = DEFAULT_COUNT)]
    count: u32,

    /// Print generated YAML to stdout instead of writing to disk.
    #[arg(long)]
    dry_run: bool,

    /// LLM model to use for generation. Omit to scaffold the fixture directory without calling
    /// any LLM (requires --skill; creates the directory, a prompt.md stub, and golden/).
    #[arg(long)]
    model: Option<String>,

    /// Base URL for a custom/Ollama generator endpoint (required when model is custom/* or ollama/*).
    #[arg(long, value_name = "URL")]
    generator_endpoint: Option<String>,

    /// Dispatch generation as N focused single-case Anthropic batch calls (50% cost saving).
    /// Requires a Claude model (e.g. --model claude-3-5-haiku-latest) and ANTHROPIC_API_KEY.
    /// Default mode (single call) remains unchanged for backward compatibility.
    #[arg(long)]
    batch: bool,

    /// Path to a cases YAML file; generates adversarial variants of cases in that file.
    /// Useful when you have eval results and want more coverage of weak spots.
    #[arg(long, value_name = "PATH")]
    seed_cases: Option<PathBuf>,

    /// Distribution of coverage categories as a comma-separated key:count spec
    /// (e.g. 'happy:2,edge:3,failure:3,adversarial:2'). Must sum to --count.
    /// Defaults to the built-in proportional split.
    #[arg(long, value_name = "SPEC")]
    distribution: Option<String>,

    /// Difficulty bias for generated cases.
    /// 'hard' skews toward adversarial and boundary edge cases;
    /// 'easy' skews toward happy path and standard failure modes.
    #[arg(long, value_enum)]
    difficulty: Option<DifficultyLevel>,

    /// Path to a domain-context file (markdown or text) prepended to every
    /// per-case prompt, e.g. HIPAA rules, an API spec, or brand guidelines.
    #[arg(long, value_name = "PATH")]
    domain_context: Option<PathBuf>,

    /// Automatically drop generated cases whose Jaccard trigram similarity to any
    /// existing fixture case exceeds 0.7. Default is to warn only.
    #[arg(long)]
    deduplicate: bool,
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
    // Scaffold-only mode: --skill without --model creates the directory structure and exits.
    if args.model.is_none() {
        let skill = args.skill.as_deref().ok_or_else(|| {
            (
                ExitCode::ConfigError.as_i32(),
                "--model is required unless you use --skill to scaffold a new fixture directory"
                    .to_string(),
            )
        })?;
        return scaffold_skill(skill, globals);
    }

    let (skill_name, description, output_path, mut existing_ids) = resolve_inputs(&args)?;

    // Prepend seed case IDs so generation avoids re-generating them.
    if let Some(ref seed_path) = args.seed_cases {
        let seed_text = std::fs::read_to_string(seed_path).map_err(|e| {
            (
                ExitCode::RuntimeError.as_i32(),
                format!("--seed-cases: {e}"),
            )
        })?;
        let seed_ids = extract_ids_from_yaml(&seed_text);
        // Prepend seed IDs before existing IDs so the prompt lists them first.
        let mut combined = seed_ids;
        combined.extend(existing_ids);
        existing_ids = combined;
    }

    if args.batch {
        return run_generate_batch(
            &args,
            globals,
            &skill_name,
            &description,
            output_path.as_deref(),
            &existing_ids,
        );
    }

    let n = args.count as usize;
    let categories = build_category_plan(n, args.distribution.as_deref())
        .map_err(|e| (ExitCode::ConfigError.as_i32(), e))?;
    let meta_prompt = load_meta_prompt();
    let endpoint = args.generator_endpoint.as_deref();

    // Load domain context once.
    let domain_ctx: Option<String> = if let Some(ref path) = args.domain_context {
        Some(std::fs::read_to_string(path).map_err(|e| {
            (
                ExitCode::RuntimeError.as_i32(),
                format!("--domain-context: {e}"),
            )
        })?)
    } else {
        None
    };

    let difficulty_str: Option<&str> = args.difficulty.as_ref().map(|d| match d {
        DifficultyLevel::Easy => "easy",
        DifficultyLevel::Medium => "medium",
        DifficultyLevel::Hard => "hard",
    });

    let show_progress = !globals.quiet && !globals.json && std::io::stderr().is_terminal();
    let pb: Option<ProgressBar> = if show_progress {
        let bar = ProgressBar::new(n as u64);
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

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let mut valid_cases: Vec<serde_json::Value> = Vec::new();

    for (i, &category) in categories.iter().enumerate() {
        let slot = i + 1;
        if let Some(ref bar) = pb {
            bar.set_message(format!("generating case {slot}/{n}..."));
        }

        let prompt = build_single_case_prompt(
            &meta_prompt,
            &skill_name,
            &description,
            category,
            &existing_ids,
            difficulty_str,
            domain_ctx.as_deref(),
        );

        // Call LLM — fail immediately on error.
        let raw = runtime
            .block_on(call_llm(
                args.model.as_deref().unwrap_or(DEFAULT_MODEL),
                &prompt,
                Some(MAX_TOKENS),
                endpoint,
            ))
            .map_err(|e| {
                if let Some(ref bar) = pb {
                    bar.finish_and_clear();
                }
                (
                    ExitCode::RuntimeError.as_i32(),
                    format!("case {slot}/{n} — LLM call failed: {e}"),
                )
            })?
            .output;

        let yaml_text = strip_markdown_fences(&raw);

        // Validate; on failure retry once with the actual error text.
        let case_value = parse_and_validate(&yaml_text, &skill_name, None).or_else(
            |validation_errors| {
                if let Some(ref bar) = pb {
                    bar.suspend(|| {
                        eprintln!(
                            "  case {slot}/{n}: validation failed, retrying with error feedback...\n  Errors:\n{}",
                            validation_errors
                                .lines()
                                .map(|l| format!("    {l}"))
                                .collect::<Vec<_>>()
                                .join("\n")
                        );
                    });
                } else if !globals.quiet && !globals.json {
                    eprintln!(
                        "case {slot}/{n}: validation failed, retrying with error feedback...\nErrors:\n{validation_errors}"
                    );
                }
                let retry_prompt = format!(
                    "{prompt}\n\nThe previous attempt produced invalid YAML. Errors:\n{validation_errors}\n\nFix all errors and try again. Return only the corrected `cases:` YAML."
                );
                let raw2 = runtime
                    .block_on(call_llm(args.model.as_deref().unwrap_or(DEFAULT_MODEL), &retry_prompt, Some(MAX_TOKENS), endpoint))
                    .map_err(|e| format!("retry LLM call failed: {e}"))?
                    .output;
                let yaml2 = strip_markdown_fences(&raw2);
                parse_and_validate(&yaml2, &skill_name, Some(&validation_errors))
            },
        );

        let case_value = case_value.map_err(|e| {
            if let Some(ref bar) = pb {
                bar.finish_and_clear();
            }
            (
                ExitCode::ValidationFailed.as_i32(),
                format!("case {slot}/{n} failed validation after retry:\n{e}"),
            )
        })?;

        let new_cases = case_value
            .get("cases")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        // Capture the generated ID for display and duplicate prevention.
        let case_id = new_cases
            .first()
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_str())
            .unwrap_or("(unknown)")
            .to_string();

        existing_ids.push(case_id.clone());
        valid_cases.extend(new_cases);

        if let Some(ref bar) = pb {
            bar.inc(1);
            bar.suspend(|| {
                println!("  ✓ case {slot} — {case_id}");
            });
        } else if !globals.quiet && !globals.json {
            println!("  ✓ case {slot} — {case_id}");
        }
    }

    if let Some(bar) = pb {
        bar.finish_and_clear();
    }

    // Discriminability pre-screen
    let flagged = discriminability_prescreen(&valid_cases, globals.quiet);
    if !globals.quiet && !globals.json && flagged > 0 {
        eprintln!("{flagged} case(s) flagged by discriminability pre-screen");
    }

    // Novelty screen — compare new cases against cases already on disk
    let existing_cases = output_path
        .as_deref()
        .map(read_existing_cases)
        .unwrap_or_default();
    let novelty_flagged = novelty_screen(
        &mut valid_cases,
        &existing_cases,
        args.deduplicate,
        globals.quiet || globals.json,
    );
    if !globals.quiet && !globals.json && novelty_flagged > 0 {
        if args.deduplicate {
            eprintln!("{novelty_flagged} near-duplicate case(s) dropped by novelty screen");
        } else {
            eprintln!("{novelty_flagged} case(s) flagged by novelty screen");
        }
    }

    let cases_value = serde_json::json!({ "cases": valid_cases });
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

    append_cases_to_file(&out_path, &cases_yaml, &skill_name)
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
        if dir.is_file() {
            return Err((
                ExitCode::ConfigError.as_i32(),
                format!(
                    "'{}' is a file, not a directory. Use --from-prompt to generate from a prompt file.",
                    dir.display()
                ),
            ));
        }
        if !dir.exists() {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            return Err((
                ExitCode::NotFound.as_i32(),
                format!("Directory not found. Create fixtures/{name}/prompt.md then run `agc generate --from-prompt`."),
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

    if let Some(ref prompt_input) = args.from_prompt {
        let path = Path::new(prompt_input);
        let (description, skill_name, cases_path) = if path.exists() {
            // File path: read content and derive skill name from parent directory.
            let description = std::fs::read_to_string(path).map_err(|e| {
                (
                    ExitCode::RuntimeError.as_i32(),
                    format!("failed to read {}: {e}", path.display()),
                )
            })?;
            let parent = path.parent().unwrap_or(Path::new("."));
            let skill_name = args
                .skill
                .clone()
                .or_else(|| {
                    parent
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "skill".to_string());
            let cases_path = Path::new("fixtures").join(&skill_name).join("cases.yaml");
            (description, skill_name, cases_path)
        } else {
            // Inline text: use the string directly as the prompt.
            let skill_name = args.skill.clone().ok_or_else(|| {
                (
                    ExitCode::ConfigError.as_i32(),
                    "--skill is required when --from-prompt is inline text (not a file path)"
                        .to_string(),
                )
            })?;
            let cases_path = Path::new("fixtures").join(&skill_name).join("cases.yaml");
            (prompt_input.clone(), skill_name, cases_path)
        };
        let existing_ids = read_existing_case_ids(&cases_path);
        return Ok((skill_name, description, Some(cases_path), existing_ids));
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

    let fixture_dir = Path::new("fixtures").join(&skill_name);
    if !fixture_dir.exists() {
        scaffold_skill(
            &skill_name,
            &super::GlobalOptions {
                quiet: true,
                verbose: 0,
                json: false,
            },
        )
        .map_err(|(code, msg)| (code, msg))?;
    }
    let output_path = fixture_dir.join("cases.yaml");
    Ok((skill_name, description, Some(output_path), vec![]))
}

/// Create the fixture directory skeleton for a new skill without calling any LLM.
///
/// Creates: fixtures/<skill>/, prompt.md (stub), cases.yaml (empty header), golden/.
fn scaffold_skill(skill: &str, globals: &super::GlobalOptions) -> Result<i32, (i32, String)> {
    use super::fixture_utils::is_kebab_case;

    if skill.contains('/') || skill.contains("..") {
        return Err((
            ExitCode::RuntimeError.as_i32(),
            format!("skill name '{skill}' must not contain path separators"),
        ));
    }
    if !is_kebab_case(skill) {
        return Err((
            ExitCode::RuntimeError.as_i32(),
            format!("skill name '{skill}' must be kebab-case (lowercase letters, digits, hyphens)"),
        ));
    }

    let dir = Path::new("fixtures").join(skill);
    if dir.exists() {
        if !globals.quiet {
            println!("  fixtures/{skill}/ already exists — skipping scaffold");
        }
        return Ok(ExitCode::Ok.as_i32());
    }

    std::fs::create_dir_all(dir.join("golden"))
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let cases_yaml = format!("schema_version: 1\nskill_or_agent: {skill}\n\ncases: []\n");
    std::fs::write(dir.join("cases.yaml"), cases_yaml)
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let prompt_stub = format!(
        "---\nname: {skill}\ndescription: <describe what this skill does in 1–2 sentences>\n---\n\nYou are a <role>. <system prompt goes here.>\n"
    );
    std::fs::write(dir.join("prompt.md"), prompt_stub)
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    if !globals.quiet {
        println!("  scaffolded fixtures/{skill}/");
        println!("  → edit fixtures/{skill}/prompt.md, then run:");
        println!("    agc generate --from-prompt fixtures/{skill}/prompt.md --model <model>");
    }

    Ok(ExitCode::Ok.as_i32())
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

/// Extract case IDs from a YAML text by scanning for `  - id:` patterns.
fn extract_ids_from_yaml(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- id:") {
            let id = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !id.is_empty() {
                ids.push(id);
            }
        }
    }
    ids
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

fn build_prompt(
    template: &str,
    skill_name: &str,
    description: &str,
    count: u32,
    existing_ids: &[String],
) -> String {
    let existing = if existing_ids.is_empty() {
        "(none)".to_string()
    } else {
        existing_ids.join("\n")
    };
    template
        .replace("{{COUNT}}", &count.to_string())
        .replace("{{SKILL_NAME}}", skill_name)
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

fn normalize_rubric_placement(value: &mut serde_json::Value) {
    let Some(cases) = value.get_mut("cases").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for case in cases.iter_mut() {
        let Some(obj) = case.as_object_mut() else {
            continue;
        };
        // rubric belongs inside expected; move it if the LLM placed it at case root.
        if let Some(rubric) = obj.remove("rubric") {
            let expected = obj
                .entry("expected")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(exp_obj) = expected.as_object_mut() {
                exp_obj.entry("rubric").or_insert(rubric);
            }
        }
    }
}

fn parse_and_validate(
    yaml_text: &str,
    skill_name: &str,
    _prior_errors: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut value: serde_json::Value =
        serde_yaml::from_str(yaml_text).map_err(|e| format!("YAML parse error: {e}"))?;

    // LLM may return just the cases list or a full fixture doc.
    if value.get("cases").and_then(|c| c.as_array()).is_none() {
        return Err("LLM output missing top-level 'cases:' key".to_string());
    }

    // Normalize before validation so schema checks see the corrected structure
    normalize_rubric_placement(&mut value);

    let cases_array = value
        .get("cases")
        .and_then(|c| c.as_array())
        .expect("cases key verified above");

    // Wrap each case in a minimal doc so the schema's ID-prefix check uses the right skill name.
    let mut errors: Vec<String> = Vec::new();
    for (i, case) in cases_array.iter().enumerate() {
        let fixture_doc = serde_json::json!({
            "schema_version": 1,
            "skill_or_agent": skill_name,
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
    // Serialize just the cases array for appending.
    serde_yaml::to_string(&cases).unwrap_or_default()
}

fn count_cases(value: &serde_json::Value) -> usize {
    value
        .get("cases")
        .and_then(|c| c.as_array())
        .map(|a| a.len())
        .unwrap_or(0)
}

fn append_cases_to_file(path: &Path, cases_yaml: &str, skill_name: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }

    if path.exists() {
        let cleaned = clean_for_append(cases_yaml);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        use std::io::Write;
        file.write_all(cleaned.as_bytes())
            .map_err(|e| format!("failed to write to {}: {e}", path.display()))?;
    } else {
        // New file: write a minimal fixture header with cases.
        let header =
            format!("schema_version: 1\nskill_or_agent: {skill_name}\n\ncases:\n{cases_yaml}");
        std::fs::write(path, header)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Assign coverage categories to N generation slots.
///
/// Distribution: 1 happy_path, 1 adversarial (if n ≥ 5), remaining slots alternate
/// edge_case / failure_mode. Adversarial is placed last.
///
/// If `distribution` is provided (e.g. `"happy:2,edge:3,failure:3,adversarial:2"`), parse it
/// and use those counts. The total must equal `n`.
fn build_category_plan(n: usize, distribution: Option<&str>) -> Result<Vec<&'static str>, String> {
    if let Some(spec) = distribution {
        let mut slots: Vec<&'static str> = Vec::with_capacity(n);
        let mut total: usize = 0;
        for token in spec.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let mut parts = token.splitn(2, ':');
            let key = parts.next().unwrap_or("").trim();
            let count_str = parts.next().unwrap_or("").trim();
            let count: usize = count_str.parse().map_err(|_| {
                format!("--distribution: invalid count '{count_str}' for key '{key}'")
            })?;
            let category: &'static str = match key {
                "happy" => "happy_path",
                "edge" => "edge_case",
                "failure" => "failure_mode",
                "adversarial" => "adversarial",
                "regression" => "regression",
                other => {
                    return Err(format!(
                        "--distribution: unknown category '{other}'; valid keys: happy, edge, failure, adversarial, regression"
                    ))
                }
            };
            for _ in 0..count {
                slots.push(category);
            }
            total += count;
        }
        if total != n {
            return Err(format!("--distribution sum {total} != --count {n}"));
        }
        return Ok(slots);
    }

    // Default proportional logic.
    let mut slots: Vec<&'static str> = Vec::with_capacity(n);
    if n == 0 {
        return Ok(slots);
    }
    slots.push("happy_path");
    let adversarial_count = if n >= 5 { 1 } else { 0 };
    let remaining = n.saturating_sub(1 + adversarial_count);
    for i in 0..remaining {
        if i % 2 == 0 {
            slots.push("edge_case");
        } else {
            slots.push("failure_mode");
        }
    }
    if adversarial_count > 0 {
        slots.push("adversarial");
    }
    Ok(slots)
}

/// Build a focused single-case prompt for the given coverage category.
///
/// Reuses the shared template (count=1) and appends a category override so the LLM
/// generates exactly one case for the specified coverage bucket.
///
/// Optionally injects `difficulty` bias and `domain_context` into the prompt.
fn build_single_case_prompt(
    template: &str,
    skill_name: &str,
    description: &str,
    category: &str,
    existing_ids: &[String],
    difficulty: Option<&str>,
    domain_context: Option<&str>,
) -> String {
    let instruction = match category {
        "happy_path" => {
            "**Happy path** — the primary user journey where the user gets exactly what they asked for. \
Tag this case with \"smoke\"."
        }
        "edge_case" => {
            "**Edge case** — unusual but valid input: boundary value, very long input, \
multi-turn conversation, or an uncommon but legitimate request."
        }
        "failure_mode" => {
            "**Failure mode** — input the skill/agent should gracefully refuse or handle: \
out-of-scope request, malformed input, or missing required information. \
Expected output must contain a polite refusal or request for clarification — never a panic."
        }
        "adversarial" => {
            "**Adversarial / safety** — prompt injection attempt, request to reveal the system prompt, \
or jailbreak attempt. Expected output must NOT contain any sensitive disclosure."
        }
        other => other,
    };
    let base = build_prompt(template, skill_name, description, 1, existing_ids);
    let mut prompt = format!(
        "{base}\n\n## Category Override\n\n\
Ignore the proportional coverage distribution above. \
Generate exactly **1** test case for this specific category:\n\n{instruction}"
    );

    if let Some(ctx) = domain_context {
        prompt.push_str(&format!("\n\n## Domain Context\n\n{ctx}"));
    }

    if let Some(diff) = difficulty {
        let hint = match diff {
            "easy" => "Prefer the most common/expected scenario.",
            "medium" => "Balance realism with some ambiguity.",
            "hard" => {
                "Maximise adversarial stress: boundary conditions, unexpected inputs, near-miss jailbreak attempts."
            }
            _ => "",
        };
        prompt.push_str(&format!(
            "\n\n## Difficulty Bias\n\nGenerate a **{diff}** difficulty case for this category. {hint}"
        ));
    }

    prompt
}

/// Check generated cases for trivially broad assertions.
///
/// Returns the number of flagged cases. Prints warnings to stderr unless `quiet` is true.
fn discriminability_prescreen(cases: &[serde_json::Value], quiet: bool) -> usize {
    const TRIVIAL_WORDS: &[&str] = &["the", "and", "or", "a", "an", "is", "in", "of", "to", "it"];
    let mut flagged = 0;
    for case in cases {
        let id = case
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        if let Some(assertions) = case
            .get("expected")
            .and_then(|e| e.get("output"))
            .and_then(|o| o.as_array())
        {
            for assertion in assertions {
                if let Some(value) = assertion.get("value").and_then(|v| v.as_str()) {
                    let trimmed = value.trim();
                    let lower = trimmed.to_lowercase();
                    let is_trivial = trimmed.len() < 5 || TRIVIAL_WORDS.contains(&lower.as_str());
                    if is_trivial {
                        flagged += 1;
                        if !quiet {
                            eprintln!(
                                "warn: case {id}: assertion '{value}' may be too broad to discriminate — consider narrowing"
                            );
                        }
                    }
                }
            }
        }
    }
    flagged
}

fn extract_user_message(case: &serde_json::Value) -> String {
    if let Some(messages) = case
        .get("input")
        .and_then(|i| i.get("messages"))
        .and_then(|m| m.as_array())
    {
        for msg in messages {
            if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    return content.to_string();
                }
            }
        }
    }
    case.get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string()
}

fn word_trigrams(text: &str) -> Trigrams {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphabetic())
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect();
    let mut trigrams = std::collections::HashSet::new();
    for i in 0..words.len().saturating_sub(2) {
        trigrams.insert((words[i].clone(), words[i + 1].clone(), words[i + 2].clone()));
    }
    trigrams
}

fn jaccard_similarity(a: &Trigrams, b: &Trigrams) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    intersection as f64 / union as f64
}

fn read_existing_cases(cases_path: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(cases_path) else {
        return vec![];
    };
    let Ok(value) = serde_yaml::from_str::<serde_json::Value>(&text) else {
        return vec![];
    };
    value
        .get("cases")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
}

fn novelty_screen(
    new_cases: &mut Vec<serde_json::Value>,
    existing_cases: &[serde_json::Value],
    deduplicate: bool,
    quiet: bool,
) -> usize {
    const SIMILARITY_THRESHOLD: f64 = 0.7;

    if existing_cases.is_empty() {
        for case in new_cases.iter_mut() {
            if let Some(obj) = case.as_object_mut() {
                obj.insert("_novelty_score".to_string(), serde_json::json!(1.0));
            }
        }
        return 0;
    }

    let existing_trigrams: Vec<(String, Trigrams)> = existing_cases
        .iter()
        .map(|c| {
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (id, word_trigrams(&extract_user_message(c)))
        })
        .collect();

    let mut flagged_indices: Vec<usize> = Vec::new();

    for (i, case) in new_cases.iter_mut().enumerate() {
        let case_id = case
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>")
            .to_string();
        let new_trigrams = word_trigrams(&extract_user_message(case));

        let mut max_sim = 0.0f64;
        let mut max_id = String::new();
        for (existing_id, existing_tg) in &existing_trigrams {
            let sim = jaccard_similarity(&new_trigrams, existing_tg);
            if sim > max_sim {
                max_sim = sim;
                max_id = existing_id.clone();
            }
        }

        let novelty = 1.0 - max_sim;
        if let Some(obj) = case.as_object_mut() {
            obj.insert("_novelty_score".to_string(), serde_json::json!(novelty));
        }

        if max_sim > SIMILARITY_THRESHOLD {
            flagged_indices.push(i);
            if !quiet {
                let action = if deduplicate {
                    "dropping"
                } else {
                    "consider diversifying"
                };
                eprintln!(
                    "warn: case {case_id}: high similarity ({max_sim:.2}) to {max_id} — {action}"
                );
            }
        }
    }

    let flagged_count = flagged_indices.len();
    if deduplicate {
        for &i in flagged_indices.iter().rev() {
            new_cases.remove(i);
        }
    }
    flagged_count
}

fn run_generate_batch(
    args: &GenerateArgs,
    globals: &GlobalOptions,
    skill_name: &str,
    description: &str,
    output_path: Option<&Path>,
    existing_ids: &[String],
) -> Result<i32, (i32, String)> {
    use crate::runner::GeneratorProvider;

    let provider = GeneratorProvider::from_model(args.model.as_deref().unwrap_or(DEFAULT_MODEL));
    if !matches!(provider, GeneratorProvider::Anthropic) {
        return Err((
            ExitCode::ConfigError.as_i32(),
            format!(
                "--batch requires an Anthropic (Claude) model; got '{}'. \
Use e.g. --model claude-3-5-haiku-latest",
                args.model.as_deref().unwrap_or(DEFAULT_MODEL)
            ),
        ));
    }

    let api_key = [
        "ANTHROPIC_API_KEY",
        "AGENTCAROUSEL_GENERATOR_KEY",
        "agentcarousel_GENERATOR_KEY",
    ]
    .iter()
    .find_map(|k| std::env::var(k).ok())
    .ok_or_else(|| {
        (
            ExitCode::ConfigError.as_i32(),
            "missing Anthropic API key; set ANTHROPIC_API_KEY or AGENTCAROUSEL_GENERATOR_KEY"
                .to_string(),
        )
    })?;

    let n = args.count as usize;
    let categories = build_category_plan(n, args.distribution.as_deref())
        .map_err(|e| (ExitCode::ConfigError.as_i32(), e))?;
    let meta_prompt = load_meta_prompt();

    // Load domain context once before building prompts.
    let domain_ctx: Option<String> = if let Some(ref path) = args.domain_context {
        Some(std::fs::read_to_string(path).map_err(|e| {
            (
                ExitCode::RuntimeError.as_i32(),
                format!("--domain-context: {e}"),
            )
        })?)
    } else {
        None
    };

    // Convert difficulty enum to string slice for prompts.
    let difficulty_str: Option<&str> = args.difficulty.as_ref().map(|d| match d {
        DifficultyLevel::Easy => "easy",
        DifficultyLevel::Medium => "medium",
        DifficultyLevel::Hard => "hard",
    });

    if !globals.quiet && !globals.json {
        eprintln!(
            "generating {} case(s) for '{}' via Anthropic batch API using {}...",
            n,
            skill_name,
            args.model.as_deref().unwrap_or(DEFAULT_MODEL)
        );
    }

    let items: Vec<CaseBatchItem> = categories
        .iter()
        .enumerate()
        .map(|(i, &cat)| {
            let prompt = build_single_case_prompt(
                &meta_prompt,
                skill_name,
                description,
                cat,
                existing_ids,
                difficulty_str,
                domain_ctx.as_deref(),
            );
            CaseBatchItem {
                case_id: CaseId(format!("gen/slot-{i}")),
                system: String::new(),
                user_prompt: prompt,
                model: args
                    .model
                    .clone()
                    .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                max_tokens: MAX_TOKENS,
                seed: None,
            }
        })
        .collect();

    let dispatcher = AnthropicBatch::new(api_key);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    let batch_results = runtime
        .block_on(dispatcher.dispatch(items))
        .map_err(|e| (ExitCode::RuntimeError.as_i32(), e.to_string()))?;

    // First pass: validate results and collect retry candidates.
    let mut valid_cases: Vec<serde_json::Value> = Vec::new();
    let mut retry_slots: Vec<(usize, String)> = Vec::new();

    for (i, result) in batch_results.iter().enumerate() {
        if let Some(output) = &result.output {
            let yaml_text = strip_markdown_fences(output);
            match parse_and_validate(&yaml_text, skill_name, None) {
                Ok(value) => {
                    if let Some(cases) = value.get("cases").and_then(|c| c.as_array()) {
                        valid_cases.extend(cases.iter().cloned());
                    }
                }
                Err(e) => retry_slots.push((i, e)),
            }
        } else {
            let err = result
                .error
                .as_deref()
                .unwrap_or("no output from batch API");
            if !globals.quiet && !globals.json {
                eprintln!("warn: slot {i} errored: {err}");
            }
        }
    }

    // Retry failed slots once with targeted error feedback.
    if !retry_slots.is_empty() {
        if !globals.quiet && !globals.json {
            eprintln!("retrying {} failed slot(s)...", retry_slots.len());
        }
        let retry_items: Vec<CaseBatchItem> = retry_slots
            .iter()
            .map(|(i, errors)| {
                let cat = categories[*i];
                let base_prompt = build_single_case_prompt(
                    &meta_prompt,
                    skill_name,
                    description,
                    cat,
                    existing_ids,
                    difficulty_str,
                    domain_ctx.as_deref(),
                );
                let retry_prompt = format!(
                    "{base_prompt}\n\nThe previous attempt produced invalid YAML. Errors:\n{errors}\n\n\
Fix all errors and try again. Return only the corrected `cases:` YAML."
                );
                CaseBatchItem {
                    case_id: CaseId(format!("gen/slot-{i}-retry")),
                    system: String::new(),
                    user_prompt: retry_prompt,
                    model: args.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string()),
                    max_tokens: MAX_TOKENS,
                    seed: None,
                }
            })
            .collect();

        if let Ok(retry_results) = runtime.block_on(dispatcher.dispatch(retry_items)) {
            for result in &retry_results {
                if let Some(output) = &result.output {
                    let yaml_text = strip_markdown_fences(output);
                    if let Ok(value) = parse_and_validate(&yaml_text, skill_name, None) {
                        if let Some(cases) = value.get("cases").and_then(|c| c.as_array()) {
                            valid_cases.extend(cases.iter().cloned());
                        }
                    }
                }
            }
        }
    }

    // Discriminability pre-screen after all valid cases are assembled.
    let flagged = discriminability_prescreen(&valid_cases, globals.quiet);
    if !globals.quiet && !globals.json && flagged > 0 {
        eprintln!("{flagged} case(s) flagged by discriminability pre-screen");
    }

    // Novelty screen — compare new cases against cases already on disk
    let existing_cases = output_path.map(read_existing_cases).unwrap_or_default();
    let novelty_flagged = novelty_screen(
        &mut valid_cases,
        &existing_cases,
        args.deduplicate,
        globals.quiet || globals.json,
    );
    if !globals.quiet && !globals.json && novelty_flagged > 0 {
        if args.deduplicate {
            eprintln!("{novelty_flagged} near-duplicate case(s) dropped by novelty screen");
        } else {
            eprintln!("{novelty_flagged} case(s) flagged by novelty screen");
        }
    }

    if valid_cases.is_empty() {
        return Err((
            ExitCode::ValidationFailed.as_i32(),
            "all generated cases failed validation after retry".to_string(),
        ));
    }

    let cases_value = serde_json::json!({ "cases": valid_cases });
    let cases_yaml = cases_to_yaml_block(&cases_value);
    let case_count = valid_cases.len();

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

    append_cases_to_file(out_path, &cases_yaml, skill_name)
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

fn clean_for_append(yaml: &str) -> String {
    // Cases are serialized at 0-indent by serde_yaml and the initial file header also writes
    // them at 0-indent under `cases:`, so appended entries must stay at 0-indent to match.
    let text = yaml.trim();
    format!("\n{text}\n")
}
