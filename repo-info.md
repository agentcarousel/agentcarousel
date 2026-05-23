# AgentCarousel — Repository Reference

## What it is

**AgentCarousel** is a Rust CLI tool (`agc`) for behavioral testing, evaluation, and compliance certification of AI agents and skills. The mental model is "pytest for LLM agents": you write declarative YAML fixture files that pin what your agent should and shouldn't say, then run them offline (mock) or live (real LLM API). An LLM-as-a-judge evaluator scores outputs semantically. Every run produces a signed evidence bundle ready for auditors, procurement teams, and government regulators (FDA SaMD, HIPAA, Joint Commission).

**Current version:** `0.6.4`  
**Language:** Rust (MSRV 1.95)  
**Binary names:** `agentcarousel` and `agc` (alias)  
**Published to:** crates.io as `agentcarousel`

---

## Repository Layout

```
agentcarousel/
├── Cargo.toml                  # workspace root (single member)
├── crates/agentcarousel/       # the entire codebase lives here
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs              # crate root + extern crate self aliases
│       ├── main.rs             # agentcarousel binary entrypoint
│       ├── bin/agc.rs          # agc binary entrypoint
│       ├── cli/                # ~25 subcommands (clap)
│       ├── core/               # shared serializable data types
│       ├── evaluators/         # rules / golden / process / judge / prompt-audit
│       ├── fixtures/           # YAML loading, mock engine, schema validation
│       ├── providers/          # shared HTTP request/response types (Gemini/OAI/Anthropic)
│       ├── reporters/          # terminal output, JSON, SQLite history, diffs, dashboard
│       └── runner/             # async tokio execution orchestration
├── fixtures/                   # bundled fixture suites (regex-builder, ambient-scribe, etc.)
├── mocks/                      # pre-recorded mock responses for offline test runs
├── schemas/                    # JSON schemas for fixture validation
├── docs/                       # fixture format docs, judge prompt guides
├── templates/                  # agc init scaffold templates
└── packaging/                  # install scripts, CI packaging
```

---

## Architecture Pattern

The crate is physically one crate but uses `extern crate self as agentcarousel_*` aliases at the bottom of `lib.rs` to simulate a multi-crate layout:

```rust
extern crate self as agentcarousel_cli;
extern crate self as agentcarousel_core;
extern crate self as agentcarousel_evaluators;
extern crate self as agentcarousel_fixtures;
extern crate self as agentcarousel_reporters;
extern crate self as agentcarousel_runner;
```

Code in each module imports from these aliases (`use agentcarousel_core::Run`) rather than `crate::core::Run`. This makes the code look and feel like separate crates with clean dependency boundaries without the build-time cost of actually splitting them.

---

## Module Responsibilities

### `core/` — Data Types
All serializable types shared across modules. Key structs:

| Type | Purpose |
|------|---------|
| `Run` | Top-level result of a fixture run: cases, summary, provenance, optional `prompt_audit` |
| `RunId` | 10-char uppercase ULID-derived opaque ID |
| `CaseResult` | Outcome for one case: `status`, `trace`, `metrics`, `eval_scores`, `input` |
| `FixtureFile` | Deserialized YAML: `skill_or_agent`, `cases[]`, bundle metadata, risk tier |
| `Case` | One test case: `input.messages[]`, `expected` (assertions/rubric), evaluator config |
| `EvalScores` | `effectiveness_score`, `rubric_scores[]`, `passed`, evaluator name, judge rationale |
| `RunSummary` | Aggregate counts, pass rate, latency, token/cost totals |
| `PromptAudit` | Run-level second-pass audit: `failure_mode`, `confidence`, `findings[]`, `suggested_fixes[]` |
| `PromptAuditFailureMode` | Enum: `Prompt` \| `Model` \| `Fixture` \| `Mixed` |

Also contains: `ModelPricing`, `JudgeProvider` dispatch, `CertificationContext`, retry helpers.

### `cli/` — Commands
Clap v4 subcommand tree. All commands dispatch through `cli::run() -> i32`. Global flags: `--json`, `--quiet`, `-v`, `--no-color`, `--config`.

**Command groups:**

| Group | Commands |
|-------|---------|
| Fixture work | `validate`, `test`, `eval`, `carousel`, `ab`, `watch`, `generate`, `lint`, `init` |
| Results | `report` (list/show), `audit` (run/suggest), `stats`, `compare`, `export` |
| Bundles & registry | `bundle` (pack/verify/pull), `publish`, `promote`, `trust-check` |
| Tooling | `dashboard`\*, `completions`, `update`, `doctor`, `help` |

\* behind `dashboard` feature flag

**`agc audit` subcommands (new in 0.6.4):**
- `agc audit run <run_id>` — runs the LLM prompt-audit pass, stores result in DB
- `agc audit suggest <run_id>` — prints stored `suggested_fixes` (no LLM call)
- `agc audit suggest <run_id> --apply` — appends suggestions as `<!-- audit:suggestions -->` block to `prompt.md`

### `evaluators/` — Scoring

| Evaluator | Trigger | How it works |
|-----------|---------|-------------|
| `RulesEvaluator` | `evaluator: rules` | Runs `output[]` assertions (contains/regex/json_path/golden_diff) |
| `GoldenEvaluator` | `evaluator: golden` | Diffs final output against a golden file; passes if similarity ≥ threshold |
| `ProcessEvaluator` | `evaluator: process` | Shells out to a command; exit 0 = pass |
| `JudgeEvaluator` | `evaluator: judge` | Calls LLM with rubric items, gets 0.0–1.0 scores + rationale per item |
| `run_prompt_audit()` | End of eval run | Run-level second pass: sends all case failures + prompt.md to judge, gets failure mode classification and suggested fixes |

The `Evaluator` trait: `fn evaluate(&self, case: &Case, result: &CaseResult) -> Result<EvalScores, EvaluatorError>`.

### `runner/` — Execution

| Submodule | Role |
|-----------|------|
| `mod.rs` | `run_fixtures()` and `run_eval()` public entry points |
| `orchestration.rs` | Parallel (`tokio::spawn` + `Semaphore`) and sequential (fail-fast) case execution; `BoundedCache` for judge dedup |
| `executor.rs` | Executes one case: generator call → evaluator → `CaseResult` |
| `generator.rs` | `call_llm()`: async HTTP to Gemini/OpenAI/Anthropic/custom endpoints |
| `aggregation.rs` | Builds `RunSummary` from `Vec<CaseResult>` |
| `sandbox.rs` | Secret scrubbing for traces |
| `tracer.rs` | `SecretScrubber` for redacting API keys from logged output |
| `git_revision.rs` | Reads `HEAD` git SHA for run provenance |

`RunnerConfig` and `EvalConfig` are the primary configuration structs passed into the runner.

### `fixtures/` — Loading

- `loader.rs`: `load_fixture(path) -> FixtureFile` — YAML/TOML deserialization with schema validation
- `mock.rs`: `MockEngine` — loads pre-recorded responses from `mocks/` directory for offline runs
- `schema.rs`: JSON schema validation against `schemas/fixture.schema.json`

### `reporters/` — Output

- `terminal.rs`: Colored terminal output using `console::style`. Key public fns: `print_terminal()`, `print_terminal_summary()`, `print_audit()`. Section headers use colored `─────` rules.
- `history.rs`: SQLite persistence via `rusqlite`. Schema: `runs(id TEXT PK, started_at TEXT, run_json TEXT)`. `persist_run()` uses `INSERT OR REPLACE` — re-persisting a run updates it in place (used by `agc audit run` to attach audit results).
- `diff.rs`: `diff_runs()` / `print_diff()` for `agc compare`
- `dashboard/`: axum-based web server (optional feature). Endpoints in `api.rs`, static assets in `assets.rs`.

### `providers/` — HTTP Shapes

Shared serde types for all three LLM providers:
- `GeminiRequest/Response/UsageMetadata`
- `OpenAiRequest/Response` (also used for OpenRouter — same API shape)
- `AnthropicRequest/Response`

Used by both `evaluators/judge.rs` (blocking `reqwest`) and `runner/generator.rs` (async `reqwest`).

---

## Key Data Flow

```
agc eval fixtures/my-skill/ --execution-mode live --judge
        │
        ├─ load_fixture() × N files → Vec<FixtureFile>
        ├─ flatten_cases() → Vec<(Case, defaults)>
        │
        ├─ [tokio parallel] for each case:
        │     ├─ call_llm() → raw LLM response
        │     ├─ evaluator.evaluate(case, result) → EvalScores
        │     └─ CaseResult { status, trace, metrics, eval_scores }
        │
        ├─ aggregation::build_summary() → RunSummary
        │
        ├─ [if judge + failures] run_prompt_audit() → PromptAudit
        │   └─ attached to Run.prompt_audit
        │
        ├─ annotate_run_cost() → stamps USD cost fields
        ├─ persist_run() → SQLite .agentcarousel.db
        └─ print_terminal() → styled terminal output
```

---

## Fixture Format Reference

```yaml
schema_version: 1
skill_or_agent: my-skill           # used for run grouping + history lookup
bundle_id: agentcarousel/my-skill  # registry identifier
bundle_version: 1.0.0
certification_track: candidate     # candidate | certified
risk_tier: low                     # low | medium | high
data_handling: synthetic-only      # synthetic-only | phi | pii | confidential

defaults:
  timeout_secs: 45
  tags: [smoke, nightly]
  evaluator: rules                 # rules | judge | golden | process | all

cases:
  - id: my-skill/case-id
    description: "human readable"
    tags: [smoke, happy-path]
    seed: 42                       # deterministic mock selection
    timeout_secs: 30               # override default

    input:
      messages:
        - role: user
          content: "..."
        - role: assistant          # can include few-shot turns
          content: "..."

    expected:
      tool_sequence:               # optional: expected tool call order
        - tool: search
          order: subsequence

      output:                      # assertion-based checks
        - kind: contains
          value: "expected substring"
        - kind: not_contains
          value: "forbidden phrase"
        - kind: regex
          value: '^\d{4}-\d{2}-\d{2}$'
        - kind: json_path
          field: "$.result.status"
          value: "ok"

      rubric:                      # judge / rules rubric items
        - id: rubric-item-id
          description: "criterion description"
          weight: 0.4              # weights should sum to 1.0 across items
          critical: true           # blocks agc promote if score < 0.95
          auto_check:              # optional assertion run before judge
            kind: regex
            value: 'pattern'

    evaluator_config:
      evaluator: judge             # override default evaluator for this case
      golden_path: golden/output.md
      golden_threshold: 0.80
      effectiveness_threshold: 0.75
      judge_prompt: "Custom judge instructions..."
```

---

## LLM Providers & Auth

| Provider | Env Var(s) | Notes |
|----------|-----------|-------|
| Gemini | `GEMINI_API_KEY` | Default judge/generator |
| OpenAI | `OPENAI_API_KEY` | |
| OpenRouter | `OPENROUTER_API_KEY` | Prefix model with `openrouter/` |
| Anthropic | `ANTHROPIC_API_KEY` | |
| Custom | `--generator-endpoint <url>` | OpenAI-compatible endpoint |

Generator model examples: `gemini-2.5-flash`, `gemini-2.5-flash-lite`, `gpt-4o`, `claude-sonnet-4-6`  
Judge model examples: `claude-haiku-4-5-20251001`, `claude-sonnet-4-6`, `gemini-2.5-flash`

---

## Persistence

- **History DB**: `.agentcarousel.db` (SQLite) in cwd. Stores all runs as serialized `Run` JSON.  
- **Run IDs**: 10-char uppercase (e.g. `ECR99Y6BWT`) — drawn from ULID random portion.  
- **`persist_run()`**: `INSERT OR REPLACE` — re-running with same ID overwrites (used to attach audit results retroactively).  
- **Configured via**: `AGENTCAROUSEL_HISTORY_DB` env var or `agentcarousel.toml`.

---

## Configuration (`agentcarousel.toml`)

Key sections (see `agentcarousel.example.toml` for full reference):

```toml
[generator]
model = "gemini-2.5-flash-lite"
max_tokens = 2048

[judge]
model = "claude-haiku-4-5-20251001"
max_tokens = 1024

[runner]
concurrency = 4
timeout_secs = 60
offline = false

[eval]
default_evaluator = "rules"
effectiveness_threshold = 0.75

[output]
format = "human"    # human | json
color = "auto"      # auto | always | never
```

---

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Failure (tests failed, regression) |
| 2 | Invalid arguments |
| 3 | Config error |
| 4 | Runtime error (IO, network, DB) |
| 5 | Not found |

---

## Build & Test

```bash
# Build slim binary (no dashboard)
cargo build -p agentcarousel --release

# Build with web dashboard
cargo build -p agentcarousel --release --features dashboard

# Run clippy (CI gate — zero warnings)
cargo clippy -p agentcarousel --all-targets --all-features -- -D warnings

# Run tests
cargo test -p agentcarousel

# Run a specific integration test
cargo test -p agentcarousel bundle_registry_flow
```

**Feature flags:**
- `dashboard` — enables axum web UI, adds `agc dashboard` command

---

## Fixtures in the Repo

| Fixture | Skill | Notes |
|---------|-------|-------|
| `fixtures/regex-builder/` | regex-builder | Reference fixture; all evaluator types, bundle manifest |
| `fixtures/ambient-scribe/` | ambient-scribe | Healthcare SOAP notes; FDA SaMD, HIPAA, Joint Commission; risk_tier=high |
| `fixtures/customer-support/` | customer-support | Refund/cancellation flows |
| `fixtures/code-reviewer/` | code-reviewer | Code review quality |
| `fixtures/ci-failure-triager/` | ci-failure-triager | CI log diagnosis |
| `fixtures/sql-query-generator/` | sql-query-generator | NL→SQL |
| + 11 more | various | See `fixtures/` directory |

---

## Prompt Audit (v0.6.4)

A second LLM judge pass that runs at the end of `agc eval` (when `--judge` is on and there are failures). Also callable retroactively via `agc audit run <run_id>`.

**Failure modes diagnosed:**
- `prompt` — prompt is underspecified; fixing the prompt likely fixes failures
- `model` — model capability ceiling; prompt wording won't help
- `fixture` — rubric thresholds/expectations miscalibrated; model output is actually reasonable
- `mixed` — multiple factors

**Output stored on `Run.prompt_audit`:**  
`{ failure_mode, confidence, findings[], suggested_fixes[], overall_rationale, judge_tokens_in/out }`

**Apply suggestions to prompt.md:**  
`agc audit suggest <run_id> --apply` — appends `<!-- audit:suggestions -->` block, no LLM call needed.

---

## Notable Implementation Details

- **Single-crate multi-module**: `extern crate self as agentcarousel_*` gives module-boundary semantics without build overhead.
- **`console` crate** for all terminal styling — `style().dim()`, `.bold()`, `.red()`, `.cyan()` etc. Color auto-disabled in non-TTY mode.
- **Blocking reqwest** for judge calls (run from blocking context within tokio via `spawn_blocking`); **async reqwest** for generator calls.
- **`indicatif`** progress bars for case-level progress during `agc eval`.
- **`minisign`** attestation for evidence bundles (external binary, invoked via shell).
- **`globset`** for fixture path expansion; **`walkdir`** for directory traversal.
- **`notify`** crate for filesystem watching in `agc watch`.
- **Mann-Whitney U test** implemented in ~30 lines of pure math in `compare.rs` — no stats crate dependency.
- **`agc compare`** auto-discovers previous run for same skill via `find_previous_run()` in history.rs.
- **`sandbox.rs`** / `tracer.rs` — `SecretScrubber` redacts API key patterns from trace output before persistence.
