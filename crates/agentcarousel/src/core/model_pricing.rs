//! Live model pricing fetched from LiteLLM and OpenRouter public APIs (no auth required).
//!
//! Call [`prefetch_pricing`] once at startup. [`lookup_pricing`] returns cached data.
//! [`annotate_run_cost`] stamps cost fields on a completed [`Run`].

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

use crate::core::models::Run;

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub prompt_usd_per_token: f64,
    pub completion_usd_per_token: f64,
}

impl ModelPricing {
    pub fn cost(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        self.prompt_usd_per_token * tokens_in as f64
            + self.completion_usd_per_token * tokens_out as f64
    }
}

static PRICING_CACHE: OnceLock<HashMap<String, ModelPricing>> = OnceLock::new();

// ── LiteLLM response structs ────────────────────────────────────────────────

#[derive(Deserialize)]
struct LiteLlmEntry {
    input_cost_per_token: Option<f64>,
    output_cost_per_token: Option<f64>,
}

// ── OpenRouter response structs ─────────────────────────────────────────────

#[derive(Deserialize)]
struct OpenRouterResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Deserialize)]
struct OpenRouterModel {
    id: String,
    pricing: OpenRouterPricing,
}

#[derive(Deserialize)]
struct OpenRouterPricing {
    prompt: Option<String>,
    completion: Option<String>,
}

// ── Fetch helpers ────────────────────────────────────────────────────────────

fn fetch_litellm() -> HashMap<String, ModelPricing> {
    let url = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap_or_default();
    let Ok(resp) = client.get(url).send() else {
        return HashMap::new();
    };
    let Ok(map): Result<HashMap<String, LiteLlmEntry>, _> = resp.json() else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(name, entry)| {
            let p = entry.input_cost_per_token?;
            let c = entry.output_cost_per_token?;
            Some((
                name,
                ModelPricing {
                    prompt_usd_per_token: p,
                    completion_usd_per_token: c,
                },
            ))
        })
        .collect()
}

fn fetch_openrouter() -> HashMap<String, ModelPricing> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap_or_default();
    let Ok(resp) = client.get("https://openrouter.ai/api/v1/models").send() else {
        return HashMap::new();
    };
    let Ok(body): Result<OpenRouterResponse, _> = resp.json() else {
        return HashMap::new();
    };
    body.data
        .into_iter()
        .filter_map(|model| {
            let p = model
                .pricing
                .prompt
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok())?;
            let c = model
                .pricing
                .completion
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok())?;
            Some((
                model.id,
                ModelPricing {
                    prompt_usd_per_token: p,
                    completion_usd_per_token: c,
                },
            ))
        })
        .collect()
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Fetch and cache pricing from LiteLLM and OpenRouter in parallel.
/// Safe to call multiple times — only fetches on the first call.
pub fn prefetch_pricing() {
    PRICING_CACHE.get_or_init(|| {
        let litellm_handle = std::thread::spawn(fetch_litellm);
        let openrouter_handle = std::thread::spawn(fetch_openrouter);

        let mut merged: HashMap<String, ModelPricing> = litellm_handle.join().unwrap_or_default();

        // OpenRouter data takes precedence (more accurate for OR-routed models)
        let or_data = openrouter_handle.join().unwrap_or_default();
        for (k, v) in or_data {
            merged.insert(k, v);
        }
        merged
    });
}

/// Look up per-token pricing for a model name.
/// Strips the `openrouter/` prefix before lookup.
pub fn lookup_pricing(model: &str) -> Option<&'static ModelPricing> {
    let cache = PRICING_CACHE.get()?;
    let key = model.strip_prefix("openrouter/").unwrap_or(model);
    cache.get(key)
}

/// Annotate a completed run with USD cost estimates.
/// Call after `run_eval()` and before `persist_run()`. No-op if pricing is unavailable.
pub fn annotate_run_cost(run: &mut Run, gen_model: &str, judge_model: Option<&str>) {
    let gen_cost = lookup_pricing(gen_model).and_then(|p| {
        let ti = run.summary.tokens_in?;
        let to = run.summary.tokens_out?;
        Some(p.cost(ti, to))
    });

    let judge_cost = judge_model.and_then(|jm| {
        let p = lookup_pricing(jm)?;
        let ti = run.summary.judge_tokens_in?;
        let to = run.summary.judge_tokens_out?;
        Some(p.cost(ti, to))
    });

    let total_cost = match (gen_cost, judge_cost) {
        (Some(g), Some(j)) => Some(g + j),
        (Some(g), None) => Some(g),
        (None, Some(j)) => Some(j),
        (None, None) => None,
    };

    run.summary.gen_cost_usd = gen_cost;
    run.summary.judge_cost_usd = judge_cost;
    run.summary.total_cost_usd = total_cost;
}

// ── Display helpers ──────────────────────────────────────────────────────────

/// Format a USD cost for terminal display: `"$0.0042"`, `"$0.12"`, or `"N/A"`.
pub fn fmt_cost(cost: Option<f64>) -> String {
    match cost {
        Some(c) => {
            if c < 0.001 {
                format!("${:.6}", c)
            } else if c < 0.01 {
                format!("${:.4}", c)
            } else {
                format!("${:.2}", c)
            }
        }
        None => "N/A".to_string(),
    }
}

/// Format a token count compactly: `"1.2k"`, `"380"`, or `"N/A"`.
pub fn fmt_tokens(count: Option<u64>) -> String {
    match count {
        Some(n) if n >= 1000 => format!("{:.1}k", n as f64 / 1000.0),
        Some(n) => n.to_string(),
        None => "N/A".to_string(),
    }
}
