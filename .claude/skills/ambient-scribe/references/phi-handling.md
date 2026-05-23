# PHI Handling

This skill operates on Protected Health Information. Every transcript is ePHI from the moment it enters the workflow. This file describes how to handle it without violating HIPAA or creating a downstream incident.

## Operating assumption

Assume:

- The deployment is covered by a Business Associate Agreement (BAA) between the covered entity and Anthropic (or whichever inference provider). If you cannot confirm this, ask the user before processing real encounters. Synthetic test transcripts are fine without a BAA.
- The transcript may contain any HIPAA identifier, including the 18 statutorily defined ones.
- Logs and telemetry may end up somewhere less protected than the inference path itself.

## The 18 HIPAA identifiers (45 CFR § 164.514(b)(2))

For reference. Treat any of these as PHI:

1. Names
2. Geographic subdivisions smaller than a state (street, city, county, ZIP except first three digits per Safe Harbor rules)
3. Dates more specific than year (DOB, admission, discharge, death) for individuals
4. Telephone numbers
5. Fax numbers
6. Email addresses
7. Social Security numbers
8. Medical record numbers
9. Health plan beneficiary numbers
10. Account numbers
11. Certificate/license numbers
12. Vehicle identifiers (incl. plates)
13. Device identifiers and serial numbers
14. Web URLs
15. IP addresses
16. Biometric identifiers
17. Full-face photos and comparable images
18. Any other unique identifying number, characteristic, or code

## Scrubbing rules

Use `scripts/phi_scrubber.py`. It supports two modes:

- **`safe_harbor`** (default): redacts all 18 identifiers per Safe Harbor de-identification. Output is suitable for non-clinical destinations (debug logs, analytics, model evaluation datasets).
- **`encounter_handle`**: replaces patient identifiers with a stable, non-reversible hash for the encounter, allowing audit-event linkage without exposing identity. Use this for the audit log.

Do not write your own redaction. The script is the canonical implementation; if you find a gap, file a change request — do not patch in place inside another script.

## What goes where

| Destination | Allowed content |
|---|---|
| The SOAP draft itself | Full PHI as needed for clinical documentation. This is the legitimate clinical use. |
| The escalation alert | Citations and a factual statement of the trigger. Patient identifiers as needed for the clinician to act. |
| The audit log | Encounter handle (hashed), transcript SHA-256, model/skill version, timestamps, counts, status flags. **Never the transcript text or the SOAP content.** |
| Error messages / debug output | Scrub via `phi_scrubber.py`. If unsure, scrub. |
| Conversation in chat with the user | If the user pasted a real transcript, treat the chat as the clinical destination. Do not echo identifiers unnecessarily. Do not include the transcript in summaries you send back. |
| Examples and documentation | Synthetic data only. Never excerpt a real transcript. |

## Minimum necessary

HIPAA's minimum-necessary principle applies to everything this skill touches. Practical rules:

- Audit events log facts about the run, not the run's contents.
- Citations cite by turn ID, not by quoting the turn into the audit log.
- Verification failures log the *type* of failure ("orphan citation"), not the failed sentence.
- Medication validator logs the count of unresolved entries, not the unresolved drug names.

## Echoing back to the user

If a user pastes a transcript and asks "what did you do with it?" the answer is:

- Drafted a SOAP note (which they're seeing).
- Computed a SHA-256 hash for traceability.
- Wrote an audit event with the fields enumerated in `assets/audit-event-schema.json`.
- Did not transmit the transcript anywhere outside the inference path required to serve this conversation.

If the user asks for a copy of what was logged, show them the audit event payload — which is by design free of PHI — and the schema.

## When PHI handling fails

If you produce output that contains PHI in a destination it shouldn't be (e.g. a debug message that included transcript content), treat it as an incident:

- Tell the user what happened and what they should consider.
- Reference HIPAA breach notification rules (45 CFR § 164.400–414) — covered entities have specific obligations and the user's privacy/security officer needs to know.
- Do not try to assess whether it "counts" as a breach. That's not your call.
- Add an audit event with `incident: true` and a short, scrubbed description of the failure mode.

## Patient-directed disclosure

A patient can request their own information, and a clinician can share with another treating clinician for treatment purposes. This skill does not adjudicate those requests — that's the EHR's release-of-information workflow. If asked, defer to it.
