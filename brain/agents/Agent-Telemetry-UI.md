---
tags:
  - agent
status: verified
latency_ms: 322.5
last_verified: 2026-10-26T06:51:41Z
git_commit: 7cb1a10-dirty
---

# Agent-Telemetry-UI

**Status:** 🟢 verified · **p50:** 322.5 ms · **commit:** `7cb1a10-dirty`

> Publishes measured telemetry into this vault (frontmatter, canvas, log) and the AG Grid / Bryntum row exports from the PayPal adapter.

**Implemented in:** `.claude/agents/telemetry-ui.md` · `harness/sync_obsidian_brain.py` · `.claude/hooks/post_task_brain_sync.sh`

**Downstream:** [[Adapter-PayPal-Commerce]] (dashboard rows) · [[Bench-ANE-FP16]] (publishes)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T06:51:41Z · bench 2026-10-26T06:51:38Z · tests 2026-10-25T08:10:13Z · commit `7cb1a10-dirty`_

| Metric | Measured |
|---|---|
| T13 · Sync telemetry into the Obsidian brain | passed · 156.4 ms |
| T20 · Sync phase-2 telemetry into the Obsidian brain | passed · 166.1 ms |

> `latency_ms` for an agent = summed wall time of its tasks' verify commands (harness/run_tasks.py).
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
