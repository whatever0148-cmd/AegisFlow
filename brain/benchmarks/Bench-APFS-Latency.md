---
tags:
  - benchmark
status: verified
latency_ms: 0.105292
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1053 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.0978 ms · p95 0.1140 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1665 ms · p95 0.2043 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 155.0 µs · p95 206.0 µs · n=400 · 12.917 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1812 ms · p95 0.2117 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1178 ms · p95 0.1338 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4407 ms · p95 0.5121 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0790 ms · p95 0.0965 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6161 ms · p95 0.6921 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4860 ms · p95 0.6595 ms · n=400 · cold 44 / waited 356 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1053 ms · p95 0.1226 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.006 ms · p95 1.141 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1050.0 µs · p95 1391.0 µs · n=300 · 7.609 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.135 ms · p95 1.351 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1288 ms · p95 0.1514 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4668 ms · p95 0.5595 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0869 ms · p95 0.1005 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 5.096 ms · p95 5.412 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.765 ms · p95 2.529 ms · n=300 · cold 149 / waited 151 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1375 ms · p95 0.3132 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 10.370 ms · p95 12.651 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 10061.0 µs · p95 12012.0 µs · n=60 · 9.509 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 8.609 ms · p95 10.630 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1433 ms · p95 0.1693 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4788 ms · p95 0.5690 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.1004 ms · p95 0.2503 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 37.644 ms · p95 60.218 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 12.428 ms · p95 14.436 ms · n=60 · cold 1 / waited 59 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 108 files / 5.255 MiB · acquire (pool warm hit) | p50 0.1148 ms · p95 0.1645 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 108 files / 5.255 MiB · clonefile (cold) | p50 1.364 ms · p95 2.261 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · kernel sys per clonefile | p50 1365.0 µs · p95 2106.0 µs · n=200 · 8.585 µs/entry |
| aegisflow_src · 108 files / 5.255 MiB · standby replenish | p50 1.393 ms · p95 2.467 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · RENAME_SWAP | p50 0.1333 ms · p95 0.1604 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · commit caller-visible | p50 0.4633 ms · p95 0.6627 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · rollback caller-visible (background reap) | p50 0.0934 ms · p95 0.1362 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · background reap | p50 8.022 ms · p95 10.031 ms · n=200 |
| aegisflow_src · 108 files / 5.255 MiB · acquire right after commit | p50 2.217 ms · p95 3.275 ms · n=200 · cold 64 / waited 136 / pool 0 |
| aegisflow_src · 108 files / 5.255 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0978 ms; hit rate 1.000; p95 0.1140 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1053 ms; hit rate 1.000; p95 0.1226 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1375 ms; hit rate 1.000; p95 0.3132 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (108 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1148 ms; hit rate 1.000; p95 0.1645 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1288 ms; p95 0.1514 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0869 ms; p95 0.1005 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1665 ms; p95 0.2043 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.006 ms; p95 1.141 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 10.370 ms; p95 12.651 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (108 files) | < 0.5 ms (same reference as the pool rows) | 1.364 ms; p95 2.261 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 12.917 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.609 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 9.509 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (108 files) | — (O(entries) inside the kernel) | 8.585 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1812 ms; p95 0.2117 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.135 ms; p95 1.351 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 8.609 ms; p95 10.630 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (108 files) | — | 1.393 ms; p95 2.467 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4407 ms; p95 0.5121 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4668 ms; p95 0.5595 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4788 ms; p95 0.5690 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (108 files) | < 0.5 ms (reference) | 0.4633 ms; p95 0.6627 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4860 ms; pool_ready 0 · pool_after_waiting_for_refill 356 · cold 44; p95 0.6595 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.765 ms; pool_ready 0 · pool_after_waiting_for_refill 151 · cold 149; p95 2.529 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 12.428 ms; pool_ready 0 · pool_after_waiting_for_refill 59 · cold 1; p95 14.436 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (108 files) | < 0.5 ms (reference) | 2.217 ms; pool_ready 0 · pool_after_waiting_for_refill 136 · cold 64; p95 3.275 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6161 ms; p95 0.6921 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 5.096 ms; p95 5.412 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 37.644 ms; p95 60.218 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (108 files) | — | 8.022 ms; p95 10.031 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0838 ms; p95 0.0972 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 5.066 ms; p95 5.553 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.286 ms; p95 2.522 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.605 ms; p95 2.626 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0978-0.1375 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.167 ms at 9 files and 10.370 ms at 1025 files, with getrusage ru_stime of 7.61-12.92 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.135 ms small, 8.609 ms medium). An acquire right after a commit finds the standby stale: at medium 1/60 fell back to a cold clone and 59/60 waited for the refill in flight, p50 12.428 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.605 ms, rollback 1.286 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.129 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.467 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.179 ms, cpuOnly 0.220 ms). One call after a 3.0 ms gap takes 0.344 ms with the CPU busy in the gap and 0.706 ms with it idle (cpuOnly 0.218 / 1.252 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.64, and faster with an idle CPU, speedup 1.77). The end-to-end ane_score stage, one call per transaction, has p50 0.483 ms, 2.70x the back-to-back p50 and 1.41x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. This run is PRELIMINARY (1-min load average 11.47 at end above the busy threshold 4); only a quiet full-scale run decides the phase result.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
