# MSP registry API contract (v0)

Normative product intent lives in [`docs/designs/milestone4-community-registry-design.md`](designs/milestone4-community-registry-design.md). This file is the **implementation-facing contract** for the first deployable service (may live outside this Rust workspace).

## Base URL

- Production (planned): `https://registry.agentcarousel.com` (example only — set at deploy time).
- Path prefix: `/v1`

## Authentication

| Endpoint | Auth |
|----------|------|
| `POST /v1/bundles` | `Authorization: Bearer` with registry write secret (client: `AGENTCAROUSEL_API_TOKEN` env; server: same value) |
| `GET /v1/bundles/{bundle_id}/trust-state` | **None** (public read) |
| `POST /v1/runs` | Same bearer as bundles |
| `GET /v1/bundles/{registry_bundle_id}/manifest` | **Optional** bearer (same as bundles if registry requires auth for reads) |
| `GET /v1/bundles/{registry_bundle_id}/file?path=…` | **Optional** bearer; `path` is the manifest-relative path (URL-encoded) |

## `POST /v1/bundles`

Registers or updates a bundle record. Initial trust state is always `Experimental`.

**Request:** `bundle.manifest.json` as JSON body or `multipart/form-data` with manifest + optional tarball (implementation choice).

**Response 201:**

```json
{
  "bundle_id": "agentcarousel/cmmc-assessor",
  "trust_state": "Experimental",
  "created_at": "2026-04-25T00:00:00Z"
}
```

## `GET /v1/bundles/{registry_bundle_id}/manifest`

Returns the registered `bundle.manifest.json` as JSON (same document clients `POST` to `/v1/bundles`). `registry_bundle_id` is URL-encoded the same way as for trust-state (e.g. slashes as `%2F`).

**Response 200:** JSON body matching the bundle manifest schema.

## `GET /v1/bundles/{registry_bundle_id}/file`

Returns bytes for one artifact listed under `fixtures` or `mocks` in the manifest.

**Query:** `path` — exact `path` field value from the manifest (e.g. `../../skills/foo.yaml` or `skills/hello.yaml`).

**Response 200:** Raw file bytes (`Content-Type: application/octet-stream` recommended).

## `GET /v1/bundles/{bundle_id}/trust-state`

**Public.** `bundle_id` is URL-encoded (e.g. `agentcarousel%2Fcmmc-assessor`).

**Response 200:**

```json
{
  "bundle_id": "agentcarousel/cmmc-assessor",
  "trust_state": "Trusted",
  "last_run_date": "2026-04-24T12:00:00Z",
  "composite_score": 0.94,
  "pass_rate": 1.0,
  "run_count": 5,
  "auditor_ref": "https://agentcarousel.com/auditors/example",
  "policy_version": "msp-policy-2026-04",
  "certified_at": "2026-05-14T00:00:00Z",
  "expires_at": "2027-05-14T00:00:00Z"
}
```

## `POST /v1/runs`

Submits an evidence pack from `agentcarousel export <RUN_ID>` (`.tar.gz`). **Implemented registry:** `multipart/form-data` with file field **`evidence`** (or `file`); optional form fields `registry_bundle_id` / `bundle_id`. (Raw `application/gzip` body is not accepted by the Sprint 1 Hono service.)

**Response 200:**

```json
{
  "run_id": "run-abc123",
  "bundle_id": "agentcarousel/cmmc-assessor",
  "trust_state": "Stable",
  "run_count": 5,
  "composite_score": 0.94,
  "threshold_met": true
}
```

## Trust state FSM

Same transition table as design doc § Trust State FSM (backend-only transitions).

## `.well-known` (attestation verification)

Publish **minisign** public key material at a stable URL, e.g. `https://agentcarousel.com/.well-known/minisign.pub`, cited from trust pages and attestation JSON (per design doc § Public Trust Page).

## CLI follow-on (tracked)

`agentcarousel publish` registers bundles and submits evidence; `agentcarousel bundle pull` downloads manifest + artifacts using the GET endpoints above. `agentcarousel bundle push` as a standalone command remains future work — registry service can ship first with `curl` examples in [`docs/registry-bundle-push-runbook.md`](registry-bundle-push-runbook.md).
