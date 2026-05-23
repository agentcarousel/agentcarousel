# ambient-scribe

An ambient clinical documentation skill for agentic IDEs and Claude-based assistants. Drafts SOAP notes from encounter transcripts with strict transcript grounding, medication validation, urgent-finding escalation, and audit logging — built so that **FDA SaMD AI Action Plan**, **HIPAA Security Risk Analysis (45 CFR § 164.308(a)(1)(ii)(A))**, and **Joint Commission LD.04.01.01** hooks are first-class, not bolted on.

## What it is

A SKILL.md-format skill plus its supporting references, assets, and scripts. Drop the folder into a SKILL.md-aware client (Claude Code, Cursor with the appropriate loader, Antigravity, Claude.ai with skill loading enabled) and the agent picks it up the next time the description matches.

## Layout

```
ambient-scribe/
├── SKILL.md                                  # entry point, workflow, behavioral rules
├── README.md
├── references/                               # loaded on demand
│   ├── soap-format.md
│   ├── medications.md
│   ├── hallucination-safeguards.md
│   ├── escalation-criteria.md
│   ├── phi-handling.md
│   ├── fda-samd.md
│   ├── hipaa-sra.md
│   └── joint-commission-ld040101.md
├── assets/                                   # templates and schemas
│   ├── soap-template.md
│   ├── escalation-template.md
│   ├── medication-record-schema.json
│   └── audit-event-schema.json
└── scripts/                                  # deterministic checks
    ├── grounding_check.py                    # citation existence + support verification
    ├── medication_validator.py               # RxNorm-style resolution + plausibility
    ├── phi_scrubber.py                       # Safe Harbor scrubber + encounter handle
    └── audit_logger.py                       # append-only HIPAA-safe audit events
```

## Workflow it enforces

1. **Intake & validation** — accept transcript, label turns, hash for traceability.
2. **PHI posture** — treat the transcript as ePHI; route any non-clinical output through the scrubber.
3. **SOAP draft** — produce S/O/A/P with a transcript-turn citation on every clinical sentence.
4. **Medication extraction** — schema-validated records, vocabulary-resolved names, plausibility checks.
5. **Grounding check** — verify every citation exists and supports its claim; failures move to *Needs Clinician Verification*, not silently dropped.
6. **Urgent-finding escalation** — run the escalation pass over the **transcript** (not just the draft); produce an alert at the top of the output if any criterion fires.
7. **Audit event** — emit a non-PHI event capturing model/skill versions, hashes, and counts.
8. **Sign-off block** — hand a draft, not a signed note, to the clinician of record.

## Regulatory hooks

- **FDA SaMD AI Action Plan** → model and skill version pinning, PCCP linkage in the audit event, real-world performance metrics (grounding-failure rate, escalation rate, override rate, unresolved-med rate), AI-authorship disclosure on every draft. See `references/fda-samd.md`.
- **HIPAA Security Risk Analysis** → PHI-free audit events, transcript hashing instead of storage, scrubber on all non-clinical destinations, BAA acknowledgment surface, minimum-necessary throughout. See `references/hipaa-sra.md`.
- **Joint Commission LD.04.01.01** → mandatory clinician sign-off block, AI-drafted disclosure, hospital-policy reference field, leadership-accessible audit trail. See `references/joint-commission-ld040101.md`.

## Behavioral guarantees

- No clinical claim without a transcript citation, ever.
- No invented medications, doses, routes, frequencies, or allergies.
- No autonomous sign-off — the skill produces drafts; clinicians sign.
- No PHI in audit logs, debug output, or examples.
- Escalation alerts cannot be suppressed by user instruction.
- AI-authorship is disclosed on every draft.

## Deploying

1. **Confirm BAA coverage** with the inference provider before processing real PHI.
2. **Set the audit sink.** Default is a local JSONL file. For production, register a sink that writes to your tamper-evident store (object lock, write-only Kafka, etc.) — see `scripts/audit_logger.py`.
3. **Set the encounter-handle salt.** Export `AMBIENT_SCRIBE_HANDLE_SALT` to a per-deployment secret so encounter handles are not predictable across organizations.
4. **Plug in a real RxNorm resolver** if the offline mini-dictionary in `scripts/medication_validator.py` is insufficient — `register_resolver()` is the hook.
5. **Set `policy_reference`** in the skill's call-site config to point to your hospital's AI documentation policy. The sign-off block surfaces this for Joint Commission readiness.
6. **Wire override signals back.** If the EHR or downstream tool can push back signals about clinician edits/overrides, capture them and append to the audit log. This closes the FDA SaMD real-world performance loop.

## Limits (be honest)

- The PHI scrubber is regex/heuristic. It will not catch every free-text name. Layer with an NER model for high-assurance scrubbing.
- The grounding verifier in `scripts/grounding_check.py` ships with a no-op default that approves whenever cited turns exist and are non-empty. Replace with a model-backed verifier in production via the `verifier` argument.
- The medication mini-dictionary is small and Western-centric. Plug in real RxNorm for clinical use.
- The skill does not perform drug–drug or drug–allergy interaction checking. That belongs in the EHR.
- The skill does not certify a deployment as FDA-cleared, HIPAA-audited, or Joint Commission-accredited. It provides the artifacts those processes need.

## License

Choose one when publishing — Apache 2.0 is a reasonable default for skill packages with regulatory hooks; the explicit patent grant matters here.
