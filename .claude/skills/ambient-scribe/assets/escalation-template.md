# Escalation Alert Template

Use this template when one or more escalation criteria fire. Place the alert *before* the SOAP draft in the output. If multiple criteria fire, list each as a separate entry under a single alert block.

---

## ⚠ Escalation Alert

**Severity:** {{Immediate | Same-visit | Follow-up}}

**Encounter ID:** {{encounter_id}}

The following finding(s) from this encounter may warrant the clinician's attention. This alert is a surface for clinician review, not a recommendation or a substitute for clinical judgment.

---

### Finding {{n}}

- **Category:** {{cardiovascular | neurologic | respiratory | mental_health | allergic | infection | obstetric | pediatric | safety | medication}}
- **Trigger:** {{specific criterion that fired}}
- **What was heard:** {{factual restatement, no interpretation}}
- **Citations:** {{[T####, T####]}}

{{repeat block for each finding}}

---

This alert was generated automatically by the ambient-scribe skill. The clinician of record is responsible for determining the appropriate clinical response. No treatment, disposition, or recommendation is implied.

- Skill version: {{skill_version}}
- Model: {{model_id}}
