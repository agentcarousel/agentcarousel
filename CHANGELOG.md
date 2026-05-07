# Changelog

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
