# AgentCarousel

**Unit tests for AI agents.** The only AI testing tool that produces evidence your auditors accept — run behavioral tests in CI, score with an LLM judge, gate on regressions, and export signed bundles ready for procurement teams and government regulators.

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
[![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
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
# Linux / macOS — slim binary (no dashboard)
curl -fsSL https://install.agentcarousel.com | sh

# Linux / macOS — full binary (includes web dashboard UI)
curl -fsSL https://install.agentcarousel.com | sh -s -- --feature dashboard

# Homebrew (macOS)
brew tap agentcarousel/agentcarousel && brew install agentcarousel

# Cargo (Rust)
cargo install agentcarousel
```

Two binary variants are available on every release:

| Variant | Asset suffix | Includes |
|---------|-------------|----------|
| Slim (default) | *(none)* | All commands except `dashboard` |
| Full | `-full` | Everything, including `agc dashboard` |

Upgrade an existing installation to the full variant at any time:

```bash
agc update --feature dashboard
```

## Quickstart

```bash
# 1. Scaffold a skill fixture
agc init --skill my-skill

# 2. Generate cases with an LLM (no hand-writing required)
agc generate --extend fixtures/my-skill/ --count 8

# 3. Run offline (mock mode, no API keys)
agc test fixtures/my-skill/

# 4. Evaluate with live generation and an LLM judge
agc eval fixtures/my-skill/ --execution-mode live --judge --model gemini-2.5-flash

# 5. Export a signed evidence bundle
agc export -l
```

See [`fixtures/regex-builder/`](fixtures/regex-builder/) for a complete fixture with all cases, golden outputs, and bundle manifest.

## Generate Fixtures

`agc generate` scaffolds validated YAML fixture cases using your configured generator LLM — no hand-writing required.

```bash
# From a skill name and description
agc generate --skill customer-support \
             --description "handles refund and cancellation requests" \
             --count 8

# From an existing system prompt file
agc generate --from-prompt fixtures/customer-support/prompt.md --count 10

# Extend an existing fixture (deduplicates against existing case IDs)
agc generate --extend fixtures/customer-support/ --count 5

# Preview without writing
agc generate --skill my-skill --description "..." --dry-run

# Machine-readable output (for agent workflows)
agc generate --skill my-skill --description "..." --dry-run --json
```

Generated cases are validated against the fixture schema before being written. If the LLM output fails validation, the command retries once with the errors appended to the prompt. The meta-prompt lives at `templates/generate-prompt.md` — teams can customize it to specify what "good coverage" means for their domain.

**Typical workflow:**

```bash
agc init --skill customer-support       # scaffold directory structure
# edit fixtures/customer-support/prompt.md
agc generate --extend fixtures/customer-support/ --count 8
agc validate fixtures/customer-support/
```

## Live Eval with LLM-as-a-Judge

```bash
# Generator LLM key (the model being tested)
export GEMINI_API_KEY=your_key        # or OPENAI_API_KEY / OPENROUTER_API_KEY

# Judge LLM key (the model scoring outputs)
export ANTHROPIC_API_KEY=your_key     # or bring your own provider

# Run skill fixtures against live APIs with LLM judge
agc eval fixtures/regex-builder/ \
  --execution-mode live \
  --evaluator all --judge \
  --model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001 \
  --runs 1
```

**Execution modes** — `--execution-mode live` hits real LLM APIs. Omit it (or pass `mock`) for deterministic offline runs.

**Evaluators** — `--evaluator all` honors each case's declared evaluator. `--evaluator judge` routes every case through the LLM judge regardless. `--evaluator mock` skips LLM calls entirely.

**Filters** — `--filter` on `skill/case-id`; `--filter-tags` accepts comma-separated tags (e.g. `database, safety`)

## CI Regression Gate

`agc compare` compares two eval runs and exits 1 when effectiveness regresses beyond a threshold — drop it into any CI pipeline as a binary pass/fail gate.

```bash
# Compare the latest run to an explicit baseline
agc compare -l --baseline <run-id> --threshold 0.05

# Auto-baseline: finds previous run for the same skill
agc compare -l

# Tag a run as a named baseline for CI reference
agc compare tag <run-id> --name prod-baseline

# JSON output for downstream tooling
agc compare -l --baseline <run-id> --json
```

**GitHub Actions example:**

```yaml
- name: Eval
  run: agc eval fixtures/ --judge --runs 3

- name: Regression gate
  run: agc compare -l --baseline ${{ vars.BASELINE_RUN_ID }} --threshold 0.05
```

Exit codes: `0` = no regression, `1` = regression exceeds threshold, `2` = error.

## Dashboard

`agc dashboard` serves a local web UI from the binary — zero config. Open `http://localhost:7421` after starting it. Available in the full binary variant.

```bash
agc dashboard                        # http://localhost:7421
agc dashboard --port 8080            # custom port
agc dashboard --db path/to/history.db
```

**Pages:**

- **`/`** — Run history index with headline metrics (total runs, pass rate, mean effectiveness) and trend sparklines
- **`/runs/:id`** — Run detail: per-case effectiveness, inline expansion with trace steps, rubric scores, and judge rationale
- **`/compare?a=:id&b=:id`** — Side-by-side run comparison with delta badges and regression highlighting; deep-linkable URL
- **`/review?run=:id`** — Judge review screen: annotate each LLM judge call as ✓ correct / ✗ wrong / ~ borderline; annotations persist to `reviews.jsonl` and are included in `agc export` evidence bundles

Install the full variant to get dashboard access:

```bash
curl -fsSL https://install.agentcarousel.com | sh -s -- --feature dashboard
# or upgrade in-place:
agc update --feature dashboard
```

## Reports

```bash
# List recent runs
agc report list

# Inspect a run
agc report show <RUN-ID>

# Export as a signed evidence bundle
agc export <RUN-ID>
agc export -l   # latest run
```

## Agent Integration

Every command emits structured JSON when `--json` is passed or stdout is not a TTY (piped to a file, another process, or an AI coding agent):

```bash
# Parse eval results in a pipeline
agc eval fixtures/ --json | jq '.data.summary.pass_rate'

# Machine-readable validate output
agc validate fixtures/ --json | jq '.data.atf_summary'

# Generate fixtures from an agent script
agc generate --extend fixtures/my-skill/ --count 5 --json
```

**Success envelope:**
```json
{ "ok": true, "command": "eval", "data": { ... } }
```

**Error envelope:**
```json
{
  "ok": false,
  "error": {
    "code": "run_not_found",
    "message": "Run 'abc123' not found in history database.",
    "suggestions": ["Run 'agc report list' to see available run IDs."]
  }
}
```

**Exit codes** (consistent across all commands):

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Failure (tests failed, regression detected) |
| 2 | Invalid arguments |
| 3 | Config error |
| 4 | Runtime error (IO, network, DB) |
| 5 | Not found |

## Configuration

Copy [`agentcarousel.example.toml`](agentcarousel.example.toml) to `agentcarousel.toml`. All configuration options are documented in the example file.

## Bundles

A bundle is a signed, distributable archive of a skill's fixture, cases, and evidence.

```bash
# Pack a bundle
agc bundle pack fixtures/regex-builder

# Verify bundle integrity
agc bundle verify fixtures/customer-support
agc bundle verify my-bundle.tar.gz

# Pull from registry
agc bundle pull customer-support-1.0.0 --url "https://api.agentcarousel.com"

# Publish to registry
agc publish fixtures/customer-support --url "https://api.agentcarousel.com"

# Publish multiple runs
agc publish fixtures/customer-support \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

## Trust Checks

Trust checks query a skill's registry state for use in CI gates and governed workflows — verify a deployed agent is certified and untampered before it runs.

```bash
# Check trust state from registry
agc trust-check customer-support@1.0.0 \
  --url "https://api.agentcarousel.com"

# Verify with local attestation
agc trust-check customer-support@1.0.0 \
  --url "https://api.agentcarousel.com" \
  --attestation ./attestation-customer-support-1.0.0.json \
  --minisign-pubkey ./your-minisign.pub
```

## Contributions

- Start here: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, open an issue before implementation.
