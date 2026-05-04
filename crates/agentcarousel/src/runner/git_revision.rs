//! Best-effort Git commit provenance for persisted runs and exports.

/// Prefer `GITHUB_SHA` in CI; otherwise run `git rev-parse HEAD` when available.
/// Returns `None` for non-git checkouts or when `git` is missing.
pub fn resolve_git_sha() -> Option<String> {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let trimmed = sha.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::resolve_git_sha;

    #[test]
    fn resolve_git_sha_succeeds_in_git_checkout() {
        let sha = resolve_git_sha();
        assert!(
            sha.is_some(),
            "expected git rev-parse HEAD in this workspace; set GITHUB_SHA in CI"
        );
        let len = sha.as_ref().unwrap().len();
        assert!(
            len == 40 || len == 64,
            "expected 40-char SHA-1 or 64-char SHA-256 git hash, got {len}"
        );
    }
}
