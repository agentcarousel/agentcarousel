# Agent Instructions

> **MANDATORY — do this before anything else, no exceptions:**
>
> 1. Read this file (`AGENTS.md`) in full — you are reading it now; complete it before proceeding.
> 2. Read [`repo-info.md`](repo-info.md) in full — complete project primer: architecture, module layout, key types, CLI commands, fixture schema, development workflow, issue tracking, and correlated repos.
>
> Do not answer, plan, search, or write any code until both files have been read. Skipping either file is a protocol violation.

## Beads Issue Tracker

This project uses **br** for issue operations and **bv** for triage intelligence. Never use `bd`.

### Session Start — orient with bv

```bash
bv --robot-next                  # Top-scored single pick for immediate work
bv --robot-triage                # Full triage: scores, blockers, quick wins, health
bv --robot-plan                  # Dependency-ordered execution plan
bv --robot-triage-by-track       # Parallel work streams for multi-agent coordination
bv --robot-triage-by-label -l "area:foo"  # Scope to a label's subgraph
bv --robot-suggest               # Smart suggestions: missing deps, labels, duplicates
bv --robot-alerts                # Drift and staleness warnings
```

### Issue Operations — use br

```bash
br ready              # View ready issues (open, unblocked, not deferred)
br list --status=open # All open issues
br show <id>          # Full issue details with dependencies
br search "keyword"   # Full-text search
br create --title="..." --description="..." --type=task --priority=2
br update <id> --claim
br close <id>
br close <id1> <id2>  # Close multiple at once
br label add <id> -l "area:foo"
br dep add <child> <parent>
```

### bv Integration Rules

- Run `bv --robot-next` at session start before touching `br`
- After creating or labelling a batch of issues, run `bv --robot-suggest` to catch gaps
- When coordinating subagents, use `bv --robot-triage-by-track` to assign each agent its own track
- Treat `bv` scores as the authoritative pick ordering — don't override without reason

### Key Concepts

- **Dependencies**: Issues can block other issues. `br ready` shows only open, unblocked work.
- **Priority**: P0=critical, P1=high, P2=medium, P3=low, P4=backlog (use numbers 0-4, not words)
- **Types**: task, bug, feature, epic, chore, docs, question
- **Blocking**: `br dep add <issue> <depends-on>` to add dependencies

## Per-Issue Workflow (enforce strictly)

For every issue worked:

1. `bv --robot-next` — let it pick the highest-scored unblocked issue
2. `br update <id> --claim` — claim before writing any code
3. Implement the change, then run in order:
   ```bash
   cargo fmt --all
   cargo clippy -p agentcarousel --all-targets --all-features -- -D warnings
   cargo test --all
   ```
   Do NOT stage or close unless all three pass. Fix errors before proceeding.
4. `git add <files>` — stage the change, then **stop and wait for the user's review**
5. Do NOT commit. The user commits after reviewing the staged diff.
6. `br close <id>` — only after the user has reviewed and committed

## Issue Structure

- Use **epics** for strategic grouping and **tasks** that depend directly on them.
- Skip the feature layer — for a small team, labels replace it.
- Label tasks with `area:baseline`, `area:auth`, `area:www`, `lang:rust`, `lang:typescript`, etc.
- After creating a batch of issues, run `bv --robot-suggest` to catch gaps.

## Pre-Push Quality Gates (mandatory before every `git push`)

```bash
cargo fmt --all
cargo clippy -p agentcarousel --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

All four must pass. Fix errors before pushing.

## PR Policy

Do NOT open a pull request until the user has explicitly reviewed the work.
