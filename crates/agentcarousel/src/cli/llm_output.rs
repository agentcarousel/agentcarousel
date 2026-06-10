//! Post-processing helpers for raw LLM-generated YAML.
//!
//! Small/local models regularly produce text-level and structural defects.
//! These two public functions normalise them so callers can parse cleanly.

use std::borrow::Cow;
use std::fmt::Write as _;

/// Strip markdown fences and sanitize YAML in a single call.
///
/// Handles every defect class observed in local-model output:
/// - markdown fences (` ```yaml ` / ` ``` `)
/// - tab indentation (YAML forbids tabs)
/// - stray `# comment`, `@annotation`, and `` `backtick` `` lines
/// - unquoted inline scalar values containing YAML-special characters
/// - unclosed single-quoted scalars (truncated model output)
pub(crate) fn prepare_llm_yaml(raw: &str) -> String {
    repair_unclosed(sanitize(strip_fences(raw.trim())))
}

/// Hoist `output`, `tool_sequence`, and `rubric` from the case root into
/// `expected` when the model placed them at the wrong nesting level.
pub(crate) fn normalize_expected_block(value: &mut serde_json::Value) {
    let Some(cases) = value.get_mut("cases").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for case in cases.iter_mut() {
        let Some(obj) = case.as_object_mut() else {
            continue;
        };
        if obj.contains_key("expected") {
            // `expected` exists — only ensure rubric is nested inside it.
            if let Some(rubric) = obj.remove("rubric") {
                if let Some(exp) = obj.get_mut("expected").and_then(|e| e.as_object_mut()) {
                    exp.entry("rubric").or_insert(rubric);
                }
            }
        } else {
            // Build `expected` from its canonical children wherever the model dropped them.
            let output = obj.remove("output");
            let tool_sequence = obj.remove("tool_sequence");
            let rubric = obj.remove("rubric");
            if output.is_some() || tool_sequence.is_some() || rubric.is_some() {
                let mut exp = serde_json::Map::new();
                if let Some(v) = output {
                    exp.insert("output".into(), v);
                }
                if let Some(v) = tool_sequence {
                    exp.insert("tool_sequence".into(), v);
                }
                if let Some(v) = rubric {
                    exp.insert("rubric".into(), v);
                }
                obj.insert("expected".into(), serde_json::Value::Object(exp));
            }
        }
    }
}

// ── private ───────────────────────────────────────────────────────────────────

/// Close single-quoted scalars that have no matching closing quote on their line.
///
/// When a local model's output is truncated mid-value, or it simply forgets the
/// closing `'`, the YAML parser treats the entire rest of the document as the
/// scalar body and reports "unexpected end of stream while scanning a quoted
/// scalar". Detecting closure is straightforward: inside a single-quoted YAML
/// scalar `''` is an escaped quote, so after stripping the opening `'` the
/// remaining content must have an **odd** number of `'` chars (the last one being
/// the closing delimiter). An even count means the scalar is unclosed; we append
/// the missing `'` to end that line.
fn repair_unclosed(text: String) -> String {
    const KEYS: &[&str] = &[
        "value:",
        "description:",
        "id:",
        "kind:",
        "message:",
        "content:",
        "rationale:",
    ];

    if !text.contains('\'') {
        return text;
    }

    let mut out = String::with_capacity(text.len() + 8);
    for line in text.lines() {
        let trimmed = line.trim_start();
        let mut closed = false;

        'keys: for key in KEYS {
            let Some(rest) = trimmed.strip_prefix(key) else {
                continue;
            };
            let scalar = rest.trim_start();
            if !scalar.starts_with('\'') {
                break 'keys;
            }
            // Count `'` chars after the opening quote.
            // Odd  → at least one closing `'` is present → properly closed.
            // Even → no closing `'` → unclosed.
            let n = scalar[1..].chars().filter(|&c| c == '\'').count();
            if n % 2 == 0 {
                out.push_str(line);
                out.push('\'');
                closed = true;
            }
            break 'keys;
        }

        if !closed {
            out.push_str(line);
        }
        out.push('\n');
    }

    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }
    out
}

fn strip_fences(text: &str) -> &str {
    text.strip_prefix("```yaml")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(text)
}

fn sanitize(text: &str) -> String {
    const SPECIAL: &[char] = &[':', '{', '}', '[', ']', '#', '&', '*', '!', '|', '>', '?'];
    const KEYS: &[&str] = &[
        "value:",
        "description:",
        "id:",
        "kind:",
        "message:",
        "content:",
        "rationale:",
    ];

    let mut out = String::with_capacity(text.len() + 64);
    for raw_line in text.lines() {
        let line = detab(raw_line);
        let line = line.as_ref();
        let trimmed = line.trim_start();

        // Drop comment lines and YAML-reserved-indicator lines.
        if trimmed.starts_with(['#', '@', '`']) {
            continue;
        }

        match try_quote_scalar(line, trimmed, KEYS, SPECIAL) {
            Some(quoted) => out.push_str(&quoted),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    // Preserve the original trailing-newline contract.
    if out.ends_with('\n') && !text.ends_with('\n') {
        out.pop();
    }
    out
}

/// Replace leading tabs with two spaces each.
/// Returns a borrowed slice when no tabs are present (zero allocation).
fn detab(line: &str) -> Cow<'_, str> {
    let tabs = line.bytes().take_while(|&b| b == b'\t').count();
    if tabs == 0 {
        return Cow::Borrowed(line);
    }
    let mut s = String::with_capacity(tabs * 2 + line.len() - tabs);
    for _ in 0..tabs {
        s.push_str("  ");
    }
    s.push_str(&line[tabs..]);
    Cow::Owned(s)
}

/// If `line` starts with a known scalar key whose inline value contains
/// YAML-special characters, return a single-quoted replacement.
/// Returns `None` when no rewrite is needed.
fn try_quote_scalar(line: &str, trimmed: &str, keys: &[&str], special: &[char]) -> Option<String> {
    let key = keys.iter().copied().find(|k| trimmed.starts_with(k))?;
    let scalar = trimmed[key.len()..].trim_start();
    if scalar.is_empty() {
        return None; // value is on the next line (block scalar)
    }
    let first = scalar.chars().next()?;
    if matches!(first, '\'' | '"' | '|' | '>') {
        return None; // already quoted or block indicator
    }
    if !scalar.chars().any(|c| special.contains(&c)) {
        return None; // no special chars — plain scalar is fine
    }
    let indent = &line[..line.len() - trimmed.len()];
    let escaped = scalar.replace('\'', "''");
    let mut out = String::with_capacity(indent.len() + key.len() + 3 + escaped.len());
    write!(out, "{indent}{key} '{escaped}'").ok();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_closes_unclosed_single_quote() {
        let input = "cases:\n  - id: x\n    value: '(?i)pattern\n    kind: regex\n";
        let out = prepare_llm_yaml(input);
        // The unclosed `value:` line should gain a closing `'`.
        assert!(out.contains("value: '(?i)pattern'"), "got: {out}");
        // The following line must survive unchanged.
        assert!(out.contains("kind: regex"), "got: {out}");
    }

    #[test]
    fn repair_leaves_closed_quote_alone() {
        let input = "cases:\n  - value: '(?i)already closed'\n    kind: regex\n";
        let out = prepare_llm_yaml(input);
        assert!(out.contains("value: '(?i)already closed'"), "got: {out}");
    }

    #[test]
    fn repair_handles_escaped_interior_quote() {
        // `it''s` has 2 single quotes → even → unclosed → gets closed
        let input = "    value: 'it''s truncated\n";
        let out = prepare_llm_yaml(input);
        assert!(out.contains("value: 'it''s truncated'"), "got: {out}");
    }

    #[test]
    fn repair_leaves_correctly_escaped_interior_alone() {
        // `it''s fine'` → 3 single quotes after opening → odd → closed
        let input = "    value: 'it''s fine'\n";
        let out = prepare_llm_yaml(input);
        assert!(out.contains("value: 'it''s fine'"), "got: {out}");
        assert!(!out.contains("fine''"), "double-closed: {out}");
    }
}
