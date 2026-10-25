---
tags:
  - benchmark
status: verified
latency_ms: 0.004542
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
---

# Bench-Cedar-Eval

**Status:** 🟢 verified · **p50:** 0.0045 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> Cedar `is_authorized` latency and the policy decision matrix (expected vs actual).

**Implemented in:** `crates/aegis-bench/src/flows.rs`

**Upstream:** [[Core-Cedar-Rules]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| is_authorized | p50 4.542 µs · p95 7.084 µs · n=20000 |
| build request + authorize | p50 7.792 µs · p95 10.708 µs · n=20000 |
| ✔ speculative commit, risk 12 | Allow (permit-low-risk) |
| ✔ commit with unredacted sensitive fields, risk 5 | Deny (default deny) |
| ✔ high-impact commit without approval, risk 61 | Deny (forbid-high-impact-without-team-approval) |
| ✔ high-impact commit with team approval, risk 61 | Allow (permit-approved-high-impact-commit) |
| ✔ order $42.00, risk 20 | Allow (permit-low-risk) |
| ✔ order $120.00, receipt unverified, risk 20 | Deny (forbid-order-over-50-usd-without-receipt) |
| ✔ order $120.00, receipt verified, risk 20 | Allow (permit-low-risk) |
| ✔ order $20.00, risk 80 | Deny (default deny) |
| ✔ branch, risk 99 with sensitive fields | Allow (permit-branch-and-rollback) |
| ✔ rollback, risk 99 | Allow (permit-branch-and-rollback) |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
