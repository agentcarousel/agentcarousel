//! Shared HTTP retry policy used by the judge evaluator and generator.

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_ms: u64,
}

pub fn retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: std::env::var("AGENTCAROUSEL_RETRY_MAX_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(6),
        base_backoff_ms: std::env::var("AGENTCAROUSEL_RETRY_BASE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(250),
        max_backoff_ms: std::env::var("AGENTCAROUSEL_RETRY_MAX_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3000),
        jitter_ms: std::env::var("AGENTCAROUSEL_RETRY_JITTER_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(150),
    }
}

pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
        || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
        || status == reqwest::StatusCode::GATEWAY_TIMEOUT
}

pub fn compute_backoff_ms(attempt: usize, retry: &RetryPolicy) -> u64 {
    let exponent = attempt.min(10) as u32;
    let base = (retry.base_backoff_ms.saturating_mul(1_u64 << exponent)).min(retry.max_backoff_ms);
    let jitter = if retry.jitter_ms == 0 {
        0
    } else {
        // Deterministic lightweight jitter from subsecond clock to avoid retry stampedes.
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0))
            % retry.jitter_ms
    };
    base.saturating_add(jitter)
}
