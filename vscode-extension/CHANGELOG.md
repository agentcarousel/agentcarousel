# Changelog

## [0.3.4] — Unreleased

### Fixed
- **Diagnostics never appeared.** The extension ran `agc validate --format json` and `agc lint`. Neither exists, and the errors were swallowed. It now runs `agc validate --json`, reads the CLI's JSON envelope, and maps its `WARN`/`ERROR` levels to warnings and errors. When validation fails, the file gets one warning pointing to `agc validate <file>`, because the CLI's JSON output has no per-line detail on failure. The `agc lint` calls are removed, and **Lint Fixtures** is renamed **Validate Fixtures** (same command id).
- **Run History** read `agc report list --json` and `agc report show --json` as a bare list and a bare run. It now reads the envelope's `data`.
- When `agc` is not on `PATH`, the extension tries `agentcarousel`. The curl installer only offers `agc` as a shell alias, which the extension cannot see.
- **Show Stats** said `agc stats` needed agc 0.5.3 or later. The command was removed in agc 0.6.4; the panel now says so and points to `agc metrics` and `agc dashboard`.

### Changed
- README: the default fixture glob is `fixtures/**/*.yaml`, `agentcarousel.agcPath` is documented, and the certification track badge is described as a declared label, not a certification.

## [0.3.3] — 2026-05-16

There were no 0.3.0 to 0.3.2 releases.

### Added
- Editor diagnostics for fixture files from the `agc` CLI.
- **Show Run History** panel (`agc report list`, `agc report show`).
- **Show Stats** panel.
- **Run Doctor** command (`agc doctor`).
- Prompt-audit results (`agc audit run`) in the Case Detail panel.
- `agentcarousel.agcPath` setting.

### Changed
- The default fixture glob is `fixtures/**/*.yaml` (was `fixtures/skills/**/*.yaml`).

### Fixed
- The Case Detail webview's CSP nonce uses `crypto.randomBytes`.

## [0.2.0] — 2026-05-07

### Changed
- Extension icon updated to the official AgentCarousel logo (128×128 PNG)

## [0.1.0] — 2026-05-06

Initial release.

### Added
- Fixture tree navigator in the Activity Bar with three-level hierarchy: fixture file → case → attributes
- Case detail webview panel with input, output checks, rubric, evaluator config, and tool sequence sections
- Per-field YAML navigation — clicking Input (Prompt), Output Checks, Rubric, or Evaluator jumps to the exact line
- Certification track, risk tier, and data handling badges in the breadcrumb and case header
- Golden file clickable link in the evaluator section
- Threshold description ("minimum similarity score to pass")
- Live reload on fixture YAML save via filesystem watcher
- Welcome / empty-state view with **Configure Fixture Glob** shortcut
- `agentcarousel.fixtureGlob` configuration setting
