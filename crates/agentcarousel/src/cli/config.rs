use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    ReadError {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    ParseError {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("HOME environment variable is not set")]
    MissingHome,
    #[error("config path not found: {path}")]
    NotFound { path: PathBuf },
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    pub runner: RunnerConfig,
    pub validate: ValidateConfig,
    pub eval: EvalConfig,
    pub generator: GeneratorConfig,
    pub judge: JudgeConfig,
    pub report: ReportConfig,
    pub output: OutputConfig,
    pub msp: MspConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerConfig {
    pub concurrency: Option<usize>,
    pub timeout_secs: u64,
    pub offline: bool,
    pub mock_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidateConfig {
    pub schema_dir: PathBuf,
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalConfig {
    pub default_evaluator: String,
    pub effectiveness_threshold: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratorConfig {
    pub model: String,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgeConfig {
    pub model: String,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportConfig {
    pub history_db: Option<PathBuf>,
    pub regression_threshold: f32,
    pub max_history_runs: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputConfig {
    pub color: String,
    pub format: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MspConfig {
    pub registry_endpoint: Option<String>,
    pub auto_upload_on_eval: bool,
    pub bundle_sync_on_pull: bool,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            runner: RunnerConfig {
                concurrency: None,
                timeout_secs: 30,
                offline: true,
                mock_dir: PathBuf::from("mocks"),
            },
            validate: ValidateConfig {
                schema_dir: PathBuf::from("fixtures/schemas"),
                strict: false,
            },
            eval: EvalConfig {
                default_evaluator: "rules".to_string(),
                effectiveness_threshold: 0.7,
            },
            generator: GeneratorConfig {
                model: "claude-3-5-sonnet".to_string(),
                max_tokens: Some(1024),
            },
            judge: JudgeConfig {
                model: "gemini-2.5-flash".to_string(),
                max_tokens: Some(1536),
            },
            report: ReportConfig {
                history_db: None,
                regression_threshold: 0.05,
                max_history_runs: Some(500),
            },
            output: OutputConfig {
                color: "auto".to_string(),
                format: "human".to_string(),
            },
            msp: MspConfig {
                registry_endpoint: None,
                auto_upload_on_eval: false,
                bundle_sync_on_pull: true,
            },
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    runner: Option<FileRunnerConfig>,
    validate: Option<FileValidateConfig>,
    eval: Option<FileEvalConfig>,
    generator: Option<FileGeneratorConfig>,
    judge: Option<FileJudgeConfig>,
    report: Option<FileReportConfig>,
    output: Option<FileOutputConfig>,
    msp: Option<FileMspConfig>,
}

#[derive(Debug, Deserialize)]
struct FileRunnerConfig {
    concurrency: Option<usize>,
    timeout_secs: Option<u64>,
    offline: Option<bool>,
    mock_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct FileValidateConfig {
    schema_dir: Option<PathBuf>,
    strict: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct FileEvalConfig {
    default_evaluator: Option<String>,
    effectiveness_threshold: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct FileGeneratorConfig {
    model: Option<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FileJudgeConfig {
    model: Option<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FileReportConfig {
    history_db: Option<PathBuf>,
    regression_threshold: Option<f32>,
    max_history_runs: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FileOutputConfig {
    color: Option<String>,
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileMspConfig {
    registry_endpoint: Option<String>,
    auto_upload_on_eval: Option<bool>,
    bundle_sync_on_pull: Option<bool>,
}

pub fn load_config(path_override: Option<&Path>) -> Result<ResolvedConfig, ConfigError> {
    let mut resolved = ResolvedConfig::default();
    let path = resolve_config_path(path_override)?;
    if let Some(path) = path {
        let contents = fs::read_to_string(&path).map_err(|source| ConfigError::ReadError {
            path: path.clone(),
            source,
        })?;
        let file: FileConfig =
            toml::from_str(&contents).map_err(|source| ConfigError::ParseError {
                path: path.clone(),
                source,
            })?;
        apply_file_config(&mut resolved, file);
    }
    Ok(resolved)
}

pub fn config_hash(config: &ResolvedConfig) -> String {
    // Hash resolved config to keep run records reproducible.
    let payload = serde_json::to_vec(config).unwrap_or_else(|_| b"{}".to_vec());
    let mut hasher = Sha256::new();
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

pub fn resolve_schema_path(config: &ResolvedConfig) -> PathBuf {
    // Resolve schema file from configured schema directory.
    config
        .validate
        .schema_dir
        .join("skill-definition.schema.json")
}

pub fn apply_history_db_env(config: &ResolvedConfig) {
    if let Some(path) = &config.report.history_db {
        let expanded = expand_tilde(path);
        env::set_var("AGENTCAROUSEL_HISTORY_DB", expanded);
    }
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(stripped) = path_str.strip_prefix("~/") {
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    path.to_path_buf()
}

fn resolve_config_path(path_override: Option<&Path>) -> Result<Option<PathBuf>, ConfigError> {
    if let Some(path) = path_override {
        if path.exists() {
            return Ok(Some(path.to_path_buf()));
        }
        return Err(ConfigError::NotFound {
            path: path.to_path_buf(),
        });
    }

    let local = PathBuf::from("agentcarousel.toml");
    if local.exists() {
        return Ok(Some(local));
    }

    let home = env::var("HOME").map_err(|_| ConfigError::MissingHome)?;
    // XDG config path fallback.
    let xdg = PathBuf::from(home).join(".config/agentcarousel/config.toml");
    if xdg.exists() {
        return Ok(Some(xdg));
    }

    Ok(None)
}

fn apply_file_config(resolved: &mut ResolvedConfig, file: FileConfig) {
    if let Some(runner) = file.runner {
        if runner.concurrency.is_some() {
            resolved.runner.concurrency = runner.concurrency;
        }
        if let Some(timeout) = runner.timeout_secs {
            resolved.runner.timeout_secs = timeout;
        }
        if let Some(offline) = runner.offline {
            resolved.runner.offline = offline;
        }
        if let Some(mock_dir) = runner.mock_dir {
            resolved.runner.mock_dir = mock_dir;
        }
    }

    if let Some(validate) = file.validate {
        if let Some(schema_dir) = validate.schema_dir {
            resolved.validate.schema_dir = schema_dir;
        }
        if let Some(strict) = validate.strict {
            resolved.validate.strict = strict;
        }
    }

    if let Some(eval) = file.eval {
        if let Some(default_evaluator) = eval.default_evaluator {
            resolved.eval.default_evaluator = default_evaluator;
        }
        if let Some(threshold) = eval.effectiveness_threshold {
            resolved.eval.effectiveness_threshold = threshold;
        }
    }

    if let Some(generator) = file.generator {
        if let Some(model) = generator.model {
            resolved.generator.model = model;
        }
        if let Some(max_tokens) = generator.max_tokens {
            resolved.generator.max_tokens = Some(max_tokens);
        }
    }

    if let Some(judge) = file.judge {
        if let Some(model) = judge.model {
            resolved.judge.model = model;
        }
        if let Some(max_tokens) = judge.max_tokens {
            resolved.judge.max_tokens = Some(max_tokens);
        }
    }

    if let Some(report) = file.report {
        if let Some(history_db) = report.history_db {
            resolved.report.history_db = Some(history_db);
        }
        if let Some(threshold) = report.regression_threshold {
            resolved.report.regression_threshold = threshold;
        }
        if let Some(max_runs) = report.max_history_runs {
            resolved.report.max_history_runs = Some(max_runs);
        }
    }

    if let Some(output) = file.output {
        if let Some(color) = output.color {
            resolved.output.color = color;
        }
        if let Some(format) = output.format {
            resolved.output.format = format;
        }
    }

    if let Some(msp) = file.msp {
        if let Some(endpoint) = msp.registry_endpoint {
            resolved.msp.registry_endpoint = Some(endpoint);
        }
        if let Some(auto_upload) = msp.auto_upload_on_eval {
            resolved.msp.auto_upload_on_eval = auto_upload;
        }
        if let Some(sync) = msp.bundle_sync_on_pull {
            resolved.msp.bundle_sync_on_pull = sync;
        }
    }
}
