mod bundle;
mod completions;
mod config;
mod doctor;
mod eval;
mod export;
mod fixture_utils;
mod lint;
mod publish;
mod registry_client;
mod report;
mod stats;
mod test;
mod trust_check;
mod update;
mod validate;

use clap::builder::styling::{AnsiColor, Color, Effects, RgbColor, Style, Styles};
use clap::{ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::CompleteEnv;

use config::{apply_history_db_env, load_config};

fn styles() -> Styles {
    let blue = Some(Color::Rgb(RgbColor(127, 255, 212)));
    let gray = Some(Color::Rgb(RgbColor(191, 189, 182)));
    let dim = Some(Color::Rgb(RgbColor(108, 118, 128)));
    Styles::styled()
        .header(Style::new().fg_color(blue))
        .usage(Style::new().fg_color(blue))
        .literal(Style::new().fg_color(gray))
        .placeholder(Style::new().fg_color(dim))
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(Style::new().fg_color(blue))
        .invalid(AnsiColor::Yellow.on_default())
}

#[derive(Debug, Parser)]
#[command(
    name = "agentcarousel",
    version,
    about = "Validate, test, and evaluate AI agents and skills using YAML fixtures.",
    styles = styles(),
)]
pub struct Cli {
    #[arg(long, global = true, help = "Disable color output")]
    no_color: bool,
    #[arg(
        short = 'q',
        long,
        global = true,
        help = "Suppress non-essential output"
    )]
    quiet: bool,
    #[arg(short = 'v', long, action = ArgAction::Count, global = true, help = "Increase output verbosity")]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

/// Options propagated from [`Cli`] into subcommands (quiet, verbose level).
pub struct GlobalOptions {
    pub quiet: bool,
    pub verbose: u8,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check YAML/TOML fixtures against the schema (no execution). Scans `.` when no paths given.
    Validate(validate::ValidateArgs),
    /// Run fixtures with mock generation (no API keys required).
    Test(test::TestArgs),
    /// Run evaluation with mock or live generation; optionally score with an LLM judge.
    Eval(eval::EvalArgs),
    /// Inspect persisted runs: list, show details, or diff two runs.
    Report(report::ReportArgs),
    /// Scaffold a new skill or agent fixture template.
    Init(InitArgs),
    /// Pack, verify, or pull fixture bundles.
    Bundle(bundle::BundleArgs),
    /// Publish a bundle and its evidence to the registry.
    Publish(publish::PublishArgs),
    /// Export run(s) as signed evidence tarballs.
    Export(export::ExportArgs),
    /// Check a bundle's trust state in the registry and optionally verify its attestation.
    TrustCheck(trust_check::TrustCheckArgs),
    /// Print a shell completion script to stdout.
    Completions(completions::CompletionsArgs),
    /// Check for and install updates to the agentcarousel CLI.
    Update(update::UpdateArgs),
    /// Check environment, config, and fixture setup for common issues.
    Doctor(doctor::DoctorArgs),
    /// Check fixture quality beyond schema: smoke coverage, rubric weights, descriptions.
    Lint(lint::LintArgs),
    /// Show historical pass-rate trends, flakiness, and latency from run history.
    Stats(stats::StatsArgs),
}

#[derive(Debug, Parser)]
pub struct InitArgs {
    /// Scaffold a skill fixture template (conflicts with --agent).
    #[arg(short = 's', long, conflicts_with = "agent")]
    skill: bool,
    /// Scaffold an agent fixture template (conflicts with --skill).
    #[arg(short = 'a', long, conflicts_with = "skill")]
    agent: bool,
    /// Kebab-case name for the new skill directory (`fixtures/{name}/`).
    name: String,
}

fn cli_command() -> clap::Command {
    Cli::command().help_template(help_template())
}

fn help_template() -> String {
    let colors = console::colors_enabled();
    let h = |s: &str| -> String {
        if colors {
            format!("\x1b[38;2;127;255;212m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    };
    let c = |s: &str| -> String {
        if colors {
            format!("\x1b[38;2;191;189;182m{s}\x1b[0m")
        } else {
            s.to_owned()
        }
    };

    let fw = h("Fixture work");
    let re = h("Results");
    let bu = h("Bundles & registry");
    let to = h("Tooling");
    let op = h("Options");

    let validate = c("validate");
    let test = c("test");
    let eval = c("eval");
    let lint = c("lint");
    let init = c("init");
    let report = c("report");
    let stats = c("stats");
    let export = c("export");
    let bundle = c("bundle");
    let publish = c("publish");
    let trust_check = c("trust-check");
    let completions = c("completions");
    let update = c("update");
    let doctor = c("doctor");
    let help = c("help");

    format!(
        r#"{{about}}

Usage:
  agc [OPTIONS] <COMMAND>
  agc validate fixtures/customer-support/cases.yaml
  agc test fixtures/customer-support/cases.yaml --filter-tags smoke

{fw}:
  {validate}     Validate YAML/TOML fixtures against the schema (no execution); scans `.` by default
  {test}         Run fixtures with mock generation (no API keys required)
  {eval}         Run evaluation with mock or live generation; optionally score with an LLM judge
  {lint}         Check fixture quality: smoke coverage, rubric weights, descriptions
  {init}         Scaffold a new skill or agent fixture template

{re}:
  {report}       Inspect persisted runs: list, show details, or diff two runs
  {stats}        Pass-rate trends, case flakiness, and latency across run history
  {export}       Export run(s) as signed evidence tarballs

{bu}:
  {bundle}       Pack, verify, or pull fixture bundles
  {publish}      Publish a bundle and its evidence to the registry
  {trust_check}  Check a bundle's trust state in the registry and optionally verify its attestation

{to}:
  {completions}  Print a shell completion script to stdout
  {update}       Check for and install updates to the agentcarousel CLI
  {doctor}       Check environment, config, and fixture setup for common issues
  {help}         Print this message or the help of the given subcommand(s)

{op}:
{{options}}

Use "agc <COMMAND> --help" for more information about a command.
"#
    )
}

/// Parse [`std::env::args`], run the selected subcommand, and return a **process exit code**
/// (`0` = success; non-zero for validation, config, or runtime failures).
pub fn run() -> i32 {
    CompleteEnv::with_factory(cli_command).complete();
    let matches = cli_command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    let config_path: Option<&std::path::Path> = match &cli.command {
        Command::Validate(a) => a.config.as_deref(),
        Command::Test(a) => a.config.as_deref(),
        Command::Eval(a) => a.config.as_deref(),
        Command::Report(a) => a.config.as_deref(),
        Command::Bundle(a) => a.config.as_deref(),
        Command::Publish(a) => a.config.as_deref(),
        Command::TrustCheck(a) => a.config.as_deref(),
        Command::Doctor(a) => a.config.as_deref(),
        Command::Stats(a) => a.config.as_deref(),
        _ => None,
    };

    let config = match load_config(config_path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("error: {err}");
            return exit_codes::ExitCode::ConfigError.as_i32();
        }
    };

    apply_history_db_env(&config);
    apply_color_settings(&config, cli.no_color);
    let globals = GlobalOptions {
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
        Command::Publish(args) => publish::run_publish(args, &config),
        Command::Export(args) => export::run_export(args),
        Command::TrustCheck(args) => trust_check::run_trust_check(args, &config),
        Command::Completions(args) => completions::run_completions(args),
        Command::Update(args) => update::run_update(args),
        Command::Doctor(args) => doctor::run_doctor(args, &config),
        Command::Lint(args) => lint::run_lint(args, &globals),
        Command::Stats(args) => stats::run_stats(args, &config, &globals),
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
        _ => {}
    }
}

fn run_init(args: InitArgs) -> i32 {
    if !args.skill && !args.agent {
        eprintln!("error: either --skill or --agent is required");
        return exit_codes::ExitCode::ConfigError.as_i32();
    }

    match init_scaffold(&args.name) {
        Ok(dir) => {
            println!("created {}/", dir.display());
            println!("  cases.yaml        — test cases");
            println!("  prompt.md         — system prompt");
            println!("  bundle.manifest.json — bundle metadata");
            println!("  golden/           — golden output files");
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
