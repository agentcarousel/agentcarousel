# Methodology

How `agc` scores agent behavior, what the numbers mean, and what the signed bundle does and doesn't prove. For term definitions see [docs/concepts.md](docs/concepts.md).

## Scoring

Every case in a fixture is scored against its rubric. Rubric items come in two kinds:

- **Deterministic (`auto_check`)** — a regex or substring assertion. Pass scores 1.0, fail scores 0.0, no model involved. Use these for anything unambiguous: "the output contains an escalation alert", "the output never contains a SQL statement".
- **Judged** — the case input, the agent's output, and the rubric item description go to an LLM judge, which scores 0–1 and writes a rationale. Use these only where correctness requires understanding context, tone, or intent.

The case score is the weighted average across rubric items. Per-item scores and judge rationales are stored with the run, so any score can be traced back to the assertion or rationale that produced it.

## Judge reliability

LLM-as-a-judge has known failure modes: score variance across runs, leniency bias, and sensitivity to phrasing. We don't claim to have eliminated them. We do the following to keep them bounded and visible:

- **Prefer deterministic checks.** If a criterion can be a regex, it is. The judge only scores what rules can't.
- **Separate judge from generator.** The judge model is configured independently of the model under test, so a model is never grading its own family by default — and you can pick any judge (Gemini, Claude, OpenAI, or a local Ollama model).
- **Repetition.** `--runs N` executes each case N times; reports show the spread, not just a point estimate.
- **Score calibration metric.** `agc metrics` includes a calibration check that compares judge scores against deterministic `auto_check` outcomes on cases that have both, so systematic judge drift shows up as a number rather than a hunch.
- **Regression framing.** CI gates compare runs against a tagged baseline (`agc compare`). Relative movement on identical fixtures is far more robust to judge noise than absolute scores.

If a judge score matters to you, read the rationale. They're all in the run history and the exported bundle.

## Compliance scoring

Cases tagged with control IDs (e.g. `fda-samd:fda-samd-medical-device-reporting`) feed per-control scores for the bundled OSCAL catalogs (NIST AI RMF, EU AI Act, ISO 42001, HIPAA, FDA SaMD, NIST SP 800-171/172/207).

A control is reported **satisfied** only when it has at least three cases and mean effectiveness ≥ 0.80. Below that it is **partial evidence**; with no covering cases it is a **gap**, with a remediation advisory. The report is deliberately conservative: it tells you what's missing instead of rounding up. `agc compliance report --oscal` emits the same results as a machine-readable OSCAL Assessment Results document.

A satisfied control means *behavioral test evidence exists for the scenarios tested*. It is not a legal determination of regulatory compliance.

## What the signed bundle proves — and what it doesn't

`agc export` produces a self-contained evidence tarball. It holds per-case results, judge rationales, metrics, OSCAL assessment results, an `evidence.json` and an in-toto `attestation.intoto.json` that follow the open [AgentCarousel Specification](https://github.com/agentcarousel/assurance-spec) (draft 0.1), and a `MANIFEST.json` with a SHA-256 hash of every file. Fixture files are not included; `evidence.json` identifies them, the system prompt, and any golden files by SHA-256 digest, so a holder of the suite can confirm what was tested. With `agc export --sign-key`, the manifest is also signed with minisign. `agc verify --minisign-pubkey` checks the hashes, the signature, and the spec conformance of the evidence, all offline. `agc trust-check` is different: it queries the registry for a bundle's trust state, so it needs network access.

System prompts are withheld by default: exported runs replace system messages with their digest, and `agc publish` uploads `prompt.md` only with `--include-prompt`.

**This proves (for a signed bundle verified against a key you trust):** the bundle is exactly what was produced at export time. Nothing was edited, added, or removed afterward. Anyone you hand it to can verify that independently. An unsigned manifest only shows the files are self-consistent — whoever edits them can regenerate the hashes.

**This does not prove:** that the tests were well-chosen, that the agent behaves correctly outside the tested scenarios, or that the evidence is independent — you ran the tests yourself. Self-run evidence is an integrity claim, not an independence claim. Independent test design and domain-expert attestation are a separate, third-party exercise (that's the [certification service](https://agentcarousel.com), which is optional and never required to use this tool).

## Known limitations

- Judge scores carry variance; treat absolute numbers with skepticism and trends with confidence.
- Fixture coverage is only as good as the scenarios you (or `agc generate`) wrote. Coverage metrics measure breadth against your own suite, not against the real world.
- Behavioral evidence is point-in-time: it describes a specific system prompt and model version on a specific date. Model or prompt changes invalidate it — re-run the suite. `evidence.json` records the prompt, fixture, and golden-file digests so a consumer can tell when evidence no longer matches a configuration.
- **Tool calls are not observed.** The runner builds tool-call steps from each case's own `expected.tool_sequence` and answers them from mocks; it does not yet capture tool calls the agent actually makes. `tool_sequence` assertions therefore check the fixture, not the agent. Evidence marks these traces `fixture_expectation` and carries a `tool_calls_not_observed` limitation; do not cite them as proof of tool-use behavior.
- Cases removed by `--filter`/`--filter-tags` are recorded as not tested (`skipped`) rather than dropped, so a filtered run never looks like a full one.
