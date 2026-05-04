use agentcarousel_core::{CaseStatus, Run};
use console::style;

pub fn print_terminal(run: &Run) {
    println!(
        "{} {} cases (pass rate {:.0}%)",
        style("Run").bold(),
        run.summary.total,
        run.summary.pass_rate * 100.0
    );
    for case in &run.cases {
        let status = match case.status {
            CaseStatus::Passed => style("PASS").green(),
            CaseStatus::Failed => style("FAIL").red(),
            CaseStatus::Skipped => style("SKIP").yellow(),
            CaseStatus::Flaky => style("FLAKY").yellow(),
            CaseStatus::TimedOut => style("TIMEOUT").red(),
            CaseStatus::Error => style("ERROR").red(),
        };
        println!("{} {}", status, case.case_id.0);
        if let Some(error) = &case.error {
            println!("  {}", style(error).dim());
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
}

pub fn print_terminal_summary(run: &Run) {
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
