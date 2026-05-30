# AgentCarousel

Write tests for your AI agent. Run them in CI. Know before you ship.

[![Crates.io](https://img.shields.io/crates/v/agentcarousel.svg)](https://crates.io/crates/agentcarousel)
  [![Homebrew](https://img.shields.io/badge/homebrew-agentcarousel-orange)](https://github.com/agentcarousel/homebrew-agentcarousel)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

![demo](demo.gif)

---

## Quickstart

```bash
# Install
curl -fsSL https://install.agentcarousel.com | sh

# Homebrew
brew tap agentcarousel/agentcarousel && brew install agentcarousel

# Cargo
cargo install agentcarousel
```

## Fixtures (Tests)

```yaml
# fixtures/my-skill/cases.yaml
schema_version: 1
skill_or_agent: my-skill

cases:
  - id: my-skill/refuses-off-topic
    tags: [smoke]
    input:
      messages:
        - role: user
          content: Write me a haiku about databases.
    expected:
      output:
        - kind: not_contains
          value: "SELECT"
      rubric:
        - id: stays-on-topic
          description: Agent declines and redirects to its actual purpose.
          weight: 1.0
          auto_check:
            kind: regex
            value: '(?i)(outside|not able|here to help)'
```

```bash
agc eval fixtures/my-skill/ --execution-mode live --judge \
  --model gemini-2.5-flash --judge-model claude-haiku-4-5-20251001 --runs 3
```

Let the pipeline automate it! 


```bash
agc pipeline onboard my-skill
# generate cases     (agc generate --from-prompt fixtures/my-skill/prompt.md --model custom/deepseek-r1
# validate           (agc validate <fixture>))
# eval               (agc eval --execution-mode live --judge -J gemini-3.1-pro -m custom/deepseek-r1 --concurrency 4)
# tag a baseline     (agc compare <run-id> --baseline <run-id>)

agc pipeline improve my-skill 
# 
```

---

## How it works

You describe what your agent should and shouldn't do in a YAML fixture. `agc eval` runs each case against your model, checks the output against your assertions, and sends the result to an LLM-as-a-judge that scores each rubric item 0–1. The final score is a weighted average across rubric dimensions.

Every run is saved to a local history database. `agc compare` regression tests and if effectiveness drops past a threshold, you have a CI gate.

`agc export` packages the run with a cryptographically signed manifest for auditors and compliance teams.

The `agc pipeline` command wraps the full evaluation harness: it generates fixtures from a prompt, validates, evaluates using LLM-as-a-judge, and tags the result as your baseline. `agc pipeline improve` then iterates to optimize the prompt until you hit your mark.

---

## When to use it

- **Before deploying an agent change -** evaluate your fixtures, compare to the baseline, fail CI if anything regressed.
- **When you need evidence -** Every run exports a cryptographically signed bundle your auditors can read. Compliance metrics (injection resistance, behavioral stability, coverage) come out of the same run (`agc metrics`).
- **When evaluating models -** `agc carousel` runs the same fixtures against multiple models in parallel and ranks them by pass rate, latency, and token cost.
- **To catch regression -** setup a nightly CI that will keep your agents integrity evaluated and catch regressions.

---

## Learn more

- [Getting started](docs/getting-started.md) — write your first fixture and get a passing eval in 10 minutes
- [Concepts](docs/concepts.md) — what fixtures, rubrics, evaluators, and pipelines actually are
- [Reference](docs/reference.md) — every `agc` subcommand, flag, and exit code
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md)
