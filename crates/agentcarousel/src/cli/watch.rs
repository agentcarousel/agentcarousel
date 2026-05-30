use agentcarousel_fixtures::load_fixture;
use agentcarousel_reporters::{persist_run, print_terminal, print_terminal_summary};
use agentcarousel_runner::{run_eval, run_fixtures, EvalConfig, GenerationMode, RunnerConfig};
use clap::Parser;
use console::style;
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::config::{config_hash, ResolvedConfig};
use super::exit_codes::ExitCode;
use super::fixture_utils::{
    apply_case_filter, apply_tag_filter, collect_fixture_paths, default_concurrency,
};
use super::GlobalOptions;

const DEBOUNCE_MS: u64 = 200;

/// Re-run your tests automatically every time you save a fixture file.
///
/// Leave this running in a terminal while you write fixtures. Every time you save a YAML file, agc re-runs only the cases in that file and prints the result immediately — no need to switch windows and type a command. Runs are saved to history so you can review them with agc report.
///
/// Uses mock generation by default (no API key needed). Add --eval to switch to the full eval pipeline. Press Ctrl-C to stop.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc watch fixtures/my-skill/               # re-run cases on every save (no API key needed)\n  agc watch fixtures/ --filter-tags smoke    # only re-run smoke-tagged cases\n  agc watch fixtures/my-skill/ --eval        # use the full eval pipeline instead of basic test\n  agc watch fixtures/ --timeout 60           # set a 60-second per-case timeout\n\nExit codes:\n  0  stopped normally (Ctrl-C)\n  4  could not set up the file watcher"
)]
pub struct WatchArgs {
    /// Fixture files or directories to watch (default: fixtures).
    #[arg(value_name = "PATHS", default_value = "fixtures")]
    paths: Vec<PathBuf>,
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Use the eval pipeline instead of the default test pipeline.
    #[arg(long)]
    eval: bool,
    /// Glob matched against full case ids (`skill/case-id`).
    #[arg(short = 'f', long)]
    filter: Option<String>,
    /// Comma-separated case tags to include (e.g. `smoke,fast`).
    #[arg(
        short = 'g',
        long = "filter-tags",
        value_name = "TAG",
        value_delimiter = ','
    )]
    filter_tags: Option<Vec<String>>,
    /// Maximum number of cases to run in parallel.
    #[arg(short = 'c', long)]
    concurrency: Option<usize>,
    /// Per-case timeout in seconds.
    #[arg(short = 't', long)]
    timeout: Option<u64>,
}

pub fn run_watch(args: WatchArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match RecommendedWatcher::new(tx, Config::default()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error: failed to create file watcher: {e}");
            return ExitCode::RuntimeError.as_i32();
        }
    };

    let mut registered = 0usize;
    for path in &args.paths {
        let watch_path: &Path = if path.is_file() {
            path.parent().unwrap_or(path.as_path())
        } else {
            path.as_path()
        };
        if let Err(e) = watcher.watch(watch_path, RecursiveMode::Recursive) {
            eprintln!("error: cannot watch {}: {e}", watch_path.display());
            return ExitCode::RuntimeError.as_i32();
        }
        registered += 1;
    }

    let mode_label = if args.eval { "eval" } else { "test" };
    eprintln!(
        "{} watching {} path(s) [{mode_label} mode] — Ctrl-C to quit",
        style("🎠").bold(),
        registered,
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("tokio runtime");

    let concurrency = args
        .concurrency
        .or(config.runner.concurrency)
        .or_else(default_concurrency)
        .unwrap_or(1);

    loop {
        // Block until first relevant fixture-file event.
        let mut changed: HashSet<PathBuf> = HashSet::new();
        loop {
            match rx.recv() {
                Ok(Ok(event)) => {
                    if is_relevant(&event) {
                        collect_fixtures(&event, &mut changed);
                    }
                    if !changed.is_empty() {
                        break;
                    }
                }
                Ok(Err(e)) => eprintln!("watch error: {e}"),
                Err(_) => return ExitCode::Ok.as_i32(),
            }
        }

        // Debounce: drain events for DEBOUNCE_MS before running.
        let deadline = Instant::now() + Duration::from_millis(DEBOUNCE_MS);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    if is_relevant(&event) {
                        collect_fixtures(&event, &mut changed);
                    }
                }
                Ok(Err(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return ExitCode::Ok.as_i32(),
            }
        }

        if changed.is_empty() {
            continue;
        }

        let now = chrono::Local::now().format("%H:%M:%S");
        eprintln!(
            "\n{} [{}] {} file(s) changed — running {mode_label}",
            style("→").cyan().bold(),
            now,
            changed.len(),
        );

        let changed_paths: Vec<PathBuf> = changed.into_iter().collect();
        run_once(
            &changed_paths,
            &args,
            config,
            globals,
            &runtime,
            concurrency,
        );
    }
}

fn run_once(
    paths: &[PathBuf],
    args: &WatchArgs,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
    runtime: &tokio::runtime::Runtime,
    concurrency: usize,
) {
    let fixture_paths = collect_fixture_paths(paths);
    if fixture_paths.is_empty() {
        return;
    }

    let mut fixtures = Vec::new();
    for path in fixture_paths {
        match load_fixture(&path) {
            Ok(f) => {
                let f = apply_case_filter(f, args.filter.as_deref());
                let f = apply_tag_filter(f, args.filter_tags.as_deref());
                fixtures.push(f);
            }
            Err(err) => eprintln!("  error loading {}: {err}", path.display()),
        }
    }
    if fixtures.is_empty() {
        return;
    }

    let runner_config = RunnerConfig {
        concurrency,
        timeout_secs: args.timeout.unwrap_or(config.runner.timeout_secs),
        offline: config.runner.offline,
        mock_dir: config.runner.mock_dir.clone(),
        generation_mode: GenerationMode::MockOnly,
        generator_model: Some(config.generator.model.clone()),
        generator_max_tokens: config.generator.max_tokens,
        generator_endpoint: None,
        fail_fast: false,
        mock_strict: false,
        command: "watch".to_string(),
        agentcarousel_version: env!("CARGO_PKG_VERSION").to_string(),
        config_hash: config_hash(config),
        run_id: None,
        batch_collect_id: None,
    };

    let generator_model = config.generator.model.clone();
    let mut run = if args.eval {
        let eval_config = EvalConfig {
            runner: runner_config,
            runs: 1,
            seed: 0,
            evaluator: config.eval.default_evaluator.clone(),
            judge: false,
            judge_model: None,
            judge_endpoint: None,
            judge_max_tokens: None,
            effectiveness_threshold: config.eval.effectiveness_threshold,
            progress: false,
        };
        runtime.block_on(run_eval(fixtures, eval_config))
    } else {
        runtime.block_on(run_fixtures(fixtures, runner_config))
    };
    run.summary.generator_model = Some(generator_model);
    run.summary.command_line = Some(std::env::args().collect::<Vec<_>>().join(" "));

    let _ = persist_run(&run);

    if globals.quiet {
        print_terminal_summary(&run);
    } else {
        print_terminal(&run, globals.verbose > 0);
    }
}

fn is_relevant(event: &notify::Event) -> bool {
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
}

fn is_fixture_ext(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("yaml") | Some("yml") | Some("toml")
    )
}

fn collect_fixtures(event: &notify::Event, out: &mut HashSet<PathBuf>) {
    for path in &event.paths {
        if path.is_file() && is_fixture_ext(path) {
            out.insert(path.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_fixture_ext;
    use std::path::Path;

    #[test]
    fn fixture_ext_accepts_yaml_yml_toml() {
        assert!(is_fixture_ext(Path::new("cases.yaml")));
        assert!(is_fixture_ext(Path::new("cases.yml")));
        assert!(is_fixture_ext(Path::new("config.toml")));
    }

    #[test]
    fn fixture_ext_rejects_other_extensions() {
        assert!(!is_fixture_ext(Path::new("prompt.md")));
        assert!(!is_fixture_ext(Path::new("notes.txt")));
        assert!(!is_fixture_ext(Path::new("noext")));
    }
}
