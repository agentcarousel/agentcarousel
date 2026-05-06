use agentcarousel_core::{CaseResult, CaseStatus, Run};
use console::style;

/// Human-oriented case id: segment after the last `/`, or the full id.
fn case_label(case_id: &str) -> &str {
    case_id.rsplit_once('/').map(|(_, s)| s).unwrap_or(case_id)
}

fn header_skill_label(run: &Run) -> String {
    if let Some(ref s) = run.skill_or_agent {
        return s.clone();
    }
    run.cases
        .first()
        .map(|c| {
            c.case_id
                .0
                .rsplit_once('/')
                .map(|(p, _)| p.to_string())
                .unwrap_or_else(|| c.case_id.0.clone())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "fixtures".to_string())
}

fn run_subtitle(run: &Run) -> String {
    let cmd = run.command.as_str();
    if run.runner_mock_only {
        let mut parts: Vec<&str> = Vec::new();
        if run.runner_offline {
            parts.push("offline");
        }
        if run.runner_mock_strict {
            parts.push("mock-strict");
        }
        let inner = if parts.is_empty() {
            "mock".to_string()
        } else {
            parts.join(" · ")
        };
        format!("Running {cmd} ({inner})")
    } else {
        format!("Running {cmd} (live)")
    }
}

fn case_duration_secs(case: &CaseResult) -> f64 {
    let ms = case.metrics.total_latency_ms as f64 / 1000.0;
    if ms <= 0.0 {
        0.1
    } else {
        ms
    }
}

fn cli_binary_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "agentcarousel".to_string())
}

fn print_case_failure_details(case: &CaseResult) {
    if let Some(out) = case.trace.final_output.as_ref() {
        if !out.is_empty() && matches!(case.status, CaseStatus::Failed | CaseStatus::Error) {
            let one_line = out.replace('\n', " ").trim().to_string();
            let esc = if one_line.chars().count() > 120 {
                format!("{}…", one_line.chars().take(117).collect::<String>())
            } else {
                one_line
            };
            println!("               agent replied: \"{}\"", style(esc).dim());
        }
    }
    if let Some(scores) = case.eval_scores.as_ref() {
        for rs in &scores.rubric_scores {
            if rs.rubric_id == "rules" {
                if let Some(rat) = rs.rationale.as_ref() {
                    println!("             › rules: {}", style(rat).dim());
                }
                break;
            }
        }
    }
    if let Some(err) = case.error.as_ref() {
        if !err.is_empty() {
            if err.contains(';') && err.len() > 60 {
                for part in err.split(';').map(str::trim).filter(|p| !p.is_empty()) {
                    println!("             › {}", style(part).dim());
                }
            } else {
                println!("             › {}", style(err).dim());
            }
        }
    }
    if matches!(case.status, CaseStatus::Failed) {
        println!(
            "             {}",
            style("› Agent quarantined. Certificate NOT issued.").dim()
        );
    }
}

/// Full terminal report (eval/test/report): banner, case rows, summary, run id hint.
pub fn print_terminal(run: &Run) {
    let skill = header_skill_label(run);
    let n = run.summary.total;
    println!(
        "🎠 AgentCarousel v{} · {} · {} cases",
        run.agentcarousel_version, skill, n
    );
    println!();
    println!("{}", run_subtitle(run));
    println!();

    let col_w = run
        .cases
        .iter()
        .map(|c| case_label(&c.case_id.0).chars().count())
        .max()
        .unwrap_or(0)
        .max(40);

    for case in &run.cases {
        let label = case_label(&case.case_id.0);
        let secs = case_duration_secs(case);
        let pad = col_w.saturating_sub(label.chars().count());
        let padding = " ".repeat(pad);

        match case.status {
            CaseStatus::Passed => println!("    ✅  PASS  {}{} ({:.1}s)", label, padding, secs),
            CaseStatus::Failed => println!("    ❌  FAIL  {}{} ({:.1}s)", label, padding, secs),
            CaseStatus::Skipped => println!(
                "    {}  SKIP  {}{} ({:.1}s)",
                style("⏭").yellow(),
                label,
                padding,
                secs
            ),
            CaseStatus::Flaky => println!(
                "    {}  FLAKY {}{} ({:.1}s)",
                style("⚠").yellow(),
                label,
                padding,
                secs
            ),
            CaseStatus::TimedOut => println!(
                "    {}  TIMEOUT {}{} ({:.1}s)",
                style("⏱").red(),
                label,
                padding,
                secs
            ),
            CaseStatus::Error => println!(
                "    {}  ERROR {}{} ({:.1}s)",
                style("✖").red(),
                label,
                padding,
                secs
            ),
        }

        if matches!(
            case.status,
            CaseStatus::Failed | CaseStatus::Error | CaseStatus::TimedOut
        ) {
            println!();
            print_case_failure_details(case);
        }

        if case.metrics.runs_attempted > 1 {
            let mut stat_parts = Vec::new();
            if let Some(stddev) = case.metrics.latency_stddev_ms {
                stat_parts.push(format!("latency σ={stddev:.0}ms"));
            }
            if let Some(stddev) = case.metrics.effectiveness_stddev {
                stat_parts.push(format!("effectiveness σ={stddev:.3}"));
            }
            if !stat_parts.is_empty() {
                println!(
                    "  {}",
                    style(format!("stats: {}", stat_parts.join(", "))).dim()
                );
            }
        }
    }

    let s = &run.summary;
    let passed = s.passed;
    let total = s.total;
    let failed = s.failed;

    println!();
    println!("  ──────────────────────────────────────────────────────");
    if failed > 0 {
        let fw = if failed == 1 { "failure" } else { "failures" };
        println!(
            "  Results   {} / {} passed   {} {} (quarantined)",
            passed, total, failed, fw
        );
    } else {
        println!("  Results   {} / {} passed", passed, total);
    }

    if let Some(mean) = s.mean_effectiveness_score {
        println!("  Effectiveness score: {:.2} / 1.00", mean);
    }

    let issues = failed + s.errored + s.timed_out + s.flaky;
    if issues == 0 {
        println!(
            "  {}",
            style("Certificate: ISSUED — all checks passed").green()
        );
    } else {
        println!(
            "  {}",
            style("Certificate: NOT ISSUED — address failing cases first").red()
        );
    }

    let bin = cli_binary_name();
    let id = run.id.0.as_str();
    println!("  Run id: {}", id);
    println!("  Next:   {} report show {}", bin, id);
    println!("  ──────────────────────────────────────────────────────");
}

/// Quiet / condensed output: banner + pass-rate line + optional provider errors.
pub fn print_terminal_summary(run: &Run) {
    let skill = header_skill_label(run);
    println!(
        "🎠 AgentCarousel v{} · {} · {} cases",
        run.agentcarousel_version, skill, run.summary.total
    );
    println!(
        "{} {} cases (pass rate {:.0}%)",
        style("Run").bold(),
        run.summary.total,
        run.summary.pass_rate * 100.0
    );
    if run.summary.failed > 0 || run.summary.errored > 0 || run.summary.timed_out > 0 {
        println!(
            "{} failed, {} errored, {} timed out",
            style(run.summary.failed).red(),
            style(run.summary.errored).red(),
            style(run.summary.timed_out).red()
        );
    }
    if let Some(error_line) = format_provider_errors(&run.summary.provider_errors) {
        println!("{}", style(error_line).yellow());
    }
}

fn format_provider_errors(errors: &agentcarousel_core::ProviderErrorMetrics) -> Option<String> {
    let mut parts = Vec::new();
    if errors.status_429 > 0 {
        parts.push(format!("429={}", errors.status_429));
    }
    if errors.status_500 > 0 {
        parts.push(format!("500={}", errors.status_500));
    }
    if errors.status_503 > 0 {
        parts.push(format!("503={}", errors.status_503));
    }
    if errors.status_504 > 0 {
        parts.push(format!("504={}", errors.status_504));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("provider errors: {}", parts.join(", ")))
    }
}
