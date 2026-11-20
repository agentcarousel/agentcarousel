# AIUC-1 crosswalk

`agc` bundles a crosswalk for [AIUC-1](https://www.aiuc-1.com), the AI agent standard published by the Artificial Intelligence Underwriting Company (AIUC). Tag fixture cases with `aiuc-1:<control-id>` and `agc compliance report --framework aiuc-1` scores your run history against it.

**Pinned version:** AIUC-1 2026-Q4 (release of 2026-10-15). 50 active requirements. Retired IDs (C011, C012, D002, E007, E014) are not included.

## What the crosswalk contains, and what it doesn't

- **Control IDs and links only.** AIUC's site licenses its content for personal, noncommercial use, so `agc` does not copy AIUC requirement text, control activities, or the risk and attack taxonomies. Each entry has the AIUC-1 ID, a short label in AgentCarousel's own words, and a link to the requirement on standard.aiuc-1.com. Read the requirement there.
- **Not affiliated.** AgentCarousel is not affiliated with or endorsed by AIUC. A `Satisfied` row means your tagged cases met `agc`'s evidence rule (at least 3 cases, effectiveness ≥ 0.80). It is not an AIUC-1 certification or an AIUC evaluation, and it is not graded with AIUC's methodology.
- **Output-level evidence only.** `agc` records tool calls from fixture expectations, not from the live agent (see the [AgentCarousel Specification charter](https://github.com/agentcarousel/assurance-spec/blob/main/CHARTER.md), §6). Requirements about unauthorized actions and unsafe tool calls (B006, D003) are therefore treated as procedural until observed tool calls ship.

## How controls are classified

Each requirement is either **behavioral** (agent outputs can show whether it holds, so tagged cases count as evidence) or **procedural** (a policy, process, or product capability; tagged cases never count). The split is AgentCarousel's own judgment, and it errs toward procedural. 15 of the 50 are behavioral.

The five third-party testing requirements (A009, B001, C010, D004, E018) are procedural: the control is appointing an independent tester and keeping a quarterly cadence. An evidence bundle that an independent party produces with `agc` is evidence *for* those controls.

## Tagging cases

```yaml
- id: my-agent/no-invented-findings
  tags:
    - aiuc-1:D001
```

```bash
agc compliance report --framework aiuc-1 --skill my-agent
agc compliance gaps --framework aiuc-1          # behavioral controls with no cases, with links
```

`agc export` writes `compliance_aiuc-1.md` into the evidence bundle only when the scored runs contain at least one `aiuc-1:` tag. Bundles that don't use the crosswalk don't mention AIUC-1.

To change a label or classification locally, put a JSON array of controls with the same `tag` in `./frameworks/*.json` or `~/.agentcarousel/frameworks/*.json`; entries there replace the bundled ones.

## Requirements

| ID | Label (AgentCarousel wording) | Evidence | Notes |
|---|---|---|---|
| [A001](https://standard.aiuc-1.com/data-and-privacy/establish-data-use-policy) | Rules for how customer inputs may be used | procedural |  |
| [A002](https://standard.aiuc-1.com/data-and-privacy/define-output-rights) | Ownership and permitted use of agent outputs | procedural |  |
| [A003](https://standard.aiuc-1.com/data-and-privacy/implement-contextual-data-safeguards) | Least-privilege data access for the agent | procedural |  |
| [A004](https://standard.aiuc-1.com/data-and-privacy/protect-ip-trade-secrets) | Safeguarding confidential business information | procedural |  |
| [A005](https://standard.aiuc-1.com/data-and-privacy/prevent-cross-customer-data-exposure) | Isolation between different customers' data | behavioral |  |
| [A006](https://standard.aiuc-1.com/data-and-privacy/prevent-pii-leakage) | Personal data does not leak in outputs | behavioral |  |
| [A007](https://standard.aiuc-1.com/data-and-privacy/prevent-ip-violations) | Outputs avoid infringing others' IP | behavioral |  |
| [A008](https://standard.aiuc-1.com/data-and-privacy/prevent-secrets-leakage) | Credentials and secrets stay private | behavioral |  |
| [A009](https://standard.aiuc-1.com/data-and-privacy/3rd-party-testing-for-data-and-privacy-risk) | Independent data and privacy testing, quarterly | procedural | Procedural: the control is appointing an independent tester and keeping the quarterly cadence. An evidence bundle produced by an independent party with agc is evidence for this control. |
| [B001](https://standard.aiuc-1.com/security/test-adversarial-robustness) | Independent adversarial security testing, quarterly | procedural | Procedural: the control is appointing an independent tester and keeping the quarterly cadence. An evidence bundle produced by an independent party with agc is evidence for this control. |
| [B002](https://standard.aiuc-1.com/security/detect-adversarial-input) | Adversarial inputs are detected | behavioral | Output-level evidence only: cases can show the agent refuses or flags adversarial input in its responses. |
| [B003](https://standard.aiuc-1.com/security/limit-technical-over-disclosure) | Limits on disclosing system internals | procedural |  |
| [B004](https://standard.aiuc-1.com/security/prevent-ai-endpoint-scraping) | Protection against endpoint scraping | procedural |  |
| [B005](https://standard.aiuc-1.com/security/implement-real-time-input-filtering) | Runtime filtering of inputs | behavioral | Output-level evidence only: cases show the effect of filtering on responses, not the filter component itself. |
| [B006](https://standard.aiuc-1.com/security/enforce-contextual-access-controls) | Agent acts only within its authorized scope | procedural | Procedural for now: needs observed tool calls, which agc does not yet record (tool calls are synthesized from fixture expectations). Not claimable from current agc evidence. |
| [B007](https://standard.aiuc-1.com/security/enforce-ai-access-privileges) | User access rights to the AI system | procedural |  |
| [B008](https://standard.aiuc-1.com/security/protect-model-deployment-environment) | Securing the deployment environment | procedural |  |
| [B009](https://standard.aiuc-1.com/security/limit-output-over-exposure) | Outputs reveal no more than necessary | behavioral |  |
| [B010](https://standard.aiuc-1.com/security/promote-secure-code-patterns) | Generated code follows secure patterns | behavioral |  |
| [C001](https://standard.aiuc-1.com/safety/define-ai-risk-taxonomy) | Documented AI risk categories | procedural |  |
| [C002](https://standard.aiuc-1.com/safety/conduct-pre-deployment-testing) | Testing before deployment | procedural | Procedural: pre-deployment testing is a process. An agc evidence bundle is evidence for it; the control is the process itself. |
| [C003](https://standard.aiuc-1.com/safety/prevent-harmful-outputs) | Outputs contain no harmful content | behavioral |  |
| [C004](https://standard.aiuc-1.com/safety/prevent-out-of-scope-outputs) | Agent stays within its intended task | behavioral |  |
| [C005](https://standard.aiuc-1.com/safety/prevent-other-high-risk-outputs) | High-risk outputs specific to the agent's domain are prevented | behavioral |  |
| [C006](https://standard.aiuc-1.com/safety/prevent-output-vulnerabilities) | Outputs are safe for downstream systems | behavioral |  |
| [C007](https://standard.aiuc-1.com/safety/flag-high-risk-recommendations) | High-risk outputs are routed to human review | procedural | Procedural: routing to human review is a workflow design; cases can show the agent flags high-risk content, which supports but does not satisfy this control. |
| [C008](https://standard.aiuc-1.com/safety/monitor-ai-risk-categories) | Ongoing monitoring of risk categories | procedural |  |
| [C009](https://standard.aiuc-1.com/safety/collect-real-time-feedback) | Live user feedback and intervention | procedural | Procedural: feedback and intervention are product capabilities, not agent outputs. |
| [C010](https://standard.aiuc-1.com/safety/3rd-party-testing-for-harmful-outputs) | Independent safety testing, quarterly | procedural | Procedural: the control is appointing an independent tester and keeping the quarterly cadence. An evidence bundle produced by an independent party with agc is evidence for this control. |
| [D001](https://standard.aiuc-1.com/reliability/prevent-hallucinated-outputs) | Hallucinated content is prevented | behavioral |  |
| [D003](https://standard.aiuc-1.com/reliability/restrict-unsafe-tool-calls) | Unsafe tool calls are restricted | procedural | Procedural for now: needs observed tool calls, which agc does not yet record (tool calls are synthesized from fixture expectations). Not claimable from current agc evidence. |
| [D004](https://standard.aiuc-1.com/reliability/3rd-party-testing-of-tool-calls) | Independent reliability testing, quarterly | procedural | Procedural: the control is appointing an independent tester and keeping the quarterly cadence. An evidence bundle produced by an independent party with agc is evidence for this control. |
| [E001](https://standard.aiuc-1.com/accountability/ai-failure-plan-for-security-breaches) | Incident plan for security breaches | procedural |  |
| [E002](https://standard.aiuc-1.com/accountability/ai-failure-plan-for-harmful-outputs) | Incident plan for harmful outputs | procedural |  |
| [E003](https://standard.aiuc-1.com/accountability/ai-failure-plan-for-hallucinations) | Incident plan for hallucinations | procedural |  |
| [E004](https://standard.aiuc-1.com/accountability/assign-accountability) | Named owners accountable for the AI system | procedural |  |
| [E005](https://standard.aiuc-1.com/accountability/assess-cloud-vs-on-prem-processing) | Documented security of data storage | procedural |  |
| [E006](https://standard.aiuc-1.com/accountability/conduct-vendor-due-diligence) | Due diligence on vendors and models | procedural |  |
| [E008](https://standard.aiuc-1.com/accountability/review-internal-processes) | Periodic review of internal processes | procedural |  |
| [E009](https://standard.aiuc-1.com/accountability/monitor-3rd-party-access) | Oversight of third-party access | procedural |  |
| [E010](https://standard.aiuc-1.com/accountability/establish-ai-acceptable-use-policy) | Acceptable use policy for the AI system | procedural |  |
| [E011](https://standard.aiuc-1.com/accountability/record-processing-locations) | Records of where processing happens | procedural |  |
| [E012](https://standard.aiuc-1.com/accountability/document-regulatory-compliance) | Documentation of regulatory compliance | procedural |  |
| [E013](https://standard.aiuc-1.com/accountability/implement-quality-management-system) | Quality management system | procedural |  |
| [E015](https://standard.aiuc-1.com/accountability/log-model-activity) | Activity logging for the AI system | procedural |  |
| [E016](https://standard.aiuc-1.com/accountability/implement-ai-disclosure-mechanisms) | Users are told they are interacting with AI | procedural | Procedural: a case can show the agent says it is an AI when asked, but the disclosure mechanism is product design. |
| [E017](https://standard.aiuc-1.com/accountability/document-system-transparency-policy) | Transparency policy for the system | procedural |  |
| [E018](https://standard.aiuc-1.com/accountability/3rd-party-testing-for-accountability-risk) | Independent accountability testing, quarterly | procedural | Procedural: the control is appointing an independent tester and keeping the quarterly cadence. An evidence bundle produced by an independent party with agc is evidence for this control. |
| [F001](https://standard.aiuc-1.com/society/prevent-ai-cyber-misuse) | No assistance with cyberattacks | behavioral |  |
| [F002](https://standard.aiuc-1.com/society/prevent-catastrophic-misuse) | No assistance with catastrophic misuse | behavioral |  |
