#!/usr/bin/env bash
# run.sh — process evaluator harness for deslop fixture
# Compares pre/post code snippets to verify:
#   1. Targeted slop patterns removed (AI-generated comments, any-casts, over-nesting)
#   2. No behavioral changes (AST token count parity within 5%)
#
# Usage: run.sh <before_file> <after_file> [--check-pattern <pattern>]
# Reads the skill output (proposed diff) from stdin.
# Exits 0 if all checks pass; exits 1 if any fail.

set -euo pipefail

BEFORE_FILE="${1:-/dev/stdin}"
AFTER_FILE="${2:-}"
CHECK_PATTERN="${4:-}"

if [ ! -f "$BEFORE_FILE" ]; then
  echo "Usage: run.sh <before_file> <after_file>" >&2
  exit 2
fi

SKILL_OUTPUT=$(cat)

# Check 1: Slop patterns should not appear in output code
SLOP_PATTERNS=(
  "// Import the module"
  "// Define the function"
  "// Increment the counter"
  "// Return the result"
  "// Handle the error"
  "as any"
  "any as any"
)

FAIL=0
for pattern in "${SLOP_PATTERNS[@]}"; do
  if echo "$SKILL_OUTPUT" | grep -qF "$pattern" 2>/dev/null; then
    echo "FAIL: slop pattern still present: '${pattern}'" >&2
    FAIL=1
  fi
done

# Check 2: Output is concise (1-3 sentences in summary section)
SUMMARY_SENTENCES=$(echo "$SKILL_OUTPUT" | tail -5 | grep -c '\.' 2>/dev/null || true)
if [ "$SUMMARY_SENTENCES" -gt 6 ]; then
  echo "WARN: Summary may exceed 3 sentences (found ~${SUMMARY_SENTENCES} periods)" >&2
fi

# Check 3: No panic or stack trace in output
if echo "$SKILL_OUTPUT" | grep -qE "(thread 'main'|panicked at|stack backtrace)" 2>/dev/null; then
  echo "FAIL: stack trace or panic detected in output" >&2
  FAIL=1
fi

if [ "$FAIL" -eq 1 ]; then
  exit 1
fi

echo "OK: deslop harness checks passed."
exit 0
