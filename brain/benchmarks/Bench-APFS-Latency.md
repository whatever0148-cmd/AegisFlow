---
tags:
  - benchmark
status: verified
latency_ms: 0.107375
last_verified: 2026-10-25T15:38:36Z
git_commit: b3f6810
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1074 ms · **commit:** `b3f6810` · **bench run: PRELIMINARY**

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:38:39Z · bench 2026-10-25T15:38:36Z · tests 2026-10-25T08:10:13Z · commit `b3f6810`_

> **PRELIMINARY: 1-min load average 13.13 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.1002 ms · p95 0.1130 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1723 ms · p95 0.1953 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 161.0 µs · p95 208.0 µs · n=400 · 13.417 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1878 ms · p95 0.2103 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1192 ms · p95 0.1381 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4483 ms · p95 0.5025 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0867 ms · p95 0.0962 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6474 ms · p95 0.6974 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.5075 ms · p95 0.6823 ms · n=400 · cold 5 / waited 395 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1074 ms · p95 0.1318 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.014 ms · p95 1.199 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1091.0 µs · p95 1865.0 µs · n=300 · 7.906 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.117 ms · p95 1.366 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1296 ms · p95 0.1510 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4650 ms · p95 0.5368 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0883 ms · p95 0.1081 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 5.053 ms · p95 6.016 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.700 ms · p95 2.335 ms · n=300 · cold 145 / waited 155 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1465 ms · p95 0.1794 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 7.729 ms · p95 11.302 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 8856.0 µs · p95 12685.0 µs · n=60 · 8.371 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 7.943 ms · p95 9.945 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1423 ms · p95 0.1732 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4987 ms · p95 0.6287 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.1130 ms · p95 0.1937 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 38.498 ms · p95 69.912 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 10.820 ms · p95 15.168 ms · n=60 · cold 1 / waited 59 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 108 files / 5.265 MiB · acquire (pool warm hit) | p50 0.1052 ms · p95 0.1275 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 108 files / 5.265 MiB · clonefile (cold) | p50 1.338 ms · p95 2.061 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · kernel sys per clonefile | p50 1458.0 µs · p95 2185.0 µs · n=200 · 9.170 µs/entry |
| aegisflow_src · 108 files / 5.265 MiB · standby replenish | p50 1.468 ms · p95 2.598 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · RENAME_SWAP | p50 0.1272 ms · p95 0.1452 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · commit caller-visible | p50 0.4596 ms · p95 0.5216 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · rollback caller-visible (background reap) | p50 0.0897 ms · p95 0.1052 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · background reap | p50 7.893 ms · p95 9.033 ms · n=200 |
| aegisflow_src · 108 files / 5.265 MiB · acquire right after commit | p50 2.153 ms · p95 2.867 ms · n=200 · cold 93 / waited 107 / pool 0 |
| aegisflow_src · 108 files / 5.265 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1002 ms; hit rate 1.000; p95 0.1130 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1074 ms; hit rate 1.000; p95 0.1318 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1465 ms; hit rate 1.000; p95 0.1794 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (108 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1052 ms; hit rate 1.000; p95 0.1275 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1296 ms; p95 0.1510 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0883 ms; p95 0.1081 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1723 ms; p95 0.1953 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.014 ms; p95 1.199 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 7.729 ms; p95 11.302 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (108 files) | < 0.5 ms (same reference as the pool rows) | 1.338 ms; p95 2.061 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 13.417 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.906 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 8.371 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (108 files) | — (O(entries) inside the kernel) | 9.170 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1878 ms; p95 0.2103 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.117 ms; p95 1.366 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 7.943 ms; p95 9.945 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (108 files) | — | 1.468 ms; p95 2.598 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4483 ms; p95 0.5025 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4650 ms; p95 0.5368 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4987 ms; p95 0.6287 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (108 files) | < 0.5 ms (reference) | 0.4596 ms; p95 0.5216 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.5075 ms; pool_ready 0 · pool_after_waiting_for_refill 395 · cold 5; p95 0.6823 ms; n=400 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.700 ms; pool_ready 0 · pool_after_waiting_for_refill 155 · cold 145; p95 2.335 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 10.820 ms; pool_ready 0 · pool_after_waiting_for_refill 59 · cold 1; p95 15.168 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (108 files) | < 0.5 ms (reference) | 2.153 ms; pool_ready 0 · pool_after_waiting_for_refill 107 · cold 93; p95 2.867 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6474 ms; p95 0.6974 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 5.053 ms; p95 6.016 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 38.498 ms; p95 69.912 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (108 files) | — | 7.893 ms; p95 9.033 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0847 ms; p95 0.1037 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 5.082 ms; p95 6.599 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.289 ms; p95 2.064 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.643 ms; p95 2.466 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.1002-0.1465 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.172 ms at 9 files and 7.729 ms at 1025 files, with getrusage ru_stime of 7.91-13.42 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.117 ms small, 7.943 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 10.820 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.643 ms, rollback 1.289 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.130 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.465 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.175 ms, cpuOnly 0.220 ms). One call after a 3.0 ms gap takes 0.368 ms with the CPU busy in the gap and 0.851 ms with it idle (cpuOnly 0.212 / 1.461 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.58, and faster with an idle CPU, speedup 1.72). The end-to-end ane_score stage, one call per transaction, has p50 0.374 ms, 2.14x the back-to-back p50 and 1.02x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (1-min load average 13.13 at end above the busy threshold 4); only a quiet full-scale run decides the phase result.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
