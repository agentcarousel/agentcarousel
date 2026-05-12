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
- **Live evals when you want them** - plug in OpenAI, Anthropic, Gemini, or OpenRouter as generator and judge. Diff runs. Catch regressions.
- **Compliance-aware fixtures** - Risk tier, data handling, certification track — the metadata your governance program already tracks, baked into the test format.

## Install

```bash
# Install (Linux — Windows: download .zip from Releases)
curl -fsSL https://install.agentcarousel.com | sh

# Homebrew (macOS)
brew tap agentcarousel/agentcarousel && brew install agentcarousel

# Cargo (Rust)
cargo install agentcarousel
```

## Quickstart

```bash
# Scaffold a fixture
agentcarousel init --skill my-agent

# Run it offline — no API keys needed
agentcarousel test --offline true

# Validate
agentcarousel validate fixtures/skills/cmmc-assessor.yaml

# Eval
agentcarousel eval fixtures/skills/cmmc-assessor.yaml

# Export evidence bundle
agentcarousel export <RUN-ID>
```

## Live Eval with LLM-as-a-judge

```bash
export GEMINI_API_KEY=gemini_key
export OPENROUTER_API_KEY=or_key
export ANTHROPIC_API_KEY=claude_api_key
export OPENAI_API_KEY=openai_key
agentcarousel eval --execution-mode live --judge \
  --model gemini-2.5-flash \
  --judge --judge-model claude-haiku-4-5-20251001 \
  --evaluator all \
  --runs 1 \
```

## Configuration (`agentcarousel.toml`)

Copy `agentcarousel.example.toml` to `agentcarousel.toml` and customize as needed.

## Bundle workflows

```bash
# Create a distributable bundle archive
agentcarousel bundle pack fixtures/bundles/my-bundle --out my-bundle.tar.gz

# Verify bundle integrity and structure
agentcarousel bundle verify my-bundle.tar.gz

# Pull bundle manifest + artifacts from the registry
agentcarousel bundle pull cmmc-assessor-1.0.0 --url "https://api.agentcarousel.com"
```

## Publish to registry

```bash
# Publish bundle + evidence in one flow
agentcarousel publish fixtures/bundles/cmmc-assessor \
  --url "https://api.agentcarousel.com"

# Publish multiple matching local runs (newest first)
agentcarousel publish fixtures/bundles/cmmc-assessor \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

## Trust checks

```bash
# Registry trust-state check
agentcarousel trust-check cmmc-assessor@1.0.0 \
  --url "https://api.agentcarousel.com"

# Optional offline attestation verification
agentcarousel trust-check cmmc-assessor@1.0.0 \
  --url "https://api.agentcarousel.com" \
  --attestation ./attestation-cmmc-assessor-1.0.0.json \
  --minisign-pubkey ./your-minisign.pub
```

## Contributions

- Start here: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, open an issue before implementation.
