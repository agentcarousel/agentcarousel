# Agent Instructions

**Before doing anything else, read [`repo-info.md`](repo-info.md) in full.** It contains the complete project primer: architecture, module layout, key types, CLI commands, fixture schema, development workflow, issue tracking, and correlated repos. Do not skip it.

---

This project uses **br** (beads) for issue tracking. The CLI binary is `br` — never use `bd`.

## Quick Reference

```bash
br ready              # Find available work
br show <id>          # View issue details
br update <id> --claim  # Claim work atomically
br close <id>         # Complete work
```

## Non-Interactive Shell Commands

**Always use `rg` (ripgrep), never `grep`.**

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

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

1. `br update <id> --claim` — claim before writing code
2. Implement the change
3. `cargo test --all` — do NOT stage or close unless tests pass
4. `git add <files>` — stage the change, then **stop and wait for user review**
5. Do NOT commit. The user will commit after reviewing the staged diff.
6. `br close <id>` — only after the user has reviewed and committed

## Pre-Push Quality Gates (mandatory before every `git push`)

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo build --release
```

All four must pass. Fix errors before pushing.

## PR Policy

Do NOT open a pull request until the user has explicitly reviewed the work.
