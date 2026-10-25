---
tags:
  - agent
status: failing
latency_ms: 322360.3
last_verified: 2026-10-25T15:38:38Z
git_commit: b3f6810
---

# Agent-QA-Reviewer

**Status:** 🔴 failing · **p50:** 322360.3 ms · **commit:** `b3f6810`

> Runs the no-stub gate, the release test suite and the hardware benchmark. A task only counts as done when its verify command exits 0.

**Implemented in:** `.claude/agents/qa-reviewer.md` · `.claude/hooks/verify_no_stubs.sh`

**Downstream:** [[Core-APFS-Clonefile]] (tests) · [[Core-ANE-Classifier]] (tests) · [[Bench-EndToEnd-Rollback]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| T10 · No-stub gate: zero TODO/FIXME/unimplemented!/hardcoded metrics | passed · 221.2 ms |
| T11 · Release test suite | passed · 65004.2 ms |
| T12 · Hardware benchmark suite: phase-1 sections of the T17 run | passed · 594.5 ms |
| T17 · Phase-2 hardware benchmark: pool vs cold clonefile, kernel time, reaper, window model (units interleaved) | passed · 255034.8 ms |
| T18 · Phase-2 target gate: 11/11 gated rows of targets_summary pass | failed · 1505.6 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
