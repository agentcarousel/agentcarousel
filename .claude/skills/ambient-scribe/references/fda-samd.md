# FDA SaMD AI Action Plan — Implementation Notes

The FDA's AI/ML Software-as-a-Medical-Device Action Plan articulates expectations for AI/ML-based SaMD: Good Machine Learning Practice (GMLP), a Predetermined Change Control Plan (PCCP), real-world performance monitoring, and transparency to users. Whether an ambient scribe is a SaMD subject to FDA premarket review depends on intended use and risk classification. This skill is built so that *if* a deployment is regulated as SaMD, the artifacts FDA expects already exist; and *if* it isn't, the same artifacts make the system safer and easier to audit.

## What this skill implements

### 1. Model and skill version pinning

Every audit event records:
- `model_id` — e.g. `claude-opus-4-7`
- `skill_version` — semver string from this package
- `prompt_hash` — SHA-256 of the SKILL.md plus all referenced files at run time

This is the minimum traceability you need to answer "what software produced this output," which is the question every SaMD post-market investigation starts with.

### 2. Predetermined Change Control Plan (PCCP) hooks

A PCCP describes which kinds of changes are anticipated and how they will be evaluated without requiring a new submission. This skill exposes:

- A `change_control` block in the audit event schema where deployments record which PCCP version was active.
- A `references/CHANGELOG.md` (created and maintained by the deploying organization) that lists changes by category: prompt changes, escalation criteria changes, medication vocabulary changes, scrubber rules changes.
- A convention that *behavior-changing* updates increment the minor version; *bug fixes that do not change behavior* increment patch; *anything affecting clinical safety* requires re-evaluation per the deploying organization's PCCP and increments major.

The skill does not enforce the PCCP — the deploying organization does — but it provides the seams.

### 3. Real-world performance monitoring

The audit event captures the metrics needed to compute, at population scale:

- **Grounding-failure rate** — fraction of drafts where one or more claims failed grounding.
- **Escalation rate by tier** — how often Immediate / Same-visit / Follow-up alerts fire.
- **Override rate** — fraction of drafts where the clinician edited or rejected an escalation alert (requires the EHR or downstream tool to push back the override signal).
- **Unresolved medication rate** — fraction of drafts containing one or more unresolved medication tokens.
- **Time-to-sign-off** — wall-clock from draft generation to clinician sign-off (requires EHR push-back).

These are leading indicators. A jump in grounding-failure rate after a model upgrade is the first place to look.

### 4. Transparency to users

The "AI-Drafted: yes — clinician review required before sign-off" header on every SOAP draft is the user-facing transparency disclosure. It is mandatory and it is not configurable to "off."

The sign-off block at the end of every draft repeats the disclosure in plain language. Both serve the FDA expectation that users of AI/ML SaMD know they are interacting with AI.

### 5. Good Machine Learning Practice alignment

The 10 GMLP principles (FDA / Health Canada / MHRA, 2021) have direct hooks in this skill:

| GMLP Principle | Hook in this skill |
|---|---|
| Multi-disciplinary expertise | Reference files separate clinical (SOAP, medications, escalation) from compliance (HIPAA, FDA, JC) so each can be reviewed by the right experts. |
| Good software engineering practices | Scripts are auditable, schemas are versioned, audit logging is append-only. |
| Clinical study representativeness | Captured by the deploying organization's evaluation process, not the skill itself. |
| Independence of training and test data | N/A at skill level (the model is upstream); evaluation harnesses must respect this. |
| Reference standards | SOAP format, RxNorm for meds, HIPAA Safe Harbor for redaction. |
| Model design tailored to data and intended use | This is a documentation drafting skill, not a diagnostic skill — scope is constrained intentionally. |
| Performance assessment in the clinical environment | Real-world performance metrics enumerated above. |
| Training data and model are appropriate to deployment | Deploying organization's responsibility; skill version is logged. |
| Users provided clear information | Transparency disclosures in every draft; AI-drafted label. |
| Monitor performance after deployment | Audit log + metrics enumerated above. |

## What this skill does NOT do

- It does not classify itself under FDA risk tiers. The deploying organization makes that determination based on intended use.
- It does not generate FDA submission artifacts. A submission requires far more than what a skill can produce — clinical validation, software lifecycle processes, cybersecurity documentation, labeling, etc.
- It does not detect drift in the underlying foundation model. Drift detection requires a held-out evaluation set and periodic re-evaluation by the deploying organization.

## When the user asks "is this FDA cleared?"

The honest answer: the skill itself is not a regulated device. Whether a deployment of this skill is a regulated device depends on intended use, claims made, and risk to patients. If the deployment is intended for clinical decision-making in a way that meets the SaMD definition and is not exempt under the 21st Century Cures Act CDS provisions, it likely requires FDA engagement. Refer the user to their regulatory affairs team and/or to FDA's Digital Health Center of Excellence resources.

Do not represent the skill as cleared, approved, or authorized.
