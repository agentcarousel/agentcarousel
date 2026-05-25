# AgentCarousel

**Unit tests for AI agents.** Run behavioral tests in CI, score outputs with an LLM judge, gate on regressions, rank which model costs and performs the best, generate compliance metrics for auditors, and export auditable signed bundles.

[Install](#install) · [Quickstart](#quickstart) · [LLM-as-a-Judge Eval](#live-eval-with-llm-as-a-judge) · [Multi-Model Comparisons](#multi-model-comparison) · [A/B Tests](#ab-prompt-comparison) · [Compliance Metrics](#compliance-metrics) · [Reports & Export](#reports)

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
[![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/agentcarousel/agentcarousel)](https://github.com/agentcarousel/agentcarousel/releases)

<img width="692" height="414" alt="demo" src="https://github.com/user-attachments/assets/c55df92c-fa4a-44b6-a381-23fe0329a5c4" />

## Why agentcarousel

- **Behavioral certainty before deployment:** Declarative YAML fixtures pin what your agent should and shouldn't say. Same inputs, same outputs, every time, without touching a live API.
- **Evidence that stands up to scrutiny:** Every run exports a signed bundle (`.tar.gz` + minisign attestation) that a domain expert can certify, ready for auditors, procurement teams, and government regulators.
- **Semantic scoring, not just pattern matching:** An LLM-as-a-judge evaluates outputs with contextual understanding, catching regressions that keyword checks miss.
- **Built for regulated environments:** Risk tier, data handling, and certification track are first-class fixtures. Integrates into CI and produces governance artifacts for your compliance program.

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

`agc generate` scaffolds validated YAML fixture cases using your configured generator LLM. No manual writing required.

```bash
# From a skill name and description
agc generate --skill customer-support \
             --description "handles refund and cancellation requests" \
             --count 8

# From an existing system prompt file
agc generate --from-prompt fixtures/customer-support/prompt.md --count 10

# Extend an existing fixture (deduplicates against existing case IDs)
agc generate --extend fixtures/customer-support/ --count 5
```

**Typical fixture workflow:**

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
  --model gemini-2.5-flash-lite \
  --judge-model claude-haiku-4-5-20251001 \
  --runs 1
```

**Execution modes:** `--execution-mode live` hits real LLM APIs. Omit it (or pass `mock`) for deterministic offline runs.

**Evaluators:** `--evaluator all` honors each case's declared evaluator. `--evaluator judge` routes every case through the LLM judge regardless. Omit `--evaluator` (or pass `rules`) for assertion-based scoring with no LLM judge calls.

**Filters:** `--filter` matches on `skill/case-id`; `--filter-tags` accepts comma-separated tags (e.g. `database, safety`).

**Token and cost tracking:** After each run, `agc eval` prints token and USD cost totals (generator and judge separately) when live API data is available. The same values are saved to `run.json` and `report.md` for audit trails.

## Multi-Model Comparison

`agc carousel` runs the same fixture suite against multiple models in parallel and prints a ranked comparison table showing pass rate, effectiveness score, latency p50, token usage, and cost per model. Every model's run is saved to history so `agc compare` and the dashboard compare view work immediately.

```bash
# Rank three models head-to-head
agc carousel \
  --models gpt-4o,gemini-2.5-flash,claude-sonnet-4-6 \
  fixtures/my-skill/

# With judge scoring
agc carousel \
  --models gpt-4o,gemini-2.5-flash \
  fixtures/ \
  --evaluator all --judge \
  --judge-model claude-haiku-4-5-20251001

# OpenRouter models (free and open-weight)
agc carousel \
  --models openrouter/deepseek/deepseek-chat:free,gpt-4o \
  fixtures/my-skill/
```

**Recommended workflow for the most complete ranking:**

```bash
# 1. Record and promote golden outputs for all your fixtures
agc eval fixtures/ --execution-mode live --judge
agc promote <run-id>

# 2. Rules-based baseline across models
agc carousel --models m1,m2,m3 fixtures/

# 3. Judge scoring for deeper signal
agc carousel --models m1,m2,m3 fixtures/ -e judge --judge

# 4. Drill into the top two
agc compare <run-a> --baseline <run-b>
```

## A/B Prompt Comparison

`agc ab` runs the same fixture against two prompts concurrently and produces a head-to-head comparison. Pass rate, effectiveness score, per-case winners, and cost per variant.

```bash
# Compare two prompt variants (mock, no API key needed)
agc ab \
  --a prompts/old.md \
  --b prompts/new.md \
  fixtures/my-skill/

# Live generation
agc ab \
  --a prompts/old.md --b prompts/new.md \
  fixtures/ \
  --execution-mode live \
  --model gemini-2.5-flash

# With judge scoring
agc ab \
  --a prompts/old.md --b prompts/new.md \
  fixtures/ \
  --judge \
  --judge-model claude-haiku-4-5-20251001
```

## CI Regression Gate

`agc compare` exits `1` when effectiveness drops below a baseline. With ≥5 scored cases, a [Mann-Whitney U test](https://en.wikipedia.org/wiki/Mann%E2%80%93Whitney_U_test) also requires p < `--significance` so one noisy run does not fail CI.

```bash
# Compare latest run to the previous run for the same skill (auto-baseline)
agc compare -l --threshold 0.05

# Or pin a named local baseline
agc compare tag <run-id> --name prod-baseline
agc compare -l --baseline prod-baseline --threshold 0.05
```

**GitHub Actions (Regression Gate):**

```yaml
jobs:
  eval:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # Persist the history DB across runs so agc compare has a previous run to diff against
      - uses: actions/cache@v4
        with:
          path: .agentcarousel.db
          key: agentcarousel-${{ github.ref }}
          restore-keys: agentcarousel-
      - run: curl -fsSL https://install.agentcarousel.com | sh
      # Run 3 times per case to reduce variance from model non-determinism
      - run: agc eval fixtures/ --execution-mode live --evaluator all --judge --runs 3
        env:
          GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
      # Exit 1 if effectiveness drops >5% vs the previous run for the same skill
      - run: agc compare -l --threshold 0.05
```

Exit codes: `0` pass, `1` regression, `4` runtime error, `5` run/baseline not found.

## Dashboard

`agc dashboard` serves a local web UI from the binary. Open `http://localhost:7421` after starting it. Available in the full binary or with feature-flag `dashboard`.

```bash
agc dashboard                        # http://localhost:7421
agc dashboard --port 8080            # custom port
agc dashboard --db path/to/history.db
```

## Compliance Metrics

`agc metrics` generates a compliance-ready report with four cross-domain measurements. The output is designed to be read by auditors, procurement reviewers, and compliance teams — not just engineers.

```bash
# Metrics from run history + auto-discovered fixtures
agc metrics --skill customer-support

# Point directly at fixture files
agc metrics --fixture fixtures/my-skill/

# Export machine-readable JSON for an evidence bundle
agc metrics --skill customer-support --json > metrics.json

# Widen the history window
agc metrics --skill customer-support --limit 50
```

**What each metric measures:**

| Metric | What it tells you | Source |
|--------|------------------|--------|
| Prompt Injection Resistance | How reliably the agent blocks adversarial injection attempts (0–100) | Run history |
| Behavioral Stability | Whether effectiveness scores are drifting over time — stable, improving, or degrading | Run history |
| Test Coverage Completeness | What fraction of the risk taxonomy (happy path, edge cases, adversarial, error handling, etc.) the fixture suite covers | Fixture files |
| Score Accuracy (Calibration) | Whether the automated judge scores actually predict pass/fail outcomes | Judged run history |

**How `--skill` and `--fixture` interact:**

- `--skill my-skill` automatically loads fixture files from `fixtures/my-skill/` (errors if the directory doesn't exist)
- `--fixture fixtures/my-skill/` reads the skill name from the fixture files and filters run history automatically
- Providing both flags validates that they agree — mismatched skill names produce a clear error

## Reports

```bash
# List recent runs
agc report list

# Inspect a run (add -v / --verbose for full case details)
agc report show <RUN-ID>
agc report show <RUN-ID> --verbose

# Export as a signed evidence bundle (always includes full detail)
agc export <RUN-ID>
agc export -l   # latest run
```

Every exported tarball includes:
- `run.json` — full run record with all case results
- `metrics.json` — compliance metrics scoped to the run's skill
- `report.md` — human-readable report with a Compliance Metrics table auditors can read directly
- `MANIFEST.json` — SHA-256 fingerprints of every file in the archive
- `environment_fingerprint.json` and `fixture_bundle.lock` for reproducibility

## Exit Codes

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

# Pull from registry (set AGENTCAROUSEL_API_TOKEN first)
agc bundle pull customer-support-1.0.0 --url "https://api.agentcarousel.com"

# Publish to registry
agc publish fixtures/customer-support --url "https://api.agentcarousel.com"

# Publish multiple runs
agc publish fixtures/customer-support \
  --url "https://api.agentcarousel.com" \
  --all-runs --limit 5
```

## Trust Checks

Trust checks query an agent's registry state and confirm a deployed agent is certified and untampered. Use them in CI gates and governed workflows before the agent runs.

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

- Contributing guide: [`CONTRIBUTING.md`](CONTRIBUTING.md)
- Security policy: [`SECURITY.md`](SECURITY.md)
- Changelog: [`CHANGELOG.md`](CHANGELOG.md)

For fixture contributions, open an issue before implementation.
