#!/usr/bin/env bash
# scripts/update-homebrew.sh
# Updates the Homebrew formula with a new tag and its calculated SHA256.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <tag> <path-to-formula>"
  exit 1
fi

TAG="$1"
FORMULA_PATH="$2"

if [[ ! -f "$FORMULA_PATH" ]]; then
  echo "Error: Formula file not found at $FORMULA_PATH"
  exit 1
fi

# Calculate SHA256 of the source archive from GitHub
URL="https://github.com/agentcarousel/agentcarousel/archive/refs/tags/${TAG}.tar.gz"
echo "Fetching archive to calculate checksum: ${URL}"
SHA256=$(curl -fsSL "$URL" | shasum -a 256 | awk '{print $1}')

if [ -z "$SHA256" ]; then
  echo "Error: Failed to calculate SHA256"
  exit 1
fi

echo "New SHA256: ${SHA256}"

# Update the formula file using sed
# We use a temporary backup extension for compatibility between BSD (macOS) and GNU sed
sed -i.bak -E "s|url \"https://github.com/agentcarousel/agentcarousel/archive/refs/tags/v.*\.tar\.gz\"|url \"${URL}\"|" "$FORMULA_PATH"
sed -i.bak -E "s|sha256 \".*\"|sha256 \"${SHA256}\"|" "$FORMULA_PATH"

rm "${FORMULA_PATH}.bak"
echo "Successfully updated ${FORMULA_PATH}"
