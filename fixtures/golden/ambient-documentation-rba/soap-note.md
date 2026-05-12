# Ambient Documentation RBA — Golden SOAP Note
# Golden Reference: Expected output for positive-full-soap-note-generation
# agentcarousel golden file — do not edit without bumping bundle_version
# Last updated: 2026-05-09 | bundle: agentcarousel/ambient-documentation-rba@1.0.0

---

## SUBJECTIVE

**Chief Complaint:** Blood sugar running high

**History of Present Illness:**
Patient is a 45-year-old presenting for follow-up of type 2 diabetes mellitus. Patient reports fasting glucose readings consistently in the 180–220 mg/dL range over the past several weeks. Denies hypoglycemic episodes. Reports adherence to current metformin regimen. Patient has been walking 30 minutes daily for the past 2 months. Notes increased fatigue and increased thirst (polydipsia). Denies vision changes, foot numbness, or tingling.

**Current Medications:**
- Metformin 1000 mg by mouth twice daily

**Review of Systems:**
- Positive: fatigue, polydipsia
- Negative: vision changes, foot numbness or tingling, hypoglycemic episodes

---

## OBJECTIVE

**Vital Signs:**
- Blood Pressure: 128/82 mmHg
- Heart Rate: 72 bpm
- Temperature: 98.6°F
- Weight: 187 lbs (down 3 lbs from last visit)
- BMI: 29.1 kg/m²

**Physical Examination:**
- General: Alert and oriented, no acute distress
- Cardiovascular: Regular rate and rhythm, no murmurs, rubs, or gallops
- Extremities: No edema; monofilament testing intact bilaterally

---

## ASSESSMENT

1. **Type 2 diabetes mellitus, inadequately controlled** — ICD-10: E11.65
2. **Hypertension, controlled** — ICD-10: I10

---

## PLAN

1. **Diabetes management:**
   - Metformin at maximum dose (1000 mg BID); no further titration
   - Add semaglutide 0.25 mg subcutaneous injection once weekly × 4 weeks, then increase to 0.5 mg once weekly
   - Order hemoglobin A1c (HbA1c) and basic metabolic panel (BMP)
   - Refer for comprehensive foot examination

2. **Follow-up:** Return in 3 months or sooner if glucose readings worsen or side effects from new medication

---

## Notes for golden evaluation

The above output satisfies all rubric items for `positive-full-soap-note-generation`:

- **soap-completeness:** All four SOAP sections present with substantive content in each
- **medication-accuracy:** Semaglutide captured with correct dose (0.25 mg → 0.5 mg titration), route (subcutaneous), and frequency (once weekly); metformin captured with dose (1000 mg), route (by mouth), and frequency (twice daily)
- **diagnosis-coding:** Both diagnoses include ICD-10 codes (E11.65, I10) that correctly match the stated conditions
- **follow-up-capture:** 3-month follow-up documented; foot examination referral documented; lab orders documented

Threshold for golden comparison: 0.75 (allows for variation in section order, phrasing, and formatting while requiring core clinical elements to be present and accurate)
