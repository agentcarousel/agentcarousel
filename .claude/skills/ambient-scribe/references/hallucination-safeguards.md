# Hallucination Safeguards

The hardest failure mode in ambient documentation is plausible fabrication: a SOAP note that reads beautifully and contains things the patient never said. This file describes the controls that exist to prevent that.

## The grounding contract

Every clinical claim in the SOAP draft must satisfy one of:

1. **Cited and supported.** The claim ends with `[T####]` citations, and the cited turns contain content that supports the claim.
2. **Marked as inferred.** The claim is in `Needs Clinician Verification` with an explanation of why it could not be grounded.
3. **Absent.** The claim is not in the draft.

There is no fourth option. "Implied by context" is not a fourth option. "Standard of care" is not a fourth option. "The clinician would obviously want this documented" is not a fourth option.

## What counts as "supported"

A turn supports a claim if a competent reader could read the turn and agree the claim is a fair representation of its content. Specifically:

- **Direct statement** — turn contains the fact verbatim or near-verbatim. Easy case.
- **Reasonable paraphrase** — turn says "my chest hurts when I walk up stairs," claim says "exertional chest pain." OK.
- **Aggregated symptoms** — claim summarizes multiple turns ("ROS positive for fatigue, weight loss, night sweats" with citations to the three relevant turns). OK.
- **Negation** — turn explicitly denies, claim records the denial. OK.

A turn does **not** support a claim if:

- The turn is about something else and only adjacent.
- The turn is the clinician *asking*, not the patient *answering*. "Any chest pain?" cited as evidence the patient has chest pain is wrong.
- The claim adds quantitative or diagnostic content not in the turn ("severe" added to "pain"; "uncontrolled" added to "blood pressure was 142/88").
- The claim picks one possibility from an ambiguous statement.

## The grounding check

After drafting, run `scripts/grounding_check.py`. It does three things:

1. **Citation existence.** Every cited turn ID must exist in the transcript. Any orphan citation → grounding failure.
2. **Citation coverage.** Every clinically meaningful sentence in S/O/A/P must have at least one citation. Sentences without citations → grounding failure.
3. **Support verification.** For each cited claim, the script presents the claim and the cited turn(s) back to the model and asks: does this turn support this claim, yes or no, and if no, why not. The model's verification answers become part of the audit log.

Anything that fails grounding is **not silently dropped**. It is moved to a `Needs Clinician Verification` section with the original claim, the citations attempted, and the failure reason. The clinician decides whether to keep, edit, or delete it.

## Anti-confabulation defaults

When in doubt, prefer absence to invention. Specific defaults:

- **Allergies not addressed** → `Allergies: Not addressed in encounter.` Never `NKDA`.
- **Vitals not stated** → omit. Don't write "vitals stable."
- **Past medical history not discussed this visit** → omit from this note. The EHR has the prior list.
- **Family history not discussed this visit** → omit.
- **Review of systems not performed** → write `ROS: Not performed.` Don't generate a default ROS.
- **Physical exam not described** → write `Exam: Not described in encounter.`
- **Assessment the clinician didn't articulate** → leave it out. Do not "complete" the differential.
- **Plan elements not stated** → leave them out. Do not add "follow up in 2 weeks" if the clinician didn't say so.

## What the model is allowed to use its training for

The model's medical knowledge is a tool for *understanding* the transcript, not for *adding* to it. Allowed uses:

- Recognizing that "ACE-inhibitor cough" describes a known phenomenon and selecting it as the right phrase when the clinician used different words to mean the same thing.
- Understanding that "BID" means twice daily so the medication record is filled correctly.
- Recognizing a brand–generic relationship (Lipitor → atorvastatin).
- Recognizing that a described symptom cluster matches a syndrome the clinician *also named* in the transcript.

Disallowed uses:

- Suggesting a diagnosis the clinician didn't suggest.
- "Completing" a differential.
- Adding a plan step that wasn't articulated.
- Using prior knowledge about the patient (you don't have prior visits; even if you did, this draft documents *this* encounter).

## When the user asks you to add ungrounded content

Common requests, all declined politely:

- "Just add a normal physical exam, the clinician forgot to dictate one." → No. The exam either happened and was documented, or it didn't. Move to `Needs Clinician Verification` with a note that a physical exam was not described.
- "Fill in the standard ROS." → No. ROS not performed reads as not performed.
- "Make the assessment more comprehensive." → If "comprehensive" means "include things the clinician said that I left out," yes. If it means "include things that are clinically standard but not stated," no.
- "Predict what the clinician would order." → No. Plan reflects what was ordered, not what might be ordered.

In each case: explain briefly that the skill does not generate ungrounded clinical content, offer to surface what *was* missing as a verification item, and proceed.
