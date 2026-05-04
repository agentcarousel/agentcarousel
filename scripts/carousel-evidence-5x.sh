#!/usr/bin/env bash
# Run five carousel eval passes with judge for a certification fixture (API keys required).
# Usage: ./scripts/carousel-evidence-5x.sh fixtures/skills/cmmc-assessor.yaml
set -euo pipefail
FIXTURE="${1:-fixtures/skills/cmmc-assessor.yaml}"
BIN="${AGENTCAROUSEL_BIN:-./target/release/agentcarousel}"
OUT_DIR="${CAROUSEL_OUT_DIR:-reports/carousel-runs}"
mkdir -p "$OUT_DIR"
for i in 1 2 3 4 5; do
  SEED=$((42000 + i))
  echo "=== pass $i seed=$SEED ==="
  "$BIN" eval "$FIXTURE" \
    --execution-mode live \
    --evaluator all \
    --filter '*judge-spot*' \
    --judge \
    --runs 1 \
    --seed "$SEED" \
    --model gemini-2.5-flash \
    --judge-model gemini-2.5-flash \
    --disable-max-tokens \
    --progress \
    --format json | tee "$OUT_DIR/$(basename "$FIXTURE" .yaml)-carousel-${i}.json"
done
