use agentcarousel_core::{judge_key_candidates, judge_provider_from_model, prefetch_pricing};
use agentcarousel_evaluators::run_prompt_audit;
use agentcarousel_reporters::{fetch_run, persist_run, print_audit};
use chrono::Utc;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::io::{stderr, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

/// Prompt-audit analysis commands: run a second-pass audit against a saved run or apply its
/// suggestions to your prompt.md.
#[derive(Debug, Parser)]
pub struct AuditArgs {
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    command: AuditCommand,
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Run the prompt-audit LLM analysis against a saved run and store the result.
    ///
    /// Loads the run, calls the judge to diagnose whether failures are due to prompt
    /// design, model capability, or fixture miscalibration, and saves the result back
    /// to the history DB so `agc report show <id>` displays it going forward.
    #[command(
        after_help = "Examples:\n  agc audit run ECR99Y6BWT\n  agc audit run ECR99Y6BWT --prompt fixtures/my-skill/prompt.md\n  agc audit run ECR99Y6BWT --model claude-opus-4-7 --json\n  agc audit run path/to/run.json"
    )]
    Run {
        /// Run ID from `agc report list`, or a path to run.json / a directory containing it.
        run_id: String,
        /// Path to the prompt.md to audit against (default: auto-discovered from fixtures/<skill>/prompt.md).
        #[arg(long)]
        prompt: Option<PathBuf>,
        /// Judge model to use (overrides config judge.model).
        #[arg(long)]
        model: Option<String>,
        /// Do not save the audit result back to the history database.
        #[arg(long)]
        no_save: bool,
    },
    /// Print suggestions from a stored audit, or apply them to your prompt.md.
    ///
    /// Reads the `suggested_fixes` from a previously-stored audit result — no LLM call.
    /// Without `--apply`, prints suggestions to stdout. With `--apply`, appends them as
    /// a commented block to prompt.md so you can review and integrate them like a diff.
    #[command(
        after_help = "Examples:\n  agc audit suggest ECR99Y6BWT                           # print suggestions\n  agc audit suggest ECR99Y6BWT --apply                   # append to prompt.md\n  agc audit suggest ECR99Y6BWT --apply --prompt fixtures/my-skill/prompt.md"
    )]
    Suggest {
        /// Run ID from `agc report list`, or a path to run.json / a directory containing it.
        run_id: String,
        /// Append the suggestions as a commented block to prompt.md.
        #[arg(long)]
        apply: bool,
        /// Path to the prompt.md to write into (default: auto-discovered from fixtures/<skill>/prompt.md).
        #[arg(long)]
        prompt: Option<PathBuf>,
    },
}

pub fn run_audit_command(args: AuditArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    match args.command {
        AuditCommand::Run {
            run_id,
            prompt,
            model,
            no_save,
        } => run_audit(run_id, prompt, model, no_save, config, globals),
        AuditCommand::Suggest {
            run_id,
            apply,
            prompt,
        } => run_suggest(run_id, apply, prompt, globals),
    }
}

fn run_audit(
    run_id: String,
    prompt_path: Option<PathBuf>,
    model: Option<String>,
    no_save: bool,
    config: &ResolvedConfig,
    globals: &GlobalOptions,
) -> i32 {
    let run = match load_run_for_audit(&run_id) {
        Ok(r) => r,
        Err(err) => {
            if globals.json {
                JsonOutput::err(
                    "audit",
                    JsonError::new("not_found", err).with_suggestions(vec![
                        "Run 'agc report list' to see available run IDs.".to_string(),
                    ]),
                )
                .print();
            } else {
                eprintln!("error: {err}");
            }
            return ExitCode::NotFound.as_i32();
        }
    };

    let prompt_text = match resolve_prompt(prompt_path.as_deref(), &run) {
        Some(t) => t,
        None => {
            let msg = "could not find prompt.md — pass --prompt <path> to specify it explicitly";
            if globals.json {
                JsonOutput::err("audit", JsonError::new("not_found", msg)).print();
            } else {
                eprintln!("error: {msg}");
                eprintln!(
                    "hint: looked for fixtures/{}/prompt.md",
                    run.skill_or_agent.as_deref().unwrap_or("<skill>")
                );
            }
            return ExitCode::NotFound.as_i32();
        }
    };

    let judge_model = model.unwrap_or_else(|| config.judge.model.clone());
    let judge_provider = judge_provider_from_model(&judge_model);

    let has_key = judge_key_candidates(judge_provider)
        .iter()
        .any(|k| std::env::var(k).is_ok());
    if !has_key {
        let keys = judge_key_candidates(judge_provider).join(", ");
        let msg = format!(
            "set one of {} to run audit for model '{}'",
            keys, judge_model
        );
        if globals.json {
            JsonOutput::err("audit", JsonError::new("auth_error", msg)).print();
        } else {
            eprintln!("error: {msg}");
        }
        return ExitCode::ConfigError.as_i32();
    }

    prefetch_pricing();
    let show_spinner = !globals.quiet && !globals.json && stderr().is_terminal();
    let spinner: Option<ProgressBar> = if show_spinner {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("spinner template")
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.set_message(format!(
            "Running prompt audit for run {} with {}...",
            &run.id.0, judge_model
        ));
        pb.enable_steady_tick(Duration::from_millis(120));
        Some(pb)
    } else {
        None
    };

    let audit_max_tokens = config.judge.max_tokens.map(|t| t.max(4096));
    let audit = match run_prompt_audit(&prompt_text, &run.cases, &judge_model, audit_max_tokens) {
        Ok(a) => {
            if let Some(ref pb) = spinner {
                pb.finish_and_clear();
            }
            a
        }
        Err(err) => {
            if let Some(ref pb) = spinner {
                pb.finish_and_clear();
            }
            if globals.json {
                JsonOutput::err("audit", JsonError::new("runtime_error", err.to_string())).print();
            } else {
                eprintln!("error: prompt audit failed: {err}");
            }
            return ExitCode::RuntimeError.as_i32();
        }
    };

    let mut run = run;
    run.prompt_audit = Some(audit);

    if !no_save {
        if let Err(err) = persist_run(&run) {
            eprintln!("warn: audit result was not saved to history: {err}");
            eprintln!(
                "hint: rerun `agc audit run {}` to retry the save",
                &run.id.0
            );
        }
    }

    if globals.json {
        let value = serde_json::to_value(&run.prompt_audit).unwrap_or(serde_json::Value::Null);
        JsonOutput::ok("audit", value).print();
    } else {
        println!();
        print_audit(&run);
    }

    ExitCode::Ok.as_i32()
}

fn run_suggest(
    run_id: String,
    apply: bool,
    prompt_path: Option<PathBuf>,
    globals: &GlobalOptions,
) -> i32 {
    let run = match load_run_for_audit(&run_id) {
        Ok(r) => r,
        Err(err) => {
            if globals.json {
                JsonOutput::err(
                    "audit suggest",
                    JsonError::new("not_found", err).with_suggestions(vec![
                        "Run 'agc report list' to see available run IDs.".to_string(),
                    ]),
                )
                .print();
            } else {
                eprintln!("error: {err}");
            }
            return ExitCode::NotFound.as_i32();
        }
    };

    let audit = match &run.prompt_audit {
        Some(a) => a,
        None => {
            let msg = format!(
                "run {} has no stored audit — run `agc audit run {}` first",
                run_id, run_id
            );
            if globals.json {
                JsonOutput::err("audit suggest", JsonError::new("not_found", &*msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            return ExitCode::NotFound.as_i32();
        }
    };

    if audit.suggested_fixes.is_empty() {
        if globals.json {
            JsonOutput::ok("audit suggest", serde_json::json!({ "suggestions": [] })).print();
        } else {
            println!("no suggestions in the stored audit for run {run_id}");
        }
        return ExitCode::Ok.as_i32();
    }

    if globals.json && !apply {
        let pairs: Vec<serde_json::Value> = audit
            .suggested_fixes
            .iter()
            .enumerate()
            .map(|(i, fix)| {
                let mut obj = serde_json::json!({ "title": fix });
                if let Some(imp) = audit.suggested_implementations.get(i) {
                    obj["implementation"] = serde_json::Value::String(imp.clone());
                }
                obj
            })
            .collect();
        JsonOutput::ok("audit suggest", serde_json::json!({ "suggestions": pairs })).print();
        return ExitCode::Ok.as_i32();
    }

    if !apply {
        println!("Suggestions for run {run_id}:");
        println!();
        for (i, fix) in audit.suggested_fixes.iter().enumerate() {
            println!("  {}. {}", i + 1, fix);
            if let Some(imp) = audit.suggested_implementations.get(i) {
                println!();
                for line in imp.lines() {
                    println!("     {}", line);
                }
                println!();
            }
        }
        println!(
            "hint: run `agc audit suggest {} --apply` to append these to prompt.md",
            run_id
        );
        return ExitCode::Ok.as_i32();
    }

    // --apply: find and write to prompt.md
    let prompt_file = match resolve_prompt_path(prompt_path.as_deref(), &run) {
        Some(p) => p,
        None => {
            let msg = "could not find prompt.md — pass --prompt <path> to specify it";
            if globals.json {
                JsonOutput::err("audit suggest", JsonError::new("not_found", msg)).print();
            } else {
                eprintln!("error: {msg}");
                eprintln!(
                    "hint: looked for fixtures/{}/prompt.md",
                    run.skill_or_agent.as_deref().unwrap_or("<skill>")
                );
            }
            return ExitCode::NotFound.as_i32();
        }
    };

    let existing = match fs::read_to_string(&prompt_file) {
        Ok(s) => s,
        Err(err) => {
            let msg = format!("could not read {}: {err}", prompt_file.display());
            if globals.json {
                JsonOutput::err("audit suggest", JsonError::new("io_error", &*msg)).print();
            } else {
                eprintln!("error: {msg}");
            }
            return ExitCode::RuntimeError.as_i32();
        }
    };

    let date = Utc::now().format("%Y-%m-%d").to_string();
    let mut block = format!(
        "\n<!-- audit:suggestions run={} generated={} -->\n",
        run_id, date
    );
    for (i, fix) in audit.suggested_fixes.iter().enumerate() {
        block.push_str(&format!("<!-- fix {}: {} -->\n", i + 1, fix));
        if let Some(implementation) = audit.suggested_implementations.get(i) {
            block.push_str(implementation.trim());
            block.push_str("\n\n");
        }
    }
    block.push_str("<!-- /audit:suggestions -->\n");

    let updated = format!("{}{}", existing.trim_end_matches('\n'), block);

    if let Err(err) = fs::write(&prompt_file, &updated) {
        let msg = format!("could not write {}: {err}", prompt_file.display());
        if globals.json {
            JsonOutput::err("audit suggest", JsonError::new("io_error", &*msg)).print();
        } else {
            eprintln!("error: {msg}");
        }
        return ExitCode::RuntimeError.as_i32();
    }

    let impl_count = audit
        .suggested_implementations
        .iter()
        .filter(|s| !s.trim().is_empty())
        .count();

    if globals.json {
        JsonOutput::ok(
            "audit suggest",
            serde_json::json!({
                "applied": audit.suggested_fixes.len(),
                "with_implementations": impl_count,
                "prompt_path": prompt_file.display().to_string(),
            }),
        )
        .print();
    } else {
        println!(
            "applied {} suggestion(s) to {} ({} with worked implementations)",
            audit.suggested_fixes.len(),
            prompt_file.display(),
            impl_count,
        );
        println!();
        for (i, fix) in audit.suggested_fixes.iter().enumerate() {
            let has_impl = audit.suggested_implementations.get(i).is_some();
            println!(
                "  {}. {} {}",
                i + 1,
                fix,
                if has_impl { "[+implementation]" } else { "" }
            );
        }
        println!();
        println!("Review and remove the <!-- audit:suggestions --> block after integrating.");
    }

    ExitCode::Ok.as_i32()
}

fn load_run_for_audit(run_ref: &str) -> Result<agentcarousel_core::Run, String> {
    let path = Path::new(run_ref);
    if path.exists() {
        let json_path = if path.is_dir() {
            path.join("run.json")
        } else {
            path.to_path_buf()
        };
        if !json_path.is_file() {
            return Err(format!(
                "expected {} to be a run.json file or a directory containing run.json",
                run_ref
            ));
        }
        let raw = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", json_path.display()))
    } else {
        fetch_run(run_ref).map_err(|e| e.to_string())
    }
}

fn resolve_prompt(explicit: Option<&Path>, run: &agentcarousel_core::Run) -> Option<String> {
    if let Some(path) = explicit {
        return fs::read_to_string(path)
            .ok()
            .filter(|s| !s.trim().is_empty());
    }
    let skill = run.skill_or_agent.as_deref()?;
    let candidate = PathBuf::from("fixtures").join(skill).join("prompt.md");
    fs::read_to_string(&candidate)
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn resolve_prompt_path(explicit: Option<&Path>, run: &agentcarousel_core::Run) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    let skill = run.skill_or_agent.as_deref()?;
    let candidate = PathBuf::from("fixtures").join(skill).join("prompt.md");
    candidate.is_file().then_some(candidate)
}
