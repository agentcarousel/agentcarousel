# Trust posture (bounded claims)

This document states **non-goals** and **honest limits** for AgentCarousel as an ATF-adjacent evidence and CI tool. It complements [ATF ecosystem crosswalk](atf-ecosystem.md) and [distribution security policy](../distribution/SECURITY.md).

## Non-goals

- **Agent identity passports**: we do not implement DID/VC, wallet binding, or runtime agent authentication beyond what your evaluators and environment provide.
- **Certification of your program**: the CLI produces **artifacts**; mapping those to ATF maturity or organizational certification is the customer’s responsibility.
- **Legal or contractual advice**: fixture fields such as `data_handling` are **declarative labels** for process and filtering—they are not a substitute for privacy review.

## Signing and integrity

- **Today**: evidence tarballs include `run.json`, `fixture_bundle.lock`, `environment_fingerprint.json`, `REDACTION_POLICY.md`, and **`MANIFEST.json`** listing **SHA-256** digests of staged files. That supports **integrity detection** (tampering after export) when the tarball and manifest are stored together.
- **Not shipped in MVP**: cryptographic **signing** of tarballs (for example Sigstore/cosign, or organization KMS-backed signatures). Recommended direction for adopters is to sign **exports/attestations** at release or handoff boundaries—not every CI commit.
- **What signing will mean later**: a signature would attest **who produced** an export bundle and that it **matched** the signed payload at signing time; it does not by itself prove that every case in the run was “safe” in production.

## Redaction

- Trace and log paths in the runner apply **heuristic scrubbing** of common secret patterns (tokens, keys). That is **best-effort**, not a formal data-loss prevention (DLP) engine.
- `export` currently writes a **minimal stub** `REDACTION_POLICY.md` string. Treat it as a **placeholder** until a versioned **redaction profile** (markdown or machine-readable) is defined, tested, and wired to export.

## Training vs product enforcement

**Org policy (training):** third-party **security awareness** content (for example Trail of Bits’ curated [`security-awareness`](https://github.com/trailofbits/skills-curated/tree/main/plugins/security-awareness) plugin) is appropriate for **humans and agents as readers**—it is **not** a substitute for product-side redaction, DLP, or export policy enforcement.

**Product slot (enforcement):** a future **redaction profile** (fields, defaults, automated tests) would be a **separate contract** from “agents read a skill.” Until then, the export stub is not an operational data-governance control.

## Vulnerability reporting

For undisclosed security issues, follow [distribution/SECURITY.md](../distribution/SECURITY.md): do not use a public issue for the initial report.
