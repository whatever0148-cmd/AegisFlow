---
tags:
  - core
status: verified
latency_ms: 0.105292
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1053 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.0978 ms vs 0.1665 ms p50 (1.7x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1053 ms vs 1.006 ms p50 (9.6x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1375 ms vs 10.370 ms p50 (75.4x) · hits 60 / misses 0 |
| branch · aegisflow_src (108 files) · pool warm hit vs cold clonefile | 0.1148 ms vs 1.364 ms p50 (11.9x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 155.0 µs = 12.917 µs/entry · sys share of wall 1.001 |
| kernel time per clonefile · small (138 entries) | sys p50 1050.0 µs = 7.609 µs/entry · sys share of wall 0.9992 |
| kernel time per clonefile · medium (1058 entries) | sys p50 10061.0 µs = 9.509 µs/entry · sys share of wall 0.9920 |
| kernel time per clonefile · aegisflow_src (159 entries) | sys p50 1365.0 µs = 8.585 µs/entry · sys share of wall 0.9939 |
| cold clonefile wall cost per tree entry (fit) | 9.931 µs/entry + -167.4 µs |
| kernel sys time per tree entry (fit) | 9.605 µs/entry + -124.9 µs |
| standby replenish (refill thread) · tiny | p50 0.1812 ms · p95 0.2117 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.135 ms · p95 1.351 ms · n=300 |
| standby replenish (refill thread) · medium | p50 8.609 ms · p95 10.630 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.393 ms · p95 2.467 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6161 ms · p95 0.6921 ms · n=400 |
| background reap per detached tree · small | p50 5.096 ms · p95 5.412 ms · n=300 |
| background reap per detached tree · medium | p50 37.644 ms · p95 60.218 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 8.022 ms · p95 10.031 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0978-0.1375 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.167 ms at 9 files and 10.370 ms at 1025 files, with getrusage ru_stime of 7.61-12.92 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.135 ms small, 8.609 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 12.428 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.605 ms, rollback 1.286 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.129 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.467 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.179 ms, cpuOnly 0.220 ms). One call after a 3.0 ms gap takes 0.344 ms with the CPU busy in the gap and 0.706 ms with it idle (cpuOnly 0.218 / 1.252 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.64, and faster with an idle CPU, speedup 1.77). The end-to-end ane_score stage, one call per transaction, has p50 0.483 ms, 2.70x the back-to-back p50 and 1.41x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (1-min load average 11.47 at end above the busy threshold 4); only a quiet full-scale run decides the phase result.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
