#!/usr/bin/env bash
# Build and start the Aegis Console daemon, wait until it answers, and open it in the browser.
#
#   bash harness/console.sh            # http://127.0.0.1:4173 (AEGIS_CONSOLE_PORT overrides)
#
# The daemon runs in the foreground of this script; Ctrl-C (or SIGTERM) stops it gracefully
# (pending approval branches rolled back, warm pool dropped, reaper drained); a second Ctrl-C exits
# at once. Its log is also written to artifacts/aegis-console.log.
#
# Everything here uses 127.0.0.1, not "localhost": macOS resolves localhost to [::1] first, and
# another local process could be listening there (the daemon binds [::1] too when it can, and warns
# when it cannot).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
PORT="${AEGIS_CONSOLE_PORT:-4173}"
URL="http://127.0.0.1:${PORT}"

if curl -fsS --max-time 2 "$URL/api/health" >/dev/null 2>&1; then
  echo "Something already answers on $URL; not starting a second daemon." >&2
  exit 1
fi

cargo build --release -p aegis-daemon
mkdir -p artifacts
LOG="artifacts/aegis-console.log"
"$ROOT/target/release/aegis-daemon" serve > >(tee "$LOG") 2>&1 &
PID=$!
trap 'kill -INT "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true' INT TERM

for _ in $(seq 1 120); do
  if curl -fsS --max-time 2 "$URL/api/health" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "aegis-daemon exited during startup; see $LOG" >&2
    wait "$PID" || true
    exit 1
  fi
  sleep 0.5
done
if ! curl -fsS --max-time 2 "$URL/api/health" >/dev/null 2>&1; then
  echo "aegis-daemon did not answer $URL/api/health within 60 s; see $LOG" >&2
  kill -INT "$PID" 2>/dev/null || true
  exit 1
fi
echo "Aegis Console: $URL (Ctrl-C to stop)"
open "$URL" || echo "open $URL in a browser"
wait "$PID"
