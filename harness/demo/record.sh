#!/usr/bin/env bash
# Records the console showcase: artifacts/aegisflow_demo.mp4 plus three high-resolution screenshots.
# Needs the daemon running (bash harness/console.sh) and Google Chrome installed. Nothing is staged:
# the recorder clicks the real console and the daemon runs every scenario for real.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
URL="${AEGIS_CONSOLE_URL:-http://127.0.0.1:${AEGIS_CONSOLE_PORT:-4173}/}"
NODE="$(command -v node || ls -1 "$HOME"/.nvm/versions/node/*/bin/node 2>/dev/null | tail -n 1)"
[ -n "$NODE" ] || { echo "record.sh: node not found" >&2; exit 127; }
mkdir -p target
swiftc -O harness/demo/frames_to_mp4.swift -o target/frames_to_mp4 2>/dev/null
"$NODE" harness/demo/record_demo.mjs --url "$URL" --out artifacts --work target/demo-recording
target/frames_to_mp4 target/demo-recording/frames artifacts/aegisflow_demo.mp4 60
