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

### Live Reload
The tree auto-refreshes whenever you save a fixture YAML. No manual refresh required.

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
- A workspace containing `fixtures/skills/**/*.yaml` files that conform to the [AgentCarousel fixture schema](https://agentcarousel.com)

---

## Extension Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `agentcarousel.fixtureGlob` | `fixtures/skills/**/*.yaml` | Glob pattern for fixture discovery (relative to workspace root) |

You can also change the glob via **AgentCarousel: Configure Fixture Glob** in the Command Palette (`Cmd+Shift+P`).

---

## Certification Tracks

AgentCarousel uses a four-tier certification model:

| Track | Meaning |
|-------|---------|
| `trusted` | Production-certified; passed full adversarial suite |
| `stable` | Validated; suitable for staging and pre-production |
| `candidate` | Under evaluation; passes smoke tests |
| `none` | No certification track assigned |

---

## About AgentCarousel

[AgentCarousel](https://agentcarousel.com) provides enterprise-grade quality assurance and trust infrastructure for autonomous AI agents. Our platform stress-tests agents against adversarial scenarios, certifies reliability and safety, and gives AI teams the confidence to deploy in production.

> *"Train and Trust Your Agentic Workforce"*

---

## Links

- [agentcarousel.com](https://agentcarousel.com)
- [GitHub](https://github.com/agentcarousel/agentcarousel)
- [Report an issue](https://github.com/agentcarousel/agentcarousel/issues)
