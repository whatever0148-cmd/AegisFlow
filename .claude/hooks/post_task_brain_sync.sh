#!/usr/bin/env bash
# Post-task gate: release tests -> measured telemetry -> Obsidian brain.
#
#   1. cargo test --release --workspace                       log: artifacts/last_test.log
#   2. aegis-bench, when sources changed since the last bench  -> artifacts/aegis_phase1_telemetry.json
#      (AEGIS_FORCE_BENCH=1 always runs it, AEGIS_SKIP_BENCH=1 never does)
#   3. harness/record_test_results.py                          -> artifacts/aegis_telemetry.json
#   4. harness/sync_obsidian_brain.py                          -> brain/ (Obsidian reloads live)
#
# `--hook` = Claude Code SubagentStop mode: exits 2 on failure so the sub-agent keeps working;
# the stop_hook_active guard prevents re-entry loops.
set -uo pipefail
ROOT="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT" || exit 0
export PATH="$HOME/.cargo/bin:$PATH"

if [ "${1:-}" = "--hook" ]; then
  payload="$(cat || true)"
  if printf '%s' "$payload" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true'; then
    exit 0
  fi
fi

mkdir -p artifacts
LOG=artifacts/last_test.log
t0=$(date +%s)
cargo test --release --workspace >"$LOG" 2>&1
test_status=$?
t1=$(date +%s)

bench_status=0
if [ "${AEGIS_SKIP_BENCH:-0}" != "1" ] && [ "$test_status" -eq 0 ]; then
  stale=1
  if [ -f artifacts/aegis_phase1_telemetry.json ] && [ "${AEGIS_FORCE_BENCH:-0}" != "1" ]; then
    newer=$(find crates policies models Cargo.toml -type f -newer artifacts/aegis_phase1_telemetry.json \
              -not -path '*/target/*' 2>/dev/null | head -1)
    [ -z "$newer" ] && stale=0
  fi
  if [ "$stale" -eq 1 ]; then
    cargo run --release -q -p aegis-bench -- --out artifacts/aegis_phase1_telemetry.json >artifacts/last_bench.log 2>&1
    bench_status=$?
  fi
fi

python3 harness/record_test_results.py --log "$LOG" --exit-code "$test_status" \
  --duration-s "$((t1 - t0))" --bench artifacts/aegis_phase1_telemetry.json --out artifacts/aegis_telemetry.json
python3 harness/sync_obsidian_brain.py

if [ "$test_status" -ne 0 ]; then
  echo "post-task gate: cargo test --release failed (full log: $LOG)" >&2
  grep -E '^(test .* FAILED|error(\[E[0-9]+\])?:|thread .* panicked)' "$LOG" | head -20 >&2
  exit 2
fi
if [ "$bench_status" -ne 0 ]; then
  echo "post-task gate: aegis-bench failed (log: artifacts/last_bench.log)" >&2
  tail -20 artifacts/last_bench.log >&2
  exit 2
fi
echo "post-task gate: release tests passed; telemetry synced into brain/"
