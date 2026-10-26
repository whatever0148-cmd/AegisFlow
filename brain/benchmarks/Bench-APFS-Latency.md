---
tags:
  - benchmark
status: verified
latency_ms: 0.1055
last_verified: 2026-10-26T04:52:47Z
git_commit: b5edd4f
---

# Bench-APFS-Latency

**Status:** 🟢 verified · **p50:** 0.1055 ms · **commit:** `b5edd4f`

> Per fixture: WarmBranchPool warm-hit acquire against the cold clonefile (wall and kernel time per entry), standby replenish, RENAME_SWAP commit, acquire right after a commit, caller-visible rollback with background reaping, the APFS collision cases and live-tree integrity after every phase.

**Implemented in:** `crates/aegis-bench/src/apfs.rs` · `crates/aegis-bench/src/main.rs`

**Upstream:** [[Core-APFS-Clonefile]] (measured by) · [[Core-Memory-Snapshot]] (measured by)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T04:52:49Z · bench 2026-10-26T04:52:47Z · tests 2026-10-25T08:10:13Z · commit `b5edd4f`_

| Metric | Measured |
|---|---|
| tiny · 9 files / 0.0170 MiB · acquire (pool warm hit) | p50 0.0951 ms · p95 0.1160 ms · n=400 · hits 400 / misses 0 |
| tiny · 9 files / 0.0170 MiB · clonefile (cold) | p50 0.1630 ms · p95 0.2003 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · kernel sys per clonefile | p50 152.0 µs · p95 199.0 µs · n=400 · 12.667 µs/entry |
| tiny · 9 files / 0.0170 MiB · standby replenish | p50 0.1825 ms · p95 0.2182 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · RENAME_SWAP | p50 0.1150 ms · p95 0.1267 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · commit caller-visible | p50 0.4164 ms · p95 0.4781 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · rollback caller-visible (background reap) | p50 0.0793 ms · p95 0.0975 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · background reap | p50 0.6093 ms · p95 0.6953 ms · n=400 |
| tiny · 9 files / 0.0170 MiB · acquire right after commit | p50 0.4409 ms · p95 0.5227 ms · n=400 · cold 11 / waited 389 / pool 0 |
| tiny · 9 files / 0.0170 MiB · live-tree integrity | 7/7 phases |
| small · 129 files / 1.076 MiB · acquire (pool warm hit) | p50 0.1055 ms · p95 0.1171 ms · n=300 · hits 300 / misses 0 |
| small · 129 files / 1.076 MiB · clonefile (cold) | p50 1.036 ms · p95 1.303 ms · n=300 |
| small · 129 files / 1.076 MiB · kernel sys per clonefile | p50 1007.0 µs · p95 1164.0 µs · n=300 · 7.297 µs/entry |
| small · 129 files / 1.076 MiB · standby replenish | p50 1.026 ms · p95 1.143 ms · n=300 |
| small · 129 files / 1.076 MiB · RENAME_SWAP | p50 0.1234 ms · p95 0.1297 ms · n=300 |
| small · 129 files / 1.076 MiB · commit caller-visible | p50 0.4376 ms · p95 0.4766 ms · n=300 |
| small · 129 files / 1.076 MiB · rollback caller-visible (background reap) | p50 0.0880 ms · p95 0.0963 ms · n=300 |
| small · 129 files / 1.076 MiB · background reap | p50 4.870 ms · p95 5.146 ms · n=300 |
| small · 129 files / 1.076 MiB · acquire right after commit | p50 1.470 ms · p95 1.726 ms · n=300 · cold 195 / waited 105 / pool 0 |
| small · 129 files / 1.076 MiB · live-tree integrity | 7/7 phases |
| medium · 1025 files / 8.560 MiB · acquire (pool warm hit) | p50 0.1125 ms · p95 0.1247 ms · n=60 · hits 60 / misses 0 |
| medium · 1025 files / 8.560 MiB · clonefile (cold) | p50 6.635 ms · p95 7.915 ms · n=60 |
| medium · 1025 files / 8.560 MiB · kernel sys per clonefile | p50 6299.0 µs · p95 6775.0 µs · n=60 · 5.954 µs/entry |
| medium · 1025 files / 8.560 MiB · standby replenish | p50 6.087 ms · p95 6.714 ms · n=60 |
| medium · 1025 files / 8.560 MiB · RENAME_SWAP | p50 0.1335 ms · p95 0.1490 ms · n=60 |
| medium · 1025 files / 8.560 MiB · commit caller-visible | p50 0.4425 ms · p95 0.4843 ms · n=60 |
| medium · 1025 files / 8.560 MiB · rollback caller-visible (background reap) | p50 0.0890 ms · p95 0.0991 ms · n=60 |
| medium · 1025 files / 8.560 MiB · background reap | p50 35.080 ms · p95 35.931 ms · n=60 |
| medium · 1025 files / 8.560 MiB · acquire right after commit | p50 9.717 ms · p95 10.462 ms · n=60 · cold 14 / waited 46 / pool 0 |
| medium · 1025 files / 8.560 MiB · live-tree integrity | 7/7 phases |
| aegisflow_src · 113 files / 5.485 MiB · acquire (pool warm hit) | p50 0.0991 ms · p95 0.1137 ms · n=200 · hits 200 / misses 0 |
| aegisflow_src · 113 files / 5.485 MiB · clonefile (cold) | p50 1.261 ms · p95 1.421 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · kernel sys per clonefile | p50 1238.0 µs · p95 1378.0 µs · n=200 · 7.549 µs/entry |
| aegisflow_src · 113 files / 5.485 MiB · standby replenish | p50 1.179 ms · p95 1.441 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · RENAME_SWAP | p50 0.1231 ms · p95 0.1363 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · commit caller-visible | p50 0.4294 ms · p95 0.4940 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · rollback caller-visible (background reap) | p50 0.0832 ms · p95 0.0973 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · background reap | p50 7.549 ms · p95 7.814 ms · n=200 |
| aegisflow_src · 113 files / 5.485 MiB · acquire right after commit | p50 1.725 ms · p95 2.058 ms · n=200 · cold 112 / waited 88 / pool 0 |
| aegisflow_src · 113 files / 5.485 MiB · live-tree integrity | 7/7 phases |
| method (pool rows) | idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, then edit, wait_ready, and the timed rollback (refill finished, reaper idle); 5 warm-up acquires first. Contended cases are measured separately: acquire_after_commit and collisions |

**Gated targets (from `targets_summary`)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| branch acquire (WarmBranchPool warm hit) · tiny (9 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0951 ms; hit rate 1.000; p95 0.1160 ms; n=400 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · small (129 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1055 ms; hit rate 1.000; p95 0.1171 ms; n=300 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · medium (1025 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.1125 ms; hit rate 1.000; p95 0.1247 ms; n=60 | ✅ PASS |
| branch acquire (WarmBranchPool warm hit) · aegisflow_src (113 files) | < 0.5 ms p50 on warm hits, hit rate >= 1 (pool settled before each acquire) | 0.0991 ms; hit rate 1.000; p95 0.1137 ms; n=200 | ✅ PASS |
| renameatx_np RENAME_SWAP commit (syscall only) · small | < 0.5 ms | 0.1234 ms; p95 0.1297 ms; n=300 | ✅ PASS |
| rollback · caller-visible (background reap) · small | < 0.15 ms | 0.0880 ms; p95 0.0963 ms; n=300 | ✅ PASS |

**Informational (not gated)**

| Target row | Target | Measured | Result |
|---|---|---|---|
| cold clonefile branch · tiny (9 files) | < 0.5 ms (same reference as the pool rows) | 0.1630 ms; p95 0.2003 ms; n=400 | not gated (would pass) |
| cold clonefile branch · small (129 files) | < 0.5 ms (same reference as the pool rows) | 1.036 ms; p95 1.303 ms; n=300 | not gated (would fail) |
| cold clonefile branch · medium (1025 files) | < 0.5 ms (same reference as the pool rows) | 6.635 ms; p95 7.915 ms; n=60 | not gated (would fail) |
| cold clonefile branch · aegisflow_src (113 files) | < 0.5 ms (same reference as the pool rows) | 1.261 ms; p95 1.421 ms; n=200 | not gated (would fail) |
| kernel time per clonefile entry (getrusage ru_stime) · tiny (9 files) | — (O(entries) inside the kernel) | 12.667 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · small (129 files) | — (O(entries) inside the kernel) | 7.297 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · medium (1025 files) | — (O(entries) inside the kernel) | 5.954 µs | not gated |
| kernel time per clonefile entry (getrusage ru_stime) · aegisflow_src (113 files) | — (O(entries) inside the kernel) | 7.549 µs | not gated |
| standby replenish (refill thread, off the caller's path) · tiny (9 files) | — | 0.1825 ms; p95 0.2182 ms; n=400 | not gated |
| standby replenish (refill thread, off the caller's path) · small (129 files) | — | 1.026 ms; p95 1.143 ms; n=300 | not gated |
| standby replenish (refill thread, off the caller's path) · medium (1025 files) | — | 6.087 ms; p95 6.714 ms; n=60 | not gated |
| standby replenish (refill thread, off the caller's path) · aegisflow_src (113 files) | — | 1.179 ms; p95 1.441 ms; n=200 | not gated |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · tiny (9 files) | < 0.5 ms (reference) | 0.4164 ms; p95 0.4781 ms; n=400 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · small (129 files) | < 0.5 ms (reference) | 0.4376 ms; p95 0.4766 ms; n=300 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · medium (1025 files) | < 0.5 ms (reference) | 0.4425 ms; p95 0.4843 ms; n=60 | not gated (would pass) |
| commit · caller-visible (lock + checks + RENAME_SWAP + version) · aegisflow_src (113 files) | < 0.5 ms (reference) | 0.4294 ms; p95 0.4940 ms; n=200 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · tiny (9 files) | < 0.5 ms (reference) | 0.4409 ms; pool_ready 0 · pool_after_waiting_for_refill 389 · cold 11; p95 0.5227 ms; n=400 | not gated (would pass) |
| acquire immediately after a commit (stale standby) · small (129 files) | < 0.5 ms (reference) | 1.470 ms; pool_ready 0 · pool_after_waiting_for_refill 105 · cold 195; p95 1.726 ms; n=300 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · medium (1025 files) | < 0.5 ms (reference) | 9.717 ms; pool_ready 0 · pool_after_waiting_for_refill 46 · cold 14; p95 10.462 ms; n=60 | not gated (would fail) |
| acquire immediately after a commit (stale standby) · aegisflow_src (113 files) | < 0.5 ms (reference) | 1.725 ms; pool_ready 0 · pool_after_waiting_for_refill 88 · cold 112; p95 2.058 ms; n=200 | not gated (would fail) |
| background reap per detached tree (reaper thread) · tiny (9 files) | — | 0.6093 ms; p95 0.6953 ms; n=400 | not gated |
| background reap per detached tree (reaper thread) · small (129 files) | — | 4.870 ms; p95 5.146 ms; n=300 | not gated |
| background reap per detached tree (reaper thread) · medium (1025 files) | — | 35.080 ms; p95 35.931 ms; n=60 | not gated |
| background reap per detached tree (reaper thread) · aegisflow_src (113 files) | — | 7.549 ms; p95 7.814 ms; n=200 | not gated |
| rollback detach only (rename into trash) · small | < 0.15 ms (reference) | 0.0853 ms; p95 0.0930 ms; n=300 | not gated (would pass) |
| rollback · synchronous (detach + inline delete, phase-1 style) · small | < 0.15 ms (reference) | 4.889 ms; p95 5.270 ms; n=300 | not gated (would fail) |
| rollback right after acquire (refill clone in flight) · small | < 0.15 ms (reference) | 1.115 ms; p95 1.270 ms; n=150 | not gated (would fail) |
| commit right after acquire (refill clone in flight) · small | < 0.5 ms (reference) | 1.410 ms; p95 1.593 ms; n=150 | not gated (would fail) |
| live-tree integrity after every APFS and end-to-end phase | all checks pass; store empty after pool drop (precondition of the APFS gated rows) | checks passed 30 · checks 30 · stores clean yes | not gated (would pass) |

> Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 0.0951-0.1125 ms across 4 fixtures (960 hits, 0 misses; a row passes only without misses). The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 0.163 ms at 9 files and 6.635 ms at 1025 files, with getrusage ru_stime of 5.95-12.67 us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 1.026 ms small, 6.087 ms medium). An acquire right after a commit finds the standby stale: at medium 14/60 fell back to a cold clone and 46/60 waited for the refill in flight, p50 9.717 ms. A commit or rollback issued while the refill clone runs waits behind it (small: commit 1.410 ms, rollback 1.115 ms p50). The commit row times the RENAME_SWAP syscall (small p50 0.123 ms); the caller-visible commit (lock, checks, swap, version) has p50 0.438 ms. The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 0.177 ms, cpuOnly 0.219 ms). One call after a 3.0 ms gap takes 0.359 ms with the CPU busy in the gap and 0.730 ms with it idle (cpuOnly 0.225 / 1.512 ms: cpuAndNeuralEngine is slower than cpuOnly with a busy CPU, speedup 0.63, and faster with an idle CPU, speedup 2.07). The end-to-end ane_score stage, one call per transaction, has p50 0.355 ms, 2.01x the back-to-back p50 and 0.99x the busy-gap isolated p50. Live-tree integrity: 30/30 checks passed, stores clean: true. One run on Apple M2, 11/11 gated rows passed. Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs 0.67 cores (p95 1.26, max 1.57, 72 samples) below the busy threshold of 2.00 cores, sources unchanged during the run; the end-of-run load average and the total external CPU (system minus own: mean 2.38 cores, 123.65 core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational.
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
