use clap::Parser;

use super::exit_codes::ExitCode;
use super::fixture_utils::is_kebab_case;

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Scaffold a skill fixture template (conflicts with --agent).
    #[arg(short = 's', long, conflicts_with = "agent")]
    skill: bool,
    /// Scaffold an agent fixture template (conflicts with --skill).
    #[arg(short = 'a', long, conflicts_with = "skill")]
    agent: bool,
    /// Kebab-case name for the new skill directory (`fixtures/{name}/`).
    pub name: String,
}

pub fn run_init(args: InitArgs) -> i32 {
    if !args.skill && !args.agent {
        eprintln!("error: either --skill or --agent is required");
        return ExitCode::ConfigError.as_i32();
    }

    match init_scaffold(&args.name) {
        Ok(dir) => {
            println!("created {}/", dir.display());
            println!("  cases.yaml        — test cases");
            println!("  prompt.md         — system prompt");
            println!("  bundle.manifest.json — bundle metadata");
            println!("  golden/           — golden output files");
            ExitCode::Ok.as_i32()
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::RuntimeError.as_i32()
        }
    }
}

fn init_scaffold(name: &str) -> Result<std::path::PathBuf, String> {
    let safe_name = sanitize_fixture_name(name)?;
    let skill_dir = std::path::Path::new("fixtures").join(&safe_name);

    if skill_dir.exists() {
        return Err(format!("fixture already exists: {}", skill_dir.display()));
    }

    let golden_dir = skill_dir.join("golden");
    std::fs::create_dir_all(&golden_dir)
        .map_err(|err| format!("failed to create {}: {err}", golden_dir.display()))?;

    let cases = FIXTURE_TEMPLATE.replace("<skill-or-agent-id>", &safe_name);
    std::fs::write(skill_dir.join("cases.yaml"), cases)
        .map_err(|err| format!("failed to write cases.yaml: {err}"))?;

    let prompt = format!("You are a {safe_name} skill. Describe what this skill does and what constraints it should follow.\n");
    std::fs::write(skill_dir.join("prompt.md"), prompt)
        .map_err(|err| format!("failed to write prompt.md: {err}"))?;

    let manifest = BUNDLE_MANIFEST_TEMPLATE
        .replace("<skill-or-agent-id>", &safe_name)
        .replace("<org>", "agentcarousel");
    std::fs::write(skill_dir.join("bundle.manifest.json"), manifest)
        .map_err(|err| format!("failed to write bundle.manifest.json: {err}"))?;

    Ok(skill_dir)
}

const FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/fixture-skeleton.yaml"
));

const BUNDLE_MANIFEST_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/bundle-manifest-skeleton.json"
));

fn sanitize_fixture_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("fixture name cannot be empty".to_string());
    }
    if std::path::Path::new(name).components().count() != 1 {
        return Err("fixture name must not include path separators".to_string());
    }
    if !is_kebab_case(name) {
        return Err("fixture name must be lowercase kebab-case".to_string());
    }
    Ok(name.to_string())
}
