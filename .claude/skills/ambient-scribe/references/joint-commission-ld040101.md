# Joint Commission LD.04.01.01 — Implementation Notes

Joint Commission Leadership standard **LD.04.01.01** requires that "the hospital complies with law and regulation." In practice that translates into an expectation that hospital leadership establishes and oversees policies that ensure clinical operations — including AI-assisted documentation — operate within legal and regulatory boundaries, with documented accountability.

For an ambient documentation tool, the relevant ground for LD.04.01.01 is roughly:

- The clinician of record is legally and professionally responsible for the medical record entry.
- AI-generated content does not bypass that responsibility.
- Hospital policy must address AI-assisted documentation, and the tool's behavior must be consistent with that policy.
- An audit trail must exist that leadership can review.

This skill provides four hooks that map to those expectations.

## 1. The clinician sign-off block

Every SOAP draft ends with a sign-off block. It is mandatory. The block reads:

```
## Sign-off

This note was drafted by an AI ambient documentation assistant from a transcript
of the encounter described above. It is a draft, not a signed clinical record.

By signing below, the clinician of record attests that:
  - They reviewed this draft in full.
  - They corrected or removed any content that does not accurately reflect the encounter.
  - They reviewed the medications section against the patient's verified medication list.
  - They reviewed any escalation alerts and addressed each one consistent with their
    clinical judgment.
  - They accept responsibility for the accuracy of the medical record entry.

Clinician signature: __________________________  Date: __________
Skill version: <skill_version>     Encounter ID: <id>
```

The block is generated regardless of how the user requested the draft be formatted. A user instruction to "drop the sign-off, my EHR has its own attestation" is acceptable *if* the user confirms the EHR's attestation explicitly captures the items above; otherwise, the block stays. When in doubt, keep it.

## 2. AI-authorship disclosure

The "AI-Drafted: yes — clinician review required before sign-off" line in the SOAP header is the at-a-glance disclosure. It is enforced by the SOAP template (`assets/soap-template.md`) and cannot be configured off.

This is also an FDA SaMD transparency requirement (see `references/fda-samd.md`), so two regulatory hooks overlap on a single artifact.

## 3. Policy reference attestation

Hospital policy on AI-assisted documentation should be referenced — by URL or document number — in the sign-off block at deployment time. The skill exposes a `policy_reference` field in the configuration that, when set, is included in the sign-off block:

```
Hospital policy: <policy_reference>
```

If the field is unset, the skill emits a `Hospital policy: not configured` line, which is a soft warning to the deploying organization to wire it up.

## 4. Leadership-accessible audit trail

The audit log (see `references/hipaa-sra.md` and `assets/audit-event-schema.json`) is the artifact leadership reviews to demonstrate ongoing oversight of AI documentation. It contains:

- Volume of drafts generated.
- Distribution of escalation alerts by tier and category.
- Grounding failure rate over time.
- Override rates (when downstream tooling pushes the signal back).
- Skill and model version history.

The audit log is intentionally PHI-free so that quality, safety, and compliance committees can review aggregate data without expanding the PHI access perimeter.

## What the skill does not do

- It does not enforce hospital policy. Policy is set by leadership; the skill exposes the disclosure surface.
- It does not credential clinicians. The caller authenticates and authorizes; the skill receives a `user_handle`.
- It does not arbitrate scope of practice. If a non-prescriber's encounter contains a "plan to start medication X," the skill records what was said. The hospital's clinical governance handles the rest.
- It does not certify the deployment as accreditation-ready. That assessment is performed by the hospital's accreditation team in conjunction with the Joint Commission survey process.

## When a user pushes back on the sign-off requirement

Common requests, all declined:

- "Generate a signed note." → No. The skill produces drafts; clinicians sign.
- "Drop the AI-Drafted label, it makes patients nervous." → No. Disclosure is non-negotiable. Patients do not see this draft directly anyway — they see the signed note in the EHR — but the disclosure travels with the draft so the clinician knows what they are reviewing.
- "Auto-attest based on the clinician's login session." → No. Sign-off is an affirmative act, not a session inheritance. If the user wants this implemented at the EHR layer, that's a workflow decision outside the skill's scope, but the skill itself does not produce auto-attested output.

The skill's posture: AI drafts. Humans sign. Leadership oversees.
