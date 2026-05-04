# CMMC 2.0 Level 2 Gap Analysis — Acme Defense LLC
# Golden Reference: Expected POA&M / Gap Table Structure
# agentcarousel golden file — do not edit without bumping bundle_version
# Last updated: 2026-04-20 | bundle: agentcarousel/cmmc-assessor@1.0.0

---

## SPRS Score Summary

| Metric | Value |
|--------|-------|
| Starting baseline | 110 |
| OTS practices identified | ~18 (estimate from SSP narrative) |
| Estimated SPRS score | **-34 to -20** (range; exact value depends on severity weight assignments) |
| Previous SPRS submission | -47 (14 months ago) |

*Note: DoD Assessment Methodology assigns weights of 5, 3, or 1 per practice based on security impact. Exact score requires full 110-practice walkthrough.*

---

## Domain Gap Summary

| Domain | Practices | OTS Findings | Met | Status |
|--------|-----------|--------------|-----|--------|
| AC — Access Control | 22 | 2–3 | ~19 | Partially Met |
| IA — Identification & Authentication | 11 | 2 | ~9 | Partially Met |
| CM — Configuration Management | 9 | 2 | ~7 | Partially Met |
| IR — Incident Response | 3 | 2 | 1 | Not Met (OTS) |
| AU — Audit & Accountability | 9 | 2 | ~7 | Partially Met |
| SC — System & Comms Protection | 16 | 2 | ~14 | Partially Met |
| SI — System & Info Integrity | 7 | 1 | ~6 | Partially Met |
| RA — Risk Assessment | 3 | 1 | 2 | Partially Met |
| CA — Security Assessment | 4 | 1 | 3 | Partially Met |
| MP — Media Protection | 9 | 1 | ~8 | Partially Met |
| PS — Personnel Security | 2 | 1 | 1 | Partially Met |
| PE — Physical Protection | 6 | 1 | ~5 | Partially Met |
| MA — Maintenance | 6 | 1 | ~5 | Partially Met |
| AT — Awareness & Training | 3 | 1 | 2 | Partially Met |

---

## POA&M Table — Key Findings

| # | Practice ID | Domain | Finding | Severity | Remediation Action | Responsible Party | Target Date |
|---|-------------|--------|---------|----------|--------------------|-------------------|-------------|
| 1 | 3.5.3 | IA | MFA not enforced for on-premises CUI workstations; password-only | High (5-pt) | Deploy MFA to all CUI workstation logons via AD FS or hardware token | IT Security | Q2 |
| 2 | 3.3.1 | AU | Audit log retention is 30 days; minimum required is 90 days | High (5-pt) | Extend Windows Event Log retention to 90 days; evaluate SIEM for central aggregation | IT Operations | Q2 |
| 3 | 3.13.5 | SC | No network segmentation between CUI systems and general IT | High (5-pt) | Implement VLAN separation for CUI assets; deploy micro-segmentation firewall rules | Network Engineering | Q3 |
| 4 | 3.4.1 | CM | No formal configuration baseline for servers; only workstations documented | Medium (3-pt) | Create and approve server baseline configuration documents; align with DISA STIG | IT Operations | Q2 |
| 5 | 3.6.1 | IR | IR plan dated 2022; no tabletop in 18 months; no CISA reporting procedure | Medium (3-pt) | Update IR plan to current requirements; conduct tabletop; add CISA/US-CERT reporting procedure | CISO | Q2 |
| 6 | 3.11.1 | RA | No formal risk assessment in 18 months | Medium (3-pt) | Complete NIST 800-30 risk assessment; schedule annual cadence | Risk Manager | Q2 |
| 7 | 3.9.1 | MP | Removable media policy exists but USB encryption not enforced | Low (1-pt) | Enforce BitLocker-to-Go or equivalent via GPO; update removable media policy | IT Security | Q3 |
| 8 | 3.5.7 | IA | 11 service accounts have non-expiring passwords | Medium (3-pt) | Rotate service account passwords; implement managed service accounts (gMSA) | AD Admins | Q2 |

---

## Evaluator guidance

This golden file is used with `golden_threshold: 0.75–0.80`. The evaluator should:
1. Verify all 14 domain abbreviations appear in the response.
2. Verify SPRS score is numeric and within plausible range (-100 to 110).
3. Verify POA&M table headers match (Practice ID, Domain/Finding, Severity, Remediation, Date).
4. Allow ±20% variation in the specific findings listed (the exact count depends on the assessor's interpretation of partial evidence).
5. Fail if any practice IDs outside 3.1.x–3.14.x appear.
