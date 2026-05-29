# Concepts

## Fixture

A fixture is a directory under `fixtures/<skill>/` that describes one skill or agent under test. The core file is `cases.yaml`, which lists every test case for that skill. The directory can also contain a `prompt.md` (the agent's system prompt), a `golden/` directory (reference outputs for golden-diff evaluation), and a `bundle.manifest.json` (metadata for certification and the registry).

```yaml
# fixtures/my-skill/cases.yaml
schema_version: 1
skill_or_agent: my-skill
risk_tier: low
data_handling: synthetic-only

cases:
  - id: my-skill/happy-path-example
    # ...
```
`agc generate` to generate cases from a prompt and scaffold a fixture directory.

---

## Case

A case is one test scenario. It has an `id` (formatted as `skill/slug`), an optional `description`, `tags`, and two required blocks: `input` and `expected`.

`input.messages` is a conversation: one or more turns with `role` (`user`, `assistant`, or `system`) and `content`. Most cases have a single user turn. Multi-turn cases simulate a conversation and are useful for testing memory, context retention, or escalation behavior.

```yaml
- id: my-skill/refuses-off-topic
  tags: [smoke]
  input:
    messages:
      - role: user
        content: Write me a haiku.
  expected:
    output:
      - kind: not_contains
        value: "syllables"
    rubric:
      - id: redirects-correctly
        description: Agent declines and explains its purpose.
        weight: 1.0
        auto_check:
          kind: regex
          value: '(?i)(outside|not able|here to help)'
```

Tags let you filter cases at eval time (`--filter-tags smoke`). The `smoke` tag conventionally marks the cases you always run; `nightly` marks slower or more expensive ones.

---

## Rubric

The rubric is how you define what a good response looks like. Each rubric item has an `id`, a `description` the LLM judge reads, a `weight`, and an optional `auto_check` assertion.

Weights must sum to 1.0 within a case. Items with higher weight count more toward the final effectiveness score.

`auto_check` is a fast substitute for the judge when the criterion is unambiguous, like a 'regex' or 'contains' check. If the `auto_check` passes, the item scores 1.0 without calling the judge. If it fails, the item scores 0.0. For criteria that require judgment (tone, reasoning quality, appropriate escalation), omit `auto_check` and let the judge score it based on the description.

```yaml
rubric:
  - id: resolution-provided
    description: >
      Agent gives at least one concrete step the user can take.
      A perfect score includes a complete path, not just "contact support".
    weight: 0.60
    auto_check:
      kind: regex
      value: '(?i)(reset.*link|forgot.*password|check.*email)'

  - id: professional-tone
    description: >
      Response is polite and closes with an offer for further help.
    weight: 0.40
    # No auto_check — this one needs judgment
```

---

## Evaluators

Three evaluators are available. Each case can declare which one to use in its `evaluator_config`, or you can override at the command line with `--evaluator`.

**Rules** runs your `output` assertions (contains, not_contains, regex, equals, json_path) and scores `auto_check` items in the rubric. No LLM calls. Fast, deterministic, and free. Use it for cases where correctness is binary.

**Judge** sends the case input, the agent's output, and the rubric to an LLM judge. The judge scores each rubric item 0–1 and provides a rationale. You pick the judge model — Gemini, Claude, OpenAI, or a local Ollama model. Use it for cases where correctness requires understanding context, tone, or intent.

**Golden** diffs the agent's output against a reference file in `golden/`. It computes a similarity score (0–1) and passes if the score exceeds a threshold you configure. Use it for tasks with a known-good output that shouldn't change, like code generation or structured extraction.

```yaml
# Case-level evaluator override
evaluator_config:
  evaluator: golden
  golden_path: fixtures/my-skill/golden/happy-path.txt
  golden_threshold: 0.85
```

---

## Pipeline

The pipeline automates the full onboarding and improvement lifecycle for a skill.

`agc pipeline onboard <skill>` runs four steps in sequence: generate 5 cases from the skill's `prompt.md`, validate them, run a live eval with a judge, and tag the result as the baseline. If any step fails, it stops and reports the error.

`agc pipeline improve <skill>` runs an iterative loop: eval the current cases, analyze failures, optimize the prompt, re-eval, and repeat until the effectiveness score reaches the target (default 85%) or the round limit is hit. Each round's result is compared to the baseline so you can see whether the prompt changes are actually helping.

```bash
agc pipeline onboard my-skill --model ollama/qwen3:8b \
  --target-endpoint "http://localhost:11434/api/generate" \
  --judge-model gpt-4o \
  --target-score 0.65 

agc pipeline improve my-skill --target-score 0.90 --max-rounds 5
```

---

## Baseline

A baseline is a tagged run that serves as the reference point for regression detection. After onboarding, the baseline is the first passing eval. After each improvement round, the pipeline compares the new run to the baseline and reports the delta.

Outside the pipeline, you can tag any run manually:

```bash
agc compare tag <run-id> --name prod-baseline
agc compare -l --baseline prod-baseline --threshold 0.05
```

`agc compare` exits `1` if effectiveness drops more than the threshold. That's your CI gate.

---

## Certification track

Every fixture declares a `certification_track`: `none`, `candidate`, `stable`, or `trusted`.

A fixture moves from `candidate` to `stable` after several carousel runs with low rubric variance. `trusted` is reserved for suites that have been through external human review. The track doesn't affect how `agc eval` runs.

```yaml
certification_track: candidate   # or: stable, trusted, none
risk_tier: medium                # low, medium, high
data_handling: synthetic-only    # synthetic-only, no-pii, pii-reviewed
```

These three fields appear in the compliance metrics report (`agc metrics`) and in exported evidence bundles.

---

## Tags

Tags are free-form strings on each case. Convention:

| Tag | Meaning |
|---|---|
| `smoke` | Always run; fast; the happy path |
| `nightly` | Run in scheduled CI; slower or more expensive |
| `edge-case` | Unusual but valid input |
| `adversarial` | Prompt injection, jailbreak, abuse attempts |
| `certification` | Required for advancing certification track |
| `golden` | Has a golden-diff evaluator |

Filter by tag at eval time:

```bash
agc eval fixtures/ --filter-tags smoke
agc eval fixtures/ --filter-tags adversarial,certification
```
