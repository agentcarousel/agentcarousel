# SOAP Note Template

Fill in the placeholders. Citations `[T####]` are required on every clinical sentence in S/O/A/P. Do not remove the AI-Drafted disclosure line.

---

## SOAP Note (AI Draft — Pending Clinician Review)

- **Encounter ID:** {{encounter_id}}
- **Date/Time:** {{iso8601_timestamp}}
- **Clinician of Record:** {{clinician_name_or_pending}}
- **Patient:** {{patient_handle}}
- **Model:** {{model_id}}
- **Skill version:** ambient-scribe v{{skill_version}}
- **Transcript SHA-256:** {{transcript_hash_short}}…
- **AI-Drafted:** yes — clinician review required before sign-off

---

### Subjective

{{subjective_paragraph_with_citations}}

**Allergies:** {{allergies_or_not_addressed}}

**Review of Systems:** {{ros_or_not_performed}}

### Objective

**Vitals:** {{vitals_quoted_or_omitted}}

**Exam:** {{exam_findings_or_not_described}}

**Labs/Imaging discussed:** {{labs_imaging_or_omitted}}

### Assessment

{{assessment_with_citations}}

### Plan

{{plan_items_with_citations}}

---

## Medications

| Medication | Dose | Route | Frequency | Status | Citations | Notes |
|---|---|---|---|---|---|---|
{{medication_rows}}

---

## Needs Clinician Verification

{{verification_items_or_none}}

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

- Skill version: {{skill_version}}
- Encounter ID: {{encounter_id}}
- Hospital policy: {{policy_reference_or_not_configured}}
