# Changelog

## 0.8.0 — 2026-06-02

**Theme: OSCAL-native compliance attestation.**

`agc` can now produce machine-readable compliance evidence that auditors and GRC tools accept directly. A new `crates/oscal` workspace crate implements the OSCAL data model; a tag-driven scoring engine maps fixture cases to control IDs; and three new CLI commands surface the results.

### Added

- **`crates/oscal`** — new workspace crate with Serde types for the full OSCAL data model: `Catalog`, `Group`, `Control`, `Part`, `ComponentDefinition`, `ControlImplementation`, `ImplementedRequirement`, `AssessmentResults`, `Finding`, `FindingTarget`, `Observation`, and `RelevantEvidence`. Round-trip JSON parse tested against bundled community catalogs.
- **Bundled OSCAL catalogs** — NIST SP 800-171, 800-172, 800-207, NIST AI RMF, EU AI Act, ISO 42001, HIPAA, and FDA SaMD catalog JSON files shipped inside `crates/oscal/catalogs/`; loaded at runtime via `load_catalog()`.
- **`CaseResult.tags`** — tags field added to `CaseResult` (core/models.rs) and populated from `Case.tags` at all eight orchestration construction sites, giving every scored case its control-ID annotations.
- **`agc compliance report`** — Markdown compliance report: per-control pass/fail table with effectiveness scores, case counts, gap advisory, and overall framework attestation status. `--framework` accepts `nist-ai-rmf`, `nist-800-171`, `nist-800-172`, `eu-ai-act`, `iso-42001`, `hipaa`, `fda-samd`, `nist-800-207`, or `all`. `--model` scopes scoring to a specific generator model. `--json` emits a structured envelope for pipeline use.
- **`agc compliance gaps`** — Lists controls that are `NotSatisfied` or `PartialEvidence` with suggested case improvements. Designed for remediation workflows.
- **`agc compliance generate-cases`** — Generates fixture cases pre-tagged with control IDs for a specified framework, accelerating coverage of compliance gaps.
- **OSCAL Assessment Results export** — `serialize_assessment_results()` produces a valid `assessment-results.oscal.json` artifact; included in every evidence tarball alongside `metrics.json`. Findings carry `status`, `reason`, and `relevant-evidence` links.
- **`--framework` on `agc metrics`** — `framework_controls` field on `MetricResult` replaces the old `compliance_hook`; `agc metrics --framework <id>` scopes the control table to a single framework.
- **Satisfaction threshold** — Documented at 0.80 (configurable per-framework); requires a minimum of three cases before a control can receive `Satisfied` status.
- **OSCAL finding status** — Distinguishes `PartialEvidence` (some cases pass, some fail) from outright failure; `reason` field explains the gap.

### Fixed

- Byte-index string slice panic on multi-byte UTF-8 characters in control IDs in the compliance terminal renderer (`agc-inp6`).
- Double mock-engine lookup in `executor::run_case_inner` (`agc-xaqg`).
- Unreachable JSON error branch in `agc compliance report --framework all` (`agc-aa4n`).
- OSCAL assessment results `start`/`end` timestamps now reflect the actual evaluation run window, not report-generation time (`agc-cfy0`).
- OSCAL serializer calls `collapse_scores` before iterating findings, preventing duplicate finding entries per control (`agc-p9pj`).
- `EvalScores.passed` no longer hardcoded to `1.0` in the judge evaluator (`agc-j7t6`).
- Silent `JoinError` in `run_eval_cases` no longer silently drops case results (`agc-smum`).
- `_novelty_score` internal metadata no longer written to `cases.yaml` output (`agc-rsd9`).

---

## 0.7.0 — 2026-05-29

### Added
- `agc pipeline onboard/improve` — end-to-end skill lifecycle automation
- Fixture generation with novelty screening and deduplication
- Ollama and custom model judge endpoint support
- `agc optimize` — automated prompt tuning with failure clustering
- Batch eval via Anthropic and OpenAI Batch APIs
- Discrimination scoring flags low-quality generated cases
- `--distribution`, `--difficulty`, `--domain-context` flags for `agc generate`
- Judge system prompt includes score calibration anchors (0.0 / 0.5 / 1.0 definitions) for consistent inter-run scoring
- `golden_normalize_whitespace` option on `evaluator_config` — collapses whitespace before diffing, useful for structured output
- `agc validate` errors on rubric weights that don't sum to 1.0 and warns on trivially broad `auto_check` values

### Fixed
- Judge endpoint auto-inherited from generator for ollama/custom models
- Default runner timeout raised from 30 s to 120 s for local inference
- `role: model` (Gemini convention) accepted as alias for `assistant`
- `tool:` field accepts `name`/`tool_name` aliases to survive LLM variants
- Multi-batch fixture append no longer corrupts case indentation

### Removed
- 14 bundled example fixture suites trimmed to 3 canonical suites

---

## 0.6.5 - May 2026

### Features

- **`agc metrics`** — Compliance-focused performance report with four cross-domain metrics: Prompt Injection Resistance (0–100 score aggregated across adversarial cases), Behavioral Stability (drift in effectiveness score over run history), Test Coverage Completeness (percentage of a built-in 7-category risk taxonomy covered by the fixture suite), and Score Accuracy (Expected Calibration Error measuring how well automated judge scores predict actual pass/fail outcomes). Output is designed to be readable by auditors and procurement reviewers, not just engineers. `--json` emits a structured envelope for evidence bundles.

  `--skill` and `--fixture` flags are linked: `--skill <name>` auto-discovers `fixtures/<name>/` (errors with exit 5 if the directory is missing); `--fixture <path>` reads the skill name from the fixture's `skill_or_agent` field and scopes run history automatically. Providing both with conflicting skill names produces a structured `skill_fixture_mismatch` error (exit 2).

- **`agc export` metrics embedding** — Every evidence tarball now includes two additional artifacts: `metrics.json` (machine-readable compliance metrics, SHA-256 fingerprinted in `MANIFEST.json`) and a Compliance Metrics table injected into `report.md` between the certification scope and the case list. Metrics are scoped to the exported run's `skill_or_agent` across the most recent 20 runs of that skill's history.
- **Git commit provenance** — Run records now capture `git_sha` at eval time: `GITHUB_SHA` is preferred in CI; falls back to `git rev-parse HEAD` for local runs. The SHA surfaces in `environment_fingerprint.json` inside every evidence tarball, giving auditors a traceable link from the archived results back to the exact code revision that produced them.

### Removals

- **`agc stats` removed** — Replaced by `agc metrics`. The old command showed pass-rate trends and case flakiness; the new command covers the same territory as part of a broader compliance-oriented report that also ships in every evidence bundle.

---

## 0.6.4 - May 2026

### Features

- **`agc audit run <run-id>`** — Second-pass LLM analysis against a saved run. Loads the run from history, calls the configured judge model to diagnose whether failures stem from prompt design, model capability, or fixture miscalibration, and saves the result back to the history DB so `agc report show <id>` and the VS Code extension display it going forward. Accepts a run ID from `agc report list`, a path to `run.json`, or a directory containing one. `--prompt <path>` overrides the auto-discovered `fixtures/<skill>/prompt.md`; `--model` overrides the judge model from config; `--no-save` skips writing the result back. `--json` emits a structured envelope for pipeline use.
- **`agc audit suggest <run-id>`** — Reads `suggested_fixes` from a previously stored audit result with no further LLM call. Without `--apply`, prints the numbered suggestion list and any worked implementations to stdout. With `--apply`, appends the suggestions as a commented `<!-- audit:suggestions -->` block to `prompt.md` so you can review and integrate them like a diff. Exit 0 on success; exit 4 (not found) when the run has no stored audit (run `agc audit run <id>` first).

---

## 0.6.3 - May 2026

### Changes

- **`agc report diff` removed** — Use `agc compare` instead. `compare` covers the same two-run diff with the addition of Mann-Whitney significance testing, registry baselines, and proper CI exit codes. The `agc report` command retains `list` and `show`.
- **Shared provider types** — `GeminiRequest/Response`, `OpenAiRequest/Response`, and `AnthropicRequest/Response` structs were duplicated between `evaluators/judge.rs` and `runner/generator.rs`. Extracted into a new `providers` module as the single canonical definition; both callers import from there. No behavior change.

### Chores

- Removed `#[allow(dead_code)]` from `exit_codes.rs`; all `ExitCode` variants are actively used.
- README rewritten: full command reference, fixture format field tables, architecture overview, GitHub Actions CI example, troubleshooting section.

---

## 0.6.2 - May 2026

### Features

- **`agc promote <run_id>`** — Promotes golden files from a saved run without re-executing anything. Loads the run from local SQLite history, loads fixture YAML from disk for rubric metadata, and applies the quality gate: effectiveness ≥ 0.90 and all critical rubric items ≥ 0.95. Writes golden files for passing cases and prints a styled summary table with effectiveness score, baseline delta (from a `.meta.json` sidecar), and promoted/blocked status. Blocked cases list each failing rubric item with its score and the required threshold. `--registry` exports the run as a signed evidence tarball and submits it to `https://api.agentcarousel.com` (`POST /v1/runs`), recording the run in the agentcarousel_registry PostgreSQL database. Token falls back to `AGENTCAROUSEL_API_TOKEN` or stored credentials from `agc login`. Exit 0 when all eligible cases promoted; exit 1 if any blocked; exit 4 on infrastructure errors.
- **Rubric `critical` field** — Optional `critical: true` annotation on rubric items marks them as non-negotiable for promotion. `agc promote` requires all `critical: true` items to score ≥ 0.95 before writing a golden file. Fixtures without any `critical: true` items fall back to treating items with `weight ≥ 0.45` as critical. Fully backwards-compatible — all existing fixture YAML files deserialise without changes.
- **Token and cost display** — Gen and judge token counts are now rendered in cyan with a combined total; cost renders in bold yellow. The run summary block (`agc eval`) shows gen and judge as separate rows, then a combined total and cost. The footer cost line uses the same colour scheme. Judge tokens are included in the summary even when the generator had no tokens.

### Breaking changes

- **`--update-golden` removed** — The eval flag that blindly overwrote golden files is gone. Use `agc eval --execution-mode live --judge` to produce a scored run, then `agc promote <run_id>` to write the golden file once the quality gate clears.

### Changes

- **Run ID format** — Run IDs are now 10-character uppercase strings (e.g. `3NDEKTSV4R`) drawn from the random portion of a ULID. Replaces the previous 26-character full ULID. IDs fit untruncated in all table columns (`agc carousel`, `agc ab`, `agc report list`). Existing IDs in local history remain accessible; `agc report show` supports prefix matching as a fallback.
- **Registry SSL** — Postgres connections now use SSL by default (`rejectUnauthorized: false`). Set `AGENTCAROUSEL_REGISTRY_DB_SSL=false` to disable for local dev.

### Fixes

- Golden `path` values in four fixture YAML files (`github-actions-generator`, `database-migration-advisor`, `dockerfile-linter`, `sql-query-generator`) pointed to a non-existent centralised `fixtures/golden/<skill>/` layout. Corrected to the co-located `fixtures/<skill>/golden/` paths that match the actual files on disk. The `github-actions-generator` golden also had a `.yml` extension where the file is `.txt`.

### Chores

- Removed six orphaned stubs from `mocks/agent-response.json` for `rag-qa` and `tool-call-correctness`, which no longer have fixture directories. Rehashed bundle manifests for `customer-support`, `code-reviewer`, and `terraform-sentinel-scaffold` to match the updated mock file.

## 0.6.1 - May 2026

### Features

- **`agc watch`** — Filesystem watcher for live fixture re-evaluation. `agc watch fixtures/my-skill/ [--eval]` detects changes to fixture YAML files, debounces 200 ms, and automatically re-runs only the affected cases. Collapses the edit→run→read loop to under 2 seconds. Uses the `notify` crate; fully additive on top of existing eval infrastructure.
- **`agc carousel`** — Multi-model fixture evaluation. `agc carousel --models gpt-4o,claude-sonnet-4-6,gemini-2.5-flash fixtures/my-skill/` runs the same fixture suite against N models in parallel and produces a ranked comparison table: effectiveness score, pass rate, mean latency, estimated cost. Each model's run is persisted to history so the dashboard compare view works immediately. Includes a progress bar per model.
- **`agc ab`** — A/B prompt variant comparison. `agc ab --a prompt-v1.md --b prompt-v2.md fixtures/skill/` runs identical fixture cases against two system prompts concurrently and produces a head-to-head: per-case winner, per-rubric effectiveness delta, overall pass-rate comparison, and cases that flipped status. JSON output supported for pipeline use.
- **Statistical significance on `agc compare`** — When both runs have N≥5 scored cases, applies the Mann-Whitney U test (non-parametric, no normality assumption) to determine whether the effectiveness score delta is statistically significant. Surfaces p-value alongside the raw delta (`Δ −0.03, p=0.004 ★ significant`). New `--significance <alpha>` flag (default 0.05); exit code 1 (regression) only fires when the delta exceeds threshold **and** p < alpha. Backwards-compatible: skips the test and notes it when N<5. No new dependencies — Mann-Whitney U implemented in ~30 lines of pure math.

### Chores

- Drop `openrouter-rs` crate; replace with raw `reqwest` calls in `generator.rs`. Removes ~100 KiB from the release binary and eliminates the `derive_builder` and `dotenvy_macro` transitive dependencies. Behavior is identical — OpenRouter's API is OpenAI-compatible and the existing `OpenAiRequest`/`OpenAiResponse` structs are reused directly.

## 0.6.0 - May 2026

**Theme: From solo eval tool to team-scale CI platform.**

### Features

- **`agc generate`** — LLM-powered fixture case generation. Point it at a skill description, an existing `prompt.md`, or an existing fixture directory and it scaffolds validated YAML cases using your configured generator model. Retries once with validation errors appended if the LLM output fails schema validation. `--dry-run` writes to stdout for pipeline use; `--json` emits a structured envelope for agent workflows. Uses the same `GeneratorProvider` / `call_provider_blocking` infrastructure as `agc eval` — no new HTTP code.
- **`agc compare`** — CI regression gate. Compares two eval runs by effectiveness score and pass rate; exits 1 when regression exceeds `--threshold` (default 0.05). Supports explicit `--baseline <run-id>`, named baselines (`agc compare tag <run-id> --name prod-baseline`), and auto-baseline (previous run for same skill). Structured `--json` output for downstream tooling.
- **`agc dashboard`** — Embedded web UI served from a single binary, zero config. Run `agc dashboard` and open `http://localhost:7421`. Four pages: run history index with trend sparklines, run detail with inline case expansion, side-by-side run comparison with delta badges, and a judge review screen for annotating LLM judge calls (✓ correct / ✗ wrong / ~ borderline). Annotations persist to `reviews.jsonl` alongside the history DB. SSE keeps the dashboard live as new runs arrive.
- **`--json` / TTY detection** — Every command emits a structured JSON envelope (`{"ok": true, "command": "...", "data": {...}}`) when `--json` is passed or stdout is not a TTY. Error paths return `{"ok": false, "error": {"code": "...", "message": "...", "suggestions": [...]}}`. Compact no-arg help when stdout is not a TTY.

### Packaging

- **Dual release variants** — Every release now ships two artifacts per platform:
  - `agentcarousel-{tag}-{triple}.tar.gz` — slim binary (default, no dashboard)
  - `agentcarousel-{tag}-{triple}-full.tar.gz` — full binary with web dashboard UI
- **`agc update --feature dashboard`** — In-place upgrade to the full variant. `agc update` without `--feature` stays on the slim variant.
- **Install script** — `--feature dashboard` flag and `AGENTCAROUSEL_FEATURES=dashboard` env var added; both select the full binary. Default install remains slim.

### Dashboard Cargo feature

`axum` and `tokio-stream` are now optional dependencies behind the `dashboard` feature flag. The default build (`cargo build -p agentcarousel --release`) produces the slim binary. Add `--features dashboard` for the full binary.

## 0.5.7 - May 2026

### Refactors

- Unified LLM provider HTTP layer: Gemini, OpenAI, Anthropic, and OpenRouter calls consolidated into `providers/` module; `evaluators/judge.rs` and `runner/generator.rs` both delegate to a single `call_provider_blocking()` path. Removes the `openrouter-rs` dependency.
- `core/error.rs` and `core/ids.rs` merged into `core/models.rs`; two micro-files eliminated with zero API change.
- `cli/mod.rs` split: `InitArgs`, `run_init`, scaffold templates, and `sanitize_fixture_name` extracted to `cli/init.rs`; `ExitCode` enum to `cli/exit_codes.rs`. `cli/mod.rs` now contains only routing logic.
- `runner/mod.rs` (948 lines) split into `runner/orchestration.rs` (execution flow) and `runner/aggregation.rs` (metrics and summary building); `runner/mod.rs` retains only public types and entry points.

## 0.5.6 - May 2026

### Chores

- Relicense from Apache-2.0 to MIT.
- Realize you're not as clever as you thought you were and take a break.
- Add prompt text to bundle schema and registry API (optional)
- Go outside, it's a nice day.

## 0.5.5 - May 2026

### Features

- Registry listing: `GET /v1/bundles` endpoint added to `agentcarousel-registry`; returns all bundles with `bundle_id`, `bundle_version`, `trust_state`, `description`, and `domain` derived from stored manifest JSON.
- `agentcarousel-www` agent index is now dynamic: `pilotAgents` hardcoded list replaced with live `listBundles()` fetch from the registry API. Any published bundle appears on `/agents` automatically without a code deploy.

## 0.5.4 - May 2026

### Bug fixes

- Fix `cargo publish` failure: schema file (`skill-definition.schema.json`) copied into crate directory (`crates/agentcarousel/schemas/`); both `include_str!` paths in `schema.rs` and `export.rs` updated to reference the in-crate copy. The workspace-relative paths (`../../schemas/`) were unreachable from the `cargo package` tarball.

## 0.5.3 - May 2026

### Features

- Custom HTTP endpoint provider (`GeneratorProvider::Custom`, `call_custom_endpoint()`); wired through `RunnerConfig` and CLI args.
- `--update-golden` flag for the golden evaluator; writes golden files in place when set.
- `agc stats` command for historical trend analysis.
- Global run timeout (`--timeout-run`); `run_timeout_secs` on `RunnerConfig`.
- p50/p95/p99 latency percentiles in `RunSummary`; shown in terminal reporter.
- Deduplicated API key candidate lists; `GeneratorProvider::key_candidates()` is now public.

### Fixture layout

Fixtures now live in per-skill directories (`fixtures/<skill>/`) containing `cases.yaml`, `prompt.md`, `bundle.manifest.json`, and `golden/`. `agc init --skill <name>` scaffolds the full structure. The old flat layout (`fixtures/skills/<skill>.yaml`) is removed.

### Fixtures

12 built-in skills: accessibility-auditor, ci-failure-triager, code-reviewer, customer-support, data-privacy-classifier, database-migration-advisor, dockerfile-linter, env-file-auditor, error-message-improver, github-actions-generator, prompt-injection-detector, regex-builder, sql-query-generator, terraform-sentinel-scaffold, unit-test-generator.

## 0.5.2 - May 2026

### Bug fixes

- `trust_check`: temp pubkey file leaked to disk after attestation (agc-1wd).
- Epic E: bounded judge response cache — `BoundedCache` with `VecDeque` FIFO eviction; `run_eval()` uses `Arc<Mutex<BoundedCache>>` instead of unbounded `HashMap` (agc-cyw, agc-gfo, agc-3t3).

## 0.5.1 - May 2026

- Feature: `agc doctor` subcommand — checks API keys, config file, history DB, fixtures directory, and JSON schema in one pass; supports `--json` for machine-readable output.
- Feature: `agc lint` subcommand — checks fixture quality beyond schema: smoke-tag coverage, judge-case descriptions, rubric weight sums, and bundle compliance fields.
- Feature: `agc validate --format sarif` — emits SARIF 2.1.0 for GitHub code scanning integration.
- Fix: `--config` and `--run-id` removed from the global flag set; they now appear only on the subcommands that consume them (`update`, `completions`, and `init` no longer advertise them).
- Fix: top-level quick-start example dropped redundant `--offline true` (mock mode is already the default for `agc test`).
- Fix: `trust-check` temp pubkey file now uses a ULID instead of the process ID, closing a predictable-name race on the temp path.

## 0.5.0 - May 2026

- Feature: `agc update` subcommand — checks GitHub for a newer release and installs it in-place with an atomic rename; supports `--check` to print availability without installing.
- Improved `--help` output: ANSI color styles, concise subcommand summaries, and `after_help` example blocks for `eval`, `test`, `validate`, `bundle`, and `trust-check`.
- Fix: release binary `strip = true` now correctly strips symbols on macOS (switched from thin LTO to fat LTO).
- Fix: update temp file uses a ULID instead of the process ID for collision-safe naming.
- Fix: `process` evaluator now emits a stderr warning when `process_cmd` is used, making the trust requirement explicit.

## 0.4.8 - May 2026

- Feature: `evaluator_config.effectiveness_threshold` per-case field; cases can now override the global `--effectiveness-threshold` flag with a case-specific pass threshold.
- Feature: Added live evaluation token consumption metrics to the terminal output.
- `agc completions <shell>` subcommand: prints a shell completion script to stdout for bash, zsh, or fish. Pipe to the appropriate completions directory to wire up tab completion.

## 0.4.7 - May 2026

- CI/CD Hardening: Fixed skip logic for automated publish jobs to correctly support manual `workflow_dispatch` releases.
- Corrected tag resolution in Homebrew formula updates to ensure consistent versioning across automated runs.

## 0.4.6 - May 2026

- Automated Homebrew Tap updates via GitHub Actions: formula version and SHA256 are now updated automatically on every tag release.
- Automated crates.io publishing using `publish-crates` action in the release workflow.

## 0.4.5 - May 2026

- Chore: remove outdated category and unused keywords from `Cargo.toml`.
- Internal branch cleanup and repository maintenance.
- Release binary size reduction.

## 0.4.4 - May 2026

- Human-readable **`validate`** terminal output: carousel banner, per-file PASS/WARN/FAIL rows, results line, heuristic coverage summary (risk tier / data handling / certification track counts), and validation status footer (aligned with eval/test reporting).
- **`eval -h`** help and field docs: clearer judge workflows (`--evaluator judge` vs `--evaluator all --judge`), narrowing judge-only runs with **`--filter`** (glob on case id) and **`--filter-tags`**.

## 0.4.3 - May 2026

- Terminal output for `eval` / `test` / `report show`: single certificate/quarantine line in the footer (no per-case quarantine); evaluator-aware failure details (judge overall rationale plus lowest rubric rows; golden/process rubric lines); humanized provider/API errors from embedded JSON.
- `report show` inherits the same terminal formatting via shared `print_terminal`.
- `report show <PATH>` accepts a path to `run.json` or an evidence directory containing `run.json`, so exported packs render with the same human-readable terminal output as history lookups.

## 0.4.2 - May 2026

- Human-readable `eval` / `test` terminal output: carousel header (version · skill · case count), offline/mock/live subtitle, padded pass/fail rows with timings, richer failure details and boxed footer (results, effectiveness, certificate, run id and `report show` hint). Run records optionally carry skill label and runner flags for consistent reporting.

## 0.4.1 - May 2026

- Align crate version with CLI `--version`, run metadata (`agentcarousel_version`), and packaging metadata for patch release.

## 0.4.0 - May 2026

- Release packaging aligned with crates.io publish (`cargo publish -p agentcarousel`).
- CI/release workflow fixes: distribution packaging script, bundle manifest hashes, `eval --filter-tags`, and validation paths for fixtures.
