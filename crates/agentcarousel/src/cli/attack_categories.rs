//! Attack categories for generated adversarial cases (`agc generate --attack-categories`).
//!
//! Each category carries a short generator brief in AgentCarousel's own words and
//! the tags it maps to: OWASP Top 10 for LLM Applications 2025 IDs, MITRE ATLAS
//! technique IDs, and AIUC-1 attack category names (names only). Tags are added to
//! generated cases deterministically, so they don't depend on the model.

use serde::Deserialize;
use std::sync::OnceLock;

const RAW: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/attack-categories.json"
));

#[derive(Debug, Deserialize)]
pub(crate) struct AttackCategory {
    pub key: String,
    pub label: String,
    pub prompt: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Vocabulary {
    categories: Vec<AttackCategory>,
}

pub(crate) fn all() -> &'static [AttackCategory] {
    static VOCAB: OnceLock<Vocabulary> = OnceLock::new();
    &VOCAB
        .get_or_init(|| serde_json::from_str(RAW).expect("bundled attack-categories.json is valid"))
        .categories
}

/// Resolve a list of category keys. An empty list selects every category.
pub(crate) fn select(keys: &[String]) -> Result<Vec<&'static AttackCategory>, String> {
    if keys.is_empty() {
        return Ok(all().iter().collect());
    }
    keys.iter()
        .map(|key| {
            all().iter().find(|c| c.key == *key).ok_or_else(|| {
                let valid: Vec<&str> = all().iter().map(|c| c.key.as_str()).collect();
                format!(
                    "--attack-categories: unknown category '{key}'; valid: {}",
                    valid.join(", ")
                )
            })
        })
        .collect()
}

/// Assign attack categories round-robin to the adversarial slots of a category plan.
/// Other slots get `None`.
pub(crate) fn assign(
    plan: &[&str],
    categories: &[&'static AttackCategory],
) -> Vec<Option<&'static AttackCategory>> {
    let mut next = 0usize;
    plan.iter()
        .map(|slot| {
            if *slot == "adversarial" && !categories.is_empty() {
                let c = categories[next % categories.len()];
                next += 1;
                Some(c)
            } else {
                None
            }
        })
        .collect()
}

/// Add `tags` to a generated case's `tags` array, keeping order and skipping duplicates.
pub(crate) fn inject_tags(case: &mut serde_json::Value, tags: &[String]) {
    let Some(obj) = case.as_object_mut() else {
        return;
    };
    let entry = obj
        .entry("tags")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = serde_json::Value::Array(Vec::new());
    }
    let arr = entry.as_array_mut().expect("tags is an array");
    for tag in tags {
        if !arr.iter().any(|t| t.as_str() == Some(tag)) {
            arr.push(serde_json::Value::String(tag.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vocabulary_loads_and_keys_are_unique() {
        let keys: std::collections::HashSet<_> = all().iter().map(|c| &c.key).collect();
        assert_eq!(keys.len(), all().len());
        assert!(all().len() >= 6);
        for c in all() {
            assert!(!c.tags.is_empty(), "{} has tags", c.key);
        }
    }

    #[test]
    fn select_rejects_unknown_keys() {
        assert!(select(&["prompt-injection-direct".to_string()]).is_ok());
        let err = select(&["nope".to_string()]).unwrap_err();
        assert!(err.contains("unknown category 'nope'"));
        assert_eq!(select(&[]).unwrap().len(), all().len());
    }

    #[test]
    fn assign_is_round_robin_over_adversarial_slots_only() {
        let cats = select(&["persuasion".to_string(), "false-scenario".to_string()]).unwrap();
        let plan = [
            "happy_path",
            "adversarial",
            "edge_case",
            "adversarial",
            "adversarial",
        ];
        let got: Vec<Option<&str>> = assign(&plan, &cats)
            .into_iter()
            .map(|c| c.map(|c| c.key.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                None,
                Some("persuasion"),
                None,
                Some("false-scenario"),
                Some("persuasion")
            ]
        );
    }

    #[test]
    fn inject_tags_is_deterministic_and_idempotent() {
        let mut case = json!({"id": "s/a", "tags": ["smoke", "security"]});
        let tags = vec!["security".to_string(), "owasp-llm:LLM01".to_string()];
        inject_tags(&mut case, &tags);
        inject_tags(&mut case, &tags);
        assert_eq!(
            case["tags"],
            json!(["smoke", "security", "owasp-llm:LLM01"])
        );

        let mut untagged = json!({"id": "s/b"});
        inject_tags(&mut untagged, &tags);
        assert_eq!(untagged["tags"], json!(["security", "owasp-llm:LLM01"]));
    }
}
