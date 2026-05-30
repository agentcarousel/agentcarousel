# CLI Reference

All `agc` subcommands, flags, and exit codes.

**Exit codes** (consistent across all commands):

| Code | Meaning |
|---|---|
| 0 | Success / all tests passed |
| 1 | Failure — tests failed or regression detected |
| 2 | Invalid input or arguments |
| 3 | Config error |
| 4 | Runtime error (IO, network, database) |
| 5 | Not found |

---

## agc eval

Run fixture cases against a model and score the results.

```bash
agc eval <fixture-path> [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--execution-mode` | `mock` | `live` to call real LLM APIs; `mock` for offline runs |
| `--model` | config | Generator model (e.g. `gemini-2.5-flash`, `ollama/qwen3:8b`) |
| `--evaluator` | `rules` | `rules`, `judge`, `golden`, or `all` |
| `--judge` | off | Enable LLM-as-judge scoring |
| `--judge-model` | config | Judge model |
| `--judge-endpoint` | — | Endpoint URL for local/custom judge models |
| `--generator-endpoint` | — | Endpoint URL for local/custom generator models |
| `--runs` | `1` | Number of times to run each case (averages scores) |
| `--filter` | — | Match cases by `skill/id` prefix |
| `--filter-tags` | — | Comma-separated tag filter (e.g. `smoke,certification`) |
| `--json` | off | Emit structured JSON instead of terminal output |

```bash
# Offline rules-based eval
agc eval fixtures/my-skill/

# Live eval with judge
agc eval fixtures/my-skill/ \
  --execution-mode live --judge \
  --model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001

# Filter to smoke tests only
agc eval fixtures/ --filter-tags smoke

# Local Ollama
agc eval fixtures/my-skill/ \
  --execution-mode live --judge \
  --model ollama/qwen3:8b \
  --generator-endpoint http://localhost:11434/api/generate \
  --judge-model ollama/qwen3:8b \
  --judge-endpoint http://localhost:11434/api/generate
```

---

## agc generate

Generate fixture cases from a skill description or existing prompt file.

```bash
agc generate [flags]
```

| Flag | Description |
|---|---|
| `--from-prompt <path>` | Generate from a `prompt.md` skill description |
| `--skill <name>` | Skill name (uses `fixtures/<name>/prompt.md`) |
| `--extend <path>` | Add cases to an existing fixture (deduplicates) |
| `--count <n>` | Number of cases to generate (default: 5) |
| `--model <model>` | Generator model |
| `--distribution` | Category mix, e.g. `happy:2,edge:2,failure:1` |
| `--difficulty` | Bias toward harder cases (`easy`, `medium`, `hard`) |
| `--domain-context` | Extra context injected into the generation prompt |
| `--seed-cases <path>` | Existing cases to use as style reference |

```bash
# From a prompt file
agc generate --from-prompt fixtures/my-skill/prompt.md --count 8

# Extend with specific distribution
agc generate --extend fixtures/my-skill/ \
  --count 5 --distribution "edge:3,adversarial:2"

# With a local model
agc generate --from-prompt fixtures/my-skill/prompt.md \
  --count 5 --model ollama/qwen3:8b \
  --generator-endpoint http://192.168.1.10:11434/api/generate
```

---

## agc validate

Check fixtures for schema errors, ID format, weight sums, and path safety.

```bash
agc validate <path> [flags]
```

```bash
agc validate fixtures/my-skill/
agc validate fixtures/                  # all fixtures
agc validate fixtures/my-skill/ --json  # machine-readable output
```

Exits `0` on clean validation, `2` on errors.

---

## agc pipeline

End-to-end skill lifecycle: onboard a new skill or iterate toward a target score.

```bash
agc pipeline onboard <skill> [flags]
agc pipeline improve <skill> [flags]
```

**onboard flags:**

| Flag | Default | Description |
|---|---|---|
| `--model` | config | Generator (target) model |
| `--target-endpoint` | — | Endpoint for local/custom generator |
| `--judge-model` | config | Judge model |
| `--judge-endpoint` | — | Endpoint for local/custom judge (auto-inherited from `--target-endpoint` for ollama models) |
| `--target-score` | `0.85` | Minimum pass rate to consider the skill stable |
| `--dry-run` | off | Print steps without calling any APIs |

**improve flags:**

| Flag | Default | Description |
|---|---|---|
| `--target-score` | baseline | Score to reach before stopping |
| `--max-rounds` | `5` | Maximum optimization rounds |
| `--target-endpoint` | — | Endpoint for generator |
| `--judge-endpoint` | — | Endpoint for judge |
| `--budget` | `0` (unlimited) | USD spend cap (useful for cloud models) |

```bash
# Full onboard with cloud models
agc pipeline onboard my-skill \
  --model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001 \
  --target-score 0.85

# Onboard with local Ollama (judge endpoint auto-inherited)
agc pipeline onboard my-skill \
  --model ollama/qwen3:8b \
  --target-endpoint http://192.168.1.10:11434/api/generate

# Improve until 90%
agc pipeline improve my-skill --target-score 0.90 --max-rounds 8
```

---

## agc optimize

Analyze eval failures and produce an improved prompt.

```bash
agc optimize <fixture-path> [flags]
```

| Flag | Description |
|---|---|
| `--run-id` | Run to analyze (default: latest) |
| `--model` | Model for optimization |
| `--output` | Where to write the improved prompt |

```bash
agc optimize fixtures/my-skill/ --output fixtures/my-skill/prompt.v2.md
```

---

## agc batch

Submit an eval as an async batch job (Anthropic Batch API or OpenAI Batch API - 50% token cost reduction)

```bash
agc batch submit <fixture-path> [flags]
agc batch status <job-id>
agc batch retrieve <job-id>
agc batch list
agc batch cancel <job-id>
```

| Flag | Description |
|---|---|
| `--model` | Model to submit to |
| `--provider` | `anthropic` or `openai` |
| `--judge` | Include judge scoring in the batch |

```bash
# Submit
agc batch submit fixtures/my-skill/ \
  --model claude-sonnet-4-6 --provider anthropic

# Check status
agc batch status <job-id>

# Retrieve results into run history when done
agc batch retrieve <job-id>
```

---

## agc metrics

Compliance-ready metrics report across four dimensions.

```bash
agc metrics [flags]
```

| Flag | Description |
|---|---|
| `--skill <name>` | Scope to a specific skill |
| `--fixture <path>` | Read skill name from fixture file |
| `--limit <n>` | Number of historical runs to include (default: 20) |
| `--json` | Machine-readable output |

```bash
agc metrics --skill my-skill
agc metrics --skill my-skill --json > metrics.json
```

**Metrics:**

| Metric | What it measures |
|---|---|
| Prompt Injection Resistance | Pass rate on adversarial injection cases (0–100) |
| Behavioral Stability | Effectiveness score drift over run history |
| Test Coverage Completeness | Fraction of the 7-category risk taxonomy covered |
| Score Accuracy (Calibration) | How well judge scores predict actual pass/fail |

---

## agc report

Inspect run history.

```bash
agc report list [--limit <n>]
agc report show <run-id> [--verbose] [--json]
```

```bash
agc report list
agc report show VCSS1FDSMJ
agc report show VCSS1FDSMJ --verbose   # full per-case output
```

---

## agc compare

Diff two runs. Exits `1` if effectiveness drops past the threshold.

```bash
agc compare <run-id> [flags]
agc compare -l [flags]              # compare latest run
agc compare tag <run-id> --name <tag>  # tag a run as a named baseline
```

| Flag | Default | Description |
|---|---|---|
| `--baseline <name-or-id>` | previous run | Named baseline or run ID to compare against |
| `--threshold <float>` | `0.05` | Max allowed drop in effectiveness (0–1) |
| `--significance` | `0.05` | p-value threshold for Mann-Whitney U test |

```bash
# Tag a baseline
agc compare tag VCSS1FDSMJ --name prod-v1

# CI gate: fail if effectiveness drops >5%
agc compare -l --baseline prod-v1 --threshold 0.05
```

---

## agc export

Package a run as a signed evidence bundle.

```bash
agc export <run-id>
agc export -l       # latest run
```

The bundle includes `run.json`, `report.md`, `metrics.json`, `MANIFEST.json`, and a SHA-256 fingerprint of every file.

---

## agc promote

Promote golden output from a passing run.

```bash
agc promote <run-id>
```

Writes the final output of each passing case to its configured `golden_path`. Creates a `.meta.json` sidecar with the run ID and effectiveness score.

---

## agc publish

Publish a fixture bundle to the registry.

```bash
agc publish <fixture-path> [flags]
```

| Flag | Description |
|---|---|
| `--url` | Registry URL |
| `--all-runs` | Include all historical runs |
| `--limit <n>` | Cap number of runs to publish |

Requires `AGENTCAROUSEL_API_TOKEN` in the environment.

---

## agc carousel

Run the same fixtures against multiple models in parallel, ranked by pass rate, cost, and latency.

```bash
agc carousel --models <m1,m2,...> <fixture-path> [flags]
```

```bash
agc carousel \
  --models gpt-4o,gemini-2.5-flash,claude-sonnet-4-6 \
  fixtures/my-skill/ \
  --evaluator all --judge \
  --judge-model claude-haiku-4-5-20251001
```

---

## agc ab

Run the same fixtures against two prompt variants. Prints per-case winners and a summary.

```bash
agc ab --a <prompt> --b <prompt> <fixture-path> [flags]
```

```bash
agc ab \
  --a prompts/v1.md --b prompts/v2.md \
  fixtures/my-skill/ \
  --execution-mode live --judge \
  --judge-model claude-haiku-4-5-20251001
```

---

## agc lint

Lint a fixture for style issues beyond schema validation: vague rubric descriptions, missing smoke tags, auto_check patterns that look too broad.

```bash
agc lint <fixture-path>
```

---

## agc watch

Watch a fixture directory and re-run eval on change.

```bash
agc watch <fixture-path> [eval-flags]
```

Useful during fixture authoring. Reruns on every save.

---

## agc doctor

Check your environment: API keys present, model reachable, history database accessible.

```bash
agc doctor
```

---

## agc dashboard

Serve a local web UI at `http://localhost:7421`.

```bash
agc dashboard
agc dashboard --port 8080
agc dashboard --db path/to/history.db
```

Available in the `--feature dashboard` flag.

---

## Configuration

Copy `agentcarousel.example.toml` to `agentcarousel.toml` in your working directory. Key settings:

```toml
[runner]
timeout_secs = 120    # per-case timeout (default: 120)

[generator]
model = "custom/gemma4"
endpoint = "http://localhost:11434/api/generate"         # local model on your inference server

[judge]
model = "claude-haiku-4-5-20251001"
```

All CLI flags override config file values. The config file is never committed — it's in `.gitignore` by default.
