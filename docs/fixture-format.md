# Fixture Format

This document describes the YAML/TOML fixture format consumed by
`agentcarousel`. The authoritative schema is
`fixtures/schemas/skill-definition.schema.json`.

Use **`bundle_id`** and **`bundle_version`** (semver recommended) at the top level of the fixture for certification bundles; treat **major** bumps as breaking for anyone pinning a bundle.

## Top-level fields

- `schema_version` (integer, required): current version is `1`.
- `skill_or_agent` (string, required): kebab-case identifier for the subject.
- `defaults` (object, optional): default settings applied to each case.
- `bundle_id` (string, optional): bundle identifier for certification tracking.
- `bundle_version` (string, optional): bundle version (semver recommended).
- `certification_track` (string, optional): `none`, `candidate`, `stable`, `trusted`.
- `risk_tier` (string, optional): `low`, `medium`, `high`.
- `data_handling` (string, optional): `synthetic-only`, `no-pii`, `pii-reviewed`.
- `cases` (array, required): one or more case definitions.

## defaults

- `timeout_secs` (integer): per-case timeout fallback.
- `tags` (array of strings): tags applied to every case.
- `evaluator` (string): default evaluator id (`rules`, `golden`, `process`, `judge`).

## Case fields

- `id` (string, required): must start with `<skill_or_agent>/`.
- `description` (string): human-readable summary of the intent.
- `tags` (array of strings): case tags for filtering.
- `input` (object, required): input payload.
- `expected` (object, required): assertions and rubric items.
- `evaluator_config` (object, optional): per-case evaluator settings.
- `timeout_secs` (integer): override timeout for this case.
- `seed` (integer): RNG seed for eval runs.

## Canonical tags

Canonical tag set for authoring:

- `smoke`: fast PR-gate case that should run on every pull request.
- `happy-path`: core success scenario for the skill/agent.
- `error-handling`: graceful failure behavior.
- `edge-case`: boundary or unusual-but-valid input behavior.
- `certification`: included in certification-focused carousels.
- `deferred`: tracked placeholder for blocked integrations.

Prefer tags in all new fixtures and examples.

## input

- `messages` (array, required): ordered message list.
- `context` (object): arbitrary structured context.
- `env_overrides` (object): non-secret environment variable overrides.

`messages` entries include:

- `role` (string): `user`, `assistant`, `system`, or `tool`.
- `content` (string): message text.

## expected

- `tool_sequence` (array): required tool calls and ordering.
- `output` (array): output assertions.
- `rubric` (array): evaluation rubric items.

### tool_sequence

- `tool` (string): tool name.
- `args_match` (object): partial JSON match for tool args.
- `order` (string): `strict`, `subsequence`, or `unordered`.

### output assertions

- `kind` (string): `contains`, `not_contains`, `equals`, `regex`, `json_path`,
  or `golden_diff`.
- `value` (string): assertion value.
- `field` (string): optional JSON pointer or top-level field name.

### rubric items

- `id` (string): stable rubric identifier.
- `description` (string): what the rubric measures.
- `weight` (number): relative weight in effectiveness score.
- `auto_check` (object): optional output assertion to score automatically.

## evaluator_config

- `evaluator` (string): `rules`, `golden`, `process`, or `judge`.
- `golden_path` (string): relative path to golden output fixture.
- `golden_threshold` (number): diff threshold for golden evaluator.
- `process_cmd` (array): command and args for external evaluator.
- `judge_prompt` (string): extra prompt for the judge evaluator.

## Templates and examples

- Template: `templates/fixture-skeleton.yaml`
- Intake: open a GitHub issue using the checklist in [CONTRIBUTING.md](https://github.com/agentcarousel/agentcarousel/blob/main/CONTRIBUTING.md) before large additions
- Tag examples: [`docs/fixture-tag-examples.md`](fixture-tag-examples.md)
- Example fixtures: `fixtures/examples/`
