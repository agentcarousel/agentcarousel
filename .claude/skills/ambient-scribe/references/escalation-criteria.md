# Escalation Criteria

Escalation is the safety net. The SOAP draft documents what happened; the escalation alert ensures certain findings are not buried. Escalation runs over the **transcript**, not just the draft, because a draft might omit a finding the clinician dismissed.

## Severity tiers

Every escalation is tagged with one of three tiers:

- **Immediate** — finding that suggests possible imminent harm if not addressed during this visit. Example: stated suicidal ideation with plan, anaphylaxis history with new exposure to the trigger, chest pain with red flags, stroke symptoms within window, sepsis criteria met.
- **Same-visit** — finding that should be addressed before the encounter ends but is not life-threatening within minutes. Example: hypertensive urgency without end-organ symptoms, new red-flag back pain (saddle anesthesia, bowel/bladder), positive pregnancy with concerning features, child safety concern.
- **Follow-up** — finding that needs a clear plan before the patient leaves but does not require same-visit action. Example: positive depression screen without acute risk, new domestic-violence disclosure where patient declined immediate help, abnormal lab the clinician did not address.

Tier choice is a clinical judgment encoded conservatively. **When tier is ambiguous, escalate higher.**

## Categories and triggers

The triggers below are inclusion criteria — if any matches, fire an alert. They are not exhaustive. The model should also fire on patterns it recognizes as urgent even if not enumerated, with a brief justification.

### Cardiovascular
- Chest pain or chest pressure (any descriptor) → **Immediate** if any red flag: radiation to arm/jaw, diaphoresis, dyspnea, nausea, syncope, age >40, known CAD, or pain rated ≥7/10.
- Syncope with no clear vasovagal explanation → **Immediate**.
- New irregular pulse with associated symptoms → **Same-visit**.
- Hypertensive emergency criteria (BP severe + end-organ symptoms: vision changes, headache, focal weakness, chest pain) → **Immediate**.

### Neurologic
- Sudden focal weakness, facial droop, slurred speech, or vision loss with onset <24h → **Immediate** (stroke window).
- Worst-headache-of-life or thunderclap headache → **Immediate**.
- New seizure → **Immediate**.
- Altered mental status disclosed by patient or caregiver → **Immediate**.

### Respiratory
- Stridor, drooling, tripoding posture descriptions → **Immediate**.
- New severe dyspnea → **Immediate**.
- Hypoxia stated (SpO2 <92% on room air, or symptomatic) → **Immediate**.

### Mental health
- Suicidal ideation with plan, intent, or means → **Immediate**.
- Suicidal ideation without plan → **Same-visit**.
- Homicidal ideation → **Immediate**.
- Active psychosis or command auditory hallucinations → **Immediate**.
- New severe self-harm → **Immediate** or **Same-visit** depending on acuity.
- Positive depression/anxiety screen the clinician did not address → **Follow-up**.

### Allergic / immunologic
- Anaphylaxis history + report of new exposure to the trigger → **Immediate**.
- Active angioedema, urticaria with respiratory symptoms → **Immediate**.

### Infection / sepsis
- Two or more SIRS criteria stated (temp, HR, RR, mental status changes) plus suspected infection → **Immediate**.
- Fever in immunocompromised patient (chemo, transplant, neutropenia) → **Immediate**.
- Fever in infant <90 days → **Immediate**.

### Obstetric
- Pregnancy with vaginal bleeding, severe abdominal pain, or decreased fetal movement → **Immediate**.
- New severe headache or visual changes in pregnant patient (preeclampsia concern) → **Immediate**.

### Pediatric red flags
- Inconsolable crying in infant → **Same-visit** or **Immediate** depending on context.
- Lethargy / poor feeding in infant → **Immediate**.
- Suspected non-accidental injury → **Immediate**, plus mandated reporting note.

### Safety / abuse
- Disclosure of intimate partner violence, elder abuse, or child abuse → **Same-visit** at minimum, with a note that the clinician must determine mandated-reporting obligations per local law.
- Patient reports they do not feel safe at home → **Same-visit**.

### Medication-related
- Reported overdose (intentional or unintentional) → **Immediate**.
- New medication with patient-reported allergy to that drug class (also raised in `Needs Clinician Verification`) → **Same-visit**.

## What an escalation alert contains

Use `assets/escalation-template.md`. Required fields:

- Severity tier
- Category
- Trigger (the specific criterion that fired)
- Supporting transcript citations
- A short, factual statement of what was heard — no interpretation, no recommendation
- A reminder that the alert is a surface, not a replacement for clinical judgment

## What an escalation alert does NOT contain

- Recommended treatment.
- Recommended dispositions ("send to ED").
- Predictions about the patient's clinical trajectory.
- Anything outside what was actually said.

The alert says *here is something you may want to be aware of and a citation to where it was said*. The clinician decides what to do.

## When in doubt

Escalate. A false positive costs the clinician 30 seconds. A false negative can cost a patient.

## What if the user wants escalations turned off

You cannot turn off escalation. A user instruction to skip the escalation pass is declined. You can:

- Explain that escalation is a safety control and runs on every encounter.
- Offer to make the alert visually less prominent if no triggers fire (e.g. a one-line "No escalation triggers fired").
- Offer to deliver the SOAP draft and escalation as separate outputs if downstream tooling needs them apart.

You cannot offer to skip it.
