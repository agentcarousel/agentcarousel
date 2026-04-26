# CI gates (blocking checks)

This repository treats **CI-blocking** checks as commands that must exit with status **0** on the default branch and on pull requests. If a step fails (non-zero exit), the GitHub Actions job fails and the pull request **cannot merge** until the failure is fixed.

## What runs on PRs

1. **Rust CI** — `cargo fmt --check`, `cargo clippy ... -D warnings`, `cargo test`.
2. **`validate`** — `agentcarousel validate fixtures/ --format json` (schema, kebab-case ids, safe paths, and warning-only heuristics such as PEM-shaped strings). Writes `validate-report.json` as an artifact for inspection.
3. **Bundle verify** — `bundle verify` on the CMMC assessor bundle (contract checks).
4. **Offline fixture tests (matrix)** — `agentcarousel test` with `--filter-tags smoke --offline true`:
   - **smoke-two**: two representative YAML fixtures.
   - **skills-smoke**: the entire `fixtures/skills` tree (breadth without running all non-smoke cases).

`negative` remains accepted as a temporary alias during migration, but `smoke` is the canonical PR gate tag.

Each matrix leg uploads its JSON log as a distinct artifact.

## Exit codes

See [`docs/exit-codes.md`](exit-codes.md). In short: validation failures use a dedicated non-zero code so automation can distinguish “bad fixtures” from generic runtime errors.

## Optional assertions with `jq`

`validate --format json` emits an object with `messages` (diagnostics) and `atf_summary` (heuristic coverage from fixture fields—not a certification claim). Example optional gate:

```bash
./target/release/agentcarousel validate fixtures/ --format json | tee validate-report.json
jq -e '.atf_summary.fixture_files_loaded > 0' validate-report.json
```

## Nightly (not PR-blocking)

Scheduled **eval** jobs may use API keys and are separate from the PR merge gates.
