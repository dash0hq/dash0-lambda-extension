#!/usr/bin/env bash
# Usage: bash opt/node/bench-scratch/run.sh   (from the repo root)
# Maps DASH0_MASK_* with the real opt/shared.sh, then runs each script in a fresh process.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
NODE_FLAGS=(--expose-gc --max-semi-space-size=512)

run() { # $1 = mode, $2 = DASH0_MASK_RULES or empty, rest = node args
  local mode="$1" rules="$2"; shift 2
  env -i PATH="$PATH" HOME="$HOME" BENCH_MODE="$mode" ${rules:+DASH0_MASK_RULES="$rules"} \
    bash -c 'source "$1" >/dev/null && map_masking_env && shift && exec node "$@"' bash "$ROOT/opt/shared.sh" "$@"
}

cd "$HERE"
run default "" "${NODE_FLAGS[@]}" bench.js > results-default.json
run custom '[".*email.*"]' "${NODE_FLAGS[@]}" bench.js > results-custom.json
run default "" correctness.js > correctness-default.txt
run custom '[".*email.*"]' correctness.js > correctness-custom.txt
for i in $(seq 1 15); do run default "" cold.js; done > cold-default.jsonl
for i in $(seq 1 15); do run custom '[".*email.*"]' cold.js; done > cold-custom.jsonl
