# AgentCarousel

**Unit tests for AI agents.** Determine trust before you deploy - run behavioral tests in CI, score with an LLM judge, and export signed evidence your auditors accept.

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
[![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/agentcarousel/agentcarousel)](https://github.com/agentcarousel/agentcarousel/releases)

<img width="692" height="414" alt="demo" src="https://github.com/user-attachments/assets/c55df92c-fa4a-44b6-a381-23fe0329a5c4" />

AgentCarousel delivers a repeatable, automated way to assess AI agent efficacy and behavior — establishing the trust required before deployment. Tests run deterministically in CI, semantic scoring comes from an LLM-as-a-judge, and results can be certified by a domain expert with a signed attestation.

## Why agentcarousel

- **Behavioral certainty before deployment** — Declarative YAML fixtures pin what your agent should and shouldn't say. Same inputs, same outputs, every time — without touching a live API.
- **Evidence that stands up to scrutiny** — Every run exports a signed bundle (`.tar.gz` + minisign attestation) a domain expert can certify — ready for auditors, procurement teams, and government regulators.
- **Semantic scoring, not just pattern matching** — An LLM-as-a-judge evaluates outputs with contextual understanding, catching regressions.
- **Built for regulated environments** — Risk tier, data handling, and certification track are first-class fixtures. Integrates into CI and produces governance artifacts for your compliance program.

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
# Scaffold a skill fixture
agentcarousel init my-skill

# Run (mock mode by default, no API keys needed)
agentcarousel test fixtures/my-skill/cases.yaml

# Validate fixture schema and rules
agentcarousel validate fixtures/regex-builder/cases.yaml

# Evaluate fixtures
agentcarousel eval fixtures/regex-builder/cases.yaml

# Export the last evaluation as an evidence tarball
agentcarousel export -l
```

See [`fixtures/regex-builder/`](fixtures/regex-builder/) for the full fixture with all cases, golden outputs, and bundle manifest.

## Live Eval with LLM-as-a-Judge

```bash
# Generator LLM key (the model being tested)
export GEMINI_API_KEY=your_key        # or OPENAI_API_KEY / OPENROUTER_API_KEY

# Judge LLM key (the model being judge)
export ANTHROPIC_API_KEY=your_key     # or bring your own provider

# Run skill fixtures for regex-builder against live APIs with LLM judge
agentcarousel eval fixtures/regex-builder/ \
  --execution-mode live \
  --evaluator all --judge \
  --model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001 \
  --runs 1
```

**Execution modes** — `--execution-mode live` hits real LLM APIs. Omit it (or pass `mock`) for deterministic offline runs.

**Evaluators** — `--evaluator all` honors each case's declared evaluator. `--evaluator judge` routes every case through the LLM judge regardless. `--evaluator mock` skips LLM calls entirely.

**Filters** — `--filter` on `skill/case-id`; `--filter-tags` accepts comma-separated tags (e.g. `database, safety`)

## Reports

```bash
# List recent runs
agentcarousel report list

# Inspect a run
agentcarousel report show <RUN-ID>

# Export as a signed evidence bundle
agentcarousel export <RUN-ID>
```

## Configuration

Copy [`agentcarousel.example.toml`](agentcarousel.example.toml) to `agentcarousel.toml`. All configuration options are documented in the example file.

## Bundles

A bundle is a signed, distributable archive of a skill's fixture, cases, and evidence.

```bash
# Pack a bundle
agentcarousel bundle pack fixtures/regex-builder

# Verify bundle integrity
agentcarousel bundle verify fixtures/customer-support
agentcarousel bundle verify my-bundle.tar.gz

# Pull from registry
agentcarousel bundle pull customer-support-1.0.0 --url "https://api.agentcarousel.com"

# Publish to registry
agentcarousel publish fixtures/customer-support --url "https://api.agentcarousel.com"

# Publish multiple runs
agentcarousel publish fixtures/customer-support \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

## Trust Checks

Trust checks query a skill's registry state for use in CI gates and governed workflows — verify a deployed agent is certified and untampered before it runs.

```bash
# Check trust state from registry
agentcarousel trust-check customer-support@1.0.0 \
  --url "https://api.agentcarousel.com"

# Verify with local attestation
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
