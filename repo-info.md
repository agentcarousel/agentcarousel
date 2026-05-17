# AgentCarousel — Repo Info

**Read this file before doing any work in this repository.**

---

## What This Is

AgentCarousel is a Rust CLI tool for behavioral testing of AI agents. It runs declarative YAML fixtures against agents (live or mocked), scores outputs with an LLM-as-a-judge, and exports signed evidence bundles for auditors and compliance programs. Binary names: `agentcarousel` and `agc` (alias).

---

## Ecosystem Repos

| Repo | Purpose |
|------|---------|
| `agentcarousel/agentcarousel` | **This repo** — CLI tool, library, fixtures, VS Code extension |
| `agentcarousel/homebrew-agentcarousel` | Homebrew tap for macOS installation (`brew install agentcarousel`) |
| `agentcarousel-registry` | Registry backend API — stores bundles, trust state, attestations; exposes `GET /v1/bundles`, `POST /v1/bundles`, trust-check endpoints |
| `agentcarousel-www` | Website and agent index — dynamic listing of published bundles via registry API |

---

## Codebase Layout

```
crates/agentcarousel/src/
├── cli/               CLI routing + all subcommands
│   ├── mod.rs         Cli, Command, run() — routing only
│   ├── init.rs        `agc init` — fixture scaffolding
│   ├── exit_codes.rs  ExitCode enum (Ok=0, Failed=1, ValidationFailed=2, ConfigError=3, RuntimeError=4)
│   ├── config.rs      TOML config loading, ResolvedConfig and *Settings sub-structs
│   ├── eval.rs        `agc eval`
│   ├── validate.rs    `agc validate` + SARIF output
│   ├── test.rs        `agc test`
│   ├── bundle.rs      `agc bundle pack/verify/pull`
│   ├── publish.rs     `agc publish`
│   ├── trust_check.rs `agc trust-check`
│   ├── report.rs      `agc report list/show/diff`
│   ├── stats.rs       `agc stats`
│   ├── doctor.rs      `agc doctor`
│   ├── lint.rs        `agc lint`
│   ├── export.rs      `agc export`
│   └── update.rs      `agc update`
│
├── core/              Shared domain types
│   ├── models.rs      All domain types: Run, Case, FixtureFile, Metrics, CoreError, new_run_id()
│   ├── judge_provider.rs  JudgeProvider enum + key resolution helpers
│   ├── retry.rs       RetryPolicy, retry_policy(), compute_backoff_ms, is_retryable_status
│   └── hex_util.rs    hex_lower() for SHA-256 digest formatting
│
├── providers/         Unified LLM HTTP layer (single source of truth)
│   └── mod.rs         call_provider_blocking(), LlmRequest, ProviderResponse — Gemini/OpenAI/Anthropic/OpenRouter
│
├── evaluators/        Scoring logic
│   ├── judge.rs       JudgeEvaluator — LLM-as-a-judge rubric scoring
│   ├── rules.rs       RulesEvaluator — assertion-based scoring
│   ├── golden.rs      GoldenEvaluator — diff against golden files
│   ├── process.rs     ProcessEvaluator — external grader process
│   ├── assertions.rs  contains/regex/json_path/etc. checks
│   └── trait_def.rs   Evaluator trait, EvaluatorError, EvaluatorKind
│
├── runner/            Async execution engine
│   ├── mod.rs         RunnerConfig, EvalConfig, GenerationMode, run_fixtures(), run_eval()
│   ├── orchestration.rs  run_sequential/parallel, run_eval_cases, flatten_cases, BoundedCache
│   ├── aggregation.rs    aggregate_case_results, build_summary, consistency_score, percentile
│   ├── executor.rs    run_case(), run_case_unscored() — per-case execution
│   ├── generator.rs   GeneratorProvider, generate_case_output(), call_custom_endpoint()
│   ├── tracer.rs      SecretScrubber, Tracer
│   └── sandbox.rs     Sandbox, SandboxGuard, secret scrubbing
│
├── fixtures/          Fixture loading and mock engine
│   ├── loader.rs      load_fixture(), load_fixture_value()
│   ├── mock.rs        MockEngine, MockStub — offline stub matching
│   └── schema.rs      JSON Schema validation
│
└── reporters/         Output formatting and persistence
    ├── terminal.rs    print_terminal(), print_terminal_summary()
    ├── history.rs     SQLite persistence: persist_run, list_runs, fetch_run
    ├── diff.rs        diff_runs(), print_diff()
    └── junit.rs       print_junit() — JUnit XML
```

**Fixture files** live in `fixtures/<skill>/`:
- `cases.yaml` — test cases (schema: `schemas/skill-definition.schema.json`)
- `prompt.md` — system prompt for the skill
- `bundle.manifest.json` — bundle metadata (id, version, org, domain)
- `golden/` — golden output files for GoldenEvaluator

**Mocks** live in `mocks/` — JSON files matched by the MockEngine for offline runs.

**Templates** live in `templates/` — `fixture-skeleton.yaml`, `bundle-manifest-skeleton.json`.

**VS Code extension** lives in `vscode-extension/` (TypeScript, separate package.json).

---

## Key Types

| Type | Where | Role |
|------|-------|------|
| `FixtureFile` | `core/models.rs` | One YAML fixture: metadata + cases |
| `Case` | `core/models.rs` | One test case: input, expected, evaluator config |
| `Run` | `core/models.rs` | Completed run: all CaseResults + RunSummary |
| `CaseResult` | `core/models.rs` | One case outcome: status, trace, metrics, eval_scores |
| `RunnerConfig` | `runner/mod.rs` | Execution tunables: concurrency, timeout, mock_dir, model, etc. |
| `EvalConfig` | `runner/mod.rs` | Extends RunnerConfig with judge/evaluator/threshold/runs |
| `JudgeProvider` | `core/judge_provider.rs` | Gemini / OpenAi / Anthropic / OpenRouter |
| `LlmRequest` | `providers/mod.rs` | Unified request to any LLM provider |
| `ResolvedConfig` | `cli/config.rs` | Merged TOML + CLI config with *Settings sub-structs |

---

## CLI Commands

| Command | What it does |
|---------|-------------|
| `agc validate [paths]` | Schema-validate fixtures; SARIF output with `--format sarif` |
| `agc test [paths]` | Run cases in mock mode (no API keys); assertion-based scoring |
| `agc eval [paths]` | Run cases with live or mock generation; LLM judge optional |
| `agc lint [paths]` | Fixture quality checks beyond schema (smoke coverage, rubric weights) |
| `agc init --skill NAME` | Scaffold `fixtures/<name>/` with cases.yaml, prompt.md, manifest, golden/ |
| `agc report list/show/diff` | Inspect persisted run history (SQLite) |
| `agc stats` | Pass-rate trends, flakiness, latency from history |
| `agc export` | Export run(s) as signed evidence tarballs |
| `agc bundle pack/verify/pull` | Bundle lifecycle |
| `agc publish` | Publish bundle + evidence to registry |
| `agc trust-check` | Query registry trust state; verify minisign attestation |
| `agc doctor` | Environment health check (API keys, config, DB, fixtures) |
| `agc completions` | Print shell completion script |
| `agc update` | Self-update from GitHub releases |

---

## Evaluators

| ID | Evaluator | How it scores |
|----|-----------|---------------|
| `rules` | `RulesEvaluator` | Assertion-based: contains, regex, json_path, tool_called, etc. |
| `golden` | `GoldenEvaluator` | Diff against golden output file; pass if similarity ≥ threshold |
| `process` | `ProcessEvaluator` | Runs external command; exit 0 = pass |
| `judge` | `JudgeEvaluator` | LLM scores each rubric item 0.0–1.0; weighted average = effectiveness_score |

`--evaluator all` respects each case's declared evaluator. `--evaluator judge` routes everything through the LLM judge.

---

## YAML Fixture Schema

```yaml
schema_version: 1
skill_or_agent: my-skill
certification_track: optional
risk_tier: optional
data_handling: optional
defaults:
  timeout_secs: 30
  tags: [smoke]
  evaluator: rules
cases:
  - id: happy-path
    tags: [smoke, happy-path]
    input:
      messages:
        - role: user
          content: "..."
    expected:
      assertions:
        - type: contains
          value: "expected phrase"
      rubric:
        - id: correctness
          description: "..."
          weight: 1.0
    evaluator_config:
      evaluator: judge
      judge_prompt: "path/to/prompt.md"   # optional override
      effectiveness_threshold: 0.8        # optional per-case override
```

---

## Development Workflow

```bash
# Build
cargo build --release

# Test
cargo test --all

# Lint (must be clean before push)
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings

# Run against a fixture (offline)
agc test fixtures/regex-builder/cases.yaml

# Run with live LLM
export GEMINI_API_KEY=...
agc eval fixtures/regex-builder/ --execution-mode live --evaluator all --judge \
  --model gemini-2.5-flash --judge-model claude-haiku-4-5-20251001
```

**Pre-push gates** (mandatory): `cargo fmt --all` → `cargo clippy -- -D warnings` → `cargo test --all` → `cargo build --release`. All four must pass.

---

## Issue Tracking

Uses **br** (beads) for issue operations and **bv** for triage intelligence.

```bash
bv --robot-next          # Top pick for immediate work
bv --robot-triage        # Full triage with scores and health
br ready                 # Unblocked open issues
br show <id>             # Issue detail
br update <id> --claim   # Claim before starting
br close <id>            # Close after user review + commit
```

**Never use `bd`.** Always `bv` to orient, then `br` to act.

---

## Per-Issue Workflow

1. `br update <id> --claim`
2. Implement
3. `cargo test --all` — must pass before staging
4. `git add <files>` — stage, then **stop and wait for user review**
5. Do NOT commit — user commits after review
6. `br close <id>` — after user has committed

---

## Config File

Copy `agentcarousel.example.toml` → `agentcarousel.toml`. Key sections: `[runner]` (concurrency, timeout), `[eval]` (effectiveness_threshold, default_evaluator), `[output]` (color, format), `[judge]`, `[generator]`.

---

## Bundle & Registry Flow

```
agc bundle pack fixtures/<skill>   →  fixtures/<skill>/bundle.manifest.json updated
agc export <run-id>                →  signed .tar.gz + attestation
agc publish fixtures/<skill>       →  posts bundle + run evidence to registry API
agc trust-check <skill>@<version>  →  queries registry; optionally verifies minisign sig
```

Registry URL: set `AGENTCAROUSEL_REGISTRY_URL` or `--url` flag.
