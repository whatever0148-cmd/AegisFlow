---
tags:
  - agent
status: verified
latency_ms: 410172.0
last_verified: 2026-10-26T06:51:41Z
git_commit: 7cb1a10-dirty
---

# Agent-QA-Reviewer

**Status:** 🟢 verified · **p50:** 410172.0 ms · **commit:** `7cb1a10-dirty`

> Runs the no-stub gate, the release test suite and the hardware benchmark. A task only counts as done when its verify command exits 0.

**Implemented in:** `.claude/agents/qa-reviewer.md` · `.claude/hooks/verify_no_stubs.sh`

**Downstream:** [[Core-APFS-Clonefile]] (tests) · [[Core-ANE-Classifier]] (tests) · [[Bench-EndToEnd-Rollback]] (gates on)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| T10 · No-stub gate: zero TODO/FIXME/unimplemented!/hardcoded metrics | passed · 232.1 ms |
| T11 · Release test suite | passed · 96728.1 ms |
| T12 · Hardware benchmark suite: phase-1 sections of the T17 run | passed · 571.6 ms |
| T17 · Phase-2 hardware benchmark: pool vs cold clonefile, kernel time, reaper, window model (units interleaved) | passed · 311066.9 ms |
| T18 · Phase-2 target gate: 11/11 gated rows of targets_summary pass | passed · 1573.3 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
