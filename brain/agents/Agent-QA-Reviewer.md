---
tags:
  - agent
status: verified
latency_ms: 316316.8
last_verified: 2026-10-25T18:15:34Z
git_commit: 3bfc312-dirty
---

# Agent-QA-Reviewer

**Status:** 🟢 verified · **p50:** 316316.8 ms · **commit:** `3bfc312-dirty`

> Runs the no-stub gate, the release test suite and the hardware benchmark. A task only counts as done when its verify command exits 0.

**Implemented in:** `.claude/agents/qa-reviewer.md` · `.claude/hooks/verify_no_stubs.sh`

**Downstream:** [[Core-APFS-Clonefile]] (tests) · [[Core-ANE-Classifier]] (tests) · [[Bench-EndToEnd-Rollback]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T18:15:35Z · bench 2026-10-25T18:15:32Z · tests 2026-10-25T08:10:13Z · commit `3bfc312-dirty`_

| Metric | Measured |
|---|---|
| T10 · No-stub gate: zero TODO/FIXME/unimplemented!/hardcoded metrics | passed · 554.8 ms |
| T11 · Release test suite | passed · 68989.7 ms |
| T12 · Hardware benchmark suite: phase-1 sections of the T17 run | passed · 525.9 ms |
| T17 · Phase-2 hardware benchmark: pool vs cold clonefile, kernel time, reaper, window model (units interleaved) | passed · 245107.8 ms |
| T18 · Phase-2 target gate: 11/11 gated rows of targets_summary pass | passed · 1138.6 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
