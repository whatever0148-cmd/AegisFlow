//! APFS section: per fixture, the cold `clonefile` branch (wall time and kernel time per call),
//! `WarmBranchPool` warm-hit acquire, standby replenish, `renameatx_np(RENAME_SWAP)` commit,
//! acquire immediately after a commit, caller-visible rollback with background reaping, the
//! reaper's own deletion time, synchronous rollback for phase-1 comparison, the APFS collision
//! cases, and live-tree integrity checks after every phase.
//!
//! Background work is idle at the start of every timed call (`wait_ready` + `wait_reaped` between
//! iterations, outside the timed region), so each number is the operation alone. The collision
//! cases (commit or rollback issued while the refill clone is still running) are measured in a
//! separate loop and reported separately.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aegis_storage::sys::{clone_tree, process_cpu_times};
use aegis_storage::{AcquireSource, PoolConfig, ReapKind, ReapRecord, WarmBranchPool, Workspace};
use serde_json::{json, Value};

use crate::util::{
    count_entries, ensure, generate_fixture, linear_fit, manifest_hash, ms, p50, percentile, round6, scaled, sha256_hex,
    stats, tree_manifest, tree_size, us, Manifest, Res, FIXTURES,
};

/// Committed versions kept per fixture workspace.
pub const RETAINED_VERSIONS: usize = 4;
const WARMUP: usize = 5;
/// Upper bound for any wait on the refill thread or the reaper.
pub const WAIT: Duration = Duration::from_secs(30);
const COMMIT_FILE: &str = "agent_commit.rs";
const EDIT_FILE: &str = "agent_edit.rs";

/// Waits until the pool holds its standbys and the reaper is idle (outside every timed region).
pub fn settle(ws: &Workspace, pool: &WarmBranchPool) -> Res<()> {
    ensure(pool.wait_ready(WAIT), "pool standby")?;
    ensure(ws.wait_reaped(WAIT), "reaper")
}

/// Live-tree integrity: after every phase the live tree must equal the expected manifest (the
/// original tree plus the content of the last commit, if any).
pub struct Integrity {
    live: PathBuf,
    expected: Manifest,
    checks: Vec<(String, bool, String)>,
}

impl Integrity {
    pub fn new(live: &Path) -> Res<Self> {
        Ok(Self { live: live.to_path_buf(), expected: tree_manifest(live)?, checks: Vec::new() })
    }

    pub fn expect_file(&mut self, rel: &str, content: &str) {
        self.expected.insert(rel.to_string(), sha256_hex(content.as_bytes()));
    }

    pub fn check(&mut self, phase: &str) -> Res<bool> {
        let now = tree_manifest(&self.live)?;
        let ok = now == self.expected;
        self.checks.push((phase.to_string(), ok, manifest_hash(&now)));
        Ok(ok)
    }

    pub fn all_ok(&self) -> bool {
        self.checks.iter().all(|(_, ok, _)| *ok)
    }

    pub fn json(&self) -> Value {
        json!({
            "method": "SHA-256 of every live file compared with the expected tree (original files plus the last committed agent file) after each phase",
            "phases": self.checks.iter().map(|(p, ok, h)| json!({"phase": p, "ok": ok, "live_sha256": h})).collect::<Vec<_>>(),
            "checks": self.checks.len(),
            "passed": self.checks.iter().filter(|(_, ok, _)| *ok).count(),
            "all_ok": self.all_ok(),
        })
    }
}

/// Reaper records matching the given tickets.
struct Reaps {
    duration_ms: Vec<f64>,
    waited_ms: Vec<f64>,
    failed: usize,
    missing: usize,
}

impl Reaps {
    fn collect(records: &[ReapRecord], tickets: &[u64], kind: ReapKind) -> Self {
        let wanted: HashSet<u64> = tickets.iter().copied().collect();
        let mine: Vec<&ReapRecord> = records.iter().filter(|r| r.kind == kind && wanted.contains(&r.ticket)).collect();
        Self {
            duration_ms: mine.iter().filter(|r| r.ok).map(|r| ms(r.duration)).collect(),
            waited_ms: mine.iter().map(|r| ms(r.waited)).collect(),
            failed: mine.iter().filter(|r| !r.ok).count(),
            missing: wanted.len().saturating_sub(mine.len()),
        }
    }

    fn json(self) -> Value {
        json!({
            "reap_ms": stats(self.duration_ms),
            "queue_wait_ms": stats(self.waited_ms),
            "failed": self.failed,
            "records_missing": self.missing,
        })
    }
}

/// One directory-level clonefile (`aegis_storage::sys::clone_tree`, the call `Workspace::branch`
/// makes) with `getrusage(RUSAGE_SELF)` deltas around it. Runs before any pool or CoreML thread
/// exists and with synchronous deletion outside the measurement, so the deltas belong to the call.
fn kernel_clonefile(live: &Path, scratch: &Path, iterations: usize, entries: usize) -> Res<Value> {
    let dst = scratch.join("kernel-clone");
    for _ in 0..WARMUP {
        clone_tree(live, &dst)?;
        fs::remove_dir_all(&dst)?;
    }
    let (mut wall, mut sys, mut user) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..iterations {
        let cpu0 = process_cpu_times()?;
        let started = Instant::now();
        clone_tree(live, &dst)?;
        let elapsed = started.elapsed();
        let cpu1 = process_cpu_times()?;
        wall.push(us(elapsed));
        sys.push(us(cpu1.system.saturating_sub(cpu0.system)));
        user.push(us(cpu1.user.saturating_sub(cpu0.user)));
        fs::remove_dir_all(&dst)?;
    }
    let (wall50, sys50) = (percentile(&wall, 50.0), percentile(&sys, 50.0));
    let per_entry = |v: Option<f64>| v.filter(|_| entries > 0).map(|v| round6(v / entries as f64));
    let zero = sys.iter().filter(|s| **s == 0.0).count();
    Ok(json!({
        "method": "getrusage(RUSAGE_SELF) ru_stime/ru_utime delta around one clonefile(CLONE_NOFOLLOW|CLONE_NOOWNERCOPY) of the live tree; no pool, reaper idle, CoreML not loaded",
        "iterations": iterations,
        "wall_us": stats(wall),
        "sys_us": stats(sys),
        "user_us": stats(user),
        "sys_us_per_entry_p50": per_entry(sys50),
        "wall_us_per_entry_p50": per_entry(wall50),
        "sys_share_of_wall_p50": wall50.zip(sys50).filter(|(w, _)| *w > 0.0).map(|(w, s)| round6(s / w)),
        "sys_zero_samples": zero,
    }))
}

fn pool_counters(s: &aegis_storage::PoolStats) -> Value {
    json!({
        "hits": s.hits,
        "misses": s.misses,
        "stale_discards": s.stale_discards,
        "replenished": s.replenished,
        "replenish_races": s.replenish_races,
        "replenish_failures": s.replenish_failures,
        "fs_events": s.fs_events,
        "fs_events_ignored": s.fs_events_ignored,
        "invalidations": s.invalidations,
        "watching_fs_events": s.watching_fs_events,
        "last_error": s.last_error,
    })
}

fn bench_fixture(name: &str, live: &Path, iterations: usize) -> Res<Value> {
    let (files, dirs, bytes) = tree_size(live)?;
    let entries = files + dirs;
    let base = live.parent().ok_or("fixture has no parent directory")?;
    let scratch = base.join("kernel");
    fs::create_dir_all(&scratch)?;
    let mut ws = Workspace::open(live)?;
    ws.set_retained_versions(RETAINED_VERSIONS);
    let ws = Arc::new(ws);
    ws.set_background_reclaim(true);
    let mut integrity = Integrity::new(live)?;
    let collide_iterations = (iterations / 2).max(5);

    // 1. Kernel time of one clonefile.
    let kernel = kernel_clonefile(live, &scratch, iterations, entries)?;
    fs::remove_dir_all(&scratch)?;
    integrity.check("kernel_clonefile")?;

    // 2. Cold branch + rollback in background-reclaim mode (no pool yet).
    for _ in 0..WARMUP {
        ws.branch("warmup")?.rollback()?;
    }
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    ws.drain_reap_records();
    let (mut cold_total, mut cold_clone, mut cold_rb, mut cold_detach, mut cold_tickets) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..iterations {
        ensure(ws.wait_reaped(WAIT), "reaper")?;
        let started = Instant::now();
        let branch = ws.branch("cold")?;
        cold_total.push(ms(started.elapsed()));
        cold_clone.push(ms(branch.clone_duration()));
        fs::write(branch.path().join(EDIT_FILE), format!("// edit {i}\n"))?;
        let started = Instant::now();
        let receipt = branch.rollback()?;
        cold_rb.push(ms(started.elapsed()));
        cold_detach.push(ms(receipt.detach));
        if !receipt.queued {
            return Err("background reclaim expected: rollback receipt was not queued".into());
        }
        cold_tickets.extend(receipt.reap_ticket);
    }
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    let cold_reaps = Reaps::collect(&ws.drain_reap_records(), &cold_tickets, ReapKind::Branch);
    integrity.check("cold_branch_background_rollback")?;

    // 3. Synchronous rollback (detach + delete on the caller's thread), comparable with phase 1.
    ws.set_background_reclaim(false);
    let (mut sync_rb, mut sync_detach, mut sync_reclaim) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..iterations {
        let branch = ws.branch("sync")?;
        fs::write(branch.path().join(EDIT_FILE), format!("// edit {i}\n"))?;
        let started = Instant::now();
        let receipt = branch.rollback()?;
        sync_rb.push(ms(started.elapsed()));
        sync_detach.push(ms(receipt.detach));
        sync_reclaim.push(ms(receipt.reclaim_inline().ok_or("synchronous reclaim expected")?));
    }
    ws.set_background_reclaim(true);
    integrity.check("synchronous_rollback")?;

    // 4. WarmBranchPool: warm-hit acquire, then (after the refill clone finished) rollback.
    let cfg = PoolConfig::default();
    let pool = WarmBranchPool::new(Arc::clone(&ws), cfg.clone())?;
    for _ in 0..WARMUP {
        settle(&ws, &pool)?;
        pool.acquire("warmup")?.0.rollback()?;
    }
    settle(&ws, &pool)?;
    pool.drain_replenish_durations();
    ws.drain_reap_records();
    let before = pool.stats();
    let (mut hit, mut hit_rename, mut hit_age, mut miss) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut rb_caller, mut rb_detach, mut rb_tickets) = (Vec::new(), Vec::new(), Vec::new());
    let mut rollback_while_refilling = 0usize;
    for i in 0..iterations {
        settle(&ws, &pool)?;
        let started = Instant::now();
        let (branch, receipt) = pool.acquire("pooled")?;
        let took = ms(started.elapsed());
        match receipt.source {
            AcquireSource::Pool => {
                hit.push(took);
                hit_rename.extend(receipt.rename.map(ms));
                hit_age.extend(receipt.standby_age.map(ms));
            }
            AcquireSource::Cold => miss.push(took),
        }
        fs::write(branch.path().join(EDIT_FILE), format!("// edit {i}\n"))?;
        ensure(pool.wait_ready(WAIT), "pool standby")?;
        rollback_while_refilling += usize::from(pool.stats().replenishing);
        let started = Instant::now();
        let receipt = branch.rollback()?;
        rb_caller.push(ms(started.elapsed()));
        rb_detach.push(ms(receipt.detach));
        if !receipt.queued {
            return Err("background reclaim expected: pooled rollback receipt was not queued".into());
        }
        rb_tickets.extend(receipt.reap_ticket);
    }
    settle(&ws, &pool)?;
    let all_attempts: Vec<f64> = hit.iter().chain(&miss).copied().collect();
    let replenish: Vec<f64> = pool.drain_replenish_durations().into_iter().map(ms).collect();
    let pool_reaps = Reaps::collect(&ws.drain_reap_records(), &rb_tickets, ReapKind::Branch);
    let after_hits = pool.stats();
    integrity.check("pool_acquire_and_rollback")?;

    // 5. Commit once the refill clone has finished, then acquire immediately after the commit.
    let (mut swap, mut commit_total, mut commit_caller) = (Vec::new(), Vec::new(), Vec::new());
    let mut after_commit = Vec::new();
    let (mut ac_pool, mut ac_waited, mut ac_cold, mut ac_stale) = (0usize, 0usize, 0usize, 0usize);
    let mut ac_wait = Vec::new();
    let mut commit_while_refilling = 0usize;
    for i in 0..iterations {
        settle(&ws, &pool)?;
        let (branch, _) = pool.acquire("commit")?;
        let text = format!("// commit {i}\n");
        fs::write(branch.path().join(COMMIT_FILE), &text)?;
        ensure(pool.wait_ready(WAIT), "pool standby")?;
        commit_while_refilling += usize::from(pool.stats().replenishing);
        let started = Instant::now();
        let receipt = branch.commit()?;
        commit_caller.push(ms(started.elapsed()));
        swap.push(ms(receipt.swap));
        commit_total.push(ms(receipt.total));
        integrity.expect_file(COMMIT_FILE, &text);
        let started = Instant::now();
        let (next, receipt) = pool.acquire("after-commit")?;
        after_commit.push(ms(started.elapsed()));
        ac_stale += receipt.stale_discarded;
        match (receipt.source, receipt.waited_for_replenish) {
            (AcquireSource::Pool, Some(w)) => {
                ac_waited += 1;
                ac_wait.push(ms(w));
            }
            (AcquireSource::Pool, None) => ac_pool += 1,
            (AcquireSource::Cold, w) => {
                ac_cold += 1;
                ac_wait.extend(w.map(ms));
            }
        }
        next.rollback()?;
    }
    settle(&ws, &pool)?;
    integrity.check("commit_then_acquire")?;

    // 6. Collision cases: rollback / commit issued right after acquire, while the refill clone
    //    started by that acquire is still running (replenish delay 0, the default).
    let (mut rb_after_acquire, mut commit_after_acquire) = (Vec::new(), Vec::new());
    for i in 0..collide_iterations {
        settle(&ws, &pool)?;
        let (branch, _) = pool.acquire("collide-rollback")?;
        fs::write(branch.path().join(EDIT_FILE), format!("// edit {i}\n"))?;
        let started = Instant::now();
        branch.rollback()?;
        rb_after_acquire.push(ms(started.elapsed()));
        settle(&ws, &pool)?;
        let (branch, _) = pool.acquire("collide-commit")?;
        let text = format!("// collide {i}\n");
        fs::write(branch.path().join(COMMIT_FILE), &text)?;
        let started = Instant::now();
        branch.commit()?;
        commit_after_acquire.push(ms(started.elapsed()));
        integrity.expect_file(COMMIT_FILE, &text);
    }
    settle(&ws, &pool)?;
    integrity.check("collision_cases")?;

    let final_stats = pool.stats();
    drop(pool);
    ensure(ws.wait_reaped(WAIT), "reaper")?;
    let store = ws.store().to_path_buf();
    let leftovers = json!({
        "branches": count_entries(&store.join("branches")),
        "standby": count_entries(&store.join("standby")),
        "trash": count_entries(&store.join("trash")),
    });
    let store_clean = ["branches", "standby", "trash"].iter().all(|k| leftovers[*k] == 0);
    let versions = ws.versions()?.len();
    integrity.check("after_pool_drop")?;
    let reaper = ws.reaper_stats();

    let hits = after_hits.hits - before.hits;
    let misses = after_hits.misses - before.misses;
    let pool = json!({
        "method": format!(
            "idle pool at the start of every timed call: settle (wait_ready + wait_reaped) before each timed acquire, \
             then edit, wait_ready, and the timed rollback (refill finished, reaper idle); {WARMUP} warm-up acquires first. \
             Contended cases are measured separately: acquire_after_commit and collisions"
        ),
        "config": {
            "capacity": cfg.capacity,
            "max_standby_age_s": cfg.max_standby_age.map(|d| d.as_secs_f64()),
            "watch_fs_events": cfg.watch_fs_events,
            "replenish_delay_ms": ms(final_stats.replenish_delay),
        },
        "acquire_hit_ms": stats(hit),
        "acquire_hit_rename_ms": stats(hit_rename),
        "acquire_hit_standby_age_ms": stats(hit_age),
        "acquire_miss_ms": stats(miss),
        "acquire_all_ms": stats(all_attempts),
        "hits": hits,
        "misses": misses,
        "replenish_ms": stats(replenish),
        "rollback_caller_ms": stats(rb_caller),
        "rollback_detach_ms": stats(rb_detach),
        "rollback_while_refill_in_flight": rollback_while_refilling,
        "background_reaper": pool_reaps.json(),
        "acquire_after_commit": {
            "acquire_ms": stats(after_commit),
            "sources": {"pool_ready": ac_pool, "pool_after_waiting_for_refill": ac_waited, "cold": ac_cold},
            "stale_discarded": ac_stale,
            "waited_for_refill_ms": stats(ac_wait),
        },
        "collisions": {
            "rollback_right_after_acquire_ms": stats(rb_after_acquire),
            "commit_right_after_acquire_ms": stats(commit_after_acquire),
        },
        "counters": pool_counters(&final_stats),
    });
    Ok(json!({
        "name": name,
        "files": files,
        "dirs": dirs,
        "entries": entries,
        "bytes": bytes,
        "iterations": iterations,
        "collision_iterations": collide_iterations,
        "clonefile_ms": stats(cold_clone),
        "branch_total_ms": stats(cold_total),
        "kernel_clonefile": kernel,
        "renameatx_swap_ms": stats(swap),
        "commit_total_ms": stats(commit_total),
        "commit_caller_ms": stats(commit_caller),
        "commit_while_refill_in_flight": commit_while_refilling,
        "rollback_detach_ms": stats(cold_detach),
        "rollback_caller_background_ms": stats(cold_rb),
        "rollback_background_reaper": cold_reaps.json(),
        "rollback_sync_caller_ms": stats(sync_rb),
        "rollback_sync_detach_ms": stats(sync_detach),
        "rollback_reclaim_ms": stats(sync_reclaim),
        "pool": pool,
        "reaper": {
            "reaped": reaper.reaped,
            "failed": reaper.failed,
            "queued_at_end": reaper.queued,
            "background": reaper.background,
            "last_error": reaper.last_error,
        },
        "store_after_pool_drop": {"leftover_entries": leftovers, "clean": store_clean, "versions_kept": versions, "retained_versions": RETAINED_VERSIONS},
        "integrity": integrity.json(),
        "live_tree_unchanged_after_rollback": integrity.all_ok(),
    }))
}

pub fn bench_apfs(fixtures: &Path, root: &Path, scale: f64) -> Res<Value> {
    let mut results = Vec::new();
    for spec in &FIXTURES {
        let live = fixtures.join(spec.name).join("live");
        fs::create_dir_all(&live)?;
        generate_fixture(&live, spec)?;
        let iterations = scaled(spec.iterations, scale);
        eprintln!("  apfs · {} ({} files) x {iterations}", spec.name, spec.dirs * spec.files_per_dir + 1);
        results.push(bench_fixture(spec.name, &live, iterations)?);
    }
    // A copy-on-write copy of this repository's own sources as a realistic tree.
    let src_live = fixtures.join("aegisflow_src").join("live");
    fs::create_dir_all(&src_live)?;
    for dir in ["crates", "policies", "models", "harness", ".claude", ".kiro"] {
        if root.join(dir).exists() {
            let status = Command::new("cp").args(["-Rc"]).arg(root.join(dir)).arg(&src_live).status()?;
            if !status.success() {
                return Err(format!("cp -Rc {dir} failed").into());
            }
        }
    }
    let iterations = scaled(200, scale);
    eprintln!("  apfs · aegisflow_src x {iterations}");
    results.push(bench_fixture("aegisflow_src", &src_live, iterations)?);

    // Least-squares fits against tree entries: cold clonefile p50 (wall) and kernel sys p50.
    let fit = |key: &dyn Fn(&Value) -> Option<f64>| {
        let points: Vec<(f64, f64)> = results.iter().filter_map(|r| Some((r["entries"].as_f64()?, key(r)?))).collect();
        linear_fit(&points)
    };
    let wall_fit = fit(&|r| p50(&r["clonefile_ms"]).map(|v| v * 1000.0));
    let sys_fit = fit(&|r| p50(&r["kernel_clonefile"]["sys_us"]));
    let fs_type = aegis_storage::sys::fs_type(fixtures).ok();
    Ok(json!({
        "filesystem": "apfs",
        "stat_type": fs_type,
        "fixtures": results,
        "clonefile_us_per_entry": wall_fit.map(|(s, _)| round6(s)),
        "clonefile_fixed_us": wall_fit.map(|(_, i)| round6(i)),
        "kernel_sys_us_per_entry_fit": sys_fit.map(|(s, _)| round6(s)),
        "kernel_sys_fixed_us_fit": sys_fit.map(|(_, i)| round6(i)),
    }))
}
