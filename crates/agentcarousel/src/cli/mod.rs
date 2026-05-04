//! Command-line interface built with [`clap`]: parse flags, load merged configuration from TOML
//! and environment, and dispatch to validate / test / eval / report / init / bundle / publish /
//! export / trust-check.
//!
//! The primary entrypoint for a binary is [`run`], which returns a process exit code.
//! [`Cli`] and [`GlobalOptions`] are public for testing or custom front-ends.

mod bundle;
mod config;
mod eval;
mod export;
mod fixture_utils;
mod publish;
mod registry_client;
mod report;
mod test;
mod trust_check;
mod validate;

use clap::{ArgAction, Parser, Subcommand};
use std::path::PathBuf;

use config::{apply_history_db_env, load_config};

/// Root command parser: global flags (`--config`, `--run-id`, verbosity, color) plus a subcommand.
///
/// Construct with [`Parser::parse`] or parse from iterator in tests.
#[derive(Debug, Parser)]
#[command(
    name = "agentcarousel",
    version,
    about = "Agent & skill evaluation CLI: validate, test, eval, report, init, bundle (pack/verify/pull), export, trust-check. Quick start (no API keys): agentcarousel validate fixtures/skills/cmmc-assessor.yaml && agentcarousel test fixtures/skills/cmmc-assessor.yaml fixtures/skills/terraform-sentinel-scaffold.yaml --filter-tags smoke --offline true. Use `agentcarousel SUBCOMMAND -h` for flags and examples."
)]
pub struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    run_id: Option<String>,
    #[arg(long, global = true)]
    no_color: bool,
    #[arg(short = 'q', long, global = true)]
    quiet: bool,
    #[arg(short = 'v', long, action = ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

/// Options propagated from [`Cli`] into subcommands (run id override, quiet, verbose level).
pub struct GlobalOptions {
    pub run_id: Option<String>,
    pub quiet: bool,
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate YAML/TOML fixtures (schema + rules; no execution). Example: agentcarousel validate fixtures/skills/foo.yaml. With no paths, scans `.` for fixtures (see `.agentcarousel-ignore`).
    Validate(validate::ValidateArgs),
    /// Run fixtures with mock generation only. Example: agentcarousel test fixtures/skills/foo.yaml --offline true --filter-tags smoke.
    Test(test::TestArgs),
    /// Run evaluation (mock or live; use `--judge` when the fixture needs it). Example: agentcarousel eval fixtures/skills/foo.yaml --execution-mode mock --runs 3.
    Eval(eval::EvalArgs),
    /// Inspect persisted runs (list / show / diff). Example: agentcarousel report list && agentcarousel report show RUN_ID.
    Report(report::ReportArgs),
    /// Create `fixtures/{name}.yaml` from a template. Example: agentcarousel init --skill my-new-skill.
    Init(InitArgs),
    /// Pack, verify, or pull a bundle (directory, bundle.manifest.json, .tar.gz, or registry id). Example: agentcarousel bundle verify fixtures/bundles/cmmc-assessor.
    Bundle(bundle::BundleArgs),
    /// Publish bundle + evidence to registry in one flow.
    Publish(publish::PublishArgs),
    /// Export run(s) as evidence tarballs. Examples: `export RUN_ID`, `export --last 5 --out-dir ./evidence`.
    Export(export::ExportArgs),
    /// Check registry trust state and optionally verify a signed attestation.
    TrustCheck(trust_check::TrustCheckArgs),
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Scaffold a skill fixture template (conflicts with --agent).
    #[arg(short = 's', long, conflicts_with = "agent")]
    skill: bool,
    /// Scaffold an agent fixture template (conflicts with --skill).
    #[arg(short = 'a', long, conflicts_with = "skill")]
    agent: bool,
    /// Kebab-case stem for the new file (`fixtures/{stem}.yaml`).
    name: String,
}

/// Parse [`std::env::args`], run the selected subcommand, and return a **process exit code**
/// (`0` = success; non-zero for validation, config, or runtime failures).
pub fn run() -> i32 {
    let cli = Cli::parse();
    let config = match load_config(cli.config.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("error: {err}");
            return exit_codes::ExitCode::ConfigError.as_i32();
        }
    };

    apply_history_db_env(&config);
    apply_color_settings(&config, cli.no_color);
    let globals = GlobalOptions {
        run_id: cli.run_id.clone(),
        quiet: cli.quiet,
        verbose: cli.verbose,
    };
    match cli.command {
        Command::Validate(args) => validate::run_validate(args, &config, &globals),
        Command::Test(args) => test::run_test(args, &config, &globals),
        Command::Eval(args) => eval::run_eval_command(args, &config, &globals),
        Command::Report(args) => report::run_report(args, &config),
        Command::Init(args) => run_init(args),
        Command::Bundle(args) => bundle::run_bundle(args, &config, &globals),
        Command::Publish(args) => publish::run_publish(args, &config, &globals),
        Command::Export(args) => export::run_export(args, &globals),
        Command::TrustCheck(args) => trust_check::run_trust_check(args, &config),
    }
}

fn apply_color_settings(config: &config::ResolvedConfig, no_color: bool) {
    if no_color {
        console::set_colors_enabled(false);
        return;
    }
    match config.output.color.as_str() {
        "always" => console::set_colors_enabled(true),
        "never" => console::set_colors_enabled(false),
        // "auto" and any other value: leave console on its default (TTY detection).
        "auto" => {}
        _ => {}
    }
}

fn run_init(args: InitArgs) -> i32 {
    if !args.skill && !args.agent {
        eprintln!("error: either --skill or --agent is required");
        return exit_codes::ExitCode::ConfigError.as_i32();
    }

    match init_scaffold(&args.name) {
        Ok(path) => {
            println!("created {}", path.display());
            exit_codes::ExitCode::Ok.as_i32()
        }
        Err(err) => {
            eprintln!("error: {err}");
            exit_codes::ExitCode::RuntimeError.as_i32()
        }
    }
}

fn init_scaffold(name: &str) -> Result<std::path::PathBuf, String> {
    let safe_name = sanitize_fixture_name(name)?;
    let rendered = FIXTURE_TEMPLATE.replace("<skill-or-agent-id>", &safe_name);

    let fixtures_dir = std::path::Path::new("fixtures");
    std::fs::create_dir_all(fixtures_dir)
        .map_err(|err| format!("failed to create fixtures dir: {err}"))?;

    let out_path = fixtures_dir.join(format!("{safe_name}.yaml"));
    if out_path.exists() {
        return Err(format!("fixture already exists: {}", out_path.display()));
    }

    std::fs::write(&out_path, rendered).map_err(|err| format!("failed to write fixture: {err}"))?;
    Ok(out_path)
}

// In-crate copy for `cargo package`; keep aligned with `templates/fixture-skeleton.yaml` at repo root.
const FIXTURE_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/templates/fixture-skeleton.yaml"
));

fn sanitize_fixture_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("fixture name cannot be empty".to_string());
    }
    if std::path::Path::new(name).components().count() != 1 {
        return Err("fixture name must not include path separators".to_string());
    }
    if !fixture_utils::is_kebab_case(name) {
        return Err("fixture name must be lowercase kebab-case".to_string());
    }
    Ok(name.to_string())
}

mod exit_codes {
    #[allow(dead_code)]
    #[derive(Debug, Clone, Copy)]
    pub enum ExitCode {
        Ok = 0,
        Failed = 1,
        ValidationFailed = 2,
        ConfigError = 3,
        RuntimeError = 4,
    }

    impl ExitCode {
        pub fn as_i32(self) -> i32 {
            self as i32
        }
    }
}
