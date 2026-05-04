# Contributing to AgentCarousel

Thanks for your interest. There are two contribution tracks: **fixtures/docs** (the most common path) and **source code**. Both start with a GitHub Issue.

---

## Security

Do **not** open a public issue for undisclosed vulnerabilities. See [SECURITY.md](SECURITY.md).

---

## Issues first

Open an issue before submitting a PR. This keeps scope clear, avoids duplicate work, and lets maintainers flag any conflicts with in-flight changes. For quick typo fixes, an issue is optional.

---

## Fixture and doc contributions

Fixtures and documentation are the primary way the community shapes what AgentCarousel tests.

### Fixture policy

Fixtures merged to `main` must:

- Pass `agentcarousel validate` and `agentcarousel test --offline true` without errors.
- Not embed secrets, private keys, or customer data.
- Pair at least one `happy-path` case with one `failure-mode` or `error-handling` case.

### Fixture proposal checklist

Open an issue with the title `Fixture proposal: <skill-or-agent-id>` and fill in:

| Section | Your answers |
|---|---|
| **Skill / agent id** | Stable kebab-case id matching the fixture filename. |
| **Goal** | One sentence: what user intent does this fixture prove? |
| **Cases** | Table or list: case id, one-line description, tags (`smoke`, `happy-path`, `error-handling`). |
| **Tool calls** | Names and trigger conditions, or "none". |

After issue acceptance, scaffold with `agentcarousel init`, then follow the `init → validate → test --offline` loop before opening a PR.

See [docs/contributing-fixtures.md](docs/contributing-fixtures.md) for the full process and [docs/fixture-format.md](docs/fixture-format.md) for field reference.

---

## Source code contributions

For bug fixes and feature work:

1. **Open an issue** describing the problem or proposal.
2. **Discuss** — wait for a maintainer to confirm the approach before investing significant time.
3. **Fork and branch** — branch off `main`; name branches `fix/`, `feat/`, or `chore/` as appropriate.
4. **Build and test locally** — see [Building from source](#building-from-source) below.
5. **Open a PR** — reference the issue, describe what changed and why, include test coverage for new behavior.
6. **Review** — address feedback; keep commits focused.

PRs without a linked issue may be closed without review.

---

## Building from source

Requires stable Rust (1.75+):

```bash
# Install directly from source
cargo install --path crates/agentcarousel --locked

# Or build a debug binary
cargo build -p agentcarousel

# Run the test suite
cargo test

# Lint
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

The installed binary lands in `~/.cargo/bin/agentcarousel`. For the pre-built installer, see the **Install** section in [README.md](README.md) (curl installer and GitHub releases).

---

## ATF evidence and attestation contributions

If you are contributing fixtures for certification-style bundles (trust state, evidence export), use `bundle_id` / `bundle_version` on the fixture and keep the bundle manifest under `fixtures/bundles/<bundle>/` in sync. See [docs/fixture-development-process.md](docs/fixture-development-process.md) for the intake → review → bundle workflow. Export and registry workflows are described in the CLI (`agentcarousel export`, `agentcarousel publish`, `agentcarousel trust-check --help`) and on [agentcarousel.com](https://agentcarousel.com).

---

## Style notes

- Keep fixture `id` fields in stable kebab-case; changing an id is a breaking change for anyone pinning it.
- New docs go in `docs/`; reference new docs from an existing page so they are reachable.
- Commit messages: `type(scope): short description` where type is `feat`, `fix`, `chore`, `docs`, or `refactor`.
