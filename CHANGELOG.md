# Changelog

## 0.4.0 - May 2026

- **Open source**: Rust source merged into this repository. Build from source with `cargo install --path crates/agentcarousel --locked`.
- **CI consolidated**: Release workflow now uses `GITHUB_TOKEN` with `contents: write`; cross-repo `PUBLIC_RELEASE_TOKEN` plumbing removed.
- **Docs site**: MkDocs Material site at [docs.agentcarousel.com](https://docs.agentcarousel.com); 21-page navigation covering getting started, reference, registry/trust, and contributing.
- **New generic fixtures**: `fixtures/skills/rag-qa.yaml` (RAG agent template) and `fixtures/skills/tool-call-correctness.yaml` (tool-calling template) with offline mocks — forkable starting points for new contributors.
- **README**: "Pytest for AI agents" positioning headline; install URL upgraded to HTTPS.
- **Distribution**: `distribution/` internal scaffolding removed; `CONTRIBUTING.md`, `SECURITY.md`, and `CHANGELOG.md` now live at repository root.

## 0.3.0 - April 26, 2026

- **`crates.io`**: Published [`agentcarousel` 0.3.0](https://crates.io/crates/agentcarousel); install with `cargo install agentcarousel` or `cargo install agentcarousel --version 0.3.0`.
- **Registry auth**: Code and docs use `AGENTCAROUSEL_API_TOKEN`. ***Registry is NOT AVAILABLE TO PUBLIC YET***
- **Docs**: Registry API / bundle-push runbooks and CSA-ATF ecosystem references updated to match current flows.

## 0.2.3 - April 25, 2026

- Expanded **rustdoc** for the library: crate-level overview, module summaries (`cli`, `core`, `runner`, `evaluators`, `fixtures`, `reporters`), and targeted docs on primary types and entrypoints for [docs.rs](https://docs.rs/agentcarousel).

## 0.2.2 - April 25, 2026

- Patch release for crates.io republish.

## 0.2.1 - April 25, 2026

- Clearer crates.io package description and keywords; README lede tightened for the registry page.

## 0.2.0 - April 25, 2026

- The CLI is now distributed as one package with two binaries: `agentcarousel` and `agc`.
- Fixture templates and workflows cleaned up for authors to use, added examples.
- Release and CI checks tightened for binaries and bundle verification.
- Docs refreshed to match current schemas and commands (`agentcarousel.com/agents`, smoke-tag flow).
- Packaging for crates.io compatibility.

## 0.1.0 - April 21, 2026

- Initial distribution and release pipeline.

