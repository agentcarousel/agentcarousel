# AgentCarousel

**Unit tests for AI agents.** Define behavior in YAML, run offline tests, export signed evidence bundles your reviewers will accept.

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
[![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/agentcarousel/agentcarousel)](https://github.com/agentcarousel/agentcarousel/releases)

<img width="692" height="414" alt="demo" src="https://github.com/user-attachments/assets/c55df92c-fa4a-44b6-a381-23fe0329a5c4" />

## Why agentcarousel

- **Deterministic by default** - Offline runs with mocks mean same inputs → same outputs, every time.
- **Built for evidence** - Every run produces a signed artifact (`.tar.gz` + `minisign` attestation) you can hand to an auditor, a reviewer, or your customer's security team.
- **Live evals when you want them** - plug in OpenAI, Anthropic, Gemini, or OpenRouter as generator and judge, then diff runs to catch regressions.
- **Compliance-aware fixtures** - Risk tier, data handling, and certification track: the metadata your governance program already tracks, baked into the test format.

## Install

```bash
# Linux install; for Windows, download .zip from Releases
curl -fsSL https://install.agentcarousel.com | sh

# Homebrew (macOS)
brew tap agentcarousel/agentcarousel && brew install agentcarousel

# Cargo (Rust)
cargo install agentcarousel
```

## Quickstart

```bash
# Scaffold a skill fixture (creates fixtures/my-skill/ directory)
agentcarousel init --skill my-skill
agentcarousel init --agent my-agent

# Run it offline (no API keys needed)
agentcarousel test fixtures/my-skill/cases.yaml --offline true

# Validate fixture schema and rules
agentcarousel validate fixtures/regex-builder/cases.yaml

# Evaluate (mock by default)
agentcarousel eval fixtures/regex-builder/cases.yaml

# Export evidence bundle
agentcarousel export <RUN-ID>
```

## Example fixture — `regex-builder`

```yaml
schema_version: 1
skill_or_agent: regex-builder

cases:
  - id: regex-builder/positive-semver
    tags: [smoke, happy-path]
    input:
      messages:
        - role: user
          content: |
            Build a regex for semantic version strings.
            Must match: 1.2.3 | 0.0.1 | 1.0.0-alpha | 2.0.0-rc.1
            Must NOT match: 1.2 | .1.2.3 | 1.a.3
            Anchor the pattern to match the full string.
    expected:
      output:
        - kind: regex
          value: '\\d+\\.\\d+\\.\\d+'
        - kind: regex
          value: '(?i)(anchor|full.*string|\^|\$)'
        - kind: not_contains
          value: '(a+)+'
```

See [`fixtures/regex-builder/`](fixtures/regex-builder/) for the full fixture with all cases, golden outputs, and bundle manifest.

## Live Eval with LLM-as-a-judge

```bash
export GEMINI_API_KEY=gemini_key
export OPENROUTER_API_KEY=or_key
export ANTHROPIC_API_KEY=claude_api_key
export OPENAI_API_KEY=openai_key

# Run all cases, judge-backed cases use the judge
agentcarousel eval fixtures/ \
  --execution-mode live \
  --evaluator all --judge \
  --model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001 \
  --runs 1

# Narrow to specific cases by id glob or tag
agentcarousel eval fixtures/ \
  --evaluator all --judge \
  --filter "customer-support/judge-*" \
  --filter-tags certification
```

`--evaluator all` uses each case's declared evaluator; `--evaluator judge` forces every case through the judge regardless. Use `--filter` (glob on `skill/case-id`) or `--filter-tags` (comma-separated) to scope runs.

## Reports

```bash
# List recent runs (newest first)
agentcarousel report list

# Show a run (human-readable, same formatting as eval/test output)
agentcarousel report show <RUN-ID>

# Also accepts a path to run.json or an evidence directory
agentcarousel report show ./evidence/my-export/

# Diff two runs to surface regressions
agentcarousel report diff <RUN-ID-A> <RUN-ID-B>

# JSON output for scripting
agentcarousel report list --json
agentcarousel report show <RUN-ID> --json
```

## Configuration (`agentcarousel.toml`)

Copy `agentcarousel.example.toml` to `agentcarousel.toml` and customize as needed.

Per-case effectiveness thresholds override the global `--effectiveness-threshold` flag via the `evaluator_config.effectiveness_threshold` field in YAML.

## Bundle workflows

```bash
# Create a distributable bundle archive
agentcarousel bundle pack fixtures/regex-builder

# Verify bundle integrity and structure
agentcarousel bundle verify fixtures/customer-support
agentcarousel bundle verify my-bundle.tar.gz

# Pull bundle manifest + artifacts from the registry
agentcarousel bundle pull customer-support-1.0.0 --url "https://api.agentcarousel.com"
```

## Publish to registry

```bash
# Publish bundle + evidence in one flow
agentcarousel publish fixtures/customer-support \
  --url "https://api.agentcarousel.com"

# Publish multiple matching local runs (newest first)
agentcarousel publish fixtures/customer-support \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

## Trust checks

```bash
# Registry trust-state check
agentcarousel trust-check customer-support@1.0.0 \
  --url "https://api.agentcarousel.com"

# Optional offline attestation verification
agentcarousel trust-check customer-support@1.0.0 \
  --url "https://api.agentcarousel.com" \
  --attestation ./attestation-customer-support-1.0.0.json \
  --minisign-pubkey ./your-minisign.pub
```

## Contributions

- Start here: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, open an issue before implementation.
