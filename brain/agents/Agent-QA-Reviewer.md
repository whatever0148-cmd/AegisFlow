---
tags:
  - agent
status: failing
latency_ms: 88783.7
last_verified: 2026-10-26T03:52:55Z
git_commit: 4403ab4-dirty
---

# Agent-QA-Reviewer

**Status:** 🔴 failing · **p50:** 88783.7 ms · **commit:** `4403ab4-dirty`

> Runs the no-stub gate, the release test suite and the hardware benchmark. A task only counts as done when its verify command exits 0.

**Implemented in:** `.claude/agents/qa-reviewer.md` · `.claude/hooks/verify_no_stubs.sh`

**Downstream:** [[Core-APFS-Clonefile]] (tests) · [[Core-ANE-Classifier]] (tests) · [[Bench-EndToEnd-Rollback]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| T10 · No-stub gate: zero TODO/FIXME/unimplemented!/hardcoded metrics | passed · 247.9 ms |
| T11 · Release test suite | passed · 6638.2 ms |
| T12 · Hardware benchmark suite: phase-1 sections of the T17 run | passed · 598.0 ms |
| T17 · Phase-2 hardware benchmark: pool vs cold clonefile, kernel time, reaper, window model (units interleaved) | passed · 80213.5 ms |
| T18 · Phase-2 target gate: 11/11 gated rows of targets_summary pass | failed · 1086.1 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
