# Fixture Development Process

> **Related:** [Fixture format](fixture-format.md) · [agentcarousel.com](https://agentcarousel.com)  
> **Templates:** [`templates/fixture-skeleton.yaml`](https://github.com/agentcarousel/agentcarousel/blob/main/templates/fixture-skeleton.yaml) · [`templates/fixture-bundle.manifest.json`](https://github.com/agentcarousel/agentcarousel/blob/main/templates/fixture-bundle.manifest.json) · [Tag examples](fixture-tag-examples.md)

---

## Purpose

This document describes a process for authoring `agentcarousel` fixtures that are correct, reproducible, and ready for submission. The emphasis is on **speed with guardrails**: most standard-tier fixtures should go from blank intake to passing CI in under two hours.

Skip ahead to the [Quick-start checklist](#quick-start-checklist) if you are iterating on an existing fixture.

---

## Tag vocabulary

Use these tags in new fixtures:

- `smoke` (fast PR gate)
- `happy-path` (core success scenario)
- `error-handling`
- `edge-case`
- `certification`
- `deferred`

---

## Roles

| Role | Responsibility |
|------|---------------|
| **Author** | Designs scenarios, writes YAML, owns mocks, runs `validate` + `test` locally |
| **Reviewer** | Peer-checks correctness and compliance; domain reviewer for certification track |
| **Operator** | Ingests evidence packs, tracks trust state, assigns auditors |
| **Domain Auditor** | Human expert who reviews Stable agents before Trusted attestation |

---

## Workflow

### Phase 1 — Intake (15–30 minutes)

Complete the **fixture proposal checklist** in [CONTRIBUTING.md](https://github.com/agentcarousel/agentcarousel/blob/main/CONTRIBUTING.md) **before writing any YAML**. That prevents scope creep and surfaces data sensitivity issues early.

Minimum questions to answer:

| Question | Why it matters |
|----------|---------------|
| What is the `skill_or_agent` id? | Determines the fixture file name and case id prefix |
| What user goal does this scenario test? | One clear goal per fixture file; edge cases are separate `cases` entries |
| Are any tool calls required? | Drives `expected.tool_sequence` design |
| What is the risk tier? (`low` / `medium` / `high`) | High-risk fixtures require a domain reviewer at review phase |
| Is input data synthetic? | Real PII in fixtures is never acceptable; synthetic data required |

**Intake gate:** Do not proceed to Phase 2 until the checklist is complete and the Author has confirmed: (a) no PII in inputs, (b) scope is bounded, (c) mocks can be written without live network calls.

---

### Phase 2 — Scenario Design (30–60 minutes)

Design **one primary use case per fixture file**. A primary use case is the happy-path: the most important thing the skill/agent should do when everything works correctly.

Add edge cases as separate `cases` entries within the same file, not as separate files, unless the edge case covers a substantially different workflow.

**Scenario structure pattern:**

```
fixture file
├── case: happy-path        (smoke, happy-path tags)
├── case: edge-case-A         (edge-case tag)
├── case: edge-case-B         (edge-case tag)
└── case: failure-mode        (error-handling tag)
```

**Pairing rule:** Always author the happy-path and at least one failure-mode case together in Phase 2. The failure-mode case almost always reveals mock gaps that are cheaper to fix before the mocks are written than after.

---

### Phase 3 — Author (time varies by complexity)

Use `agentcarousel init` as your starting point. Never write a fixture from a blank file.

```sh
# Scaffold from init, then replace with your values:
agentcarousel init --skill summarize-skill > fixtures/skill-summarize.yaml
# Or copy the annotated template:
cp templates/fixture-skeleton.yaml fixtures/my-new-fixture.yaml
```

**Author checklist (run through before Phase 4):**

- [ ] Every `case` has an `id` in `<fixture-stem>/<case-name>` format
- [ ] Every `case` has a `description` (not optional, even though schema allows it)
- [ ] Every `case` has at least one `tag`; happy-path cases include `smoke`
- [ ] `expected.tool_sequence` is present even when empty (`[]`) — makes intent explicit
- [ ] At least one `output` assertion per case
- [ ] Every rubric item has a `weight`; weights sum to `1.0` across rubric items
- [ ] Rubric items that cannot be auto-checked are documented with a comment explaining what a judge or human reviewer should look for
- [ ] Mock files referenced by `--mock-dir` cover every tool call in `expected.tool_sequence`
- [ ] `timeout_secs` is set to a realistic upper bound (not the default, not 999)

**Evaluator selection hierarchy** (use the first that works; escalate only when needed):

1. `rules` — exact match, regex, JSON path, tool sequence count. Free, deterministic, fast.
2. `golden` — diff against a known-good output file. Use when output format is stable and you have a reference output.
3. `process` — external script (Python, JS). Use when you need custom logic that doesn't fit rules/golden.
4. `judge` — LLM-as-judge. Use **only** for rubric items that genuinely require language understanding and cannot be expressed as any of the above. LLM judge adds cost, variance, and API dependency; minimize its use.

---

### Phase 4 — Self-Check (10–20 minutes)

Run all three checks locally before requesting a review. Do not skip `--mock-strict`.

```sh
# 1. Schema validation — must exit 0
agentcarousel validate fixtures/my-new-fixture.yaml --strict

# 2. Offline test with strict mock enforcement
agentcarousel test fixtures/my-new-fixture.yaml \
  --offline \
  --mock-dir mocks/ \
  --mock-strict

# 3. Eval pass (if rubric items exist)
agentcarousel eval fixtures/my-new-fixture.yaml \
  --mock-dir mocks/ \
  --offline

# 4. Inspect the run
agentcarousel report show $(agentcarousel report list --limit 1 --json | jq -r '.[0].id')
```

**Common self-check failures and fixes:**

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| `validate` exit 2, `missing field: expected` | Forgot `expected:` block | Add `expected: {tool_sequence: [], output: []}` minimum |
| `test` exit 1, tool call not matched | Mock args don't match `args_match` partial spec | Check mock file field names; `args_match` is a partial JSON match |
| `test` exit 4, timeout | `timeout_secs` too low for mock latency | Increase `timeout_secs`; check mock response time |
| `eval` exit 1, effectiveness score below threshold | Rubric weights don't reflect actual pass criteria | Adjust `weight` or tighten `auto_check` assertion |
| Flaky: passes sometimes, fails sometimes | Non-deterministic assertion (e.g., regex too loose) | Tighten regex; add seed; use `--runs 3` to surface flakiness |

If `--mock-strict` causes failures because a tool call is unmocked, do not disable `--mock-strict`. Add the missing mock.

---

### Phase 5 — Peer Review (async)

Open a PR or share the fixture file with the reviewer. Use the review checklist below as the PR description template.

**Standard review checklist:**

```markdown
## Fixture Review Checklist

**Fixture file:** `fixtures/<name>.yaml`
**Author:** @...
**Reviewer:** @...

### Correctness
- [ ] Case ids are unique and follow `<stem>/<name>` convention
- [ ] Tool sequence expectations match the described behavior
- [ ] Output assertions are necessary (not over-fitted to one specific wording)
- [ ] Edge-case inputs are realistic; not constructed to trivially pass
- [ ] Mocks are plausible; mock responses are not simplified to the point of hiding real failure modes

### Completeness
- [ ] Positive case present
- [ ] At least one failure-mode or edge case present
- [ ] All rubric weights sum to 1.0 per case
- [ ] `description` present on every case

### Safety & data
- [ ] No PII, credentials, or real API keys in fixture inputs, mock responses, or expected outputs
- [ ] `--offline` passes; no undeclared network calls
- [ ] Mocks committed with fixture; fixture does not depend on external state

### For certification track only
- [ ] `bundle_id` and `bundle_version` set in manifest
- [ ] `certification_track: candidate` in manifest
- [ ] `risk_tier` and `data_handling` set correctly
- [ ] Second reviewer (domain expert) has signed off
- [ ] Flake budget: ran `agentcarousel eval --runs 5` locally with 0 failures
```

**Certification track adds:** a domain reviewer must be assigned before the PR is merged. The domain reviewer verifies that the scenarios are realistic for the skill's stated domain and that the rubric items correctly represent quality in that domain. This is **not** the same as the formal `agentcarousel audit`.

---

### Phase 6 — Freeze and Bundle Version Bump

After review is approved and CI is green:

1. If the fixture file is part of a bundle, update `bundle_version` in `bundle.manifest.json`:
   - **Patch bump** (`1.2.0` → `1.2.1`): description/comment changes only.
   - **Minor bump** (`1.2.0` → `1.3.0`): new cases added that do not remove existing cases.
   - **Major bump** (`1.2.0` → `2.0.0`): cases removed, case ids renamed, or existing assertions made stricter. **Major bumps reset the carousel iteration counter to 0** — the agent must re-earn Stable status.

2. Recompute `sha256` entries in `bundle.manifest.json` (or run `agentcarousel bundle pack` in M3+).

3. Tag the commit if this is a bundle submission to the AGC registry.

---

## Definition of Done

A fixture is **done** when all of the following are true:

- [ ] `agentcarousel validate fixtures/<name>.yaml --strict` exits 0
- [ ] `agentcarousel test fixtures/<name>.yaml --offline --mock-dir mocks/ --mock-strict` exits 0
- [ ] JSON or JUnit XML is parseable by CI (run once in the pipeline)
- [ ] Every case has `description` and at least one tag
- [ ] PR reviewed and approved (self-review acceptable for standard tier)

### Certification

All Standard items, plus:

- [ ] Bundle manifest (`bundle.manifest.json`) present, valid, and up-to-date
- [ ] `bundle_id`, `bundle_version`, `certification_track: candidate`, `risk_tier`, `data_handling` all set
- [ ] `agentcarousel eval fixtures/<name>.yaml --offline --runs 5 --mock-dir mocks/` exits 0 with all 5 runs passing
- [ ] Effectiveness score ≥ `effectiveness_threshold` across all 5 runs
- [ ] 0 flakes across 5 local eval runs (no intermittent failures)
- [ ] Domain reviewer has approved (second reviewer, separate from Author)
- [ ] `owners` list in manifest includes at least one GitHub handle
- [ ] `policy_version` matches current AGC policy document version
- [ ] Commit sha for this bundle version recorded in PR description

---

## Quick-Start Checklist

For authors iterating on an existing fixture (not starting from scratch):

```sh
# After making changes:
agentcarousel validate fixtures/my-fixture.yaml --strict
agentcarousel test fixtures/my-fixture.yaml --offline --mock-dir mocks/ --mock-strict
# If rubric changed:
agentcarousel eval fixtures/my-fixture.yaml --offline --runs 3
# Check for regressions against previous run:
agentcarousel report diff <PREV_RUN_ID> $(agentcarousel report list --limit 1 --json | jq -r '.[0].id')
```

If `report diff` exits 1, investigate which metric degraded before merging.

---

## Expedited Tactics

The following practices dramatically reduce the time from idea to merged fixture:

**1. Start from `init` or the template — never blank YAML.**  
The template includes all optional fields as comments, preventing forgotten fields during review.

**2. Write mocks before assertions.**  
Draft the mock response first; then write the `output` assertions against what the mock actually returns. This eliminates the most common test failure: assertions written against an idealized output that doesn't match mock behavior.

**3. Use `rules` evaluator first; escalate to `judge` only for genuinely ambiguous rubrics.**  
LLM-judge adds ~1–3 seconds per case invocation plus API cost. Most rubrics are expressible as regex or JSON path. If you find yourself writing a judge for a rubric that could be a regex, rewrite it as a regex.

**4. Tag-driven CI reduces full eval to smoke-only on PRs.**  
Mark edge-case and certification cases with specific tags. Configure CI to run `--filter-tags smoke` on PRs and full evaluation only on main/nightly. This keeps PR feedback under 30 seconds.

**5. Pair happy-path + failure-mode from the start.**  
Authors who write only the happy-path in Phase 3 and add failure cases later spend 2x as long on Phase 4 because failure cases almost always reveal mock gaps.

**6. Keep mock responses minimal but realistic.**  
A mock that returns an implausibly perfect response will cause assertions to pass locally but fail against a real endpoint. Use realistic (slightly imperfect) responses in mocks: include minor formatting variation, realistic token counts, and occasional tool result delays.

**7. Run `--mock-strict` always.**  
Any unmocked tool call discovered during review or CI is a fixture authoring error, not a test runner issue. `--mock-strict` surfaces these early.

---

## Fixture File Naming Convention

```
fixtures/
├── <domain>/
│   ├── <skill-or-agent-id>.yaml           # primary fixture file
│   ├── <skill-or-agent-id>-edge.yaml      # edge cases (separate file if many)
│   └── <skill-or-agent-id>-stress.yaml    # load/stress cases (optional)
└── examples/                              # curated examples (maintained by AGC)
    └── *.yaml
```

Case ids must always match the fixture file stem:
- File `fixtures/text-processing/skill-summarize.yaml` → case ids start with `skill-summarize/`
- File `fixtures/search/agent-web-search.yaml` → case ids start with `agent-web-search/`

This is enforced by `agentcarousel validate` and the schema.

---

## Common Mistakes to Avoid

| Mistake | Why it's a problem | Correct practice |
|---------|-------------------|-----------------|
| Using real API responses in mocks | Embeds real data, possibly PII; changes over time | Use synthetic data that matches the schema of the real response |
| Setting `timeout_secs: 300` | Masks slow agents; CI takes forever | Set to 1.5× the expected real latency; investigate if exceeded |
| Writing rubric weights that don't sum to 1.0 | `eval` scoring is incorrect; effectiveness score is meaningless | Always sum weights to `1.0` per case |
| Using `kind: equals` for LLM output | LLM output varies by temperature/seed | Use `kind: contains` or `kind: regex`; reserve `equals` for structured/tool outputs |
| Omitting `tool_sequence: []` for skills with no tool calls | Ambiguous intent; reviewers don't know if tool calls were forgotten | Always include `tool_sequence: []` explicitly for zero-tool-call skills |
| Skipping `--mock-strict` in self-check | Hidden unmocked calls discovered in CI | Always run `--mock-strict` locally before requesting review |
| Checking in API keys in `env_overrides` | Security violation; keys may appear in traces | `env_overrides` is for non-secret config only; keys must come from environment |
