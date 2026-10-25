---
tags:
  - benchmark
status: verified
latency_ms: 0.105042
last_verified: 2026-10-25T17:28:23Z
git_commit: 9f1f777-dirty
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1050 ms · **commit:** `9f1f777-dirty`

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T17:28:26Z · bench 2026-10-25T17:28:23Z · tests 2026-10-25T08:10:13Z · commit `9f1f777-dirty`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.0946 ms · p95 0.1145 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1706 ms · p95 0.2072 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 160.0 µs · p95 214.0 µs · n=400 · 13.333 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1812 ms · p95 0.2138 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1162 ms · p95 0.1261 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4138 ms · p95 0.4757 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0788 ms · p95 0.0982 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6156 ms · p95 0.6955 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4420 ms · p95 0.5282 ms · n=400 · cold 18 / waited 382 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1050 ms · p95 0.1189 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.001 ms · p95 1.167 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 999.0 µs · p95 1133.0 µs · n=300 · 7.239 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.023 ms · p95 1.167 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1261 ms · p95 0.1335 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4443 ms · p95 0.4809 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0892 ms · p95 0.0978 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 4.916 ms · p95 5.121 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.418 ms · p95 1.691 ms · n=300 · cold 208 / waited 92 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1204 ms · p95 0.1471 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 7.459 ms · p95 8.260 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 6368.0 µs · p95 7387.0 µs · n=60 · 6.019 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 6.817 ms · p95 7.675 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1333 ms · p95 0.1452 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4582 ms · p95 0.5100 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.0909 ms · p95 0.1153 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 36.306 ms · p95 37.740 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 10.058 ms · p95 10.887 ms · n=60 · cold 0 / waited 60 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 111 files / 5.329 MiB · acquire (pool warm hit) | p50 0.1087 ms · p95 0.1296 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 111 files / 5.329 MiB · clonefile (cold) | p50 1.264 ms · p95 1.448 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · kernel sys per clonefile | p50 1231.0 µs · p95 1411.0 µs · n=200 · 7.599 µs/entry |
| aegisflow_src · 111 files / 5.329 MiB · standby replenish | p50 1.324 ms · p95 1.554 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · RENAME_SWAP | p50 0.1278 ms · p95 0.1456 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · commit caller-visible | p50 0.4498 ms · p95 0.5319 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · rollback caller-visible (background reap) | p50 0.0904 ms · p95 0.1104 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · background reap | p50 7.832 ms · p95 9.034 ms · n=200 |
| aegisflow_src · 111 files / 5.329 MiB · acquire right after commit | p50 1.789 ms · p95 2.047 ms · n=200 · cold 109 / waited 91 / pool 0 |
| aegisflow_src · 111 files / 5.329 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0946 ms; hit rate 1.000; p95 0.1145 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1050 ms; hit rate 1.000; p95 0.1189 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1204 ms; hit rate 1.000; p95 0.1471 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (111 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1087 ms; hit rate 1.000; p95 0.1296 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1261 ms; p95 0.1335 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0892 ms; p95 0.0978 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1706 ms; p95 0.2072 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.001 ms; p95 1.167 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 7.459 ms; p95 8.260 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (111 files) | < 0.5 ms (same reference as the pool rows) | 1.264 ms; p95 1.448 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 13.333 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.239 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 6.019 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (111 files) | — (O(entries) inside the kernel) | 7.599 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1812 ms; p95 0.2138 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.023 ms; p95 1.167 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 6.817 ms; p95 7.675 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (111 files) | — | 1.324 ms; p95 1.554 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4138 ms; p95 0.4757 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4443 ms; p95 0.4809 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4582 ms; p95 0.5100 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (111 files) | < 0.5 ms (reference) | 0.4498 ms; p95 0.5319 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4420 ms; pool_ready 0 · pool_after_waiting_for_refill 382 · cold 18; p95 0.5282 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.418 ms; pool_ready 0 · pool_after_waiting_for_refill 92 · cold 208; p95 1.691 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 10.058 ms; pool_ready 0 · pool_after_waiting_for_refill 60 · cold 0; p95 10.887 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (111 files) | < 0.5 ms (reference) | 1.789 ms; pool_ready 0 · pool_after_waiting_for_refill 91 · cold 109; p95 2.047 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6156 ms; p95 0.6955 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 4.916 ms; p95 5.121 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 36.306 ms; p95 37.740 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (111 files) | — | 7.832 ms; p95 9.034 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0862 ms; p95 0.0945 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 4.920 ms; p95 5.183 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.089 ms; p95 1.254 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.405 ms; p95 1.606 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0946-0.1204 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.171 ms at 9 files and 7.459 ms at 1025 files, with getrusage ru_stime of 6.02-13.33 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.023 ms small, 6.817 ms medium). An acquire right after a commit finds the standby stale: at medium 0/60 fell back to a cold clone and 60/60 waited for the refill in flight, p50 10.058 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.405 ms, rollback 1.089 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.126 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.444 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.182 ms, cpuOnly 0.223 ms). One call after a 3.0 ms gap takes 0.362 ms with the CPU busy in the gap and 0.730 ms with it idle (cpuOnly 0.221 / 1.210 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.61, and faster with an idle CPU, speedup 1.66). The end-to-end ane_score stage, one call per transaction, has p50 0.369 ms, 2.03x the back-to-back p50 and 1.02x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.78 cores (p95 1.66, max 1.83, 73 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.73 cores, 142.15 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
