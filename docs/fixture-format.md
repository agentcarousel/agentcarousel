# Fixture Format

This document describes the YAML/TOML fixture format consumed by
`agentcarousel`. The authoritative schema is
`schemas/skill-definition.schema.json`.

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
- `agent` (object, optional): declared identity of the agent under test, recorded in run subjects and `evidence.json`. Fields: `id` (required, absolute URI such as a `tag:` URI), `version`, `name`, `description`, `owner`, `framework`, `runtime`, `risk_class`, `tools` (list of `{name, description, scopes}`), `external_ids` (list of `{system, id}`). Mirrors the AgentCarousel Specification `agent.schema.json`.
- `cases` (array, required): one or more case definitions.

A fixture directory can also hold a `scope.yaml` scope profile describing what the evaluation covers; see [scope-profile.md](scope-profile.md).

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
- `severity` (string, optional): how serious a failure of this case would be, on the AgentCarousel severity scale: `P0` critical, `P1` major, `P2` significant, `P3` minor (uppercase). Set it when you write the case, not after a run. `severity: P0` designates the case critical: its failures count in `summary.critical_failures` in `evidence.json`, the same as a case with a critical rubric item. Failed cases are tallied by severity in `agc report show`, `report.md`, and a top-level `x-agentcarousel-severity` object in `evidence.json`. See `--fail-on-severity` on `agc test` and `agc eval`.

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

Note: the runner does not yet observe the agent's real tool calls. It emits each expected call as a trace step (source `fixture_expectation`) and answers it from mocks, so these assertions validate the fixture rather than the agent. Exported evidence carries a `tool_calls_not_observed` limitation for such cases.

### output assertions

- `kind` (string): `contains`, `not_contains`, `equals`, `regex`, `json_path`,
  or `golden_diff`.
- `value` (string): assertion value.
- `field` (string): optional JSON pointer or top-level field name.

### rubric items

- `id` (string): stable rubric identifier.
- `description` (string): what the rubric measures.
- `weight` (number): relative weight in effectiveness score. Weights across all items in a case **must sum to exactly 1.0** (±0.001); `agc validate` reports an error if they do not.
- `auto_check` (object): optional output assertion to score automatically. Values must be domain-specific — single generic words (`sorry`, `help`, `work`) will match nearly any response and are flagged as warnings by `agc validate`.
- `critical` (boolean, optional): marks the item critical. A case with any critical item is a critical case: its failures count in `summary.critical_failures` in `evidence.json`, and `agc promote` blocks promotion when a critical item scores below the promote threshold.

## evaluator_config

- `evaluator` (string): `rules`, `golden`, `process`, or `judge`.
- `golden_path` (string): relative path to golden output fixture.
- `golden_threshold` (number): diff threshold for golden evaluator.
- `golden_normalize_whitespace` (boolean, optional): when `true`, collapse whitespace runs per line before diffing. Useful for structured output (YAML, JSON, code) where indentation may vary. Default: `false`.
- `process_cmd` (array): command and args for external evaluator.
- `judge_prompt` (string): extra prompt for the judge evaluator.
- `effectiveness_threshold` (number, optional): per-case override for the effectiveness pass threshold (0.0–1.0). Takes precedence over the global `--effectiveness-threshold` flag when set.

## Templates and examples

- Template: `templates/fixture-skeleton.yaml`
- Bundle manifest template: `templates/bundle-manifest-skeleton.json`
- Intake: open a GitHub issue using the checklist in [CONTRIBUTING.md](https://github.com/agentcarousel/agentcarousel/blob/main/CONTRIBUTING.md) before large additions
- Example fixtures: [`fixtures/ambient-scribe/`](../fixtures/ambient-scribe/), [`fixtures/github-actions-generator/`](../fixtures/github-actions-generator/)
