# Scope profile (`scope.yaml`)

A scope profile records what an evaluation covers: what the agent is for, which tools and data it touches, who uses it, which instance was tested, what was left out, and how the test effort is meant to split across risk categories. It sits next to a fixture's `cases.yaml`:

```
fixtures/my-agent/
  cases.yaml
  scope.yaml
```

`scope.yaml` and `scope.yml` are reserved names: commands that walk fixture directories don't load them as fixtures.

## Commands

```bash
agc scope init --skill my-agent      # write a commented skeleton
agc scope check --skill my-agent     # validate it and compare planned vs actual coverage
agc validate fixtures/               # also checks any scope.yaml beside a fixture
```

`agc scope check` exits 2 when the profile has errors. It prints, for each risk category, the planned share, the share of cases whose tags match the category, and the cases that match no category.

`agc export` copies a valid profile into the evidence bundle as `scope.json`, lists it in `MANIFEST.json`, and adds its digest to the attestation's `evidence_documents`, so `agc verify` detects any change to it. The profile is read when you export, from the directory of the run's fixture. A profile with errors is left out, with a note.

## Fields

The schema is `schemas/scope-profile.schema.json`. Unknown fields are rejected.

| Field | Required | Meaning |
|---|---|---|
| `schema_version` | yes | `1` |
| `skill_or_agent` | yes | Must match the fixture's `skill_or_agent` |
| `use_cases` | | What the agent does for its users |
| `tools` | | `{name, access: read\|write, description}`; compared with the fixture's `agent.tools` when declared |
| `modalities` | | e.g. text, documents, images, voice |
| `data_sensitivity` | | Categories of data the agent can see or produce |
| `users` | | `{type, vulnerable}` |
| `instance_under_test` | | `{environment, matches_production, guardrails, data: synthetic\|seeded\|real, notes}` |
| `out_of_scope` | | `{item, reason}` for everything not covered |
| `risk_distribution` | | `{category, share_pct, rationale, tags}`; shares sum to 100 (±0.5). A case counts toward a category when it carries one of the category's tags |
| `tool_calls_observed` | | Must be `false`: agc records tool calls from fixture expectations, not from a running agent |

See `fixtures/ambient-scribe/scope.yaml` for a worked example. Risk categories can name tags from a crosswalk such as [AIUC-1](frameworks/aiuc-1.md) (for example `aiuc-1:D001`) so the planned split lines up with the controls you report on.
