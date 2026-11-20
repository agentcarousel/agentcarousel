//! Severity tallies and the `--fail-on-severity` gate for `agc eval` / `agc test`.

use agentcarousel_core::{CaseResult, CaseStatus, Severity};

/// Failed cases counted by severity. `unrated` counts failures whose case has no severity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SeverityTally {
    pub p0: usize,
    pub p1: usize,
    pub p2: usize,
    pub p3: usize,
    pub unrated: usize,
}

impl SeverityTally {
    pub(crate) fn count(&self, severity: Severity) -> usize {
        match severity {
            Severity::P0 => self.p0,
            Severity::P1 => self.p1,
            Severity::P2 => self.p2,
            Severity::P3 => self.p3,
        }
    }

    /// True when any failure is at or above `threshold`, or has no severity.
    /// Unrated failures fail the gate: the safe default is to treat them as serious.
    pub(crate) fn fails_gate(&self, threshold: Severity) -> bool {
        self.unrated > 0
            || Severity::ALL
                .iter()
                .any(|s| s.meets(threshold) && self.count(*s) > 0)
    }

    /// True when a P0 or P1 failure is open.
    pub(crate) fn open_p0_p1(&self) -> bool {
        self.p0 > 0 || self.p1 > 0
    }

    pub(crate) fn summary_line(&self) -> String {
        let mut line = format!(
            "Open findings by severity: P0 {} · P1 {} · P2 {} · P3 {}",
            self.p0, self.p1, self.p2, self.p3
        );
        if self.unrated > 0 {
            line.push_str(&format!(" · unrated {}", self.unrated));
        }
        line
    }
}

/// True when the case's status is a failure (not-tested cases are never failures).
pub(crate) fn is_failed_status(case: &CaseResult) -> bool {
    matches!(
        case.status,
        CaseStatus::Failed | CaseStatus::TimedOut | CaseStatus::Error | CaseStatus::Flaky
    )
}

/// Failed cases by severity, or `None` when the suite assigns no severities.
pub(crate) fn failed_tally(cases: &[CaseResult]) -> Option<SeverityTally> {
    cases
        .iter()
        .any(|c| c.severity.is_some())
        .then(|| tally(cases, is_failed_status))
}

/// Print the `--fail-on-severity` summary to stderr, so it never mixes with JSON on stdout.
pub(crate) fn print_gate_summary(tally: &SeverityTally) {
    eprintln!("{}", tally.summary_line());
    if tally.unrated > 0 {
        eprintln!(
            "{} {} failure(s) without severity counted as failing",
            console::style("note:").yellow().bold(),
            tally.unrated
        );
    }
}

/// Count the cases `is_failure` selects, by severity.
pub(crate) fn tally<'a>(
    cases: impl IntoIterator<Item = &'a CaseResult>,
    is_failure: impl Fn(&CaseResult) -> bool,
) -> SeverityTally {
    let mut t = SeverityTally::default();
    for case in cases {
        if case.status == CaseStatus::Skipped || !is_failure(case) {
            continue;
        }
        match case.severity {
            Some(Severity::P0) => t.p0 += 1,
            Some(Severity::P1) => t.p1 += 1,
            Some(Severity::P2) => t.p2 += 1,
            Some(Severity::P3) => t.p3 += 1,
            None => t.unrated += 1,
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally_of(p0: usize, p1: usize, p2: usize, p3: usize, unrated: usize) -> SeverityTally {
        SeverityTally {
            p0,
            p1,
            p2,
            p3,
            unrated,
        }
    }

    #[test]
    fn severity_ordering_runs_from_most_to_least_severe() {
        assert!(Severity::P0.meets(Severity::P1));
        assert!(Severity::P1.meets(Severity::P1));
        assert!(!Severity::P2.meets(Severity::P1));
        assert!(!Severity::P3.meets(Severity::P0));
    }

    #[test]
    fn gate_fails_only_at_or_above_threshold() {
        assert!(tally_of(0, 1, 0, 0, 0).fails_gate(Severity::P1));
        assert!(tally_of(1, 0, 0, 0, 0).fails_gate(Severity::P1));
        assert!(!tally_of(0, 0, 2, 3, 0).fails_gate(Severity::P1));
        assert!(tally_of(0, 0, 0, 1, 0).fails_gate(Severity::P3));
        assert!(!tally_of(0, 0, 0, 0, 0).fails_gate(Severity::P3));
    }

    #[test]
    fn unrated_failures_fail_the_gate() {
        assert!(tally_of(0, 0, 0, 0, 1).fails_gate(Severity::P0));
    }

    #[test]
    fn summary_mentions_unrated_only_when_present() {
        assert!(!tally_of(0, 1, 0, 0, 0).summary_line().contains("unrated"));
        assert!(tally_of(0, 0, 0, 0, 2).summary_line().contains("unrated 2"));
    }
}

#[cfg(test)]
mod case_tests {
    use agentcarousel_core::{Case, Severity};

    fn case_yaml(extra: &str) -> Case {
        serde_yaml::from_str(&format!(
            "id: s/c\n{extra}input:\n  messages:\n    - role: user\n      content: hi\nexpected: {{}}\n"
        ))
        .expect("case parses")
    }

    #[test]
    fn severity_is_optional_and_parses_when_present() {
        assert_eq!(case_yaml("").severity, None);
        assert_eq!(case_yaml("severity: P2\n").severity, Some(Severity::P2));
    }

    #[test]
    fn p0_designates_the_case_critical() {
        assert!(case_yaml("severity: P0\n").is_critical());
        assert!(!case_yaml("severity: P1\n").is_critical());
        assert!(!case_yaml("").is_critical());
    }
}
