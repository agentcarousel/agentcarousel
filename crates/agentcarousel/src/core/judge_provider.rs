//! Map a judge **model id string** to a [`JudgeProvider`] and discover API key environment
//! variable names for that stack.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgeProvider {
    Gemini,
    OpenAi,
    Anthropic,
    OpenRouter,
}

const GEMINI_KEY_ENV_CANDIDATES: [&str; 4] = [
    "AGENTCAROUSEL_JUDGE_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "agentcarousel_JUDGE_KEY",
];

const OPENAI_KEY_ENV_CANDIDATES: [&str; 3] = [
    "OPENAI_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];

const ANTHROPIC_KEY_ENV_CANDIDATES: [&str; 3] = [
    "ANTHROPIC_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];

const OPENROUTER_KEY_ENV_CANDIDATES: [&str; 3] = [
    "OPENROUTER_API_KEY",
    "AGENTCAROUSEL_JUDGE_KEY",
    "agentcarousel_JUDGE_KEY",
];

pub fn judge_provider_from_model(model: &str) -> JudgeProvider {
    let normalized = model.to_ascii_lowercase();
    if normalized.starts_with("openrouter/")
        || normalized.contains(":free")
        || normalized.contains('/')
    {
        return JudgeProvider::OpenRouter;
    }
    if normalized.starts_with("claude") {
        return JudgeProvider::Anthropic;
    }
    if normalized.starts_with("gpt")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
    {
        return JudgeProvider::OpenAi;
    }
    JudgeProvider::Gemini
}

pub fn judge_key_candidates(provider: JudgeProvider) -> &'static [&'static str] {
    match provider {
        JudgeProvider::Gemini => &GEMINI_KEY_ENV_CANDIDATES,
        JudgeProvider::OpenAi => &OPENAI_KEY_ENV_CANDIDATES,
        JudgeProvider::Anthropic => &ANTHROPIC_KEY_ENV_CANDIDATES,
        JudgeProvider::OpenRouter => &OPENROUTER_KEY_ENV_CANDIDATES,
    }
}
