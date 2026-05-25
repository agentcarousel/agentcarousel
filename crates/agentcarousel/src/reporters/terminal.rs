use agentcarousel_core::{
    fmt_cost, fmt_tokens, lookup_pricing, CaseResult, CaseStatus, EvalScores,
    PromptAuditFailureMode, Role, RubricScore, Run,
};
use console::style;
use serde_json::Value;

const HUMAN_ERROR_MAX: usize = 280;

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

fn fmt_duration(ms: f64) -> String {
    if ms < 1000.0 {
        format!("{:.0}ms", ms)
    } else {
        format!("{:.1}s", ms / 1000.0)
    }
}

fn cli_binary_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "agc".to_string())
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
        "judge" => {
            print_section_header_yellow("JUDGE");
            print_judge_failure_summary(scores);
        }
        "rules" => {
            for rs in &scores.rubric_scores {
                if rs.rubric_id == "rules" {
                    if let Some(rat) = rs.rationale.as_ref() {
                        print_section_header_yellow("RULES");
                        println!(
                            "        {}",
                            style(truncate_human(rat, HUMAN_ERROR_MAX)).dim()
                        );
                    }
                    break;
                }
            }
        }
        evaluator => {
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
            if !failing.is_empty() || !scores.passed {
                print_section_header_yellow(&evaluator.to_uppercase());
            }
            for rs in failing.iter().take(4) {
                let snippet = rs
                    .rationale
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("no rationale");
                let score_str = format!("{:.2}", rs.score);
                let score_styled = if rs.score < 0.5 {
                    style(score_str).red().bold()
                } else {
                    style(score_str).yellow().bold()
                };
                println!(
                    "        {} ({})  {}",
                    style(format!("· {}", rs.rubric_id)).bold(),
                    score_styled,
                    style(snippet).dim()
                );
            }
            if failing.is_empty() && !scores.passed {
                println!(
                    "        {}",
                    style("· below effectiveness threshold or aggregate failure").dim()
                );
            }
        }
    }
}

/// Overall judge narrative for the terminal, omitting empty / placeholder text.
fn judge_overall_summary_line(judge_rationale: Option<&str>) -> Option<String> {
    let jr = judge_rationale?.trim();
    if jr.is_empty() || jr == "judge completed without rationale" {
        None
    } else {
        Some(jr.to_string())
    }
}

fn print_judge_failure_summary(scores: &EvalScores) {
    if let Some(line) = judge_overall_summary_line(scores.judge_rationale.as_deref()) {
        println!("        {}", line);
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

    if !low.is_empty() {
        println!();
    }
    for rs in low.iter().take(4) {
        let snippet = rs
            .rationale
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("no rationale");
        let score_str = format!("{:.2}", rs.score);
        let score_styled = if rs.score < 0.5 {
            style(score_str).red().bold()
        } else {
            style(score_str).yellow().bold()
        };
        println!(
            "        {} ({})  {}",
            style(format!("· {}", rs.rubric_id)).bold(),
            score_styled,
            style(snippet).dim()
        );
    }

    if low.is_empty() && !scores.passed {
        println!(
            "        {}",
            style("· scores did not meet pass threshold").dim()
        );
    }
}

fn role_label(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn print_section_header_dim(label: &str) {
    let rule = "─".repeat(54_usize.saturating_sub(label.len()));
    println!("        {} {}", style(label).bold(), style(rule).dim());
}

fn print_section_header_cyan(label: &str) {
    let rule = "─".repeat(54_usize.saturating_sub(label.len()));
    println!(
        "        {} {}",
        style(label).bold().cyan(),
        style(rule).dim()
    );
}

fn print_section_header_yellow(label: &str) {
    let rule = "─".repeat(54_usize.saturating_sub(label.len()));
    println!(
        "        {} {}",
        style(label).bold().yellow(),
        style(rule).dim()
    );
}

fn print_case_details(case: &CaseResult) {
    let is_judged = case
        .eval_scores
        .as_ref()
        .map(|s| s.evaluator == "judge")
        .unwrap_or(false);

    if is_judged && !case.input.is_empty() {
        print_section_header_dim("INPUT");
        for msg in &case.input {
            let role = role_label(&msg.role);
            println!("        {}", style(format!("[{}]", role)).bold());
            for line in msg.content.trim().lines() {
                println!("          {}", style(line).dim());
            }
        }
        println!();
    }

    if let Some(out) = case.trace.final_output.as_ref() {
        let out = out.trim();
        if !out.is_empty() {
            print_section_header_cyan("AGENT REPLIED");
            for line in out.lines() {
                println!("          {}", line);
            }
            println!();
        }
    }

    if let Some(scores) = case.eval_scores.as_ref() {
        print_eval_failure_rationale(scores);
    }

    if let Some(err) = case.error.as_ref() {
        if !err.is_empty() {
            let human = humanize_error_line(err);
            if !human.is_empty() {
                println!("        {}", style(human).bold().red());
            }
        }
    }
    println!();
}

/// Full terminal report (eval/test/report): banner, case rows, summary, run id hint.
pub fn print_terminal(run: &Run, verbose: bool) {
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

        let disc_suffix = match case.discrimination_label.as_deref() {
            Some("high") => format!("  {}", style("[disc:high \u{2713}]").green()),
            Some("low") => format!("  {}", style("[disc:low \u{2717}]").yellow()),
            Some("marginal") => format!("  {}", style("[disc:marginal]").dim()),
            _ => String::new(),
        };

        match case.status {
            CaseStatus::Passed => println!(
                "    \u{2705}  PASS  {}{}  ({:.1}s){}",
                label, padding, secs, disc_suffix
            ),
            CaseStatus::Failed => println!(
                "    \u{274c}  FAIL  {}{}  ({:.1}s){}",
                label, padding, secs, disc_suffix
            ),
            CaseStatus::Skipped => println!(
                "    {}  SKIP  {}{}  ({:.1}s){}",
                style("\u{23ed}").yellow(),
                label,
                padding,
                secs,
                disc_suffix
            ),
            CaseStatus::Flaky => println!(
                "    {}  FLAKY {}{}  ({:.1}s){}",
                style("\u{26a0}").yellow(),
                label,
                padding,
                secs,
                disc_suffix
            ),
            CaseStatus::TimedOut => println!(
                "    {}  TIMEOUT {}{}  ({:.1}s){}",
                style("\u{23f1}").red(),
                label,
                padding,
                secs,
                disc_suffix
            ),
            CaseStatus::Error => println!(
                "    {}  ERROR {}{}  ({:.1}s){}",
                style("\u{2716}").red(),
                label,
                padding,
                secs,
                disc_suffix
            ),
        }

        if matches!(
            case.status,
            CaseStatus::Failed | CaseStatus::Error | CaseStatus::TimedOut
        ) || (verbose && matches!(case.status, CaseStatus::Passed))
        {
            println!();
            print_case_details(case);
        }

        if case.metrics.runs_attempted > 1 {
            let mut stat_parts = Vec::new();
            if let Some(stddev) = case.metrics.latency_stddev_ms {
                stat_parts.push(format!("latency σ={}", fmt_duration(stddev)));
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
    let errored = s.errored;

    println!();
    println!("  ──────────────────────────────────────────────────────");
    if failed > 0 || errored > 0 {
        let mut parts: Vec<String> = Vec::new();
        if failed > 0 {
            let fw = if failed == 1 { "failure" } else { "failures" };
            parts.push(format!("{} {}", failed, fw));
        }
        if errored > 0 {
            let ew = if errored == 1 { "error" } else { "errors" };
            parts.push(format!("{} {}", errored, ew));
        }
        println!(
            "  Results   {} / {} passed   {}",
            passed,
            total,
            parts.join("   ")
        );
    } else {
        println!("  Results   {} / {} passed", passed, total);
    }

    if let Some(mean) = s.mean_effectiveness_score {
        println!(
            "  Effectiveness score: {:.2} / 1.00  {}",
            mean,
            style("(weighted mean of rubric pass rates; 1.0 = perfect)").dim()
        );
    }

    if let (Some(p50), Some(p95), Some(p99)) =
        (s.latency_p50_ms, s.latency_p95_ms, s.latency_p99_ms)
    {
        println!(
            "  Latency p50/p95/p99  {} / {} / {}",
            fmt_duration(p50),
            fmt_duration(p95),
            fmt_duration(p99)
        );
    }

    if s.tokens_in.is_some() || s.tokens_out.is_some() {
        let ti = s.tokens_in.unwrap_or(0);
        let to = s.tokens_out.unwrap_or(0);
        println!();
        println!("  {}", style("Tokens").bold());
        println!(
            "    gen   {} in · {} out",
            style(fmt_tokens(s.tokens_in)).cyan(),
            style(fmt_tokens(s.tokens_out)).cyan(),
        );
        if s.judge_tokens_in.is_some() {
            println!(
                "    judge {} in · {} out",
                style(fmt_tokens(s.judge_tokens_in)).cyan(),
                style(fmt_tokens(s.judge_tokens_out)).cyan(),
            );
        }
        let total = ti + to + s.judge_tokens_in.unwrap_or(0) + s.judge_tokens_out.unwrap_or(0);
        println!("    total {}", style(fmt_tokens(Some(total))).cyan().bold());
        if let Some(cost) = s.total_cost_usd {
            println!(
                "    cost  {}",
                style(format!("${:.4}", cost)).yellow().bold()
            );
        }
        if let Some(m) = s.mean_tokens_per_judged_case {
            println!(
                "    avg   {} per judged case",
                style(fmt_tokens(Some(m))).dim()
            );
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

    if let Some(audit) = &run.prompt_audit {
        print_prompt_audit(audit, run.summary.judge_model.as_deref());
    }
}

/// Print the prompt-audit section standalone (used by `agc audit`).
pub fn print_audit(run: &Run) {
    if let Some(audit) = &run.prompt_audit {
        let skill = header_skill_label(run);
        println!(
            "Prompt audit  ·  {}  ·  run {}",
            skill,
            &run.id.0[..run.id.0.len().min(12)]
        );
        println!();
        print_prompt_audit(audit, run.summary.judge_model.as_deref());
    } else {
        println!("no prompt audit attached to this run");
    }
}

fn print_prompt_audit(audit: &agentcarousel_core::PromptAudit, judge_model: Option<&str>) {
    println!();
    println!(
        "  {}",
        style("── Prompt Audit ──────────────────────────────────────").dim()
    );

    let mode_label = match audit.failure_mode {
        PromptAuditFailureMode::Prompt => style("prompt").yellow().bold(),
        PromptAuditFailureMode::Model => style("model").red().bold(),
        PromptAuditFailureMode::Fixture => style("fixture").cyan().bold(),
        PromptAuditFailureMode::Mixed => style("mixed").yellow().bold(),
    };
    println!(
        "  Failure mode: {}  (confidence {:.0}%)",
        mode_label,
        audit.confidence * 100.0
    );
    println!();

    if !audit.findings.is_empty() {
        println!("  {}", style("Findings:").bold());
        for f in &audit.findings {
            println!("    • {}", f.pattern);
            println!("      {}", style(&f.root_cause).dim());
        }
        println!();
    }

    if !audit.suggested_fixes.is_empty() {
        println!("  {}", style("Suggested fixes:").bold());
        for (i, fix) in audit.suggested_fixes.iter().enumerate() {
            println!("    {}. {}", i + 1, fix);
        }
        println!();
    }

    println!(
        "  {}",
        style(wrap_to_width(&audit.overall_rationale, 72)).dim()
    );

    if audit.judge_tokens_in.is_some() || audit.judge_tokens_out.is_some() {
        let audit_cost: Option<f64> = judge_model.and_then(|m| {
            let pricing = lookup_pricing(m)?;
            let ti = audit.judge_tokens_in?;
            let to = audit.judge_tokens_out.unwrap_or(0);
            Some(
                pricing.prompt_usd_per_token * ti as f64
                    + pricing.completion_usd_per_token * to as f64,
            )
        });
        let cost_str = if let Some(c) = audit_cost {
            format!("  cost {}", style(fmt_cost(Some(c))).yellow().bold())
        } else {
            String::new()
        };
        println!(
            "  Audit tokens  {} in · {} out{}",
            style(fmt_tokens(audit.judge_tokens_in)).cyan(),
            style(fmt_tokens(audit.judge_tokens_out)).cyan(),
            cost_str,
        );
    }

    println!(
        "  {}",
        style("──────────────────────────────────────────────────────").dim()
    );
}

fn wrap_to_width(text: &str, width: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in words {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current.clone());
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines.join("\n  ")
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
