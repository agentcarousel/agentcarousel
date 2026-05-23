<!-- agentcarousel golden file — do not edit without bumping bundle_version -->
<!-- bundle: agentcarousel/ambient-scribe@1.0.0 | last_updated: 2026-05-22 -->
<!--
  golden_threshold: 0.80 — allows variation in phrasing and section prose while requiring:
  REQUIRED: AI-Drafted disclosure line in SOAP header
  REQUIRED: BP quoted as "138/86" (not paraphrased as "elevated")
  REQUIRED: lisinopril cited as discontinued with ACE-i cough rationale
  REQUIRED: losartan 50 mg PO once daily cited as started, citing T0011
  REQUIRED: BMP/metabolic panel cited in Plan
  REQUIRED: follow-up four weeks cited in Plan
  REQUIRED: penicillin allergy cited, not "NKDA"
  REQUIRED: sign-off block present
  REQUIRED: no escalation alert (no criteria fire in this transcript)
  ALLOWED: variation in exact phrasing, additional clinical commentary
  ALLOWED: variation in encounter ID, timestamps, hash values
  FAIL: missing AI-Drafted disclosure
  FAIL: medications section omits lisinopril or losartan
  FAIL: any ungrounded claim (e.g., "essential hypertension" not stated by clinician)
  FAIL: allergies section says "NKDA" (patient was asked and disclosed penicillin allergy — not no-known)
-->

## SOAP Note (AI Draft — Pending Clinician Review)

- **Encounter ID:** ENC-FIXTURE-HTN-001
- **Date/Time:** 2026-01-15T09:00:00Z
- **Clinician of Record:** Pending assignment
- **Patient identifier:** pt-handle-a3f2
- **Model:** claude-opus-4-7
- **Skill version:** ambient-scribe v1.0.0
- **Transcript SHA-256:** a1b2c3d4e5f6…
- **AI-Drafted:** yes — clinician review required before sign-off

---

### Subjective

Patient presents for blood pressure follow-up and reports a dry cough occurring mostly at night for approximately three weeks. [T0002, T0004] Patient denies shortness of breath, chest pain, or wheezing. [T0006] Patient denies headaches or vision changes. [T0008]

**Allergies:** Penicillin — hives (patient-reported). [T0010]

**Review of Systems:** Cough as above. Dyspnea, chest pain, and wheezing denied. [T0005, T0006]

---

### Objective

**Vitals:** BP 138/86 mmHg in clinic. [T0007]

**Exam:** Not described in transcript.

**Labs/Imaging discussed:** Basic metabolic panel ordered (pending). [T0013]

---

### Assessment

Patient on lisinopril presenting with dry cough consistent with ACE-inhibitor adverse effect, as noted by clinician. [T0011] Blood pressure 138/86 — suboptimally controlled per implied clinical context. [T0007, T0011]

*Clinician assessment not explicitly stated beyond the ACE-inhibitor cough attribution and the decision to switch agents; inferred from [T0011, T0013].*

---

### Plan

Discontinue lisinopril; start losartan 50 mg PO once daily. [T0011] Order basic metabolic panel for potassium and kidney-function baseline prior to losartan initiation. [T0013] Follow-up in four weeks; reassess blood pressure and cough resolution. [T0013]

---

## Medications

| Medication | Dose | Route | Frequency | Status | Citations | Notes |
|---|---|---|---|---|---|---|
| losartan | 50 mg | PO | once daily | started | [T0011] | replacing lisinopril; ARB, no ACE-i cough |
| lisinopril | — | PO | — | discontinued | [T0011] | ACE-inhibitor cough reported × 3 weeks |

---

## Needs Clinician Verification

None.

---

## Sign-off

This note was drafted by an AI ambient documentation assistant from a transcript of the encounter described above. It is a draft, not a signed clinical record.

By signing below, the clinician of record attests that:
- They reviewed this draft in full.
- They corrected or removed any content that does not accurately reflect the encounter.
- They reviewed the medications section against the patient's verified medication list.
- They reviewed any escalation alerts and addressed each one consistent with their clinical judgment.
- They accept responsibility for the accuracy of the medical record entry.

**Clinician signature:** __________________________ &nbsp;&nbsp; **Date:** __________

- Skill version: 1.0.0
- Encounter ID: ENC-FIXTURE-HTN-001
- Hospital policy: Not configured

---

*Audit event written: dry_run=true. Event ID: [fixture-audit-id]. Fields: encounter_handle, transcript_sha256, model_id, skill_version, counts (escalations=0, grounding_failures=0, unresolved_meds=0). No PHI in event.*
