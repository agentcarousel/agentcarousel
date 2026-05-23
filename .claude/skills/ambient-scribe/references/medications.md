# Medication Documentation

## What to extract

For every medication mentioned in the encounter, capture:

| Field | Required | Notes |
|---|---|---|
| `name_raw` | yes | Exactly as spoken/transcribed. Do not normalize at this stage. |
| `name_resolved` | best-effort | RxNorm-resolved name if confident, else `null` with `unresolved: true` |
| `rxcui` | best-effort | RxNorm concept unique identifier if resolved |
| `dose` | yes | The numeric value as stated, or `null` if not stated |
| `dose_unit` | yes | mg, mcg, g, mL, units, puffs, etc. |
| `route` | yes | PO, IV, IM, SC, topical, inhaled, PR, sublingual, etc. |
| `frequency` | yes | once daily, BID, TID, QID, q4h, PRN, etc. |
| `indication` | if stated | Only if the encounter named one |
| `status` | yes | one of: `started`, `continued`, `dose_changed`, `discontinued`, `held`, `prn_added`, `mentioned_only` |
| `citations` | yes | List of turn IDs supporting the entry |
| `prescriber_intent` | yes | one of: `clinician_directed`, `patient_reported`, `unclear` |

The schema in `assets/medication-record-schema.json` is the source of truth. If you change fields, update both.

## Resolution rules

1. **Verify, don't guess.** Call `scripts/medication_validator.py` for every entry. It checks RxNorm by default and can be configured to additionally check a local formulary.
2. **Misheard tokens stay raw.** If the transcript says "metroprolol" and the validator returns no match, set `name_raw="metroprolol"`, `name_resolved=null`, `unresolved=true`. Do not silently correct to "metoprolol" — flag it for clinician review.
3. **Brand vs generic.** Resolve to whichever was spoken. If the clinician said "Lipitor," `name_raw="Lipitor"`, `name_resolved="atorvastatin"`, both retained.
4. **Combination products.** If a combination drug is mentioned (e.g. "lisinopril/HCTZ"), record it as a single entry; do not split unless components were prescribed separately.

## Dose handling

- **Quote the dose.** "20 milligrams" → `dose=20, dose_unit="mg"`. "Twenty milligrams" → same.
- **Ranges and uncertainty stay uncertain.** "Twenty or maybe forty milligrams" → `dose_raw="20 or 40", dose=null, dose_unit="mg", uncertain=true`. Do not pick a value.
- **Unit mismatches are flagged.** The validator will warn if a dose unit is implausible for the route (e.g. 500 mg of inhaled albuterol).
- **Pediatric weight-based dosing.** If a dose is given as mg/kg, capture both the per-kg figure and the resulting absolute dose if stated. Don't compute the absolute dose yourself.

## Reconciliation tags

After extraction, every medication record carries a `status`:

- `started` — clinician initiated this medication today
- `continued` — patient confirmed they are still on it; no change ordered
- `dose_changed` — dose, route, or frequency adjusted
- `discontinued` — clinician stopped the medication
- `held` — paused with intent to resume
- `prn_added` — added as needed
- `mentioned_only` — appeared in dialogue without a clear prescriber action (e.g. patient said they used to take it)

Apply these conservatively. If status is genuinely unclear, set `status="mentioned_only"` and add a `Needs Clinician Verification` entry.

## Allergies and contraindications

- This skill does not check drug–drug interactions, drug–allergy interactions, or contraindications. That's the EHR's job, and inserting our own check creates a coordination hazard.
- We *do* surface allergies that were stated in the encounter, attached to the SOAP note's Allergies section. If a medication was prescribed and an allergy to that drug class was also stated this encounter, raise this in `Needs Clinician Verification` — but do not block, override, or omit the prescription. The clinician decides.

## Output shape

The Medications section in the SOAP draft is a table. Example:

```
## Medications

| Medication | Dose | Route | Frequency | Status | Citations | Notes |
|---|---|---|---|---|---|---|
| losartan | 50 mg | PO | once daily | started | [T0012] | replacing lisinopril |
| lisinopril | — | PO | — | discontinued | [T0010, T0012] | ACE-i cough |
| metroprolol *(unresolved)* | 25 mg | PO | BID | mentioned_only | [T0019] | verify spelling and active status with patient |
```

Unresolved medications are italicized with `*(unresolved)*` and appear in `Needs Clinician Verification`.
