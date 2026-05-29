mod ab;
mod audit;
mod batch_cmd;
mod bundle;
mod candidate_store;
mod candidates;
mod carousel;
mod compare;
mod completions;
mod config;
#[cfg(feature = "dashboard")]
mod dashboard;
mod doctor;
mod eval;
mod exit_codes;
mod export;
mod fixture_utils;
mod generate;
mod init;
mod lint;
mod local_config;
mod metrics;
mod optimize;
mod output;
mod pipeline;
mod promote;
mod publish;
mod registry_client;
mod report;
mod test;
mod trust_check;
mod update;
mod validate;
mod watch;

use clap::builder::styling::{AnsiColor, Color, Effects, RgbColor, Style, Styles};
use clap::{ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::CompleteEnv;
use std::io::IsTerminal;

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
    version = env!("CARGO_PKG_VERSION"),
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
    /// Emit structured JSON to stdout (auto-enabled when stdout is not a TTY).
    #[arg(long, global = true, help = "Emit structured JSON output")]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

/// Options propagated from [`Cli`] into subcommands.
pub struct GlobalOptions {
    pub quiet: bool,
    pub verbose: u8,
    pub json: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check YAML/TOML fixtures against the schema (no execution). Scans `.` when no paths given.
    Validate(validate::ValidateArgs),
    /// Run fixtures with mock generation (no API keys required).
    Test(test::TestArgs),
    /// Run evaluation with mock or live generation; optionally score with an LLM judge.
    Eval(eval::EvalArgs),
    /// Check status or collect results from an async batch job.
    Batch(batch_cmd::BatchArgs),
    /// Inspect persisted runs: list recent runs or show details of a specific run.
    Report(report::ReportArgs),
    /// Generate fixture cases for a skill using an LLM.
    Generate(generate::GenerateArgs),
    /// Serve the run-history web dashboard.
    #[cfg(feature = "dashboard")]
    Dashboard(dashboard::DashboardArgs),
    /// Scaffold a new skill or agent fixture template.
    Init(init::InitArgs),
    /// Pack, verify, or pull fixture bundles.
    Bundle(bundle::BundleArgs),
    /// Publish a bundle and its evidence to the registry.
    Publish(publish::PublishArgs),
    /// Promote golden files from a saved run; optionally submit to the registry.
    Promote(promote::PromoteArgs),
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
    /// Compute compliance metrics: injection resistance, behavioral drift, test coverage, and score calibration.
    Metrics(metrics::MetricsArgs),
    /// Compare two eval runs and gate on regressions.
    Compare(compare::CompareArgs),
    /// Run tests automatically whenever you save a fixture file.
    Watch(watch::WatchArgs),
    /// Run the same fixture suite against multiple models and get a ranked comparison.
    Carousel(carousel::CarouselArgs),
    /// Run the same fixture suite against two system prompts and get a head-to-head comparison.
    Ab(ab::AbArgs),
    /// Prompt-audit a saved run, or apply stored suggestions to prompt.md.
    Audit(audit::AuditArgs),
    /// Automated system prompt optimization loop.
    Optimize(optimize::OptimizeArgs),
    /// Skill lifecycle pipeline: onboard a new skill or improve an existing one.
    Pipeline(pipeline::PipelineArgs),
    /// List all pipeline candidate skills with their evaluation scores and metrics.
    Candidates(candidates::CandidatesArgs),
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
    let watch = c("watch");
    let carousel = c("carousel");
    let ab = c("ab");
    let generate = c("generate");
    let lint = c("lint");
    let init = c("init");
    let audit = c("audit");
    let report = c("report");
    let metrics = c("metrics");
    let compare = c("compare");
    let export = c("export");
    let bundle = c("bundle");
    let publish = c("publish");
    let promote = c("promote");
    let trust_check = c("trust-check");
    let optimize = c("optimize");
    let pipeline = c("pipeline");
    let candidates = c("candidates");
    let completions = c("completions");
    let update = c("update");
    let doctor = c("doctor");
    let help = c("help");

    #[cfg(feature = "dashboard")]
    let dashboard_line = {
        let dashboard = c("dashboard");
        format!("  {dashboard}    Serve run history, trends, and judge review in a local web UI\n")
    };
    #[cfg(not(feature = "dashboard"))]
    let dashboard_line = String::new();

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
  {carousel}     Run the same fixtures against multiple models and get a ranked comparison table
  {ab}           Run the same fixtures against two system prompts and compare head-to-head
  {watch}        Run tests automatically whenever you save a fixture file
  {generate}     Generate fixture cases for a skill using an LLM
  {optimize}     Automated system prompt optimization loop (iterative LLM-driven tuning)
  {pipeline}     Skill lifecycle pipeline: onboard a new skill or improve an existing one
  {candidates}   List pipeline candidate skills with scores, metrics, and status
  {lint}         Check fixture quality: smoke coverage, rubric weights, descriptions
  {init}         Scaffold a new skill or agent fixture template

{re}:
  {report}       List recent runs or show details of a run (to compare runs: agc compare)
  {audit}        Prompt-audit a saved run (audit run) or apply its suggestions (audit suggest)
  {metrics}      Compliance metrics: injection resistance, drift, coverage, calibration
  {compare}      Compare two eval runs and gate on regressions (CI regression gate)
  {export}       Export run(s) as signed evidence tarballs

{bu}:
  {bundle}       Pack, verify, or pull fixture bundles
  {publish}      Publish a bundle and its evidence to the registry
  {promote}      Promote golden files from a saved run; optionally submit to the registry
  {trust_check}  Check a bundle's trust state in the registry and optionally verify its attestation

{to}:
{dashboard_line}  {completions}  Print a shell completion script to stdout
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

    let stdout_is_tty = std::io::stdout().is_terminal();
    if !stdout_is_tty && std::env::args().len() == 1 {
        print_compact_help();
        return exit_codes::ExitCode::Ok.as_i32();
    }

    let matches = cli_command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    let json_mode = cli.json || !stdout_is_tty;

    let config_path: Option<&std::path::Path> = match &cli.command {
        Command::Validate(a) => a.config.as_deref(),
        Command::Test(a) => a.config.as_deref(),
        Command::Eval(a) => a.config.as_deref(),
        Command::Batch(a) => {
            if let batch_cmd::BatchCommand::Fetch { config, .. } = &a.command {
                config.as_deref()
            } else {
                None
            }
        }
        Command::Report(a) => a.config.as_deref(),
        Command::Bundle(a) => a.config.as_deref(),
        Command::Publish(a) => a.config.as_deref(),
        Command::Promote(a) => a.config.as_deref(),
        Command::TrustCheck(a) => a.config.as_deref(),
        Command::Doctor(a) => a.config.as_deref(),
        Command::Watch(a) => a.config.as_deref(),
        Command::Ab(a) => a.config.as_deref(),
        Command::Audit(a) => a.config.as_deref(),
        Command::Optimize(a) => a.config.as_deref(),
        Command::Pipeline(a) => a.config.as_deref(),
        Command::Candidates(a) => a.config.as_deref(),
        _ => None,
    };

    let mut config = match load_config(config_path) {
        Ok(config) => config,
        Err(err) => {
            if json_mode {
                output::JsonOutput::err(
                    "config",
                    output::JsonError::new("config_error", err.to_string()),
                )
                .print();
            } else {
                eprintln!("error: {err}");
            }
            return exit_codes::ExitCode::ConfigError.as_i32();
        }
    };

    let local_profile = local_config::LocalProfile::load();
    local_profile.apply_to(&mut config);

    apply_history_db_env(&config);
    if json_mode {
        console::set_colors_enabled(false);
    } else {
        apply_color_settings(&config, cli.no_color);
    }
    let globals = GlobalOptions {
        quiet: cli.quiet,
        verbose: cli.verbose,
        json: json_mode,
    };
    match cli.command {
        Command::Validate(args) => validate::run_validate(args, &config, &globals),
        Command::Test(args) => test::run_test(args, &config, &globals),
        Command::Eval(args) => eval::run_eval_command(args, &config, &globals),
        Command::Batch(args) => batch_cmd::run_batch_command(args, &config, &globals),
        Command::Report(args) => report::run_report(args, &config, &globals),
        Command::Generate(args) => generate::run_generate(args, &globals),
        #[cfg(feature = "dashboard")]
        Command::Dashboard(args) => dashboard::run_dashboard(args, &globals),
        Command::Init(args) => init::run_init(args),
        Command::Bundle(args) => bundle::run_bundle(args, &config, &globals),
        Command::Publish(args) => publish::run_publish(args, &config, &globals),
        Command::Promote(args) => promote::run_promote(args, &config, &globals),
        Command::Export(args) => export::run_export(args, &globals),
        Command::TrustCheck(args) => trust_check::run_trust_check(args, &config, &globals),
        Command::Completions(args) => completions::run_completions(args),
        Command::Update(args) => update::run_update(args),
        Command::Doctor(args) => doctor::run_doctor(args, &config),
        Command::Lint(args) => lint::run_lint(args, &globals),
        Command::Metrics(args) => metrics::run_metrics(args, &globals),
        Command::Compare(args) => compare::run_compare(args, &globals),
        Command::Watch(args) => watch::run_watch(args, &config, &globals),
        Command::Carousel(args) => carousel::run_carousel(args, &config, &globals),
        Command::Ab(args) => ab::run_ab(args, &config, &globals),
        Command::Audit(args) => audit::run_audit_command(args, &config, &globals),
        Command::Optimize(args) => optimize::run_optimize_command(args, &config, &globals),
        Command::Pipeline(args) => pipeline::run_pipeline(args, &config, &globals),
        Command::Candidates(args) => candidates::run_candidates(args, &globals),
    }
}

fn print_compact_help() {
    println!(
        "agc {} — AI agent behavioral testing\n",
        env!("CARGO_PKG_VERSION")
    );
    #[cfg(feature = "dashboard")]
    println!("COMMANDS: validate test eval carousel ab watch generate optimize pipeline candidates lint init report audit metrics export bundle publish promote trust-check compare dashboard doctor completions update\n");
    #[cfg(not(feature = "dashboard"))]
    println!("COMMANDS: validate test eval carousel ab watch generate optimize pipeline candidates lint init report audit metrics export bundle publish promote trust-check compare doctor completions update\n");
    println!("QUICK START:");
    println!("  agc init --skill my-skill");
    println!("  agc test fixtures/my-skill/");
    println!("  agc eval fixtures/my-skill/ --judge --model gemini-2.5-flash\n");
    println!("FLAGS (global): --json --quiet -v --no-color --config <path>\n");
    println!("Full help: agc <command> --help");
    println!("Docs: https://agentcarousel.com/docs");
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
