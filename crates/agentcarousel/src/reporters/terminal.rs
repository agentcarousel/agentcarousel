use agentcarousel_core::{CaseResult, CaseStatus, EvalScores, RubricScore, Run};
use console::style;
use serde_json::Value;

const HUMAN_ERROR_MAX: usize = 280;
const JUDGE_SUMMARY_MAX: usize = 160;
const RUBRIC_SNIPPET_MAX: usize = 100;

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

/// Collapse whitespace and cap length with an ellipsis (character-aware).
fn truncate_human(s: &str, max_chars: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    format!(
        "{}…",
        trimmed
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>()
    )
}

/// Pull a human-readable message from provider-style JSON (e.g. Gemini `error.message`).
fn extract_json_message(v: &Value) -> Option<String> {
    if let Some(err) = v.get("error") {
        if let Some(m) = err.get("message").and_then(|x| x.as_str()) {
            return Some(m.to_string());
        }
    }
    if let Some(m) = v.get("message").and_then(|x| x.as_str()) {
        return Some(m.to_string());
    }
    None
}

/// Shorten API / provider errors for the terminal: prefer nested JSON `message`, else trim + cap.
fn humanize_error_line(err: &str) -> String {
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(msg) = extract_json_message(&v) {
            return truncate_human(&msg, HUMAN_ERROR_MAX);
        }
    }

    if let Some(start) = trimmed.find('{') {
        let tail = trimmed[start..].trim();
        if let Ok(v) = serde_json::from_str::<Value>(tail) {
            if let Some(msg) = extract_json_message(&v) {
                let prefix = trimmed[..start].trim();
                let core = truncate_human(&msg, HUMAN_ERROR_MAX);
                if prefix.is_empty() {
                    return core;
                }
                return truncate_human(&format!("{prefix} {core}"), HUMAN_ERROR_MAX);
            }
        }
    }

    truncate_human(trimmed, HUMAN_ERROR_MAX)
}

fn print_eval_failure_rationale(scores: &EvalScores) {
    match scores.evaluator.as_str() {
        "judge" => print_judge_failure_summary(scores),
        "rules" => {
            for rs in &scores.rubric_scores {
                if rs.rubric_id == "rules" {
                    if let Some(rat) = rs.rationale.as_ref() {
                        println!(
                            "             › rules: {}",
                            style(truncate_human(rat, HUMAN_ERROR_MAX)).dim()
                        );
                    }
                    break;
                }
            }
        }
        "golden" | "process" => {
            let mut failing: Vec<&RubricScore> = scores
                .rubric_scores
                .iter()
                .filter(|r| r.score < 1.0 - f32::EPSILON)
                .collect();
            failing.sort_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for rs in failing.iter().take(3) {
                let snippet = rs
                    .rationale
                    .as_deref()
                    .map(|s| truncate_human(s, RUBRIC_SNIPPET_MAX))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "no rationale".to_string());
                println!(
                    "             › {} · {} ({:.2}): {}",
                    scores.evaluator,
                    rs.rubric_id,
                    rs.score,
                    style(snippet).dim()
                );
            }
            if failing.is_empty() && !scores.passed {
                println!(
                    "             › {}: {}",
                    scores.evaluator,
                    style("below effectiveness threshold or aggregate failure").dim()
                );
            }
        }
        other => {
            let mut low: Vec<&RubricScore> = scores
                .rubric_scores
                .iter()
                .filter(|r| r.score < 1.0 - f32::EPSILON)
                .collect();
            low.sort_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for rs in low.iter().take(2) {
                let snippet = rs
                    .rationale
                    .as_deref()
                    .map(|s| truncate_human(s, RUBRIC_SNIPPET_MAX))
                    .unwrap_or_default();
                if snippet.is_empty() {
                    continue;
                }
                println!(
                    "             › {} · {} ({:.2}): {}",
                    other,
                    rs.rubric_id,
                    rs.score,
                    style(snippet).dim()
                );
            }
        }
    }
}

/// Overall judge narrative for the terminal, omitting empty / placeholder text.
fn judge_overall_summary_line(judge_rationale: Option<&str>) -> Option<String> {
    let jr = judge_rationale?.trim();
    if jr.is_empty() {
        return None;
    }
    let t = truncate_human(jr, JUDGE_SUMMARY_MAX);
    if t.is_empty() || t == "judge completed without rationale" {
        None
    } else {
        Some(t)
    }
}

fn print_judge_failure_summary(scores: &EvalScores) {
    if let Some(line) = judge_overall_summary_line(scores.judge_rationale.as_deref()) {
        println!("             › judge: {}", style(line).dim());
    }

    let mut low: Vec<&RubricScore> = scores
        .rubric_scores
        .iter()
        .filter(|r| r.score < 1.0 - f32::EPSILON)
        .collect();
    low.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for rs in low.iter().take(2) {
        let snippet = rs
            .rationale
            .as_deref()
            .map(|s| truncate_human(s, RUBRIC_SNIPPET_MAX))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "no rationale".to_string());
        println!(
            "             › judge · {} ({:.2}): {}",
            rs.rubric_id,
            rs.score,
            style(snippet).dim()
        );
    }

    if low.is_empty() && !scores.passed {
        println!(
            "             › judge: {}",
            style("scores did not meet pass threshold").dim()
        );
    }
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
        let show_eval = matches!(case.status, CaseStatus::Failed)
            || (matches!(case.status, CaseStatus::Error | CaseStatus::TimedOut) && !scores.passed);
        if show_eval {
            print_eval_failure_rationale(scores);
        }
    }

    if let Some(err) = case.error.as_ref() {
        if !err.is_empty() {
            let human = humanize_error_line(err);
            if !human.is_empty() {
                println!("             › {}", style(human).dim());
            }
        }
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
            "  Results   {} / {} passed   {} {}",
            passed, total, failed, fw
        );
    } else {
        println!("  Results   {} / {} passed", passed, total);
    }

    if let Some(mean) = s.mean_effectiveness_score {
        println!("  Effectiveness score: {:.2} / 1.00", mean);
    }

    if let (Some(p50), Some(p95), Some(p99)) =
        (s.latency_p50_ms, s.latency_p95_ms, s.latency_p99_ms)
    {
        println!(
            "  Latency p50/p95/p99  {:.0}ms / {:.0}ms / {:.0}ms",
            p50, p95, p99
        );
    }

    if s.tokens_in.is_some() || s.tokens_out.is_some() {
        let ti = s.tokens_in.unwrap_or(0);
        let to = s.tokens_out.unwrap_or(0);
        let total = ti + to;
        println!();
        println!("  {}", style("Token Consumption 🪙").bold());
        println!("    › total: {}", style(total).cyan());
        println!("      ├─ in:  {}", ti);
        println!("      └─ out: {}", to);
        if let Some(m) = s.mean_tokens_per_judged_case {
            println!("    › avg tokens/judged case: {}", m);
        }
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
            style("Certificate: NOT ISSUED — agent quarantined until all cases pass cleanly").red()
        );
    }

    let bin = cli_binary_name();
    let id = run.id.0.as_str();
    println!("  run id: {}", id);
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

    if run.summary.tokens_in.is_some() || run.summary.tokens_out.is_some() {
        let ti = run.summary.tokens_in.unwrap_or(0);
        let to = run.summary.tokens_out.unwrap_or(0);
        let total = ti + to;
        let mut parts = vec![
            format!("total={}", total),
            format!("in={}", ti),
            format!("out={}", to),
        ];
        if let Some(m) = run.summary.mean_tokens_per_judged_case {
            parts.push(format!("avg_per_judged={}", m));
        }
        println!(
            "{}",
            style(format!("🪙  tokens: {}", parts.join(", "))).dim()
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_extracts_gemini_error_message() {
        let raw = r#"live generation failed: gemini generation failed (400 Bad Request): {
  "error": {
    "code": 400,
    "message": "API key not valid. Please pass a valid API key.",
    "status": "INVALID_ARGUMENT"
  }
}"#;
        let h = humanize_error_line(raw);
        assert!(h.contains("API key not valid"));
        assert!(!h.contains("\"error\""));
    }

    #[test]
    fn humanize_pure_json_error_object() {
        let raw = r#"{"error":{"message":"Rate limited"}}"#;
        assert_eq!(humanize_error_line(raw), "Rate limited");
    }

    #[test]
    fn humanize_fallback_truncates_long_plain_text() {
        let raw = "x".repeat(400);
        let h = humanize_error_line(&raw);
        assert!(h.ends_with('…'));
        assert!(h.chars().count() <= HUMAN_ERROR_MAX + 1);
    }

    #[test]
    fn truncate_human_collapses_whitespace() {
        assert_eq!(truncate_human("  hello   world  ", 100), "hello world");
    }

    #[test]
    fn judge_overall_summary_omits_placeholder() {
        assert_eq!(
            judge_overall_summary_line(Some("judge completed without rationale")),
            None
        );
        assert_eq!(
            judge_overall_summary_line(Some("Missing registry URL in stub.")),
            Some("Missing registry URL in stub.".to_string())
        );
    }
}
