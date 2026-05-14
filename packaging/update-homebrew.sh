#!/usr/bin/env bash
# packaging/update-homebrew.sh
# Updates the Homebrew formula with a new tag and its calculated SHA256.
#
# Tap override pattern: this script treats packaging/homebrew/agentcarousel.rb as the
# canonical in-repo copy. It always updates that file first, then applies the same
# patch to any additional path supplied as the second argument (typically the checked-out
# tap repo during CI: homebrew-tap/Formula/agentcarousel.rb).
set -euo pipefail

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  echo "Usage: $0 <tag> [path-to-tap-formula]"
  exit 1
fi

TAG="$1"
TAP_FORMULA_PATH="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INREPO_FORMULA="${SCRIPT_DIR}/homebrew/agentcarousel.rb"

if [[ ! -f "$INREPO_FORMULA" ]]; then
  echo "Error: in-repo formula not found at ${INREPO_FORMULA}"
  exit 1
fi

if [[ -n "$TAP_FORMULA_PATH" ]] && [[ ! -f "$TAP_FORMULA_PATH" ]]; then
  echo "Error: tap formula not found at ${TAP_FORMULA_PATH}"
  exit 1
fi

URL="https://github.com/agentcarousel/agentcarousel/archive/refs/tags/${TAG}.tar.gz"
echo "Fetching archive to calculate checksum: ${URL}"
SHA256=$(curl -fsSL "$URL" | shasum -a 256 | awk '{print $1}')

if [ -z "$SHA256" ]; then
  echo "Error: Failed to calculate SHA256"
  exit 1
fi

echo "New SHA256: ${SHA256}"

patch_formula() {
  local path="$1"
  sed -i.bak -E "s|url \"https://github.com/agentcarousel/agentcarousel/archive/refs/tags/v.*\.tar\.gz\"|url \"${URL}\"|" "$path"
  sed -i.bak -E "s|sha256 \".*\"|sha256 \"${SHA256}\"|" "$path"
  rm "${path}.bak"
  echo "Updated ${path}"
}

patch_formula "$INREPO_FORMULA"

if [[ -n "$TAP_FORMULA_PATH" ]]; then
  patch_formula "$TAP_FORMULA_PATH"
fi
