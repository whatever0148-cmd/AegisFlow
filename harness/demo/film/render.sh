#!/usr/bin/env bash
# Renders artifacts/aegisflow_demo.mp4 from the live console (see render_film.mjs).
# Needs the daemon running (bash harness/console.sh), Google Chrome, node and ffmpeg with libx264.
# ffmpeg comes from $FFMPEG, then PATH, then the ffmpeg-static npm package if it is installed.
# Pass --preview for a fast 30 fps check render into target/film/preview.mp4.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"
NODE="$(command -v node || ls -1 "$HOME"/.nvm/versions/node/*/bin/node 2>/dev/null | tail -n 1)"
[ -n "$NODE" ] || { echo "render.sh: node not found" >&2; exit 127; }
if [ -z "${FFMPEG:-}" ]; then
  FFMPEG="$(command -v ffmpeg || "$NODE" -e "try{console.log(require('ffmpeg-static'))}catch{}" || true)"
fi
[ -n "$FFMPEG" ] && [ -x "$FFMPEG" ] || { echo "render.sh: set FFMPEG to an ffmpeg with libx264 (e.g. npm i ffmpeg-static)" >&2; exit 127; }
FFMPEG="$FFMPEG" "$NODE" harness/demo/film/render_film.mjs "$@"
