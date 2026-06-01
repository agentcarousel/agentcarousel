use oscal::catalog::{load_catalog, CatalogSource};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkControl {
    pub framework: String,
    pub control_id: String,
    pub requirement: String,
    /// Fixture tag used to map cases to this control (e.g. `"nist-800-53:AC-1"`).
    pub tag: String,
    /// Whether this control is primarily validated by behavioral test cases.
    pub behavioral: bool,
    /// Relative importance weight for scoring (default 1.0).
    pub importance: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlScore {
    pub control: FrameworkControl,
    pub model_version: String,
    pub effectiveness_mean: f32,
    pub pass_rate: f32,
    pub case_count: u32,
    pub run_count: u32,
    pub last_run_date: Option<String>,
    pub status: ControlCoverageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlCoverageStatus {
    /// All mapped cases pass above threshold.
    Satisfied,
    /// Some cases pass but below the required count or threshold.
    PartialEvidence,
    /// No mapped fixture cases found.
    Gap,
    /// Control is documented but validated by procedure, not automated tests.
    Procedural,
}

/// Merged view of all known framework controls, keyed by framework name.
pub type FrameworkRegistry = HashMap<String, Vec<FrameworkControl>>;

/// Build the framework registry from three sources in priority order:
/// 1. Embedded OSCAL catalogs from the `oscal` crate
/// 2. `./frameworks/*.json` in the working directory
/// 3. `~/.agentcarousel/frameworks/*.json`
///
/// Later sources can extend (but not remove) controls from earlier ones.
pub fn load_framework_registry() -> FrameworkRegistry {
    let mut registry: FrameworkRegistry = HashMap::new();

    load_embedded_catalogs(&mut registry);
    load_directory_catalogs(&mut registry, std::path::Path::new("frameworks"));
    if let Some(home) = home_frameworks_dir() {
        load_directory_catalogs(&mut registry, &home);
    }

    registry
}

/// Return all `FrameworkControl` entries for the named framework, or an empty slice.
pub fn controls_for_framework<'a>(
    registry: &'a FrameworkRegistry,
    name: &str,
) -> &'a [FrameworkControl] {
    registry.get(name).map(|v| v.as_slice()).unwrap_or(&[])
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn load_embedded_catalogs(registry: &mut FrameworkRegistry) {
    for name in oscal::catalogs::EMBEDDED_CATALOG_NAMES {
        let catalog = match load_catalog(CatalogSource::Embedded(name)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let controls = registry.entry(name.to_string()).or_default();
        for control in catalog.all_controls() {
            controls.push(FrameworkControl {
                framework: name.to_string(),
                control_id: control.id.clone(),
                requirement: control.statement().unwrap_or(&control.title).to_string(),
                tag: format!("{name}:{}", control.id),
                behavioral: true,
                importance: 1.0,
            });
        }
    }
}

fn load_directory_catalogs(registry: &mut FrameworkRegistry, dir: &std::path::Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let controls: Vec<FrameworkControl> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(framework) = controls.first().map(|c| c.framework.clone()) {
            registry.entry(framework).or_default().extend(controls);
        }
    }
}

fn home_frameworks_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".agentcarousel").join("frameworks"))
}
