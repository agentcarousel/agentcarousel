#!/usr/bin/env bash
# packaging/update-homebrew.sh
# Render the Homebrew formula for a release from packaging/homebrew/agentcarousel.rb.
#
# The formula installs prebuilt binaries, so every release needs the per-platform sha256
# values. They are read from the release's SHA256SUMS (downloaded from GitHub, or from the
# file named by $SHA256SUMS_FILE, which is how the script is tested offline).
#
# Usage: update-homebrew.sh <tag> <output-formula-path>
#   <output-formula-path> is written in full (not patched), typically the checked-out tap's
#   Formula/agentcarousel.rb; its directory must exist. Use "-" to print to stdout.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <tag> <output-formula-path|->" >&2
  exit 1
fi

TAG="$1"
OUT="$2"
if ! [[ "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "Error: '${TAG}' is not a release tag (expected vMAJOR.MINOR.PATCH)" >&2
  exit 1
fi
VERSION="${TAG#v}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEMPLATE="${SCRIPT_DIR}/homebrew/agentcarousel.rb"
if [[ ! -f "${TEMPLATE}" ]]; then
  echo "Error: formula template not found at ${TEMPLATE}" >&2
  exit 1
fi

SUMS="$(mktemp)"
trap 'rm -f "${SUMS}"' EXIT
if [[ -n "${SHA256SUMS_FILE:-}" ]]; then
  cp "${SHA256SUMS_FILE}" "${SUMS}"
else
  curl -fsSL --retry 3 -o "${SUMS}" \
    "https://github.com/agentcarousel/agentcarousel/releases/download/${TAG}/SHA256SUMS"
fi

TRIPLES=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-unknown-linux-musl
  x86_64-unknown-linux-musl
)

RENDERED="$(sed -e "s|@TAG@|${TAG}|g" -e "s|@VERSION@|${VERSION}|g" "${TEMPLATE}")"
for triple in "${TRIPLES[@]}"; do
  asset="agentcarousel-${TAG}-${triple}.tar.gz"
  sha="$(awk -v f="${asset}" '$2 == f || $2 == "*" f { print $1; exit }' "${SUMS}")"
  if ! [[ "${sha}" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Error: no valid sha256 for ${asset} in SHA256SUMS" >&2
    exit 1
  fi
  key="$(printf '%s' "${triple}" | tr 'a-z-' 'A-Z_')"
  RENDERED="${RENDERED//@SHA256_${key}@/${sha}}"
done

if grep -q '@[A-Z0-9_]*@' <<<"${RENDERED}"; then
  echo "Error: unfilled placeholders remain:" >&2
  grep -n '@[A-Z0-9_]*@' <<<"${RENDERED}" >&2
  exit 1
fi

if [[ "${OUT}" == "-" ]]; then
  printf '%s\n' "${RENDERED}"
else
  if [[ ! -d "$(dirname "${OUT}")" ]]; then
    echo "Error: output directory does not exist: $(dirname "${OUT}")" >&2
    exit 1
  fi
  printf '%s\n' "${RENDERED}" > "${OUT}"
  echo "Wrote ${OUT} for ${TAG}"
fi
