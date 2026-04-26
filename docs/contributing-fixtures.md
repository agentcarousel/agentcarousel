# Contributing fixtures

This repository carries a small curated set of fixtures and mocks so people can run `validate` and offline `test` without checking out the full CLI source tree.

## Process

1. Open a GitHub Issue in this repository before sending a large fixture or new scenario. Use a title like `Fixture proposal: <skill-or-agent-id>`.
2. Fill in the intake checklist from [CONTRIBUTING.md](../CONTRIBUTING.md) (goal, cases, tool calls, tags).
3. Maintainers land accepted changes on the agreed contribution path (see [CONTRIBUTING.md](../CONTRIBUTING.md)).

This keeps fixtures aligned with released CLI behavior and schemas.

## Design hints

- Pair at least one happy-path (success) case with a failure-mode case where it makes sense.
- Prefer tags such as `smoke` or `compliance`.
- Keep mocks in sync with tool usage (`mocks/tool-mocks.json`, `mocks/agent-response.json`, etc.).