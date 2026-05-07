# AgentCarousel - Testing & Evaluation Framework for AI Agents, Skills and LLMs

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
[![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![CI](https://github.com/agentcarousel/agentcarousel/actions/workflows/ci.yml/badge.svg)](https://github.com/agentcarousel/agentcarousel/actions)
[![Docs](https://img.shields.io/badge/docs-agentcarousel.com-1f6feb)](https://docs.agentcarousel.com)

`agentcarousel` is CLI for testing, evaluating, and certifying AI agents and skills -  like `pytest` for AI behavior. You define tests, or **fixtures**, in YAML and eval agents offline, online (LLM-as-a-judge) and with human experts.

<sup>Built in Rust<sup>

> AgentCarousel proves an AI agent behaves acceptably **before deployment** and certifies it with evidence bundles.

**Use it for:** LLM-as-a-judge evals · agent regression testing · prompt testing · compliance artifacts · trusted and verified agent registry

It helps you prove an AI agent behaves acceptably before you deploy it.

## Contents

- [AgentCarousel - Testing \& Evaluation Framework for AI Agents, Skills and LLMs](#agentcarousel---testing--evaluation-framework-for-ai-agents-skills-and-llms)
  - [Contents](#contents)
  - [Who AgentCarousel is for](#who-agentcarousel-is-for)
  - [Install](#install)
      - [Homebrew](#homebrew)
      - [Cargo](#cargo)
    - [Validate fixtures](#validate-fixtures)
    - [Run offline tests](#run-offline-tests)
  - [Common Commands](#common-commands)
  - [Configuration](#configuration)
    - [Live Eval with LLM-as-a-judge](#live-eval-with-llm-as-a-judge)
    - [Bundle workflows](#bundle-workflows)
    - [Publish to registry](#publish-to-registry)
    - [Trust checks](#trust-checks)
  - [Build From Source](#build-from-source)
  - [Contributions](#contributions)

## Who AgentCarousel is for

- **AI/ML engineers** shipping agents to production — catch regressions before users do.
- **Platform & MLOps teams** running LLM evaluation (in CI) — deterministic, offline, fast.
- **Security & compliance leads** preparing **CMMC, SOC 2, or AI governance** audits — exportable evidence bundles with content hashes and optional signed attestations.
- **Skill / agent authors** publishing to the [AgentCarousel Registry](https://agentcarousel.com/registry) — trust-checked bundles end users can verify.

## Install

```bash
curl -fsSL http://install.agentcarousel.com | sh
```

#### Homebrew

```bash
brew tap agentcarousel/agentcarousel && brew install agentcarousel
```

#### Cargo

```bash
cargo install agentcarousel
```

Notes:

- Installer supports Linux and macOS.
- On Windows, download the `.zip` release asset from GitHub.
- The installer offers an `agc` alias for convenience.

### Validate fixtures

A fixture is a YAML file that defines test cases (and optional mocks) for an agent or skill so the CLI can validate, run offline tests, or evaluate behavior.

```bash
agentcarousel validate
```

With no paths, `validate` scans the current directory for fixture files.

### Run offline tests

```bash
agentcarousel test --offline true
```

## Common Commands

```bash
# Help
agentcarousel --help
agentcarousel <command> --help

# Validate fixture files (schema + rules)
agentcarousel validate fixtures/skills/my-skill.yaml

# Run fixtures with mock generation
agentcarousel test

# Evaluate (mock or live)
agentcarousel eval

# List and inspect stored runs
agentcarousel report list
agentcarousel report show <RUN_ID>
agentcarousel report diff <RUN_ID_A> <RUN_ID_B>

# Scaffold a new fixture
agentcarousel init --skill my-new-skill

# Export evidence for one run (or --last N runs)
agentcarousel export <RUN_ID>
agentcarousel export --last 1 --out-dir ./evidence
```

## Configuration

Config file lookup order:

1. `--config <path>` (explicit)
2. `./agentcarousel.toml` (project)
3. `~/.config/agentcarousel/config.toml` (user)

History database defaults:

- macOS: `~/Library/Application Support/agentcarousel/history.db`
- Linux: `~/.local/share/agentcarousel/history.db`

Override history path with:

```bash
export AGENTCAROUSEL_HISTORY_DB=/path/to/history.db
```

### Live Eval with LLM-as-a-judge

Supported providers for live generation and judging include Gemini, OpenAI, Anthropic, and OpenRouter.

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
  --verbose
```

More recipes and troubleshooting: see [`docs/fixture-development-process.md`](docs/fixture-development-process.md)

### Bundle workflows

A versioned package of fixtures, described by bundle.manifest.json with content hashes, that can be packed, verified, and published.

```bash
# Create a distributable bundle archive
agentcarousel bundle pack fixtures/bundles/my-bundle --out my-bundle.tar.gz

# Verify bundle integrity and structure
agentcarousel bundle verify my-bundle.tar.gz

# Pull bundle manifest + artifacts from the registry
agentcarousel bundle pull cmmc-assessor-1.0.0 --url "https://api.agentcarousel.com"
```

### Publish to registry

```bash
# Publish bundle + evidence in one flow
agentcarousel publish fixtures/bundles/cmmc-assessor \
  --url "https://api.agentcarousel.com"

# Publish multiple matching local runs (newest first)
agentcarousel publish fixtures/bundles/cmmc-assessor \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

### Trust checks

The [registry](https://agentcarousel.com/registry) has the published assurance state for a bundle (queryable via `trust-check`), optionally backed by a signed attestation (e.g. `minisign`) that you verify.

> **Roadmap:** Public registry submissions are coming. For now, [open an issue](https://github.com/agentcarousel/agentcarousel/issues/new)

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

## Build From Source

Prerequisites:

- Rust 1.95+

```bash
git clone https://github.com/agentcarousel/agentcarousel.git
cd agentcarousel

# Build package and binaries
cargo build -p agentcarousel

# Run from source (explicit binary)
cargo run -p agentcarousel --bin agentcarousel -- --help
```

Binaries provided by this package:

- `agentcarousel`
- `agc`

## Contributions

- Start here: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, we prefer opening an issue before implementation.
