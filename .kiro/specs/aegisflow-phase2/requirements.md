# Requirements Document — AegisFlow Phase 2

## Introduction

Phase 1 proved the pipeline (APFS copy-on-write branch → CoreML risk score → Cedar → atomic
commit or rollback) and measured where it falls short: a cold `clonefile` costs time proportional
to the number of tree entries, rollback spent most of its time deleting the detached tree, and
the per-event CoreML model ran on the CPU instead of the Neural Engine. Phase 2 moves the
per-entry clone cost off the caller's path (warm branch pool), moves deletion to a background
reaper, replaces the per-event model with a context-window model that `MLComputePlan` places on
the Neural Engine, and adds a localhost console with a daemon and credential handling.

Every acceptance criterion that names a latency is a target. Whether it is met is decided only
by `aegis-bench` measurements (`artifacts/aegis_phase1_telemetry.json`, `targets_summary`), never
by this document. Criteria ids (P2-R<n>.<m>) are referenced by `harness/TASKS.json` (T14–T20).

## Requirements

### Requirement 1 — Warm branch pool
**User Story:** As an AI coding agent, I want a private copy of the workspace without waiting for
a full tree clone, so that each speculative edit starts immediately whatever the size of the repo.

#### Acceptance Criteria
1. (P2-R1.1) WHEN an agent acquires a branch and a valid standby exists THE SYSTEM SHALL hand it out by moving the pre-cloned standby into `branches/` with one `renameatx_np(RENAME_EXCL)`, and every concurrent caller SHALL receive a distinct directory.
2. (P2-R1.2) THE SYSTEM SHALL NOT hand out a standby whose live generation, live root inode, pool invalidation epoch or age (`max_standby_age`) differs from the current state; IF no valid standby exists THEN THE SYSTEM SHALL wait for an in-flight refill of the current epoch or clone cold, and SHALL report the source (`Pool` / `Cold`) in the acquire receipt.
3. (P2-R1.3) WHILE a foreground operation (acquire, cold branch, commit, rollback detach) is running THE SYSTEM SHALL NOT start a refill clone or a standby disposal rename, so that background work does not queue ahead of a caller on the APFS volume.
4. (P2-R1.4) WHEN a branch is committed through any clone of the workspace, or `bump_generation()` is called in any process, THE SYSTEM SHALL invalidate the standbys of every pool on that store, and a branch based on the old generation SHALL get a commit conflict. WHERE the live tree is edited directly, THE SYSTEM SHALL invalidate standbys on FSEvents delivery (best effort) and SHALL offer `bump_generation()` as the hard guarantee.
5. (P2-R1.5) THE SYSTEM SHALL acquire a branch on a warm pool hit in under 0.5 ms p50 on each benchmark fixture (tiny, small, medium, aegisflow_src), and every acquire made while the pool is settled SHALL be a warm hit.
6. (P2-R1.6) THE SYSTEM SHALL commit a branch with `renameatx_np(RENAME_SWAP)` in under 0.5 ms p50 on the small fixture, and SHALL keep the previous live tree as a restorable version.

### Requirement 2 — Rollback and background reaper
**User Story:** As an agent orchestrator, I want a rejected change to disappear immediately, so
that the agent can retry without waiting for the filesystem to delete thousands of files.

#### Acceptance Criteria
1. (P2-R2.1) WHEN a branch is rolled back in background-reclaim mode (the default) THE SYSTEM SHALL return after detaching it into the store's trash, SHALL queue the deletion to a reaper thread, and SHALL report `queued = true` and a reaper ticket in the rollback receipt.
2. (P2-R2.2) THE SYSTEM SHALL record every reaper deletion (ticket, kind, queue wait, duration, success), SHALL empty the trash, and WHEN a pool is dropped SHALL leave nothing in `standby/`.
3. (P2-R2.3) THE SYSTEM SHALL return a caller-visible rollback in under 0.15 ms p50 on the small fixture, and the live tree SHALL stay byte-identical.
4. (P2-R2.4) WHERE a caller needs deletion to finish before returning, THE SYSTEM SHALL provide a synchronous reclaim mode that reports the inline deletion time.

### Requirement 3 — Context-window risk model on the Neural Engine
**User Story:** As a governance engineer, I want each event scored against the agent's recent
history on the Neural Engine, so that slow multi-step attacks are visible and the CPU stays free.

#### Acceptance Criteria
1. (P2-R3.1) THE SYSTEM SHALL ship an FP16 CoreML ML Program whose input is the last 64 events × 64 features of one session (oldest → newest, zero-padded on the left) and whose output is the 0–100 risk of the newest event; the layout SHALL be declared by `aegis_input_layout` metadata (`window`, `window_batch` for the batch model), never inferred from the shape.
2. (P2-R3.2) WHEN the model is built or benchmarked THE SYSTEM SHALL compare CoreML FP16 outputs with the FP32 reference in `models/parity_vectors.json` for single events and for multi-event sessions, within the file's `tolerance_points`.
3. (P2-R3.3) THE SYSTEM SHALL claim Neural Engine execution only when `MLComputePlan` (`.cpuAndNeuralEngine`) assigns every dispatched op of the online model to the Neural Engine, and SHALL publish the per-op device list.
4. (P2-R3.4) THE SYSTEM SHALL score one full window with `.cpuAndNeuralEngine` in under 0.6 ms p50.
5. (P2-R3.5) THE SYSTEM SHALL keep one `ContextWindow` per agent session (`scorer.new_window()`, `push`, `score_window`, `clear`), and a window holding one event SHALL score exactly like `score()` on that event.
6. (P2-R3.6) WHEN latency is compared across compute units THE SYSTEM SHALL interleave the units in rounds, and the online model SHALL be faster on `.cpuAndNeuralEngine` than on `.cpuOnly` (speedup = cpuOnly p50 / cpuAndNeuralEngine p50 > 1.0); the batch model SHALL run every dispatched op on the Neural Engine.

### Requirement 4 — Console and daemon security
**User Story:** As a developer running the console on my laptop, I want the daemon reachable only
by my own browser tab, so that a web page I visit cannot drive my workspace or read my settings.

#### Acceptance Criteria
1. (P2-R4.1) THE SYSTEM SHALL bind the daemon to `127.0.0.1` only (port from `AEGIS_CONSOLE_PORT`, default 4173).
2. (P2-R4.2) IF a request's `Host` is not `localhost:<port>` or `127.0.0.1:<port>`, or an `Origin` header is present and is not the console's own origin, THEN THE SYSTEM SHALL answer `403` (DNS-rebinding and cross-site guard).
3. (P2-R4.3) THE SYSTEM SHALL generate a random token per process, embed it only in the served `index.html`, and IF a `POST` lacks `X-Aegis-Token` with that token and `Content-Type: application/json`, or the WebSocket lacks `token`, THEN THE SYSTEM SHALL answer `401`.
4. (P2-R4.4) THE SYSTEM SHALL show only numbers the daemon measured or read from the benchmark artifact, labelled live or bench, and SHALL render every failed gated target as FAIL without re-judging it (contract: `apps/aegis-console/API.md`).

### Requirement 5 — Credential handling
**User Story:** As a user connecting Nebius, Tavily, PayPal Sandbox or chat webhooks, I want my
keys stored safely and used only when I configured them, so that nothing leaks and nothing is
called behind my back.

#### Acceptance Criteria
1. (P2-R5.1) THE SYSTEM SHALL resolve each setting from the process environment first, then from the repository `.env`; binaries that hand configuration to `from_env()` constructors SHALL copy `.env`-only values into the environment once at startup, before any thread or CoreML initialisation.
2. (P2-R5.2) THE SYSTEM SHALL write `.env` with mode 0600 through a temporary file and an atomic rename, SHALL accept secret values only through stdin or the token-protected API (never as command-line arguments), and SHALL never print, log or return a secret value, masked or not.
3. (P2-R5.3) THE SYSTEM SHALL call Nebius, Tavily, PayPal Sandbox and chat webhooks only when their credentials are configured; the benchmark SHALL additionally require `AEGIS_BENCH_LIVE=1`; tests SHALL use local listeners; WHEN a service is not configured THE SYSTEM SHALL report `enabled: false` with a reason instead of simulating a reply.

### Requirement 6 — Telemetry honesty
**User Story:** As a reviewer, I want every published number to come from a run and every missed
target to stay visible, so that the scoreboard can be trusted.

#### Acceptance Criteria
1. (P2-R6.1) THE SYSTEM SHALL take every latency, score and count in artifacts, notes and the console from a measurement; `.claude/hooks/verify_no_stubs.sh` SHALL reject stub markers and literal telemetry values in sources.
2. (P2-R6.2) THE SYSTEM SHALL write exactly 11 gated rows into `targets_summary` (branch acquire × 4 fixtures, RENAME_SWAP commit, caller-visible rollback, online model latency, online placement, online speedup, batch placement, tier-0 route), each with a `pass` flag computed only from measurements; IF a measurement is missing THEN its `pass` SHALL be null and count as not passed.
3. (P2-R6.3) THE SYSTEM SHALL publish non-gated context in `informational` (at least cold clonefile per fixture against the same 0.5 ms reference with its computed pass flag, kernel time per entry, background reap, standby replenish, acquire after a commit with its source, detach-only rollback, caller-visible commit, and online-model latency and speedup for isolated calls next to the per-transaction `ane_score` stage) and a `targets_note` generated from the measurements stating that the branch targets measure pool acquire while the cold clonefile stays O(entries) in the kernel; every published latency SHALL carry its sample count and p50 / p95.
4. (P2-R6.4) WHEN benchmarking THE SYSTEM SHALL keep background work idle at the start of each timed call, SHALL report the APFS collision cases (commit or rollback while a refill clone runs, acquire right after a commit) separately, and SHALL verify the live tree after every phase.
5. (P2-R6.5) IF any gated row did not pass THEN the harness gate (T18) SHALL exit non-zero and print each failing row with its target and measurement; numbers measured on a shared machine SHALL be labelled preliminary, and only the quiet benchmark run decides the phase result. A run SHALL count as preliminary WHEN it is reduced-scale, WHEN the 1-minute load average at its start (after the optional quiet wait) exceeds the performance-core count, WHEN the time-weighted mean attributable CPU contention during the run (user + system CPU of the other processes the benchmark can read, which in practice are the processes of the same user, from `proc_listallpids` + `proc_pid_rusage` per pid and start time about once per second on a background thread, including the CPU of children those processes reaped, excluding the benchmark process and its descendants, divided by the sampled seconds) is at or above 25% of the logical CPUs, WHEN that contention could not be measured (CPU sampler unsupported, failed or without samples, no process listing succeeded, a process listing failed, or an interval without attribution), or WHEN the measured sources changed during it; a process that starts and exits during the run SHALL be counted from its start when a process listing sees it or a readable parent reaps it during the run. The artifact SHALL record the mean, p95 and max attributable contention over the per-interval values, the sample and listing counts and the top contending processes by CPU; it SHALL also record, as informational only, the total external CPU (system-wide busy cores from `host_statistics(HOST_CPU_LOAD_INFO)` tick deltas minus the benchmark process's own CPU from `getrusage(RUSAGE_SELF)`, clamped at 0: mean, p95, max) and its unattributed core-seconds (total minus the CPU of readable processes), because it includes root and system services (`kernel_task`, `mds`, `fseventsd`, `aned`, `ANECompilerService`) that mostly respond to the benchmark's own filesystem and CoreML activity; the benchmark SHALL keep its fixtures in a directory whose name ends in `.noindex` so Spotlight does not index them; the 1-minute load average at the end of the run SHALL be recorded as informational only, because after minutes of the benchmark's own APFS, reaper and CoreML loops it mostly reflects the benchmark's own threads; IF the artifact is preliminary, reduced-scale or its source fingerprint differs from the working tree THEN the gate SHALL exit non-zero.

### Requirement 7 — Harness, specs and brain
**User Story:** As the systems architect, I want specs, task results and telemetry in one place,
so that the state of phase 2 is visible without reading logs.

#### Acceptance Criteria
1. (P2-R7.1) THE SYSTEM SHALL keep phase-2 requirements (this document) and design in `.kiro/specs/aegisflow-phase2/`, and SHALL generate `tasks.md` there from the phase-2 tasks in `harness/TASKS.json`.
2. (P2-R7.2) WHEN telemetry is synced THE SYSTEM SHALL render the pool, kernel, reaper, window-model placement, speedup, parity, gated and informational rows and `targets_note` into the Obsidian notes, the telemetry log and the canvas card, copying verdicts from the artifact.
3. (P2-R7.3) WHEN the canvas is rewritten THE SYSTEM SHALL keep the position and size of every existing node, user-added nodes and edges, and every `<!-- aegis:user:start -->` block; THE SYSTEM SHALL support dry runs against another vault, benchmark file and spec root without launching Obsidian.
