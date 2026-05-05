# agentcarousel

**Testing for AI agents** <sup>Built with Rust</sup>

`agentcarousel` is a command-line tool for testing AI agents and skills the way you’d use unit tests for code. You define fixtures (YAML) with inputs, expected outputs, and rules; the CLI validates those files, runs repeatable tests (including offline runs with mocks so you don’t need live models), and can record runs, diff them, and export evidence for CI or audit-style workflows. In short: it helps you prove an agent behaves acceptably before you ship it or publish claims about it.

## Why people use this

- Verify agent quality quickly with schema + rule checks.
- Run deterministic offline tests in CI.
- Store run history and compare results over time.
- Export evidence bundles for audits and share benchmarks.
- Publish bundles/runs to a registry and verify trust state.

## Start Here

If you just want value fast, do these three commands.

### 1) Install

```bash
curl -fsSL http://install.agentcarousel.com | sh
```

Notes:

- Installer supports Linux and macOS.
- On Windows, download the `.zip` release asset from GitHub.
- The installer offers an `agc` alias for convenience.

### 2) Validate fixtures

A fixture is a YAML file that defines test cases (and optional mocks) for an agent or skill so the CLI can validate, run offline tests, or evaluate behavior.

```bash
agentcarousel validate
```

With no paths, `validate` scans the current directory for fixture files.

### 3) Run offline tests

```bash
agentcarousel test --offline true
```

This is the safest default for CI and public fixture repos.

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

# Export evidence for one run (or newest N runs)
agentcarousel export <RUN_ID>
agentcarousel export --last 5 --out-dir ./evidence
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

### Live evaluation with judge models

Supported providers for live generation and judging include Gemini, OpenAI, Anthropic, and OpenRouter.

```bash
export GEMINI_API_KEY=your_key_here
agentcarousel eval --execution-mode live \
  --model gemini-2.5-flash \
  --judge --judge-model gemini-2.5-flash
```

More recipes and troubleshooting: see [`docs/fixture-development-process.md`](docs/fixture-development-process.md) and the [Configuration](#configuration) section above for live eval env vars.

### Bundle workflows

A versioned package of fixtures, described by bundle.manifest.json with content hashes, that can be packed, verified, and published.

```bash
# Create a distributable bundle archive
agentcarousel bundle pack fixtures/bundles/my-bundle --out my-bundle.tar.gz

# Verify bundle integrity and structure
agentcarousel bundle verify my-bundle.tar.gz

# Pull bundle manifest + artifacts from a registry (API details at agentcarousel.com when published)
agentcarousel bundle pull cmmc-assessor-1.0.0 --url "$REGISTRY_API_BASE_URL" -o ./pulled/cmmc-assessor
```

### Publish to registry

```bash
# Publish bundle + evidence in one flow
agentcarousel publish fixtures/bundles/terraform-sentinel-scaffold \
  --url "https://api.agentcarousel.com"

# Publish multiple matching local runs (newest first)
agentcarousel publish fixtures/bundles/terraform-sentinel-scaffold \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

### Trust checks

The [registry](https://agentcarousel.com/agents) has the published assurance state for a bundle (queryable via `trust-check`), optionally backed by a signed attestation (e.g. `minisign`) that you verify.

```bash
# Registry trust-state check
agentcarousel trust-check terraform-sentinel-scaffold@1.0.0 \
  --url "https://api.agentcarousel.com"

# Optional offline attestation verification
agentcarousel trust-check terraform-sentinel-scaffold@1.0.0 \
  --url "https://api.agentcarousel.com" \
  --attestation ./attestation-terraform-sentinel-scaffold-1.0.0.json \
  --minisign-pubkey ./agentcarousel-minisign.pub
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

## OSS and Contributions

- Start here: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, we prefer opening an issue before implementation.

## Releases and crates.io

- Rust crate: [crates.io/crates/agentcarousel](https://crates.io/crates/agentcarousel)
