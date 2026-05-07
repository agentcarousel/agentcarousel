# Changelog

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
