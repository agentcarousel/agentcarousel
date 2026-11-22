# AgentCarousel Fixtures

**Quality Assurance and Trust for Autonomous AI — right inside your editor.**

Stop squinting at raw YAML. The AgentCarousel Fixtures extension gives you a rich, human-readable interface for navigating, inspecting, and understanding your [AgentCarousel](https://agentcarousel.com) fixture files without ever leaving your IDE.

---

## Features

### Fixture Tree Navigator
Browse all fixture files, cases, and attributes in a structured sidebar tree. Every case is one click away.

- **Fixture files** show certification track, risk tier, and case count at a glance
- **Cases** display their evaluator type and tags inline
- **Attribute nodes** (Input, Output Checks, Rubric, Evaluator) navigate directly to the right line in the YAML

### Case Detail Panel
Click any case to open a fully rendered detail view:

- **Breadcrumb header** — `skill_name › case-id` with certification/risk/data badges
- **Input (Prompt)** — formatted message blocks, color-coded by role
- **Output Checks** — table of all assertions grouped by kind (`contains`, `regex`, `not_contains`, …)
- **Rubric** — cards with weight bars, descriptions, and auto-check patterns
- **Evaluator Config** — golden file link, threshold explained, judge prompt, or process command
- **Tool Sequence** — expected tool calls rendered as JSON

### Prompt Audit Results

After running `agc audit run <run-id>` from the terminal, the stored audit result is surfaced inline in the Case Detail panel — no extra clicks required.

- **Failure diagnosis** — the judge's verdict on whether a failure was caused by prompt design, model capability, or fixture miscalibration
- **Suggested fixes** — numbered list of LLM-generated improvement suggestions
- **Worked implementations** — where the judge provided a concrete rewrite, it appears below the suggestion in a code block
- **Apply to prompt** — run `agc audit suggest <run-id> --apply` to append suggestions as a commented block directly into `prompt.md`

The audit block is hidden when no audit has been run for a case, so the panel stays clean for runs that don't need it.

### Live Reload
The tree auto-refreshes whenever you save a fixture YAML. No manual refresh required.

### Validation Diagnostics
Saving a fixture runs `agc validate --json` on it and shows the CLI's warnings in the Problems panel. If validation fails, the file gets one warning; run `agc validate <file>` in a terminal for the details. **AgentCarousel: Validate Fixtures** checks every fixture in the workspace. Diagnostics need the `agc` (or `agentcarousel`) CLI on your `PATH`. Outside the AgentCarousel repository, `agc validate` also needs the fixture schema in `./schemas/` (see the [CLI reference](https://github.com/agentcarousel/agentcarousel/blob/main/docs/reference.md#agc-validate)).

### Run History, Doctor
**Show Run History** lists recent runs from `agc report list` and opens one with `agc report show`. **Run Doctor** prints `agc doctor` output. **Show Stats** relied on `agc stats`, which was removed in agc 0.6.4; use `agc metrics` or `agc dashboard` instead.

### Empty State & Onboarding
If no fixtures are found, the panel guides you to the right directory and lets you configure the fixture glob pattern directly from the welcome screen.

---

## Getting Started

1. Open a workspace that contains AgentCarousel fixture files (e.g., the `agentcarousel` repo)
2. Click the **◎ AgentCarousel** icon in the Activity Bar
3. Expand a fixture file to see its cases
4. Click a case to open its detail panel
5. Click **Open in Editor ↗** or any attribute node to jump to the exact YAML line

---

## Requirements

- VS Code `^1.85.0` (or any compatible fork — Cursor, Windsurf, etc.)
- A workspace containing `fixtures/**/*.yaml` files that conform to the [AgentCarousel fixture schema](https://github.com/agentcarousel/agentcarousel/blob/main/docs/fixture-format.md)
- For diagnostics and run history: the `agc` CLI ([install](https://github.com/agentcarousel/agentcarousel#quickstart))

---

## Extension Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `agentcarousel.fixtureGlob` | `fixtures/**/*.yaml` | Glob pattern for fixture discovery (relative to workspace root) |
| `agentcarousel.agcPath` | `agc` | Path to the CLI. With the default, the extension also tries `agentcarousel` |

You can also change the glob via **AgentCarousel: Change Fixture File Pattern** in the Command Palette (`Cmd+Shift+P`).

---

## Certification Track Badge

The badge shows the fixture's optional `certification_track` field: `none`, `candidate`, `stable`, or `trusted`. It is a label the fixture's author declares about where the suite stands. It is not a certification: the CLI only checks that the value is one of these four, and the field is not part of the [AgentCarousel Specification](https://github.com/agentcarousel/assurance-spec).

---

## About AgentCarousel

[AgentCarousel](https://agentcarousel.com) is an open-source CLI for writing behavioral tests for AI agents, running them in CI, and exporting evidence bundles that anyone can verify offline.

---

## Links

- [agentcarousel.com](https://agentcarousel.com)
- [GitHub](https://github.com/agentcarousel/agentcarousel)
- [Report an issue](https://github.com/agentcarousel/agentcarousel/issues)
