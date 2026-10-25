#!/usr/bin/env bash
# Build (or check) the CoreML FP16 context-window risk model in models/.
#
#   bash harness/build_model.sh                 full build: simulate, train (PyTorch, CPU), export, compile,
#                                               parity, placement + latency -> models/
#   bash harness/build_model.sh --verify-only   check existing artifacts (shapes, layout metadata, parity,
#                                               MLComputePlan placement, packages load); no training
#   bash harness/build_model.sh --sweep         architecture search -> models/arch_sweep.json
#   bash harness/build_model.sh --sweep --sweep-only A,B
#                                               re-measure the named candidates and merge them into
#                                               models/arch_sweep.json; every row keeps the measured_at
#                                               of its own run (top level: last_run_at + runs)
#
# Unit checks of the builder helpers (no training, export or probing):
#   .venv/bin/python models/tools/test_build_model.py
#
# Creates .venv with coremltools, numpy and torch on first use (torch is only needed to train).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if [ ! -x .venv/bin/python ]; then
  /usr/bin/python3 -m venv .venv
  .venv/bin/python -m pip install --quiet --upgrade pip
  .venv/bin/python -m pip install --quiet coremltools numpy
fi
needs_torch=1
for arg in "$@"; do
  if [ "$arg" = "--verify-only" ] || [ "$arg" = "--sweep" ]; then needs_torch=0; fi
done
if [ "$needs_torch" = 1 ] && ! .venv/bin/python -c "import torch" >/dev/null 2>&1; then
  .venv/bin/python -m pip install --quiet torch
fi
exec .venv/bin/python models/build_model.py "$@"
