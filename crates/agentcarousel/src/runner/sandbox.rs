use std::collections::HashMap;
use std::env;

#[derive(Debug)]
pub struct SandboxGuard {
    previous: HashMap<String, Option<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to apply sandbox")]
    ApplyError,
}

pub struct Sandbox;

impl Sandbox {
    pub fn apply(overrides: &Option<HashMap<String, String>>, offline: bool) -> SandboxGuard {
        let mut previous = HashMap::new();
        scrub_secrets(overrides.as_ref(), &mut previous);
        if let Some(overrides) = overrides {
            for (key, value) in overrides {
                previous.insert(key.clone(), env::var(key).ok());
                env::set_var(key, value);
            }
        }
        if offline {
            previous.insert(
                "AGENTCAROUSEL_OFFLINE".to_string(),
                env::var("AGENTCAROUSEL_OFFLINE").ok(),
            );
            env::set_var("AGENTCAROUSEL_OFFLINE", "1");
        }
        SandboxGuard { previous }
    }
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }
}

fn scrub_secrets(
    overrides: Option<&HashMap<String, String>>,
    previous: &mut HashMap<String, Option<String>>,
) {
    let override_keys = overrides
        .map(|map| {
            map.keys()
                .map(|key| key.to_ascii_uppercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let secret_markers = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];
    let keys: Vec<String> = env::vars().map(|(key, _)| key).collect();
    for key in keys {
        let upper_key = key.to_ascii_uppercase();
        if should_preserve_runtime_secret(&upper_key) {
            continue;
        }
        if secret_markers
            .iter()
            .any(|marker| upper_key.contains(marker))
            && !override_keys
                .iter()
                .any(|override_key| override_key == &upper_key)
        {
            previous.insert(key.clone(), env::var(&key).ok());
            env::remove_var(key);
        }
    }
}

fn should_preserve_runtime_secret(upper_key: &str) -> bool {
    const PRESERVED_SECRET_KEYS: [&str; 9] = [
        "AGENTCAROUSEL_JUDGE_KEY",
        "AGENTCAROUSEL_GENERATOR_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "OPENROUTER_API_KEY",
        "agentcarousel_JUDGE_KEY",
        "agentcarousel_GENERATOR_KEY",
    ];
    PRESERVED_SECRET_KEYS
        .iter()
        .any(|candidate| upper_key == candidate.to_ascii_uppercase())
}
