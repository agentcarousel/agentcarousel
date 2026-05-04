#!/usr/bin/env bash
# count-blocks.sh — process evaluator harness for terraform-sentinel-scaffold and deslop fixtures
# Usage: count-blocks.sh <block_type_1> [block_type_2 ...]
# Reads the skill output from stdin.
# Exits 0 if all specified block types appear at least once; exits 1 otherwise.
# Also used by the deslop fixture to verify pre/post code snippet equivalence.

set -euo pipefail

EXPECTED_TYPES=("$@")
INPUT=$(cat)

if [ ${#EXPECTED_TYPES[@]} -eq 0 ]; then
  echo "Usage: count-blocks.sh <block_type_1> [block_type_2 ...]" >&2
  exit 2
fi

FAIL=0
for block_type in "${EXPECTED_TYPES[@]}"; do
  count=$(echo "$INPUT" | grep -c "\"${block_type}\"" 2>/dev/null || true)
  if [ "$count" -lt 1 ]; then
    echo "FAIL: block type '${block_type}' not found in output" >&2
    FAIL=1
  else
    echo "OK: block type '${block_type}' found (${count} occurrence(s))"
  fi
done

if [ "$FAIL" -eq 1 ]; then
  exit 1
fi

echo "All expected block types found."
exit 0
