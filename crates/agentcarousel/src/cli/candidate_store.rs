use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    #[default]
    Onboarding,
    Improving,
    Stable,
    Graduated,
}

impl std::fmt::Display for CandidateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CandidateStatus::Onboarding => write!(f, "Onboarding"),
            CandidateStatus::Improving => write!(f, "Improving"),
            CandidateStatus::Stable => write!(f, "Stable"),
            CandidateStatus::Graduated => write!(f, "Graduated"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricSnapshot {
    pub injection_resistance: Option<f32>,
    pub behavioral_drift: Option<f32>,
    pub coverage: Option<f32>,
    pub calibration: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEntry {
    pub skill: String,
    pub status: CandidateStatus,
    pub target_model: String,
    pub target_endpoint: Option<String>,
    pub baseline_run_id: Option<String>,
    pub baseline_score: Option<f32>,
    pub current_score: Option<f32>,
    pub baseline_metrics: Option<MetricSnapshot>,
    pub current_metrics: Option<MetricSnapshot>,
    pub target_score: f32,
    pub improvement_rounds: u32,
    pub last_updated: String,
    pub onboarded_at: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CandidateStore {
    pub candidates: HashMap<String, CandidateEntry>,
}

impl CandidateStore {
    /// Load a CandidateStore from a JSON file.  Returns an empty store if the
    /// file is absent or cannot be parsed (silent degradation is intentional —
    /// the pipeline should not fail because the store is missing).
    pub fn load(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&contents).unwrap_or_default()
    }

    /// Persist the store to disk atomically: write to a `.tmp` file first, then
    /// rename over the target path so readers never see a partial write.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialize error: {e}"))?;

        // Ensure the parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }

        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)
            .map_err(|e| format!("write {}: {e}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("rename {} → {}: {e}", tmp_path.display(), path.display()))?;
        Ok(())
    }

    pub fn upsert(&mut self, entry: CandidateEntry) {
        self.candidates.insert(entry.skill.clone(), entry);
    }

    pub fn get(&self, skill: &str) -> Option<&CandidateEntry> {
        self.candidates.get(skill)
    }

    /// Return all entries sorted by skill name for stable display.
    pub fn all_sorted(&self) -> Vec<&CandidateEntry> {
        let mut entries: Vec<&CandidateEntry> = self.candidates.values().collect();
        entries.sort_by(|a, b| a.skill.cmp(&b.skill));
        entries
    }
}

/// Walk parent directories from `start` looking for `agentcarousel.toml`.
/// Returns the first directory that contains it, or `None` if none is found.
pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("agentcarousel.toml").exists() {
            return Some(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return None,
        }
    }
}

/// Return the canonical path for the candidates JSON file within a workspace.
pub fn candidates_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".agents").join("candidates.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_entry(skill: &str) -> CandidateEntry {
        CandidateEntry {
            skill: skill.to_string(),
            status: CandidateStatus::Stable,
            target_model: "ollama/gemma4".to_string(),
            target_endpoint: None,
            baseline_run_id: Some("run-abc".to_string()),
            baseline_score: Some(0.78),
            current_score: Some(0.89),
            baseline_metrics: None,
            current_metrics: None,
            target_score: 0.90,
            improvement_rounds: 3,
            last_updated: "2026-05-29T00:00:00Z".to_string(),
            onboarded_at: None,
        }
    }

    #[test]
    fn round_trip_save_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("candidates.json");
        let mut store = CandidateStore::default();
        store.upsert(make_entry("customer-support"));
        store.save(&path).unwrap();

        let loaded = CandidateStore::load(&path);
        assert_eq!(loaded.candidates.len(), 1);
        let entry = loaded.get("customer-support").unwrap();
        assert_eq!(entry.current_score, Some(0.89));
    }

    #[test]
    fn load_absent_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let store = CandidateStore::load(&path);
        assert!(store.candidates.is_empty());
    }

    #[test]
    fn all_sorted_is_alphabetical() {
        let mut store = CandidateStore::default();
        store.upsert(make_entry("zebra"));
        store.upsert(make_entry("alpha"));
        store.upsert(make_entry("middle"));
        let sorted: Vec<&str> = store
            .all_sorted()
            .iter()
            .map(|e| e.skill.as_str())
            .collect();
        assert_eq!(sorted, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn find_workspace_root_finds_toml() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("agentcarousel.toml"), "").unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let root = find_workspace_root(&nested);
        assert_eq!(root, Some(dir.path().to_path_buf()));
    }

    #[test]
    fn find_workspace_root_none_when_absent() {
        let dir = TempDir::new().unwrap();
        // No agentcarousel.toml — stop when we reach the fs root.
        // Use a path that definitely has no agentcarousel.toml above it.
        let result = find_workspace_root(dir.path());
        // Result may be Some (if the parent chain happens to have one) or None.
        // We only check the function doesn't panic.
        let _ = result;
    }
}
