#!/usr/bin/env bash
# Build (or with --verify-only, check) the CoreML FP16 risk model in models/.
# Creates .venv with coremltools on first use.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if [ ! -x .venv/bin/python ]; then
  /usr/bin/python3 -m venv .venv
  .venv/bin/python -m pip install --quiet --upgrade pip
  .venv/bin/python -m pip install --quiet coremltools numpy
fi
exec .venv/bin/python models/build_model.py "$@"
