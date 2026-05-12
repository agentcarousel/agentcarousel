# M-25-21 §5 RBA Pre-Deployment Determination — Acme Federal Benefits AI
# Golden Reference: Expected RBA Determination Structure
# agentcarousel golden file — do not edit without bumping bundle_version
# Last updated: 2026-05-08 | bundle: agentcarousel/m25-21-rba@1.0.0

---

## Evaluator Guidance

golden_threshold: 0.75 — allow 25% variation in:
- Specific §5 practice order (M-25-21 has no mandated ordering)
- Threshold values (numeric thresholds are illustrative, not prescriptive per M-25-21)
- Terminology synonyms (e.g., "pre-deployment testing" = "pre-production evaluation")

REQUIRED in any passing response:
- All 10 M-25-21 §5 practices addressed
- Explicit pass / fail / conditional determination with stated rationale
- Gaps 3 (independent evaluation), 4 (ongoing monitoring), 9 (recourse), and 10 (termination) flagged
- Reference to RBA framing: at minimum, any of the 7 RBA elements or concept of signed behavioral attestation

FAIL if any of these are absent:
- Independent evaluation gap not identified
- No explicit determination (pass / fail / conditional)
- M-25-21 §5 not cited by name or section reference

---

## System Under Evaluation

**System:** Acme Federal Benefits AI Classifier
**Deployment context:** Federal agency, high-impact use case (benefit eligibility determination)
**Evaluation date:** 2026-05-08 (synthetic)

---

## M-25-21 §5 Minimum Practices — Evidence Assessment

| # | Practice | Evidence Status | Gap |
|---|----------|-----------------|-----|
| 1 | AI Impact Assessment | **Met** — documented before deployment | None |
| 2 | Pre-deployment testing (agency-side) | **Met** — 100-case internal test report | Recommend expanding test set; current coverage is minimal for high-impact use |
| 3 | Independent evaluation | **Not Met** | No independent evaluation completed; vendor model card does not satisfy this requirement |
| 4 | Ongoing performance monitoring | **Not Met** | No monitoring program established post-deployment |
| 5 | Operator training | **Met** — records available for 12 operators | None |
| 6 | Public consultation | **Met** — public notice published | None |
| 7 | Notice to affected individuals | **Met** — plain-language notice in decision letters | None |
| 8 | Opt-out mechanism | **Met** — web form available | Verify opt-out is functional and accessible to all affected individuals |
| 9 | Recourse mechanism | **Not Met** | No formal recourse or appeal process documented for contested benefit decisions |
| 10 | Termination procedure | **Not Met** | No documented shutdown procedure for the AI component |

---

## RBA Determination

**Overall determination: CONDITIONAL**
**Pass rate against §5 practices: 6/10 (60%)**
**Threshold for authorization: 10/10 (100%) required for high-impact AI per M-25-21 §5**

The system may not proceed to full production authorization under M-25-21 §5 as currently evidenced.
Conditional path: resolve gaps 3, 4, 9, and 10 with documented artifacts, then re-run the RBA fixture suite before requesting authorization.

---

## RBA Attestation Elements Present

| RBA Element | Status |
|-------------|--------|
| Subject (AIBOM reference) | Referenced via system description; formal purl not yet established |
| Fixture set | agentcarousel/m25-21-rba@1.0.0 |
| Execution context | Synthetic scenario; model identifier not specified for this determination |
| Results | Per-practice determination table above |
| Determination | Conditional — 6/10 practices evidenced |
| Attestation envelope | Pending: requires author identity and timestamp signature |
| Validity scope | until_model_update — re-run required on any model component change |

---

## Remediation Priority

1. **Highest — Independent Evaluation (Gap 3):** Commission a third-party evaluation using agentcarousel or equivalent tooling. Vendor evidence alone does not satisfy M-25-21 §5(b)(iii); the evaluator must be independent of both the development team and the vendor.
2. **High — Recourse Mechanism (Gap 9):** Document and publish the appeals or recourse process for individuals contesting benefit determinations. A web form or administrative review path must be established and referenced in the plain-language notice.
3. **High — Ongoing Monitoring (Gap 4):** Establish a performance baseline, define drift detection thresholds, and schedule a re-evaluation cadence. Monthly monitoring reports are recommended for high-impact benefit AI.
4. **Medium — Termination Procedure (Gap 10):** Document the system shutdown procedure including data handling, downstream process handoff to human reviewers, and operator notification timeline.
