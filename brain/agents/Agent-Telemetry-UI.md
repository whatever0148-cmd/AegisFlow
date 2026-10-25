---
tags:
  - agent
status: verified
latency_ms: 417.9
last_verified: 2026-10-25T15:38:39Z
git_commit: b3f6810
---

# Agent-Telemetry-UI

**Status:** 🟢 verified · **p50:** 417.9 ms · **commit:** `b3f6810`

> Publishes measured telemetry into this vault (frontmatter, canvas, log) and the AG Grid / Bryntum row exports from the PayPal adapter.

**Implemented in:** `.claude/agents/telemetry-ui.md` · `harness/sync_obsidian_brain.py` · `.claude/hooks/post_task_brain_sync.sh`

**Downstream:** [[Adapter-PayPal-Commerce]] (dashboard rows) · [[Bench-ANE-FP16]] (publishes)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| T13 · Sync telemetry into the Obsidian brain | passed · 177.9 ms |
| T20 · Sync phase-2 telemetry into the Obsidian brain | passed · 240.0 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
