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
| `--fail-on-severity <P0\|P1\|P2\|P3>` | off | Exit 1 only when a failing case's `severity` is at or above this level (P0 is most severe). Failures below it are reported on stderr but don't fail the run; failures in cases without a severity still fail it. Without the flag, any failure exits 1. Also on `agc test` |
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
| `--attack-categories <keys>` | Attack categories assigned round-robin to adversarial slots (default: all): `prompt-injection-direct`, `prompt-injection-indirect`, `system-prompt-extraction`, `jailbreak-roleplay`, `sensitive-data-disclosure`, `persuasion`, `false-scenario`. Unknown keys exit 3 |
| `--domain-context` | Extra context injected into the generation prompt |
| `--seed-cases <path>` | Existing cases to use as style reference |

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
| `--framework <id>` | Include OSCAL control coverage table for this framework |
| `--limit <n>` | Number of historical runs to include (default: 20) |
| `--json` | Machine-readable output |

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

### agc compliance report

```bash
agc compliance report [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--framework <id>` | required | Framework to report on: `nist-ai-rmf`, `nist-800-171`, `nist-800-172`, `nist-800-207`, `eu-ai-act`, `iso-42001`, `hipaa`, `fda-samd`, `aiuc-1`, or `all` |
| `--skill <name>` | all skills | Scope scoring to cases from this skill |
| `--model <model>` | configured model | Scope scoring to this generator model |
| `--all-models` | off | Include results from all models (disables model filter) |
| `--oscal` | off | Emit an OSCAL Assessment Results JSON artifact |
| `--out <dir>` | `./compliance-output` | Directory for `--oscal` output files |
| `--run-id <id>` | `latest` | Run ID to use as the assessment window anchor |
| `--import-ap <href>` | `agentcarousel://fixture-suite` | With `--oscal`: href of the OSCAL Assessment Plan (e.g. a SAP exported from your GRC platform) written to `import-ap` |
| `--json` | off | Machine-readable output |

```bash
# Terminal report for NIST AI RMF
agc compliance report --framework nist-ai-rmf --skill my-skill

# Full framework sweep as JSON
agc compliance report --framework all --json

# Export OSCAL Assessment Results artifact
agc compliance report --framework eu-ai-act --oscal --out ./evidence/
```

**OSCAL output** validates against the NIST OSCAL 1.1.2 Assessment Results JSON schema (checked in CI). Findings cover only controls exercised by at least one case: `satisfied`, or `not-satisfied` with `reason` `fail` or `partial-evidence`. Controls with no cases appear as open `risks[]` entries, not findings; procedural (non-behavioral) controls are omitted. `reviewed-controls` lists exactly the assessed control IDs.

**Satisfaction threshold:** A control reaches `Satisfied` when pass rate ≥ 0.80 across a minimum of three tagged cases. Controls with fewer than three cases receive `PartialEvidence` regardless of pass rate.

**Frameworks:** `nist-ai-rmf` · `nist-800-171` · `nist-800-172` · `nist-800-207` · `eu-ai-act` · `iso-42001` · `hipaa` · `fda-samd` · [`aiuc-1`](frameworks/aiuc-1.md) (crosswalk of IDs, our own labels and links; `agc export` writes `compliance_aiuc-1.md` only when cases carry `aiuc-1:` tags)

**Procedural controls:** controls a catalog marks `evidence-type: procedural`, and crosswalk controls that are policy or process requirements, are reported as `Procedural`. Tagged cases never count as evidence for them.

---

### agc compliance gaps

List controls that are `NotSatisfied` or `PartialEvidence`, with remediation suggestions.

```bash
agc compliance gaps [flags]
```

| Flag | Description |
|---|---|
| `--framework <id>` | Framework to inspect (same set as `report`) |
| `--skill <name>` | Scope to a skill |
| `--model <model>` | Scope to a model |
| `--all-models` | Include all models |
| `--json` | Machine-readable output |

```bash
agc compliance gaps --framework hipaa --skill ambient-scribe
```

---

### agc compliance generate-cases

Generate fixture cases pre-tagged with OSCAL control IDs to close compliance gaps.

```bash
agc compliance generate-cases [flags]
```

| Flag | Default | Description |
|---|---|---|
| `--skill <name>` | required | Skill to generate cases for |
| `--framework <id>` | required | Source framework for control tags |
| `--tag <control-id>` | required | One or more control IDs to cover (repeatable) |
| `--count <n>` | `3` | Cases to generate per tag |
| `--model <model>` | config | Generator model |
| `--out <path>` | stdout | Output YAML file (appends to existing fixture) |
| `--json` | off | Emit structured JSON envelope |

```bash
# Generate 3 cases for NIST AI RMF GOVERN 1.1 and MAP 1.1
agc compliance generate-cases \
  --skill my-skill \
  --framework nist-ai-rmf \
  --tag nist-ai-rmf:nist-ai-rmf-govern-1.1 \
  --tag nist-ai-rmf:nist-ai-rmf-map-1.1 \
  --count 3 \
  --out fixtures/my-skill/cases.yaml
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

Package a run as an evidence bundle.

```bash
agc export <run-id>
agc export -l 1                              # latest run
agc export <run-id> --sign-key agc.key       # also sign MANIFEST.json with minisign
```

The bundle includes `run.json`, `report.md`, `metrics.json`, compliance reports, `assessment-results.oscal.json`, `evidence.json` and `attestation.intoto.json` ([AgentCarousel Specification](https://github.com/agentcarousel/assurance-spec) draft 0.1), and `MANIFEST.json` with a SHA-256 hash of every file.

`evidence.json` and the in-toto statement are built from the run record alone: each case's status (including `not_tested`), where its trace came from, how each score was decided (`deterministic`, `golden`, `process`, `llm_judge`), and machine-readable limitations. Runs recorded by earlier versions have no recorded subject, so they export without these two files. `metrics.json` and the compliance reports summarize recent history and are labelled as derived.

Assertion outcomes in `evidence.json` are conservative. A rubric item counts as `pass` only at a score of 0.9 or higher (the ✅ threshold in `report.md`), while case status follows the effectiveness threshold (default 0.7). A case can therefore be `passed` while some of its judge-scored assertions show `fail`; every assertion also carries its raw `score`.

By default, system messages in `run.json`/`report.md` are replaced by `[system prompt withheld; sha256:…]`, prompt-audit rewrites are omitted, and `prompt.md` is not added. `--include-prompt` discloses them and records the choice in `evidence.json`.

| Flag | Description |
|---|---|
| `-l, --last N` | Export the N most recent runs (max 50) |
| `-o, --out` | Output path for a single run |
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

Recomputes every hash in `MANIFEST.json` and fails on any mismatched, missing, or unlisted file. With `--minisign-pubkey`, a valid `MANIFEST.json.minisig` is also required. Accepts a `.tar.gz` or an extracted bundle directory.

When the bundle carries `evidence.json` and `attestation.intoto.json`, verify also checks them against the AgentCarousel Specification v0.1: JSON Schemas (resolved locally, no network), semantic rules such as "summary counts equal per-case statuses" and "synthesized tool traces carry a `tool_calls_not_observed` limitation", and cross-document digests. The JSON output reports `conformance`: `conformant`, or `not_present` for older bundles. Bundles exported by agc 0.9.0 carry the spec's retired provisional identifiers (`https://assurance-spec.invalid/`) and fail as nonconformant; run `agc export <run-id>` again with 0.9.1 or later to re-create them.

| Result | Meaning |
|---|---|
| `verified` (exit 0) | Hashes match and the signature is valid for the given key |
| `integrity_only` (exit 0) | Hashes match; no public key given, so authenticity was not checked |
| `nonconformant` (exit 1) | Hashes (and signature) match, but `evidence.json` or the statement violates a spec rule; the rule ids are listed |
| failure (exit 1) | Hash mismatch, missing/unlisted file, missing or invalid signature |

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
| `--include-prompt` | Upload `prompt.md` and export evidence with system prompts included |

Requires `AGENTCAROUSEL_API_TOKEN` in the environment. By default only the prompt's SHA-256 is reported (`prompt_disclosure: digest_only`); nothing containing prompt text is uploaded. An evidence tarball found on disk is reused as-is, with whatever disclosure it was exported with.

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

Included in release binaries. When building from source, enable it with `cargo install agentcarousel --features dashboard`.

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
