#!/usr/bin/env bash
# Sync the vendored assurance-spec schemas from a local checkout of the spec repo.
#
#   scripts/sync-assurance-schemas.sh            # copy schemas and refresh SHA256SUMS
#   scripts/sync-assurance-schemas.sh --check    # exit 1 if the vendored copy differs
#
# SPEC_REPO defaults to an `assurance-spec` checkout beside this repo (or beside its
# worktrees directory). Local-only: CI checks the vendored files against the committed
# SHA256SUMS instead (cargo test).
set -euo pipefail
VERSION="0.1"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [[ -z "${SPEC_REPO:-}" ]]; then
  for candidate in "$ROOT/../assurance-spec" "$ROOT/../../assurance-spec"; do
    if [[ -d "$candidate/schemas/$VERSION" ]]; then
      SPEC_REPO="$candidate"
      break
    fi
  done
fi
SRC="${SPEC_REPO:-}/schemas/$VERSION"
DST="$ROOT/crates/agentcarousel/schemas/assurance/$VERSION"

if [[ ! -d "$SRC" ]]; then
  echo "error: spec schemas not found (set SPEC_REPO to an assurance-spec checkout)" >&2
  exit 2
fi

if [[ "${1:-}" == "--check" ]]; then
  if diff -r --exclude=SHA256SUMS "$SRC" "$DST" >/dev/null; then
    echo "vendored assurance schemas match $SRC"
    exit 0
  fi
  diff -r --exclude=SHA256SUMS "$SRC" "$DST" || true
  echo "error: vendored assurance schemas differ from $SRC; run without --check" >&2
  exit 1
fi

mkdir -p "$DST"
cp "$SRC"/*.schema.json "$DST"/
(cd "$DST" && shasum -a 256 -- *.schema.json > SHA256SUMS)
echo "synced $(find "$DST" -name '*.schema.json' | wc -l | tr -d ' ') schemas into $DST"
