//! Capture the configuration identity of a run (assurance spec §1.3): which fixture files,
//! system prompt, golden references, judge, and declared agent were evaluated, bound by
//! SHA-256 content digests.
use crate::core::hex_util::hex_lower;
use agentcarousel_core::{
    AgentDescriptor, Case, FileDigest, FixtureFile, JudgeSettings, Role, RunSubject,
};
use agentcarousel_evaluators::JUDGE_TEMPERATURE;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// `sha256:<hex>` of a file's raw bytes, or `None` if it cannot be read.
pub(crate) fn file_digest(path: &Path) -> Option<FileDigest> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(FileDigest {
        path: display_path(path),
        sha256: format!("sha256:{}", hex_lower(hasher.finalize().as_ref())),
    })
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Build the run subject. Call with the fixtures *after* filtering: `excluded_cases` still
/// count toward the suite, so filtering never shrinks the recorded configuration.
pub(crate) fn build_run_subject(
    fixture_paths: &[PathBuf],
    fixtures: &[FixtureFile],
    generation_mode: &str,
    judge_model: Option<&str>,
) -> RunSubject {
    let fixture_files = fixture_paths
        .iter()
        .filter_map(|path| file_digest(path))
        .collect();
    let all_cases: Vec<&Case> = fixtures
        .iter()
        .flat_map(|f| f.cases.iter().chain(f.excluded_cases.iter()))
        .collect();
    RunSubject {
        agent: single_declared_agent(fixtures),
        generation_mode: Some(generation_mode.to_string()),
        fixture_files,
        suite_case_count: all_cases.len() as u32,
        system_prompt: fallback_system_prompt(&all_cases),
        golden_files: golden_digests(fixtures),
        judge: judge_model.map(|model| JudgeSettings {
            model: model.to_string(),
            temperature: JUDGE_TEMPERATURE,
        }),
    }
}

/// The declared agent, only when every fixture declares the same one.
fn single_declared_agent(fixtures: &[FixtureFile]) -> Option<AgentDescriptor> {
    let first = fixtures.first()?.agent.clone()?;
    fixtures
        .iter()
        .all(|f| f.agent.as_ref() == Some(&first))
        .then_some(first)
}

/// Mirrors the generator's resolution: a case without an inline `role: system` message uses
/// `fixtures/<skill>/prompt.md`. Recorded only when exactly one such prompt applies.
fn fallback_system_prompt(cases: &[&Case]) -> Option<FileDigest> {
    let skills: BTreeSet<&str> = cases
        .iter()
        .filter(|case| !case.input.messages.iter().any(|m| m.role == Role::System))
        .filter_map(|case| case.id.0.split('/').next())
        .collect();
    let digests: Vec<FileDigest> = skills
        .into_iter()
        .filter_map(|skill| file_digest(&PathBuf::from("fixtures").join(skill).join("prompt.md")))
        .collect();
    match digests.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn golden_digests(fixtures: &[FixtureFile]) -> Vec<FileDigest> {
    let paths: BTreeSet<PathBuf> = fixtures
        .iter()
        .flat_map(|f| f.cases.iter().chain(f.excluded_cases.iter()))
        .filter_map(|case| case.evaluator_config.as_ref()?.golden_path.clone())
        .collect();
    paths.iter().filter_map(|path| file_digest(path)).collect()
}
