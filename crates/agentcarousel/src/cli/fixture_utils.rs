use agentcarousel_core::{Case, FixtureFile};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn collect_fixture_paths(inputs: &[PathBuf]) -> Vec<PathBuf> {
    collect_fixture_paths_with_ignore(inputs, None)
}

pub fn collect_fixture_paths_with_ignore(
    inputs: &[PathBuf],
    ignore_file: Option<&Path>,
) -> Vec<PathBuf> {
    let ignore_set = load_ignore_set(ignore_file);
    let is_ignored = |path: &Path| ignore_set.as_ref().is_some_and(|set| set.is_match(path));

    let mut paths = Vec::new();
    for input in inputs {
        if input.is_dir() {
            for entry in WalkDir::new(input).into_iter().filter_map(Result::ok) {
                if entry.file_type().is_file()
                    && is_fixture_file(entry.path())
                    && !is_ignored(entry.path())
                {
                    paths.push(entry.path().to_path_buf());
                }
            }
        } else if input.is_file() && !is_ignored(input) {
            paths.push(input.clone());
        }
    }
    paths
}

pub fn apply_case_filter(mut fixture: FixtureFile, filter: Option<&str>) -> FixtureFile {
    let Some(filter) = filter else { return fixture };
    let id_matcher = Glob::new(filter)
        .ok()
        .map(|pattern| pattern.compile_matcher());
    let cases: Vec<Case> = fixture
        .cases
        .into_iter()
        .filter(|case| match &id_matcher {
            Some(matcher) => matcher.is_match(&case.id.0),
            None => true,
        })
        .collect();
    fixture.cases = cases;
    fixture
}

/// Keep only cases whose `tags` intersect this list (any tag match is enough).
pub fn apply_tag_filter(mut fixture: FixtureFile, tags: Option<&[String]>) -> FixtureFile {
    let Some(tag_list) = tags.filter(|list| !list.is_empty()) else {
        return fixture;
    };
    let normalized_needles: Vec<String> = tag_list.iter().map(|t| normalize_tag(t)).collect();
    fixture.cases.retain(|case| {
        normalized_needles
            .iter()
            .any(|needle| case.tags.iter().any(|tag| normalize_tag(tag) == *needle))
    });
    fixture
}

pub fn default_concurrency() -> Option<usize> {
    std::thread::available_parallelism()
        .ok()
        .map(|value| value.get())
}

pub fn is_kebab_case(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

fn is_fixture_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("yaml") | Some("yml") | Some("toml")
    )
}

fn normalize_tag(value: &str) -> String {
    match value {
        "negative" => "smoke".to_string(),
        "positive" => "happy-path".to_string(),
        _ => value.to_string(),
    }
}

fn load_ignore_set(ignore_file: Option<&Path>) -> Option<GlobSet> {
    let path = ignore_file?;
    let contents = std::fs::read_to_string(path).ok()?;
    let mut builder = GlobSetBuilder::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(glob) = Glob::new(line) {
            builder.add(glob);
        }
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::{apply_tag_filter, is_kebab_case};
    use agentcarousel_core::{Case, CaseId, CaseInput, Expected, FixtureFile, Message, Role};

    fn sample_case(id: &str, tags: &[&str]) -> Case {
        Case {
            id: CaseId(id.to_string()),
            description: None,
            tags: tags.iter().map(|t| (*t).to_string()).collect(),
            input: CaseInput {
                messages: vec![Message {
                    role: Role::User,
                    content: "x".into(),
                }],
                context: None,
                env_overrides: None,
            },
            expected: Expected {
                tool_sequence: None,
                output: None,
                rubric: None,
            },
            evaluator_config: None,
            timeout_secs: None,
            seed: None,
        }
    }

    #[test]
    fn tag_filter_keeps_matching_cases_only() {
        let mut f = FixtureFile {
            schema_version: 1,
            skill_or_agent: "demo".into(),
            defaults: None,
            cases: vec![
                sample_case("demo/a", &["negative", "a"]),
                sample_case("demo/b", &["nightly"]),
            ],
            bundle_id: None,
            bundle_version: None,
            certification_track: None,
            risk_tier: None,
            data_handling: None,
        };
        f = apply_tag_filter(f, Some(&["negative".into()]));
        assert_eq!(f.cases.len(), 1);
        assert_eq!(f.cases[0].id.0, "demo/a");
    }

    #[test]
    fn tag_filter_supports_negative_smoke_aliases() {
        let mut f = FixtureFile {
            schema_version: 1,
            skill_or_agent: "demo".into(),
            defaults: None,
            cases: vec![
                sample_case("demo/a", &["negative"]),
                sample_case("demo/b", &["smoke"]),
                sample_case("demo/c", &["nightly"]),
            ],
            bundle_id: None,
            bundle_version: None,
            certification_track: None,
            risk_tier: None,
            data_handling: None,
        };
        f = apply_tag_filter(f, Some(&["smoke".into()]));
        assert_eq!(f.cases.len(), 2);
        assert_eq!(f.cases[0].id.0, "demo/a");
        assert_eq!(f.cases[1].id.0, "demo/b");
    }

    #[test]
    fn kebab_case_rejects_invalid_values() {
        assert!(!is_kebab_case(""));
        assert!(!is_kebab_case("-starts-with-dash"));
        assert!(!is_kebab_case("ends-with-dash-"));
        assert!(!is_kebab_case("double--dash"));
        assert!(!is_kebab_case("ContainsUppercase"));
        assert!(!is_kebab_case("has_underscore"));
    }

    #[test]
    fn kebab_case_accepts_valid_values() {
        assert!(is_kebab_case("abc"));
        assert!(is_kebab_case("abc-123"));
        assert!(is_kebab_case("a1-b2-c3"));
    }
}
