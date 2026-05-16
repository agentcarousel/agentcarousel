# Changelog

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
