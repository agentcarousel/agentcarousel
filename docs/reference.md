# CLI Reference

All `agc` subcommands and their main flags, in the order `agc --help` lists them. `agc <command> --help` is always authoritative and lists every flag, examples and exit codes.

`agc` and `agentcarousel` are the same binary. Homebrew and `cargo install` provide both names. The curl installer installs `agentcarousel` and offers to add `agc` as a shell alias.

**Global flags** (every command): `--json` (structured JSON output), `-q, --quiet`, `-v, --verbose` (repeatable), `--no-color`. Some commands also switch to JSON when stdout is not a terminal; pass `--json` explicitly in scripts.

**Exit codes** (shared conventions; some commands add their own, listed under each command):

| Code | Meaning |
|---|---|
| 0 | Success / all tests passed |
| 1 | Failure — tests failed or regression detected |
| 2 | Invalid input or arguments |
| 3 | Config error |
| 4 | Runtime error (IO, network, database) |
| 5 | Not found |

---

## Fixture work

## agc validate

Check fixtures for schema errors, ID format, weight sums, and path safety. Scans `.` when no path is given.

```bash
agc validate [paths] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `-s, --schema <file>` | config (`validate.schema_dir`) | JSON Schema file to validate against |
| `-x, --strict` | off | Fail on warnings too |
| `--config <file>` | `./agentcarousel.toml` | Config file path |

```bash
agc validate fixtures/my-skill/
agc validate fixtures/                  # all fixtures
agc validate fixtures/my-skill/ --json  # machine-readable output
agc validate --strict                   # treat warnings as errors
```

Exits `0` when valid, `2` on validation errors, `4` on a runtime error, `5` when the path is not found.

`agc validate` currently reads the schema from `./schemas/skill-definition.schema.json` and fails with `schema error: No such file or directory` in a directory without it. Fetch it once with `curl -fsSL --create-dirs -o schemas/skill-definition.schema.json https://agentcarousel.com/schemas/fixture/v1.json`, or pass `--schema`.

---

## agc test

Run fixtures with mock responses. No API keys needed.

```bash
agc test [paths] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `-o, --offline <true\|false>` | config | Use mock responses instead of calling an API |
| `-O, --no-offline-default` | off | Ignore the config-level offline default |
| `-m, --mock-dir <dir>` | config | Directory to load or write mock responses |
| `-f, --filter <glob>` | — | Glob matched against full case ids (`skill/case-id`) |
| `-g, --filter-tags <tags>` | — | Comma-separated tags; keep cases having any of them |
| `-c, --concurrency <n>` | config | Maximum cases in parallel |
| `-t, --timeout <secs>` | config | Per-case timeout |
| `-F, --fail-fast` | off | Stop after the first failing case |
| `--fail-on-severity <P0\|P1\|P2\|P3>` | off | Exit 1 only for failures at or above this severity (see `agc eval`) |
| `--run-id <id>` | generated | Run id stored in the history database |
| `--generator-endpoint <url>` | — | Base URL for a custom agent endpoint (generator model `custom`) |

```bash
agc test fixtures/my-skill/cases.yaml
agc test fixtures/ --filter-tags smoke
agc test fixtures/ --concurrency 4
```

Exits `0` when all cases pass, `1` when any fail, `4` on a runtime error, `5` when the path is not found.

---

## agc eval

Run fixture cases against a model and score the results.

```bash
agc eval [paths] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `-x, --execution-mode <mode>` | `mock` | `mock` (offline), `live` (call the generator API), or `batch` (submit to the provider's async batch API, about 50% cheaper; see `agc batch`) |
| `-m, --model <model>` | config | Generator model (e.g. `gemini-2.5-flash`, `ollama/qwen3:8b`) |
| `-e, --evaluator <name>` | `rules` | `rules`, `golden`, `process`, `judge`, or `all` (`all` uses each case's `evaluator_config`) |
| `-j, --judge` | off | Enable LLM-as-judge scoring. With the default `rules` evaluator, `--judge` switches to the judge evaluator |
| `-J, --judge-model <model>` | config | Judge model |
| `--judge-endpoint <url>` | — | Endpoint for a `custom` or `ollama/<name>` judge |
| `--generator-endpoint <url>` | — | Endpoint for a `custom` or `ollama/<name>` generator |
| `-n, --runs <n>` | `1` | Runs per case (use more than 1 to detect flakiness) |
| `-F, --filter <glob>` | — | Glob matched against full case ids (`skill/case-id`) |
| `--filter-tags <tags>` | — | Comma-separated tags; keep cases having any of them (e.g. `smoke,certification`) |
| `-c, --concurrency <n>` | config | Maximum cases in parallel |
| `-t, --timeout <secs>` | config | Per-case timeout |
| `-s, --seed <n>` | `0` | Random seed for mock generation |
| `-D, --measure-discrimination` | off | Run three passes (current, blank and degraded prompt) and score how well each case discriminates. Needs `live` or `batch` |
| `-M, --disable-max-tokens` | off | Omit `max_tokens` from generator requests (not supported for Anthropic models) |
| `--fail-on-severity <P0\|P1\|P2\|P3>` | off | Exit 1 only when a failing case's `severity` is at or above this level (P0 is most severe). Failures below it are reported on stderr but don't fail the run; failures in cases without a severity still fail it. Without the flag, any failure exits 1. Also on `agc test` |
| `--run-id <id>` | generated | Run id stored in the history database |
| `--config <file>` | `./agentcarousel.toml` | Config file path |

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

Exits `0` when all cases pass, `1` when any fail or score below threshold, `4` on a runtime error, `5` when the path is not found.

---

## agc carousel

Run the same fixtures against several models and rank them by pass rate, cost, and latency.

```bash
agc carousel --models <m1,m2,...> [paths] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--models <list>` | required | Comma-separated generator models. OpenRouter models: prefix with `openrouter/` |
| `-e, --evaluator <name>` | `rules` | `rules`, `golden`, `process`, `judge`, or `all` |
| `-j, --judge` | off | Enable the LLM judge for judge-scored cases |
| `-J, --judge-model <model>` | config | Judge model |
| `-n, --runs <n>` | `1` | Runs per case per model |
| `-f, --filter <glob>` / `-g, --filter-tags <tags>` | — | Case filters, as for `agc test` |
| `-c, --concurrency <n>` | `1` for live | Per-model case concurrency |
| `-t, --timeout <secs>` | config | Per-case timeout |

```bash
agc carousel \
  --models gpt-4o,gemini-2.5-flash,claude-sonnet-4-6 \
  fixtures/my-skill/ \
  --evaluator judge --judge \
  --judge-model claude-haiku-4-5-20251001
```

Exits `0` when every model passes, `1` when any model has failures, `4` on a runtime error.

---

## agc ab

Run the same fixtures against two system prompts and compare them head-to-head.

```bash
agc ab --a <prompt-file> --b <prompt-file> [paths] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--a <file>` / `--b <file>` | required | System prompt files for variants A and B |
| `-x, --execution-mode <mode>` | `mock` | `mock` or `live` |
| `-m, --model <model>` | config | Generator model |
| `-e, --evaluator <name>` | `rules` | `rules`, `golden`, `process`, `judge`, or `all` |
| `-j, --judge` / `-J, --judge-model <model>` | off / config | LLM judge scoring |
| `--generator-endpoint <url>` / `--judge-endpoint <url>` | — | Endpoints for `custom` or `ollama/<name>` models |
| `-n, --runs <n>` | `1` | Runs per case per variant |
| `--threshold <float>` | `0.05` | Effectiveness delta needed to declare a winner |
| `-f, --filter <glob>` / `--filter-tags <tags>` | — | Case filters |
| `-c, --concurrency <n>` / `-t, --timeout <secs>` | — | Concurrency per variant (1 live, 4 mock) and per-case timeout |

```bash
agc ab \
  --a prompts/v1.md --b prompts/v2.md \
  fixtures/my-skill/ \
  --execution-mode live --judge \
  --judge-model claude-haiku-4-5-20251001
```

Exits `0` when B is equivalent to or better than A, `1` when B regresses, `4` on a runtime error.

---

## agc generate

Generate fixture cases from a skill description or an existing prompt file.

```bash
agc generate [flags]
```

| Flag | Description |
|---|---|
| `--skill <name>` | Skill name; writes `fixtures/<name>/cases.yaml` |
| `--description <text>` | Description of the skill, used to build the generation prompt |
| `--from-prompt <path-or-text>` | System prompt to use as the skill description (file path or inline text; with inline text, also pass `--skill`) |
| `--extend <dir>` | Add cases to an existing fixture directory (deduplicates against existing ids) |
| `-n, --count <n>` | Number of cases to generate (default: 5) |
| `--model <model>` | Generator model. Omit it (with `--skill`) to scaffold `fixtures/<skill>/` (`cases.yaml`, a `prompt.md` stub and `golden/`) without calling an LLM |
| `--generator-endpoint <url>` | Endpoint for a `custom/*` or `ollama/*` model |
| `--distribution <spec>` | Category mix that sums to `--count`, e.g. `happy:2,edge:3,failure:3,adversarial:2` |
| `--difficulty <level>` | `easy`, `medium`, or `hard` (hard skews toward adversarial and boundary cases) |
| `--attack-categories <keys>` | Attack categories assigned round-robin to adversarial slots (default: all): `prompt-injection-direct`, `prompt-injection-indirect`, `system-prompt-extraction`, `jailbreak-roleplay`, `sensitive-data-disclosure`, `persuasion`, `false-scenario`. Unknown keys exit 3 |
| `--domain-context <path>` | Markdown or text file prepended to every per-case prompt (e.g. HIPAA rules, an API spec) |
| `--seed-cases <path>` | Cases YAML to generate adversarial variants of |
| `--deduplicate` | Drop generated cases too similar to existing ones (default: warn only) |
| `--batch` | Generate as single-case Anthropic batch calls (50% cheaper; needs a Claude model and `ANTHROPIC_API_KEY`) |
| `--dry-run` | Print the generated YAML instead of writing it |

**Attack-category tags.** `agc generate` adds these tags to each adversarial case itself, so they don't depend on the model. `security`, `ai-safety` and `prompt-injection` also count toward adversarial coverage in `agc metrics`.

| Key | Tags |
|---|---|
| `prompt-injection-direct` | `security`, `prompt-injection`, `owasp-llm:LLM01`, `mitre-atlas:AML.T0051.000`, `aiuc-1-attack:direct-injection` |
| `prompt-injection-indirect` | `security`, `prompt-injection`, `owasp-llm:LLM01`, `mitre-atlas:AML.T0051.001`, `aiuc-1-attack:indirect-injection` |
| `system-prompt-extraction` | `security`, `owasp-llm:LLM07`, `mitre-atlas:AML.T0056` |
| `jailbreak-roleplay` | `security`, `ai-safety`, `mitre-atlas:AML.T0054` |
| `sensitive-data-disclosure` | `security`, `owasp-llm:LLM02`, `mitre-atlas:AML.T0057` |
| `persuasion` | `ai-safety`, `aiuc-1-attack:persuasion` |
| `false-scenario` | `ai-safety`, `aiuc-1-attack:false-scenario` |

Pinned: OWASP Top 10 for LLM Applications 2025; MITRE ATLAS 5.6.0; AIUC-1 attack category names (2026-Q4, names only).

```bash
# Scaffold a new fixture directory (no LLM call)
agc generate --skill my-skill

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

Exits `0` when cases are written (or shown with `--dry-run`), `2` when the output still fails validation after a retry, `3` on a config error, `4` on a runtime error.

---

## agc optimize

Iteratively rewrite a system prompt until the pass rate reaches a target, within a budget.

```bash
agc optimize [path] [flags]
agc optimize --skill <name> [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--skill <name>` | — | Shorthand for `fixtures/<name>/` (instead of a path) |
| `--target-score <float>` | `0.9` | Stop when the pass rate reaches this value |
| `--iterations <n>` (alias `--max-iter`) | `5` | Maximum optimization iterations |
| `--budget <usd>` | `10` | Maximum USD across all eval and LLM calls |
| `--model <model>` | config | Generator model for eval runs |
| `--judge-model <model>` | config | Judge model for scoring and analysis |
| `--generator-endpoint <url>` / `--judge-endpoint <url>` | — | Endpoints for `custom` or `ollama/<name>` models |
| `--output <file>` | `optimize-report-<run_id>.json` | Where to write the optimization report |
| `--dry-run` | off | Show what would happen without LLM calls |

```bash
agc optimize fixtures/my-skill/
agc optimize fixtures/my-skill/ --target-score 0.95 --max-iter 5 --budget 15
```

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
| `--target-model <model>` | config | Generator (target) model |
| `--target-endpoint <url>` | — | Endpoint for a local or custom generator |
| `--judge-model <model>` | config | Judge model |
| `--judge-endpoint <url>` | — | Endpoint for a local or custom judge (inherited from `--target-endpoint` for Ollama models) |
| `--target-score <float>` | `0.85` | Minimum pass rate to consider the skill stable |
| `--dry-run` | off | Print steps without calling any APIs |

**improve flags:**

| Flag | Default | Description |
|---|---|---|
| `--target-score <float>` | `0.85` | Score to reach before stopping (overrides the local profile) |
| `--max-rounds <n>` | `5` | Maximum improvement rounds |
| `--target-endpoint <url>` | — | Endpoint for the generator |
| `--judge-endpoint <url>` | — | Endpoint for the judge |
| `--budget <usd>` | `0` (unlimited) | USD spend cap (useful for cloud models) |

```bash
# Full onboard with cloud models
agc pipeline onboard my-skill \
  --target-model gemini-2.5-flash \
  --judge-model claude-haiku-4-5-20251001 \
  --target-score 0.85

# Onboard with local Ollama (judge endpoint inherited)
agc pipeline onboard my-skill \
  --target-model ollama/qwen3:8b \
  --target-endpoint http://192.168.1.10:11434/api/generate

# Improve until 90%
agc pipeline improve my-skill --target-score 0.90 --max-rounds 8
```

---

## agc candidates

List pipeline candidate skills with their scores, metrics, and status.

```bash
agc candidates [--status onboarding|improving|stable|graduated]
```

---

## agc batch

Check on or collect an async batch eval. Submit one with `agc eval --execution-mode batch`, which prints the batch id.

```bash
agc eval fixtures/my-skill/ --execution-mode batch --model claude-haiku-4-5-20251001
agc batch status <batch-id>   # current status
agc batch fetch <batch-id>    # wait for completion, then run the eval pipeline on the results
```

`agc batch` is not listed in `agc --help`.

---

## Results

## agc report

Inspect run history.

```bash
agc report list [--limit <n>]     # default 20
agc report show <run-id>
```

```bash
agc report list
agc report show VCSS1FDSMJ
agc report show VCSS1FDSMJ --verbose   # full per-case output
```

---

## agc audit

Prompt-audit a saved run against its system prompt, or apply the audit's suggestions.

```bash
agc audit run <run-id|run.json> [flags]
agc audit suggest <run-id|run.json> [--apply] [--prompt <path>]
```

| Flag | Command | Description |
|---|---|---|
| `--prompt <path>` | both | `prompt.md` to audit against or write into (default: `fixtures/<skill>/prompt.md`) |
| `--model <model>` | run | Judge model (overrides config) |
| `--judge-endpoint <url>` | run | Endpoint for a `custom/*` or `ollama/*` judge |
| `--no-save` | run | Don't save the audit result to the history database |
| `--apply` | suggest | Append the suggestions to `prompt.md` as a commented block |

---

## agc metrics

Compliance-ready metrics report across four dimensions.

```bash
agc metrics [flags]
```

| Flag | Description |
|---|---|
| `--skill <name>` | Scope to a skill (discovers `fixtures/<name>/`) |
| `--fixture <path>` | Infer the skill from a fixture file or directory |
| `--run-id <id>` | Analyze one run instead of the history window |
| `--limit <n>` | Number of historical runs to analyze (default: 20) |
| `--domain healthcare` | Add four healthcare checks mapped to HIPAA and FDA SaMD |
| `--framework <id>` | Include a per-control OSCAL coverage table for this framework |

```bash
agc metrics --skill my-skill
agc metrics --skill my-skill --framework nist-ai-rmf --json > metrics.json
```

**Metrics:**

| Metric | What it measures |
|---|---|
| Prompt Injection Resistance | Pass rate on adversarial injection cases (0–100) |
| Behavioral Stability | Effectiveness score drift over run history |
| Test Coverage Completeness | Fraction of the 7-category risk taxonomy covered |
| Score Accuracy (Calibration) | How well judge scores predict actual pass/fail |

---

## agc compliance

Map fixture evidence to OSCAL compliance frameworks. Requires cases tagged `<framework>:<control-id>` using the catalog's control IDs (e.g. `nist-ai-rmf:nist-ai-rmf-govern-1.1`, `aiuc-1:D001`).

Scoring uses the last `--limit` runs in history. Unless `--all-models` is passed, only runs from the configured generator model are scored.

### agc compliance report

```bash
agc compliance report [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--framework <id>` | `nist-ai-rmf` | `nist-ai-rmf`, `nist-800-171`, `nist-800-172`, `nist-800-207`, `eu-ai-act`, `iso-42001`, `hipaa`, `fda-samd`, `aiuc-1`, or `all` |
| `--skill <name>` | all skills | Scope scoring to cases from this skill |
| `--limit <n>` | `20` | Number of historical runs to analyze |
| `--all-models` | off | Include runs from every model in history |
| `--oscal` | off | Emit OSCAL Assessment Results JSON instead of Markdown (not with `--framework all`) |
| `--out <path>` | stdout | Write to this file. With `--framework all`, a directory with one file per framework |
| `--run-id <id>` | `latest` | Run id used as the evidence anchor in OSCAL output |
| `--import-ap <href>` | `agentcarousel://fixture-suite` | With `--oscal`: href of the OSCAL Assessment Plan (e.g. a SAP exported from your GRC platform) written to `import-ap` |

```bash
# Markdown report for NIST AI RMF
agc compliance report --framework nist-ai-rmf --skill my-skill

# Every framework, one file each
agc compliance report --framework all --out ./compliance/

# OSCAL Assessment Results artifact
agc compliance report --framework eu-ai-act --oscal --out ./evidence/eu-ai-act.oscal.json
```

**OSCAL output** validates against the NIST OSCAL 1.1.2 Assessment Results JSON schema (checked in CI). Findings cover only controls exercised by at least one case: `satisfied`, or `not-satisfied` with `reason` `fail` or `partial-evidence`. Controls with no cases appear as open `risks[]` entries, not findings; procedural (non-behavioral) controls are omitted. `reviewed-controls` lists exactly the assessed control IDs. This output does not yet follow the AgentCarousel Specification's OSCAL mapping ([spec/08 §8.3](https://github.com/agentcarousel/assurance-spec/blob/main/spec/08-interop.md)); see the [charter status table](https://github.com/agentcarousel/assurance-spec/blob/main/CHARTER.md#6-reference-implementation-status).

**Satisfaction threshold:** A control reaches `Satisfied` when pass rate ≥ 0.80 across a minimum of three tagged cases. Controls with fewer than three cases receive `PartialEvidence` regardless of pass rate.

**Frameworks:** `nist-ai-rmf` · `nist-800-171` · `nist-800-172` · `nist-800-207` · `eu-ai-act` · `iso-42001` · `hipaa` · `fda-samd` · [`aiuc-1`](frameworks/aiuc-1.md) (crosswalk of IDs, our own labels and links; `agc export` writes `compliance_aiuc-1.md` only when cases carry `aiuc-1:` tags)

**Procedural controls:** controls a catalog marks `evidence-type: procedural`, and crosswalk controls that are policy or process requirements, are reported as `Procedural`. Tagged cases never count as evidence for them.

---

### agc compliance gaps

List controls with no fixture coverage, with remediation advisories.

```bash
agc compliance gaps [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--framework <id>` | `nist-ai-rmf` | Framework to inspect (same set as `report`) |
| `--skill <name>` | all skills | Scope to a skill |
| `--limit <n>` | `20` | Number of historical runs to analyze |
| `--all-models` | off | Include runs from every model in history |

```bash
agc compliance gaps --framework hipaa --skill ambient-scribe
```

---

### agc compliance generate

Generate fixture cases pre-tagged with OSCAL control IDs to close compliance gaps.

```bash
agc compliance generate --skill <name> --tag <control-tag> [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--skill <name>` | required | Skill to generate cases for |
| `--tag <control-tag>` | required | Control tag to cover, in `<framework>:<control-id>` form (repeatable) |
| `--count <n>` | `3` | Cases to generate per tag |
| `--model <model>` | config | Generator model |
| `--generator-endpoint <url>` | — | Endpoint for a `custom/*` or `ollama/*` model |
| `--out <path>` | `fixtures/<skill>/cases.yaml` | Write to this path instead |
| `--dry-run` | off | Print the YAML instead of writing it |

```bash
# Generate 3 cases each for NIST AI RMF GOVERN 1.1 and MAP 1.1
agc compliance generate \
  --skill my-skill \
  --tag nist-ai-rmf:nist-ai-rmf-govern-1.1 \
  --tag nist-ai-rmf:nist-ai-rmf-map-1.1 \
  --count 3
```

---

## agc scope

Create or check a fixture's agent-under-test scope profile. See [scope-profile.md](scope-profile.md).

```bash
agc scope init --skill my-agent     # writes fixtures/my-agent/scope.yaml
agc scope check --skill my-agent    # validates it; planned vs actual coverage per risk category
agc scope check fixtures/my-agent   # same, by directory
```

Exit codes: `0` valid, `2` validation errors, `5` no profile or directory found.

---

## agc compare

Diff two runs. Exits `1` if effectiveness drops past the threshold.

```bash
agc compare [run-id] [flags]           # omit run-id (or pass -l) for the latest run
agc compare tag <run-id> --name <tag>  # tag a run as a named baseline
```

| Flag | Default | Description |
|---|---|---|
| `-l, --latest` | — | Use the latest run as the current run |
| `--baseline <name-or-id>` | — | Named baseline or run id to compare against |
| `--threshold <float>` | `0.05` | Max allowed drop in effectiveness (0–1) |
| `--significance <alpha>` | `0.05` | Significance level for the Mann-Whitney U test. With five or more matched cases, exit 1 only when the drop exceeds `--threshold` and p < alpha |
| `--registry` | off | Fetch the baseline from a registry instead of local history (see [Bundles and registry](#bundles-and-registry)) |
| `--local-fallback` | on | With `--registry`, fall back to local history if the fetch fails |
| `--registry-url <url>` / `--token <token>` | env | Registry URL (else `AGENTCAROUSEL_REGISTRY_URL`) and API token (else `AGENTCAROUSEL_API_TOKEN`) |

```bash
# Tag a baseline
agc compare tag VCSS1FDSMJ --name prod-v1

# CI gate: fail if effectiveness drops >5%
agc compare -l --baseline prod-v1 --threshold 0.05
```

Exits `0` when there is no regression, `1` when the regression exceeds the threshold, `4` on a runtime error, `5` when a run is not found.

---

## agc export

Package a run as an evidence bundle.

```bash
agc export <run-id>
agc export --last 1                          # latest run
agc export <run-id> --sign-key agc.key       # also sign MANIFEST.json with minisign
```

The bundle includes `run.json`, `report.md`, `metrics.json`, compliance reports, `assessment-results.oscal.json`, `evidence.json` and `attestation.intoto.json` ([AgentCarousel Specification](https://github.com/agentcarousel/assurance-spec) draft 0.1), and `MANIFEST.json` with a SHA-256 hash of every file. A valid [scope profile](scope-profile.md) is added as `scope.json`.

`evidence.json` and the in-toto statement are built from the run record alone: each case's status (including `not_tested`), where its trace came from, how each score was decided (`deterministic`, `golden`, `process`, `llm_judge`), and machine-readable limitations. Runs recorded by earlier versions have no recorded subject, so they export without these two files. `metrics.json` and the compliance reports summarize recent history and are labelled as derived.

Assertion outcomes in `evidence.json` are conservative. A rubric item counts as `pass` only at a score of 0.9 or higher (the ✅ threshold in `report.md`), while case status follows the effectiveness threshold (default 0.7). A case can therefore be `passed` while some of its judge-scored assertions show `fail`; every assertion also carries its raw `score`.

By default, system messages in `run.json`/`report.md` are replaced by `[system prompt withheld; sha256:…]`, prompt-audit rewrites are omitted, and `prompt.md` is not added. `--include-prompt` discloses them and records the choice in `evidence.json`.

| Flag | Description |
|---|---|
| `-l, --last N` | Export the N most recent runs (max 50) |
| `-o, --out` | Output path for a single run (default: `./agentcarousel-evidence-<run_id>.tar.gz`) |
| `-d, --out-dir` | Output directory with `--last` |
| `--sign-key` | minisign secret key; writes `MANIFEST.json.minisig` |
| `--import-ap` | OSCAL Assessment Plan href for `assessment-results.oscal.json` |
| `--minisign-bin` | minisign binary (default: `minisign`) |
| `--include-prompt` | Keep system prompts verbatim and add `prompt.md` (if unchanged since the run) |

An unsigned manifest proves the files are consistent with each other, not that they are authentic: anyone who edits a file can regenerate the hashes. Sign the manifest when the bundle leaves your hands. For CI, create a passwordless key with `minisign -G -W` and keep the secret key in your CI secret store.

---

## agc verify

Check an evidence bundle produced by `agc export`.

```bash
agc verify agentcarousel-evidence-<run-id>.tar.gz
agc verify agentcarousel-evidence-<run-id>.tar.gz --minisign-pubkey agc.pub
```

| Flag | Description |
|---|---|
| `--minisign-pubkey <path>` | minisign public key; a valid `MANIFEST.json.minisig` is then required |
| `--minisign-bin <path>` | minisign binary (default: `minisign`) |

Recomputes every hash in `MANIFEST.json` and fails on any mismatched, missing, or unlisted file. With `--minisign-pubkey`, a valid `MANIFEST.json.minisig` is also required. Accepts a `.tar.gz` or an extracted bundle directory.

When the bundle carries `evidence.json` and `attestation.intoto.json`, verify also checks them against the AgentCarousel Specification v0.1: JSON Schemas (resolved locally, no network), semantic rules such as "summary counts equal per-case statuses" and "synthesized tool traces carry a `tool_calls_not_observed` limitation", and cross-document digests. The JSON output reports `conformance`: `conformant`, or `not_present` for older bundles. Bundles exported by agc 0.9.0 carry the spec's retired provisional identifiers (`https://assurance-spec.invalid/`) and fail as nonconformant; run `agc export <run-id>` again with 0.9.1 or later to re-create them. The spec's [conformance vectors](https://github.com/agentcarousel/assurance-spec/tree/main/conformance) check this behavior.

| Result | Meaning |
|---|---|
| `verified` (exit 0) | Hashes match and the signature is valid for the given key |
| `integrity_only` (exit 0) | Hashes match; no public key given, so authenticity was not checked |
| `nonconformant` (exit 1) | Hashes (and signature) match, but `evidence.json` or the statement violates a spec rule; the rule ids are listed |
| failure (exit 1) | Hash mismatch, missing/unlisted file, missing or invalid signature |

---

## Bundles and registry

`agc bundle pull`, `agc publish`, `agc trust-check`, `agc compare --registry` and `agc promote --registry` talk to an AgentCarousel registry. AgentCarousel does not currently run a public registry for these commands, so they need the URL of a registry you operate:

- `bundle pull`, `publish` and `trust-check`: `--url` (alias `--registry-url`), else `msp.registry_endpoint` in `agentcarousel.toml`, else the `REGISTRY_API_BASE_URL` or `REGISTRY_URL` environment variable.
- `compare --registry` and `compare tag --registry`: `--registry-url`, else `AGENTCAROUSEL_REGISTRY_URL`.
- `promote --registry`: `--registry-url`, else `https://api.agentcarousel.com`, which is not available at the moment.

Write operations need `AGENTCAROUSEL_API_TOKEN` (or `--token`).

## agc bundle

Pack, verify, or pull fixture bundles.

```bash
agc bundle pack [dir] [-o <out.tar.gz>]       # dir holds bundle.manifest.json (default: .)
agc bundle verify [path]                      # check a bundle directory or .tar.gz against its manifest
agc bundle pull <registry-bundle-id> [flags]  # download a bundle from a registry
```

| Flag | Command | Description |
|---|---|---|
| `-o, --out <path>` | pack, pull | Output archive (pack) or directory (pull; default `pulled-bundles/<id>`) |
| `--url <url>` | pull | Registry URL (see above) |
| `--verify` | pull | Run `bundle verify` on the download |

Manifest paths must be plain relative paths inside the bundle: `..`, absolute paths and backslashes are rejected, and links in archives are refused.

---

## agc publish

Publish a fixture bundle and its evidence to a registry.

```bash
agc publish [bundle-dir-or-manifest] [flags]
```

| Flag | Description |
|---|---|
| `--url <url>` | Registry URL (see [Bundles and registry](#bundles-and-registry)) |
| `-r, --run-id <id>` | Run to submit (default: latest run matching the manifest's `bundle_id` and `bundle_version`) |
| `-a, --all-runs` | Submit every matching run |
| `-l, --limit <n>` | Cap the runs processed with `--all-runs` (newest first) |
| `-e, --evidence <path>` | Explicit evidence tarball |
| `-n, --dry-run` | Resolve and print values without writing to the registry |
| `--include-prompt` | Upload `prompt.md` and export evidence with system prompts included |

Requires `AGENTCAROUSEL_API_TOKEN` in the environment. By default only the prompt's SHA-256 is reported (`prompt_disclosure: digest_only`); nothing containing prompt text is uploaded. An evidence tarball found on disk is reused as-is, with whatever disclosure it was exported with.

---

## agc promote

Promote golden output from a saved run.

```bash
agc promote <run-id> [fixture-paths] [flags]
```

Writes the final output of each eligible case to its configured `golden_path` and creates a `.meta.json` sidecar with the run id and effectiveness score. A quality gate applies: cases below the promote threshold, or with a critical rubric item below it, are blocked.

| Flag | Description |
|---|---|
| `--bootstrap-golden` | Write run outputs to golden files without the quality gate, creating `golden/` and adding a golden evaluator to cases that lack one |
| `--registry` | Also submit the run evidence to a registry (see [Bundles and registry](#bundles-and-registry)) |
| `--registry-url <url>` / `--token <token>` | Registry URL and API token |

Exits `0` when all eligible cases are promoted, `1` when the quality gate blocks any, `4` when the run is not found or the registry call fails.

---

## agc trust-check

Check a bundle's trust state in a registry, and optionally verify a local attestation.

```bash
agc trust-check <bundle-id>[@version] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--url <url>` | config/env | Registry URL (see [Bundles and registry](#bundles-and-registry)) |
| `--min-trust <state>` | `trusted` | `experimental`, `carousel-candidate`, `stable`, or `trusted` |
| `--attestation <file>` | — | Local attestation JSON to verify with minisign |
| `--minisign-pubkey <path-or-url>` | — | Public key for `--attestation` (`http://` URLs are refused) |

Registry trust states are an AgentCarousel registry listing policy. They are not part of the AgentCarousel Specification.

---

## Tooling

## agc dashboard

Serve a local web UI at `http://localhost:7421`.

```bash
agc dashboard
agc dashboard --port 8080
agc dashboard --db path/to/history.db
```

Included in release binaries. When building from source, enable it with `cargo install agentcarousel --features dashboard`.

---

## agc watch

Re-run cases whenever a fixture file is saved.

```bash
agc watch [paths] [--eval] [--filter <glob>] [--filter-tags <tags>] [-c <n>] [-t <secs>]
```

`--eval` uses the eval pipeline instead of `agc test`. Not included in release binaries: build from source with `cargo install agentcarousel --features watch`.

---

## agc completions

Print a shell completion script.

```bash
agc completions bash >> ~/.bashrc
agc completions zsh > ~/.zfunc/_agc
agc completions fish > ~/.config/fish/completions/agc.fish
```

---

## agc update

Check for and install a newer release.

```bash
agc update           # upgrade in place
agc update --check   # print whether an update is available
agc update --yes     # skip the confirmation prompt (CI)
```

The download is checked against the release's `SHA256SUMS`; without a matching checksum, nothing is installed. `--feature dashboard` is accepted for 0.9.x scripts and is no longer needed.

---

## agc doctor

Check your environment: API keys present, model reachable, history database accessible.

```bash
agc doctor
agc doctor --json
```

---

## Configuration

Copy `agentcarousel.example.toml` to `agentcarousel.toml` in your working directory. Key settings:

```toml
[runner]
timeout_secs = 120    # per-case timeout

[generator]
model = "custom/gemma4"
endpoint = "http://localhost:11434/api/generate"         # local model on your inference server

[judge]
model = "claude-haiku-4-5-20251001"
```

All CLI flags override config file values. `agentcarousel.toml` holds local settings such as endpoints; keep it out of version control.
