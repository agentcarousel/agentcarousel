# SOAP Note Format

## Structure

A SOAP note has four sections, in this order:

- **S — Subjective**: what the patient reports. History of present illness, review of systems, relevant past history if newly disclosed in this encounter, social context if discussed.
- **O — Objective**: what the clinician observed or measured. Vitals, exam findings, lab/imaging results referenced or obtained, mental status if assessed.
- **A — Assessment**: the clinician's interpretation. Differential, working diagnosis, problem list updates. **Only what the clinician said**, not what you would assess.
- **P — Plan**: what happens next. Medications started/changed/stopped, tests ordered, referrals, follow-up interval, patient education delivered.

Sections may be empty. If the encounter contained no objective findings, the Objective section reads "Not addressed in encounter." Do not invent content to fill a section.

## Header (required)

Every draft starts with a header block:

```
## SOAP Note (AI Draft — Pending Clinician Review)
- Encounter ID: <id>
- Date/Time: <ISO 8601>
- Clinician of Record: <name or "Pending assignment">
- Patient identifier: <de-identified handle, never the legal name in shared output>
- Model: <e.g. claude-opus-4-7>
- Skill version: ambient-scribe v<x.y.z>
- Transcript SHA-256: <first 12 chars>…
- AI-Drafted: yes — clinician review required before sign-off
```

The "AI-Drafted: yes" line is the FDA SaMD transparency disclosure. It is mandatory and must be visually prominent.

## Citation convention

Every clinically meaningful sentence ends with a transcript citation in square brackets:

- Single turn: `Patient reports chest pressure radiating to the left arm beginning two hours ago. [T0014]`
- Multiple turns: `Pain is described as 7/10, sharp, worsened by exertion. [T0014, T0017, T0019]`
- Range: `Patient denies recent travel, sick contacts, or new medications. [T0021–T0024]`

Sentences without citations are not allowed in S/O/A/P. If a sentence is editorial connective tissue ("The following history was obtained:"), you do not need a citation, but you also do not need the sentence — prefer to omit it.

## What citing means

A citation is a contract: it says *the cited turn(s) contain content that supports this claim*. The grounding check (`scripts/grounding_check.py`) will verify the turn IDs exist and re-evaluate whether they support the claim. False citations are treated as hallucinations, not typos.

If you are tempted to cite a turn that does not actually contain the supporting content, you are about to hallucinate. Stop. Move the claim to `Needs Clinician Verification` instead.

## Quoting vs paraphrasing

- **Numbers are quoted, not interpreted.** "BP 142/88 [T0008]" — yes. "Hypertensive [T0008]" — only if the clinician said "hypertensive" in T0008.
- **Patient quotes are quoted when prognostically meaningful.** "Patient states 'I feel like I'm going to die' [T0014]" — quote, because that exact phrasing matters clinically. Routine descriptions can be paraphrased.
- **Negatives are reported only if asserted.** "Denies fever" requires a turn where fever was asked about and denied. Absence of mention ≠ denial.

## Allergies, social history, family history

- **Allergies**: if not addressed in the encounter, the field reads `Not addressed in encounter`, not `NKDA`. NKDA is a clinical assertion that requires it to have been asked.
- **Social history**: include only what was discussed this encounter. Do not pull from prior visits unless the clinician explicitly references and confirms prior content in this transcript.
- **Family history**: same rule.

## Worked example (excerpt)

Given turns:

```
T0008  Clinician: Your blood pressure today is 142 over 88.
T0009  Patient: Is that bad?
T0010  Clinician: It's higher than where we want you. Has the lisinopril been giving you any cough?
T0011  Patient: A little, mostly at night.
T0012  Clinician: Okay, let's switch you to losartan. We'll start at 50 milligrams once a day.
```

Correct draft fragment:

```
**Subjective:** Patient reports a mild nighttime cough on lisinopril. [T0011]

**Objective:** BP 142/88 in clinic. [T0008]

**Assessment:** Suboptimally controlled hypertension on lisinopril; ACE-inhibitor associated cough reported by patient. [T0008, T0010, T0011]

**Plan:** Discontinue lisinopril; start losartan 50 mg PO daily. [T0012]
```

Incorrect draft fragment (do not produce this):

```
**Assessment:** Hypertension, poorly controlled. Likely essential hypertension given age and risk factors.
```

This is wrong because (a) "likely essential hypertension" was not in the transcript, and (b) it has no citation. Editorial inference without grounding is the failure mode this skill is designed to prevent.
