use serde::Deserialize;

use super::config::ResolvedConfig;

const LOCAL_CONFIG_FILE: &str = "agentcarousel.local.toml";

#[derive(Debug, Default, Deserialize)]
pub struct LocalProfile {
    pub generator: Option<LocalGeneratorSettings>,
    pub judge: Option<LocalJudgeSettings>,
    pub pipeline: Option<LocalPipelineSettings>,
}

#[derive(Debug, Deserialize)]
pub struct LocalGeneratorSettings {
    pub model: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LocalJudgeSettings {
    pub model: Option<String>,
    pub endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LocalPipelineSettings {
    pub target_score: Option<f32>,
    pub max_rounds: Option<u32>,
}

impl LocalProfile {
    /// Load `agentcarousel.local.toml` from the current working directory.
    /// If the file is absent or cannot be parsed, returns a default (no-op) profile.
    pub fn load() -> Self {
        let Ok(contents) = std::fs::read_to_string(LOCAL_CONFIG_FILE) else {
            return Self::default();
        };
        toml::from_str(&contents).unwrap_or_default()
    }

    /// Merge non-`None` fields from this local profile into `config`.
    ///
    /// The merge order is: CLI flag > local profile > agentcarousel.toml defaults.
    /// CLI flags take precedence at the command level; this function handles the
    /// layer between agentcarousel.toml and the command-line.
    pub fn apply_to(&self, config: &mut ResolvedConfig) {
        if let Some(gen) = &self.generator {
            if let Some(model) = &gen.model {
                config.generator.model = model.clone();
            }
            if let Some(endpoint) = &gen.endpoint {
                config.generator.endpoint = Some(endpoint.clone());
            }
        }
        if let Some(judge) = &self.judge {
            if let Some(model) = &judge.model {
                config.judge.model = model.clone();
            }
            // Note: judge.endpoint is not stored on ResolvedConfig (JudgeSettings has no
            // endpoint field).  It is surfaced via LocalProfile::judge_endpoint() and
            // threaded into pipeline commands directly.
        }
    }

    /// Return the judge endpoint from the local profile, if set.
    pub fn judge_endpoint(&self) -> Option<&str> {
        self.judge.as_ref().and_then(|j| j.endpoint.as_deref())
    }

    /// Return the generator endpoint from the local profile, if set.
    pub fn generator_endpoint(&self) -> Option<&str> {
        self.generator.as_ref().and_then(|g| g.endpoint.as_deref())
    }

    /// Return the pipeline target score from the local profile, if set.
    pub fn target_score(&self) -> Option<f32> {
        self.pipeline.as_ref().and_then(|p| p.target_score)
    }

    /// Return the pipeline max rounds from the local profile, if set.
    pub fn max_rounds(&self) -> Option<u32> {
        self.pipeline.as_ref().and_then(|p| p.max_rounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_from_toml(s: &str) -> LocalProfile {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn empty_toml_gives_default() {
        let p: LocalProfile = toml::from_str("").unwrap();
        assert!(p.generator.is_none());
        assert!(p.judge.is_none());
        assert!(p.pipeline.is_none());
    }

    #[test]
    fn apply_to_sets_generator_model_and_endpoint() {
        let p = profile_from_toml(
            r#"
[generator]
model    = "ollama/gemma4"
endpoint = "http://localhost:11434/api/generate"
"#,
        );
        let mut config = ResolvedConfig::default();
        p.apply_to(&mut config);
        assert_eq!(config.generator.model, "ollama/gemma4");
        assert_eq!(
            config.generator.endpoint.as_deref(),
            Some("http://localhost:11434/api/generate")
        );
    }

    #[test]
    fn apply_to_sets_judge_model() {
        let p = profile_from_toml(
            r#"
[judge]
model = "ollama/gemma4"
"#,
        );
        let mut config = ResolvedConfig::default();
        p.apply_to(&mut config);
        assert_eq!(config.judge.model, "ollama/gemma4");
    }

    #[test]
    fn judge_endpoint_accessor() {
        let p = profile_from_toml(
            r#"
[judge]
endpoint = "http://192.168.1.161:11434/api/generate"
"#,
        );
        assert_eq!(
            p.judge_endpoint(),
            Some("http://192.168.1.161:11434/api/generate")
        );
    }

    #[test]
    fn pipeline_accessors() {
        let p = profile_from_toml(
            r#"
[pipeline]
target_score = 0.85
max_rounds   = 5
"#,
        );
        assert_eq!(p.target_score(), Some(0.85));
        assert_eq!(p.max_rounds(), Some(5));
    }

    #[test]
    fn missing_file_gives_default_profile() {
        // Load from a guaranteed-absent path by changing env just for this call.
        // Since LocalProfile::load() reads the CWD file, and we can't change CWD
        // safely here, we just verify the Default impl works.
        let p = LocalProfile::default();
        assert!(p.generator_endpoint().is_none());
        assert!(p.judge_endpoint().is_none());
        assert!(p.target_score().is_none());
        assert!(p.max_rounds().is_none());
    }
}
