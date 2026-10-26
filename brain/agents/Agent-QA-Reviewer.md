---
tags:
  - agent
status: verified
latency_ms: 336813.6
last_verified: 2026-10-26T07:03:04Z
git_commit: c3ad550
---

# Agent-QA-Reviewer

**Status:** 🟢 verified · **p50:** 336813.6 ms · **commit:** `c3ad550`

> Runs the no-stub gate, the release test suite and the hardware benchmark. A task only counts as done when its verify command exits 0.

**Implemented in:** `.claude/agents/qa-reviewer.md` · `.claude/hooks/verify_no_stubs.sh`

**Downstream:** [[Core-APFS-Clonefile]] (tests) · [[Core-ANE-Classifier]] (tests) · [[Bench-EndToEnd-Rollback]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| T10 · No-stub gate: zero TODO/FIXME/unimplemented!/hardcoded metrics | passed · 236.8 ms |
| T11 · Release test suite | passed · 91753.1 ms |
| T12 · Hardware benchmark suite: phase-1 sections of the T17 run | passed · 1239.2 ms |
| T17 · Phase-2 hardware benchmark: pool vs cold clonefile, kernel time, reaper, window model (units interleaved) | passed · 241952.8 ms |
| T18 · Phase-2 target gate: 11/11 gated rows of targets_summary pass | passed · 1631.7 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
