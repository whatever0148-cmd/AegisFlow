---
tags:
  - core
status: verified
latency_ms: 0.114
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
crate: aegis_storage
---

# Core-APFS-Clonefile

**Status:** 🟢 verified · **p50:** 0.1140 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> Copy-on-write workspace branches: `clonefile(src, dst, CLONE_NOFOLLOW | CLONE_NOOWNERCOPY)` clones the live tree; its cost is O(entries) inside the kernel. `WarmBranchPool` keeps pre-cloned standbys (refilled off the caller's path, invalidated by commits, generation bumps and FSEvents) and hands one out with a single `renameatx_np(RENAME_EXCL)`. Branches dropped without a commit are detached into trash and deleted by a background reaper.

**Implemented in:** `crates/aegis-storage/src/lib.rs` · `crates/aegis-storage/src/pool.rs` · `crates/aegis-storage/src/reaper.rs` · `crates/aegis-storage/src/fsevents.rs` · `crates/aegis-storage/src/sys.rs`

**Upstream:** [[Agent-Core-Builder]] (builds) · [[Agent-QA-Reviewer]] (tests)
**Downstream:** [[Adapter-Nebius-Nemotron]] (verifies patches in CoW branch) · [[Adapter-Switch-MCP]] (branch tools) · [[Bench-APFS-Latency]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 9 passed, 0 failed (2 suites) |
| branch · tiny (9 files) · pool warm hit vs cold clonefile | 0.1067 ms vs 0.1759 ms p50 (1.6x) · hits 400 / misses 0 |
| branch · small (129 files) · pool warm hit vs cold clonefile | 0.1140 ms vs 1.034 ms p50 (9.1x) · hits 300 / misses 0 |
| branch · medium (1025 files) · pool warm hit vs cold clonefile | 0.1497 ms vs 7.321 ms p50 (48.9x) · hits 60 / misses 0 |
| branch · aegisflow_src (113 files) · pool warm hit vs cold clonefile | 0.1109 ms vs 1.322 ms p50 (11.9x) · hits 200 / misses 0 |
| kernel time per clonefile · tiny (12 entries) | sys p50 182.0 µs = 15.167 µs/entry · sys share of wall 0.9945 |
| kernel time per clonefile · small (138 entries) | sys p50 1041.0 µs = 7.543 µs/entry · sys share of wall 0.9948 |
| kernel time per clonefile · medium (1058 entries) | sys p50 7354.0 µs = 6.951 µs/entry · sys share of wall 0.9987 |
| kernel time per clonefile · aegisflow_src (164 entries) | sys p50 1285.0 µs = 7.835 µs/entry · sys share of wall 0.9970 |
| cold clonefile wall cost per tree entry (fit) | 6.804 µs/entry + 129.3 µs |
| kernel sys time per tree entry (fit) | 6.842 µs/entry + 118.6 µs |
| standby replenish (refill thread) · tiny | p50 0.2157 ms · p95 0.4907 ms · n=400 |
| standby replenish (refill thread) · small | p50 1.097 ms · p95 2.285 ms · n=300 |
| standby replenish (refill thread) · medium | p50 7.247 ms · p95 9.271 ms · n=60 |
| standby replenish (refill thread) · aegisflow_src | p50 1.310 ms · p95 2.199 ms · n=200 |
| background reap per detached tree · tiny | p50 0.6879 ms · p95 1.688 ms · n=400 |
| background reap per detached tree · small | p50 5.097 ms · p95 7.247 ms · n=300 |
| background reap per detached tree · medium | p50 37.545 ms · p95 44.267 ms · n=60 |
| background reap per detached tree · aegisflow_src | p50 7.857 ms · p95 9.424 ms · n=200 |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1067-0.1497 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.176 ms at 9 files and 7.321 ms at 1025 files, with getrusage ru_stime of 6.95-15.17 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.097 ms small, 7.247 ms medium). An acquire right after a commit finds the standby stale: at medium 8/60 fell back to a cold clone and 52/60 waited for the refill in flight, p50 10.158 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.463 ms, rollback 1.072 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.130 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.472 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.181 ms, cpuOnly 0.229 ms). One call after a 3.0 ms gap takes 0.360 ms with the CPU busy in the gap and 0.674 ms with it idle (cpuOnly 0.238 / 1.529 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.66, and faster with an idle CPU, speedup 2.27). The end-to-end ane_score stage, one call per transaction, has p50 0.395 ms, 2.18x the back-to-back p50 and 1.10x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs)); only a quiet full-scale run decides the phase result; the end-of-run load average and the total external CPU (system minus own: mean 4.11 cores, 146.58 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.

> `latency_ms` = WarmBranchPool warm-hit acquire p50 on the `small` fixture (what a caller waits for); the cold clonefile rows above are what each refill still costs in the kernel.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
