#!/usr/bin/env bash
# check-contains.sh PATTERN1 [PATTERN2 ...]
# Reads {"case":..., "result":...} JSON from stdin.
# Passes (exit 0, {"passed":true}) if final_output matches ALL patterns (case-insensitive ERE).
set -euo pipefail

input=$(cat)
output=$(printf '%s' "$input" | python3 -c "
import json, sys
d = json.load(sys.stdin)
print(d.get('result', {}).get('trace', {}).get('final_output', ''))
" 2>/dev/null || true)

for pattern in "$@"; do
  if ! printf '%s' "$output" | grep -qiE "$pattern"; then
    printf '{"passed":false}\n'
    exit 0
  fi
done

printf '{"passed":true}\n'
