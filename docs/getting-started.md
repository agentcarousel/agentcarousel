# Getting started

## Install

Use the installer (Linux and macOS):

```bash
curl -fsSL "http://install.agentcarousel.com" | sh
```

Or pin a **specific release**:

```bash
curl -fsSL "https://raw.githubusercontent.com/agentcarousel/agentcarousel/v0.2.0/install.sh" | sh
```

On Windows, download the `.zip` from the release page.

Ensure the install directory is on your `PATH` (the installer defaults to `$HOME/.local/bin`).

## Proof in one minute (no API keys)

From a clone of this repo:

```bash
agentcarousel validate fixtures/skills/example-skill.yaml
agentcarousel test fixtures/skills/example-skill.yaml --offline true --filter-tags smoke
```

You should see validation succeed and both smoke-tagged cases pass.

To verify validation catches broken fixtures (must exit non-zero):

```bash
set +e
agentcarousel validate fixtures/examples/invalid-skill.yaml
code=$?
set -e
test "${code}" -ne 0
```

## Live evaluation (optional)

Live runs need provider API keys and are not required for the smoke workflow. When ready, set one of `GEMINI_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, or `OPENROUTER_API_KEY`, then run:

```bash
agentcarousel eval --help
```

## Where to read next

| Goal | Doc |
|------|-----|
| Mental model (fixture, case, run, evidence) | [concepts.md](concepts.md) |
| Proposing a new fixture | [contributing-fixtures.md](contributing-fixtures.md) |