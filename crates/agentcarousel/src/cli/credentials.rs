// Read-only credential store access — used by publish, promote, compare, and doctor.

pub fn load_stored_token() -> Option<String> {
    file_load()
}

fn credentials_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".config")
        .join("agentcarousel")
        .join("credentials.toml")
}

fn file_load() -> Option<String> {
    let content = std::fs::read_to_string(credentials_path()).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(after_key) = trimmed.strip_prefix("token") else {
            continue;
        };
        if !after_key.starts_with(|c: char| c.is_whitespace() || c == '=') {
            continue;
        }
        let Some(after_eq) = after_key.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = after_eq.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}
