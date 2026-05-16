# Concepts

This is a quick conceptual description for `agentcarousel`: what the components are and how they fit together. Deeper documentation on the CLI can be found at [docs.rs/agentcarousel](https://docs.rs/agentcarousel)).

## Fixture

A **fixture** is a per-skill directory under `fixtures/<skill>/` containing:

- `cases.yaml` — YAML file describing the skill or agent under test, with metadata (`skill_or_agent`) and a list of **cases**. Each case has an `id`, `input` (usually chat prompts), and `expected` assertions.
- `prompt.md` — system prompt for the skill or agent.
- `bundle.manifest.json` — bundle metadata for certification and registry workflows.
- `golden/` — golden output files referenced by `golden_diff` assertions.

Example: `fixtures/regex-builder/cases.yaml` — a skill with smoke-tagged happy-path and failure-mode cases.

Use `agentcarousel init --skill <name>` to scaffold the full directory structure.

## Case tags

Cases may list `tags` (e.g. `smoke`). The CLI can filter on tags using `--filter-tags`:

```bash
agentcarousel test fixtures/regex-builder/cases.yaml --offline true --filter-tags smoke
```

## Mock / offline runs

With `--offline true`, generation uses mocks from `mocks/` that give reproducible regression outputs without calling external LLM APIs.

Offline passes are useful, but they are not a full quality signal by themselves.

## Evaluation and judges

`eval` mode can run mock or live generation and optional judge models for rubric scoring. Live paths are variable; use multi-run sampling (`--runs`) and separate tolerance gates instead of relying on one run.

External evaluator processes can follow a stdin/stdout JSON contract; see the `evaluators` module and related types on [docs.rs/agentcarousel](https://docs.rs/agentcarousel) if you build custom scorers.

## Evidence and history

The CLI can persist run history and export evidence packs for a run id (`export`). Use this when you need audit-style artifacts alongside console or JSON reports.
