# Contributing

This repository is the distribution repo for the `agentcarousel` CLI. All new ideas (e.g. fixtures, docs) should start as a **GitHub Issue**.

---

## Security

Do **not** open a public issue for undisclosed vulnerabilities. Follow [SECURITY.md](SECURITY.md).

---

## Fixture proposals (issue first)

1. **Open an issue** in this repository. Use a title such as `Fixture proposal: <skill-or-agent-id>`.
2. In the issue body, work through the intake checklist below.
3. Wait for maintainer feedback. They may ask for clarifications.
4. Implementation happens in the canonical workspace. After an issue is accepted, maintainers either land the change themselves or point you to the agreed contribution path (for example a private fork, patch bundle, or source repo).

### Intake checklist

Copy the block into your issue and fill it in.


| Section              | Your answers                                                                                        |
| -------------------- | --------------------------------------------------------------------------------------------------- |
| **Skill / agent id** | Stable kebab-case id (matches fixture naming).                                                      |
| **Goal**             | One sentence: what user intent does this fixture prove?                                             |
| **Cases**            | Table or list: case id, one-line description, tags (`smoke`, `happy-path`, `error-handling`, etc.). |
| **Tool calls**       | If the skill/agent uses tools: names and when they run; if none, say so.                            |


**Rules of engagement**:

- Pair a **happy path** with at least one **failure-mode** case in the design before implementation.
- Prefer `agentcarousel init` to scaffold templates.

---

## Building from source

Use the **Releases** assets and `install.sh` as documented in [README.md](README.md).
