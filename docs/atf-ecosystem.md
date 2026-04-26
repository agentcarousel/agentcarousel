# AgentCarousel and the Agentic Trust Framework (ATF)

The [Agentic Trust Framework (ATF)](https://github.com/massivescale-ai/agentic-trust-framework) is a **governance specification**: five trust elements, maturity language, and conformance guidance—not a library you import. AgentCarousel is positioned as a **tooling layer that produces auditable evidence** (validation, offline/online runs, reports, exports) and **CI gates**, without claiming product-level ATF certification for itself or for your agents.

Public mirror (synced default branch): short ATF ↔ fixture trace in [`docs/CROSSWALKS.md`](./CROSSWALKS.md) (synced to the public repo per the layout in `distribution/public-hub/REPOSITORY.md`).

**Authoritative ATF sources**

- [Repository](https://github.com/massivescale-ai/agentic-trust-framework)
- [SPECIFICATION.md](https://github.com/massivescale-ai/agentic-trust-framework/blob/main/SPECIFICATION.md)
- [CONFORMANCE.md](https://github.com/massivescale-ai/agentic-trust-framework/blob/main/CONFORMANCE.md)
- [IMPLEMENTATION_PATTERNS.md](https://github.com/massivescale-ai/agentic-trust-framework/blob/main/IMPLEMENTATION_PATTERNS.md)

## Crosswalk: ATF elements → CLI → artifacts

The table below maps ATF’s five elements to what AgentCarousel can **evidence today**. Maturity labels are intentional: **strong** means first-class, automated artifacts; **partial** means metadata, heuristics, or docs-only; **not applicable** means out of scope for this CLI.

| ATF element | AgentCarousel surface | Primary artifacts | Evidence strength |
|-------------|------------------------|-------------------|---------------------|
| **Identity** | Fixture `skill_or_agent` id (kebab-case), bundle `bundle_id` / `bundle_version`, persisted `run.json` with run id | `validate` output, SQLite runs, `export` tarball `run.json` | **Partial** (naming and bundle identity, not DID/VC “agent passports”) |
| **Behavior** | `test`, `eval` with rules/golden/process/judge evaluators; case-level `expected` and rubrics | `run.json`, evaluator outputs in DB, JSON/JUnit from `--format` | **Strong** for defined cases; scope is **fixture-bound** behavior, not full production telemetry |
| **Data governance** | Fixture fields `risk_tier`, `data_handling`, trace scrubbing in runner | Fixture YAML, `validate --format json` `atf_summary`, export `REDACTION_POLICY.md` (stub until a profile ships) | **Partial** (declarative policy fields + export stub; see [trust posture](trust-posture.md)) |
| **Segmentation** | `validate` + offline `test` in CI; tag filters (`--filter-tags`); bundles | CI logs, `validate-report.json`, matrix test JSON artifacts | **Strong** for **contract** and **negative-path** gates in-repo |
| **Incident response** | `report list` / `report show` / `report diff`; persisted history for regression | SQLite + JSON exports | **Partial** (evidence retention and diff; not a SOAR playbooks product) |

## What we do not claim

- **No ATF conformance certification** of AgentCarousel or your product from using these commands alone—customers map controls using your evidence plus their governance program.
- **No cryptographic signing** of exports in the default MVP (integrity via `MANIFEST.json` hashes; signing deferred—see [trust posture](trust-posture.md)).
- **No DID/VC or runtime identity federation**—fixtures identify *subjects under test*, not production identity wallets.

## Training vs product enforcement

Human and agent **security awareness** (for example curated awareness content such as [Trail of Bits security-awareness](https://github.com/trailofbits/skills-curated/tree/main/plugins/security-awareness)) is **organizational training**, not an executable redaction engine inside AgentCarousel.

A future **redaction profile** (fields, defaults, tests) would be a **product contract** separate from “agents read a skill.” Today’s export ships a **stub** `REDACTION_POLICY.md` until that profile exists—see [trust posture](trust-posture.md).

## Related docs

- [Trust posture](trust-posture.md) — non-goals, signing stance, redaction stub honesty
- [Fixture format](fixture-format.md) — schema and governance-related fields
- [CI gates](ci-gates.md) — what fails a PR
