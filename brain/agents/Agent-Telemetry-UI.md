---
tags:
  - agent
status: verified
latency_ms: 350.3
last_verified: 2026-10-25T17:28:26Z
git_commit: 9f1f777-dirty
---

# Agent-Telemetry-UI

**Status:** 🟢 verified · **p50:** 350.3 ms · **commit:** `9f1f777-dirty`

> Publishes measured telemetry into this vault (frontmatter, canvas, log) and the AG Grid / Bryntum row exports from the PayPal adapter.

**Implemented in:** `.claude/agents/telemetry-ui.md` · `harness/sync_obsidian_brain.py` · `.claude/hooks/post_task_brain_sync.sh`

**Downstream:** [[Adapter-PayPal-Commerce]] (dashboard rows) · [[Bench-ANE-FP16]] (publishes)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T17:28:26Z · bench 2026-10-25T17:28:23Z · tests 2026-10-25T08:10:13Z · commit `9f1f777-dirty`_

| Metric | Measured |
|---|---|
| T13 · Sync telemetry into the Obsidian brain | passed · 168.8 ms |
| T20 · Sync phase-2 telemetry into the Obsidian brain | passed · 181.5 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
