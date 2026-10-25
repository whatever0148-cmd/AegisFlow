//! AegisFlow hardware benchmark (phase 2).
//!
//! Measures, on this Mac:
//! * APFS, per fixture: cold `clonefile` branch (wall time and kernel time per call),
//!   `WarmBranchPool` warm-hit acquire, standby replenish, `renameatx_np(RENAME_SWAP)` commit,
//!   acquire right after a commit, caller-visible rollback with background reaping, the reaper's
//!   deletion time, synchronous rollback, the APFS collision cases and live-tree integrity;
//! * CoreML: the online context-window model per compute unit (units interleaved in rounds),
//!   `MLComputePlan` per-op placement, cpuOnly / cpuAndNeuralEngine speedup, parity against
//!   `models/parity_vectors.json`, and the batch model;
//! * Cedar evaluation, full pool-branch -> score_window -> Cedar transactions, PayPal order
//!   preparation with ed25519 receipts, tier-0 recovery routing and MCP tool round-trips.
//!
//! Results (sample counts and percentiles, missed targets included) go to
//! `artifacts/aegis_phase1_telemetry.json` (the file name is kept for the existing consumers).
//! `targets_summary` holds exactly the gated targets; `informational` holds context rows;
//! `run_conditions` says whether the run is preliminary (reduced scale, busy machine at the
//! start, attributable CPU contention from other programs during the run, contention not
//! measured, sources changed during the run); `sources` fingerprints the measured code.
//!
//!   cargo run --release -p aegis-bench -- [--out PATH] [--quick | --scale F] [--quiet-wait S] [--work-dir DIR]
//!   cargo run --release -p aegis-bench -- --verify phase1|gate [--artifact PATH]
//!
//! A reduced-scale run (`--quick`, `--scale` < 1) refuses to write the canonical artifact; pass
//! `--out` elsewhere. `--quiet-wait S` waits up to S seconds for the 1-minute load average to drop
//! to the busy threshold before measuring; during the run a background sampler measures the CPU
//! other programs use (`external`, rule in `conditions`). Fixtures live in
//! `target/aegis-bench.noindex` (or `aegis-bench-<pid>.noindex` under `--work-dir`); the
//! `.noindex` suffix keeps Spotlight from indexing them. `--verify` checks an existing artifact
//! (see `gate`).
//!
//! `.env` is loaded through `aegis-config` before anything else. Remote services are never
//! called unless `AEGIS_BENCH_LIVE=1` and their credentials are set.

mod ane;
mod apfs;
mod conditions;
mod external;
mod flows;
mod gate;
mod util;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adapter_paypal::{aggrid_export, bryntum_export};
use aegis_ane_scorer::AneScorer;
use aegis_governance::time::utc_timestamp;
use aegis_governance::GovernanceEngine;
use serde_json::{json, Value};

use conditions::Conditions;
use util::{git_info, host_info, load_average, p50, p95, performance_cores, round6, source_fingerprint, Res};

/// Gated targets (limits, not measurements).
const TARGET_BRANCH_ACQUIRE_MS: f64 = 0.5;
const TARGET_RENAMEATX_SWAP_MS: f64 = 0.5;
const TARGET_ROLLBACK_CALLER_MS: f64 = 0.15;
const TARGET_ANE_INFERENCE_MS: f64 = 0.6;
const TARGET_ANE_SPEEDUP: f64 = 1.0;
const TARGET_TIER0_ROUTE_MS: f64 = 0.4;
/// Every acquire after `settle()` must be a warm hit: a miss means the pool failed.
const TARGET_SETTLED_HIT_RATE: f64 = 1.0;
/// Reference for informational cold-clone rows (the phase-1 branch target).
const REFERENCE_CLONEFILE_MS: f64 = 0.5;
const GATED_ROWS: usize = 11;
const CANONICAL_OUT: &str = "artifacts/aegis_phase1_telemetry.json";
/// Fixture directories end in `.noindex`, which Spotlight skips, so the benchmark's clone and
/// delete churn does not start `mds` / `mdworker` indexing work during the run.
const NOINDEX_SUFFIX: &str = ".noindex";
const DEFAULT_WORK_DIR: &str = "target/aegis-bench.noindex";
const APFS_PRECONDITION: &str = "live-tree integrity after every APFS and end-to-end phase, store clean after pool drop";

/// Latency row from a stats summary (`n`, `p50`, `p95`) whose unit is `to_ms` milliseconds.
/// `pass` is null when there are no samples.
fn latency_row(metric: String, target_ms: f64, samples: &Value, to_ms: f64) -> Value {
    let p50_ms = p50(samples).map(|v| round6(v * to_ms));
    json!({
        "metric": metric,
        "target": format!("< {target_ms} ms"),
        "measured_p50_ms": p50_ms,
        "p95_ms": p95(samples).map(|v| round6(v * to_ms)),
        "n": samples["n"].as_u64().unwrap_or(0),
        "pass": p50_ms.map(|m| m < target_ms),
    })
}

/// Informational latency row: same fields, target marked as a reference.
fn reference_row(metric: String, target_ms: f64, samples: &Value, to_ms: f64) -> Value {
    let mut row = latency_row(metric, target_ms, samples, to_ms);
    row["target"] = json!(format!("< {target_ms} ms (reference)"));
    row
}

/// Informational latency row without a reference target.
fn context_row(metric: String, samples: &Value, to_ms: f64) -> Value {
    let mut row = latency_row(metric, f64::INFINITY, samples, to_ms);
    row["target"] = json!("—");
    row["pass"] = Value::Null;
    row
}

/// A latency verdict combined with preconditions; null stays null (no measurement).
fn gate_pass(latency: &Value, preconditions: &[bool]) -> Value {
    json!(latency.as_bool().map(|ok| ok && preconditions.iter().all(|p| *p)))
}

/// "faster" / "slower" for a cpuOnly-over-cpuAndNeuralEngine speedup.
fn faster_or_slower(speedup: Option<f64>) -> &'static str {
    match speedup {
        Some(s) if s > TARGET_ANE_SPEEDUP => "faster",
        Some(s) if s < TARGET_ANE_SPEEDUP => "slower",
        Some(_) => "as fast",
        None => "n/a",
    }
}

fn placement_row(metric: &str, plan: &Value) -> Value {
    let dispatched = plan["operations"].as_u64();
    let on_ane = plan["preferred"]["neural_engine"].as_u64();
    let pass = match (dispatched, on_ane) {
        (Some(d), Some(a)) if d > 0 => Some(a == d),
        _ => None,
    };
    json!({
        "metric": metric,
        "target": "all ops on ANE",
        "measured": plan["preferred"],
        "dispatched_ops": dispatched,
        "pass": pass,
    })
}

fn info_row(metric: String, target: &str, key: &str, value: Option<f64>, pass: Option<bool>) -> Value {
    let mut row = json!({"metric": metric, "target": target, "pass": pass});
    row[key] = json!(value);
    row
}

fn fixture_label(f: &Value) -> String {
    format!("{} ({} files)", f["name"].as_str().unwrap_or("?"), f["files"])
}

fn fmt_opt(v: Option<f64>, digits: usize) -> String {
    v.map(|x| format!("{x:.digits$}")).unwrap_or_else(|| "n/a".to_string())
}

fn min_max(values: &[f64]) -> Option<(f64, f64)> {
    let lo = values.iter().copied().reduce(f64::min)?;
    let hi = values.iter().copied().reduce(f64::max)?;
    Some((lo, hi))
}

struct Summary {
    gated: Vec<Value>,
    informational: Vec<Value>,
    note: String,
    /// Live-tree integrity held in every phase and every store was clean after its pool dropped.
    integrity_ok: bool,
}

struct IntegrityVerdict {
    checks: u64,
    passed: u64,
    stores_clean: bool,
    ok: bool,
}

fn integrity_verdict(fixtures: &[&Value], e2e: &Value) -> IntegrityVerdict {
    let mut checks = 0u64;
    let mut passed = 0u64;
    for integrity in fixtures.iter().map(|f| &f["integrity"]).chain(std::iter::once(&e2e["integrity"])) {
        checks += integrity["checks"].as_u64().unwrap_or(0);
        passed += integrity["passed"].as_u64().unwrap_or(0);
    }
    let stores_clean = !fixtures.is_empty() && fixtures.iter().all(|f| f["store_after_pool_drop"]["clean"] == true);
    IntegrityVerdict { checks, passed, stores_clean, ok: checks > 0 && passed == checks && stores_clean }
}

fn summarize(apfs: &Value, ane: &Value, router: &Value, e2e: &Value, host: &Value) -> Summary {
    let fixtures: Vec<&Value> = apfs["fixtures"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let fixture = |name: &str| fixtures.iter().copied().find(|f| f["name"] == name).cloned().unwrap_or(Value::Null);
    let small = fixture("small");
    let medium = fixture("medium");
    // Integrity is a precondition of every APFS gated row (P2-R2.3: the live tree stays
    // byte-identical); it is computed first so those rows can depend on it.
    let integrity = integrity_verdict(&fixtures, e2e);

    // ---- gated rows (exactly GATED_ROWS) ----
    let mut gated = Vec::new();
    for f in &fixtures {
        let pool = &f["pool"];
        let mut row = latency_row(
            format!("branch acquire (WarmBranchPool warm hit) · {}", fixture_label(f)),
            TARGET_BRANCH_ACQUIRE_MS,
            &pool["acquire_hit_ms"],
            1.0,
        );
        let hits = pool["hits"].as_u64().unwrap_or(0);
        let misses = pool["misses"].as_u64().unwrap_or(0);
        let hit_rate = (hits + misses > 0).then(|| round6(hits as f64 / (hits + misses) as f64));
        let hit_ok = hit_rate.is_some_and(|r| r >= TARGET_SETTLED_HIT_RATE);
        row["target"] = json!(format!(
            "< {TARGET_BRANCH_ACQUIRE_MS} ms p50 on warm hits, hit rate >= {TARGET_SETTLED_HIT_RATE} (pool settled before each acquire)"
        ));
        row["measured_hit_rate"] = json!(hit_rate);
        row["fixture"] = f["name"].clone();
        row["hits"] = json!(hits);
        row["misses"] = json!(misses);
        row["miss_p50_ms"] = json!(p50(&pool["acquire_miss_ms"]));
        row["requires"] = json!(APFS_PRECONDITION);
        row["live_tree_integrity"] = json!(integrity.ok);
        row["pass"] = gate_pass(&row["pass"], &[hit_ok, integrity.ok]);
        gated.push(row);
    }
    let mut swap = latency_row(
        "renameatx_np RENAME_SWAP commit (syscall only) · small".into(),
        TARGET_RENAMEATX_SWAP_MS,
        &small["renameatx_swap_ms"],
        1.0,
    );
    swap["requires"] = json!(APFS_PRECONDITION);
    swap["live_tree_integrity"] = json!(integrity.ok);
    swap["pass"] = gate_pass(&swap["pass"], &[integrity.ok]);
    gated.push(swap);
    let mut rb = latency_row(
        "rollback · caller-visible (background reap) · small".into(),
        TARGET_ROLLBACK_CALLER_MS,
        &small["pool"]["rollback_caller_ms"],
        1.0,
    );
    rb["requires"] = json!(APFS_PRECONDITION);
    rb["live_tree_integrity"] = json!(integrity.ok);
    rb["pass"] = gate_pass(&rb["pass"], &[integrity.ok]);
    gated.push(rb);
    let ane_p50 = p50(&ane["runs"]["cpu_and_neural_engine"]);
    let cpu_p50 = p50(&ane["runs"]["cpu_only"]);
    gated.push(latency_row(
        "CoreML FP16 online window model · cpuAndNeuralEngine · back-to-back calls".into(),
        TARGET_ANE_INFERENCE_MS,
        &ane["runs"]["cpu_and_neural_engine"],
        1.0,
    ));
    gated.push(placement_row("online model executes on the Neural Engine (MLComputePlan)", &ane["compute_plan"]));
    let speedup = ane["speedup_cpu_only_over_ane_p50"].as_f64();
    gated.push(json!({
        "metric": "online model, back-to-back calls: cpuAndNeuralEngine faster than cpuOnly",
        "target": format!("speedup > {TARGET_ANE_SPEEDUP} (cpuOnly p50 / cpuAndNeuralEngine p50)"),
        "measured": {"cpuAndNeuralEngine p50 ms": ane_p50, "cpuOnly p50 ms": cpu_p50, "speedup": speedup},
        "cpu_and_neural_engine_p50_ms": ane_p50,
        "cpu_only_p50_ms": cpu_p50,
        "speedup": speedup,
        "n": ane["runs"]["cpu_and_neural_engine"]["n"].as_u64().unwrap_or(0),
        "n_cpu_only": ane["runs"]["cpu_only"]["n"].as_u64().unwrap_or(0),
        "pass": speedup.map(|s| s > TARGET_ANE_SPEEDUP),
    }));
    gated.push(placement_row("batch model executes on the Neural Engine (MLComputePlan)", &ane["batch"]["compute_plan"]));
    gated.push(latency_row("tier-0 route (ANE + Cedar)".into(), TARGET_TIER0_ROUTE_MS, &router["tier0_route_us"], 1e-3));

    // ---- informational rows (not gated) ----
    let mut info = Vec::new();
    for f in &fixtures {
        let mut row = latency_row(format!("cold clonefile branch · {}", fixture_label(f)), REFERENCE_CLONEFILE_MS, &f["clonefile_ms"], 1.0);
        row["target"] = json!(format!("< {REFERENCE_CLONEFILE_MS} ms (same reference as the pool rows)"));
        row["fixture"] = f["name"].clone();
        info.push(row);
    }
    for f in &fixtures {
        let mut row = info_row(
            format!("kernel time per clonefile entry (getrusage ru_stime) · {}", fixture_label(f)),
            "— (O(entries) inside the kernel)",
            "measured_per_entry_us",
            f["kernel_clonefile"]["sys_us_per_entry_p50"].as_f64(),
            None,
        );
        row["fixture"] = f["name"].clone();
        row["entries"] = f["entries"].clone();
        row["sys_us_p50"] = json!(p50(&f["kernel_clonefile"]["sys_us"]));
        info.push(row);
    }
    for f in &fixtures {
        let mut row = context_row(
            format!("standby replenish (refill thread, off the caller's path) · {}", fixture_label(f)),
            &f["pool"]["replenish_ms"],
            1.0,
        );
        row["fixture"] = f["name"].clone();
        info.push(row);
    }
    for f in &fixtures {
        let mut row = reference_row(
            format!("commit · caller-visible (lock + checks + RENAME_SWAP + version) · {}", fixture_label(f)),
            TARGET_RENAMEATX_SWAP_MS,
            &f["commit_caller_ms"],
            1.0,
        );
        row["fixture"] = f["name"].clone();
        info.push(row);
    }
    for f in &fixtures {
        let ac = &f["pool"]["acquire_after_commit"];
        let mut row = reference_row(
            format!("acquire immediately after a commit (stale standby) · {}", fixture_label(f)),
            TARGET_BRANCH_ACQUIRE_MS,
            &ac["acquire_ms"],
            1.0,
        );
        row["fixture"] = f["name"].clone();
        row["measured_sources"] = ac["sources"].clone();
        info.push(row);
    }
    for f in &fixtures {
        let mut row = context_row(
            format!("background reap per detached tree (reaper thread) · {}", fixture_label(f)),
            &f["pool"]["background_reaper"]["reap_ms"],
            1.0,
        );
        row["fixture"] = f["name"].clone();
        info.push(row);
    }
    info.push(reference_row(
        "rollback detach only (rename into trash) · small".into(),
        TARGET_ROLLBACK_CALLER_MS,
        &small["pool"]["rollback_detach_ms"],
        1.0,
    ));
    info.push(reference_row(
        "rollback · synchronous (detach + inline delete, phase-1 style) · small".into(),
        TARGET_ROLLBACK_CALLER_MS,
        &small["rollback_sync_caller_ms"],
        1.0,
    ));
    info.push(reference_row(
        "rollback right after acquire (refill clone in flight) · small".into(),
        TARGET_ROLLBACK_CALLER_MS,
        &small["pool"]["collisions"]["rollback_right_after_acquire_ms"],
        1.0,
    ));
    info.push(reference_row(
        "commit right after acquire (refill clone in flight) · small".into(),
        TARGET_RENAMEATX_SWAP_MS,
        &small["pool"]["collisions"]["commit_right_after_acquire_ms"],
        1.0,
    ));
    let rb_collide = p50(&small["pool"]["collisions"]["rollback_right_after_acquire_ms"]);
    let commit_collide = p50(&small["pool"]["collisions"]["commit_right_after_acquire_ms"]);
    info.push(reference_row(
        "CoreML FP16 online window model · all (compute units) · back-to-back calls".into(),
        TARGET_ANE_INFERENCE_MS,
        &ane["runs"]["all"],
        1.0,
    ));
    let iso = &ane["isolated"];
    let gap_ms = iso["method"]["gap_ms"].as_f64();
    let gap_text = fmt_opt(gap_ms, 1);
    for (key, label) in [("busy_gap", "busy CPU"), ("idle_gap", "idle CPU")] {
        let mut row = reference_row(
            format!("CoreML FP16 online window model · cpuAndNeuralEngine · isolated calls ({gap_text} ms {label} gap before each)"),
            TARGET_ANE_INFERENCE_MS,
            &iso[key]["runs"]["cpu_and_neural_engine"],
            1.0,
        );
        row["pattern"] = json!(key);
        info.push(row);
    }
    for (key, label) in [("busy_gap", "busy CPU"), ("idle_gap", "idle CPU")] {
        let runs = &iso[key]["runs"];
        let s = iso[key]["speedup_cpu_only_over_ane_p50"].as_f64();
        info.push(json!({
            "metric": format!("online model, isolated calls ({gap_text} ms {label} gap): cpuAndNeuralEngine faster than cpuOnly"),
            "target": format!("speedup > {TARGET_ANE_SPEEDUP} (reference)"),
            "measured_speedup": s,
            "cpu_and_neural_engine_p50_ms": p50(&runs["cpu_and_neural_engine"]),
            "cpu_only_p50_ms": p50(&runs["cpu_only"]),
            "cpu_and_neural_engine_p95_ms": p95(&runs["cpu_and_neural_engine"]),
            "cpu_only_p95_ms": p95(&runs["cpu_only"]),
            "n": runs["cpu_and_neural_engine"]["n"].as_u64().unwrap_or(0),
            "n_cpu_only": runs["cpu_only"]["n"].as_u64().unwrap_or(0),
            "pattern": key,
            "pass": s.map(|v| v > TARGET_ANE_SPEEDUP),
        }));
    }
    let e2e_score = &e2e["stage_us"]["ane_score"];
    info.push(reference_row(
        "CoreML online window model · one call per transaction (end-to-end ane_score stage, rollback path)".into(),
        TARGET_ANE_INFERENCE_MS,
        e2e_score,
        1e-3,
    ));
    info.push(info_row(
        "CoreML batch window model · per window, amortized (cpuAndNeuralEngine)".into(),
        "—",
        "measured_p50_us",
        ane["batch"]["runs"]["cpu_and_neural_engine"]["amortized_us_per_window_p50"].as_f64(),
        None,
    ));
    let batch_speedup = ane["batch"]["speedup_cpu_only_over_ane_p50"].as_f64();
    info.push(info_row(
        "batch model: cpuAndNeuralEngine faster than cpuOnly".into(),
        &format!("speedup > {TARGET_ANE_SPEEDUP} (reference)"),
        "measured_speedup",
        batch_speedup,
        batch_speedup.map(|s| s > TARGET_ANE_SPEEDUP),
    ));
    let tol = ane["parity"]["tolerance_points"].as_f64();
    let parity_max = ane["parity"]["windows"]["max_abs_diff"].as_f64();
    info.push(info_row(
        "online model parity · sessions (score_window vs FP32 reference, max |diff| points)".into(),
        &tol.map(|t| format!("<= {t} points (parity file tolerance)")).unwrap_or_else(|| "—".into()),
        "measured_points",
        parity_max,
        parity_max.zip(tol).map(|(m, t)| m <= t),
    ));
    info.push(context_row("end-to-end rollback path (pool + score_window + Cedar) · e2e".into(), &e2e["rollback_path_ms"], 1.0));
    info.push(json!({
        "metric": "live-tree integrity after every APFS and end-to-end phase",
        "target": "all checks pass; store empty after pool drop (precondition of the APFS gated rows)",
        "measured": {"checks passed": integrity.passed, "checks": integrity.checks, "stores clean": integrity.stores_clean},
        "pass": integrity.ok,
    }));

    // ---- note generated from the measurements ----
    let hits: Vec<f64> = fixtures.iter().filter_map(|f| p50(&f["pool"]["acquire_hit_ms"])).collect();
    let kernel: Vec<f64> = fixtures.iter().filter_map(|f| f["kernel_clonefile"]["sys_us_per_entry_p50"].as_f64()).collect();
    let total_hits: u64 = fixtures.iter().filter_map(|f| f["pool"]["hits"].as_u64()).sum();
    let total_misses: u64 = fixtures.iter().filter_map(|f| f["pool"]["misses"].as_u64()).sum();
    let tiny = fixture("tiny");
    let ac = &medium["pool"]["acquire_after_commit"];
    let ac_n = ac["acquire_ms"]["n"].as_u64().unwrap_or(0);
    let gated_pass = gated.iter().filter(|r| r["pass"] == true).count();
    let range = |v: &[f64], d: usize| min_max(v).map(|(lo, hi)| format!("{lo:.d$}-{hi:.d$}")).unwrap_or_else(|| "n/a".into());
    let e2e_score_ms = p50(e2e_score).map(|u| u / 1000.0);
    let iso_ane = |k: &str| p50(&iso[k]["runs"]["cpu_and_neural_engine"]);
    let iso_cpu = |k: &str| p50(&iso[k]["runs"]["cpu_only"]);
    let iso_speedup = |k: &str| iso[k]["speedup_cpu_only_over_ane_p50"].as_f64();
    let ratio = |a: Option<f64>, b: Option<f64>| a.zip(b).filter(|(_, b)| *b > 0.0).map(|(a, b)| format!("{:.2}", a / b)).unwrap_or_else(|| "n/a".into());
    let note = format!(
        "Rows 1-4 measure WarmBranchPool::acquire on a warm hit: one renameatx_np(RENAME_EXCL) of a standby cloned earlier, p50 {hit_range} ms across {nfx} fixtures ({total_hits} hits, {total_misses} misses; a row passes only without misses). \
         The gated acquire and rollback rows are timed with the pool idle (settle before each acquire, rollback after the refill finished); the contended cases are reported next to them. \
         The pool does not make clonefile cheaper: a cold clonefile is still O(entries) inside the kernel, p50 {tiny_p50} ms at {tiny_files} files and {medium_p50} ms at {medium_files} files, \
         with getrusage ru_stime of {kernel_range} us per tree entry, and every acquire is paid for by one refill clone off the caller's path (standby replenish p50 {rep_small} ms small, {rep_medium} ms medium). \
         An acquire right after a commit finds the standby stale: at medium {ac_cold}/{ac_n} fell back to a cold clone and {ac_wait}/{ac_n} waited for the refill in flight, p50 {ac_p50} ms. \
         A commit or rollback issued while the refill clone runs waits behind it (small: commit {cc} ms, rollback {rc} ms p50). \
         The commit row times the RENAME_SWAP syscall (small p50 {swap_p50} ms); the caller-visible commit (lock, checks, swap, version) has p50 {commit_caller} ms. \
         The online-model rows time back-to-back calls (cpuAndNeuralEngine p50 {b2b} ms, cpuOnly {b2b_cpu} ms). One call after a {gap_text} ms gap takes {busy} ms with the CPU busy in the gap and {idle} ms with it idle \
         (cpuOnly {cbusy} / {cidle} ms: cpuAndNeuralEngine is {busy_word} than cpuOnly with a busy CPU, speedup {sbusy}, and {idle_word} with an idle CPU, speedup {sidle}). \
         The end-to-end ane_score stage, one call per transaction, has p50 {e2e_ms} ms, {e2e_vs_b2b}x the back-to-back p50 and {e2e_vs_busy}x the busy-gap isolated p50. \
         Live-tree integrity: {ipassed}/{ichecks} checks passed, stores clean: {iclean}{ifail}. \
         One run on {cpu}, {gated_pass}/{GATED_ROWS} gated rows passed.",
        hit_range = range(&hits, 4),
        nfx = fixtures.len(),
        tiny_p50 = fmt_opt(p50(&tiny["clonefile_ms"]), 3),
        tiny_files = tiny["files"],
        medium_p50 = fmt_opt(p50(&medium["clonefile_ms"]), 3),
        medium_files = medium["files"],
        kernel_range = range(&kernel, 2),
        rep_small = fmt_opt(p50(&small["pool"]["replenish_ms"]), 3),
        rep_medium = fmt_opt(p50(&medium["pool"]["replenish_ms"]), 3),
        ac_cold = ac["sources"]["cold"],
        ac_wait = ac["sources"]["pool_after_waiting_for_refill"],
        ac_p50 = fmt_opt(p50(&ac["acquire_ms"]), 3),
        cc = fmt_opt(commit_collide, 3),
        rc = fmt_opt(rb_collide, 3),
        swap_p50 = fmt_opt(p50(&small["renameatx_swap_ms"]), 3),
        commit_caller = fmt_opt(p50(&small["commit_caller_ms"]), 3),
        b2b = fmt_opt(ane_p50, 3),
        b2b_cpu = fmt_opt(cpu_p50, 3),
        busy = fmt_opt(iso_ane("busy_gap"), 3),
        idle = fmt_opt(iso_ane("idle_gap"), 3),
        cbusy = fmt_opt(iso_cpu("busy_gap"), 3),
        cidle = fmt_opt(iso_cpu("idle_gap"), 3),
        busy_word = faster_or_slower(iso_speedup("busy_gap")),
        idle_word = faster_or_slower(iso_speedup("idle_gap")),
        sbusy = fmt_opt(iso_speedup("busy_gap"), 2),
        sidle = fmt_opt(iso_speedup("idle_gap"), 2),
        e2e_ms = fmt_opt(e2e_score_ms, 3),
        e2e_vs_b2b = ratio(e2e_score_ms, ane_p50),
        e2e_vs_busy = ratio(e2e_score_ms, iso_ane("busy_gap")),
        ipassed = integrity.passed,
        ichecks = integrity.checks,
        iclean = integrity.stores_clean,
        ifail = if integrity.ok { "" } else { " (FAILED: the APFS gated rows are marked failed and the benchmark exits non-zero)" },
        cpu = host["cpu"].as_str().unwrap_or("this Mac"),
    );
    tag_sections(&mut gated, &mut info);
    Summary { gated, informational: info, note, integrity_ok: integrity.ok }
}

/// Labels every summary row with the bench section it comes from (`apfs`, `ane`, `router`,
/// `e2e`, `integrity`) so consumers can group rows without parsing metric names.
fn tag_sections(gated: &mut [Value], info: &mut [Value]) {
    for row in gated.iter_mut().chain(info.iter_mut()) {
        let metric = row["metric"].as_str().unwrap_or("").to_string();
        let section = if metric.contains("integrity") {
            "integrity"
        } else if metric.contains("CoreML") || metric.contains("model") {
            "ane"
        } else if metric.starts_with("tier-0") {
            "router"
        } else if metric.starts_with("end-to-end") {
            "e2e"
        } else {
            "apfs"
        };
        row["section"] = json!(section);
    }
}

fn print_rows(title: &str, rows: &[Value], gated: bool) {
    println!("{title}");
    for row in rows {
        let verdict = match (row["pass"].as_bool(), gated) {
            (Some(true), true) => "PASS",
            (Some(false), true) => "MISS",
            (None, true) => "n/a ",
            (Some(true), false) => "info+",
            (Some(false), false) => "info-",
            (None, false) => "info ",
        };
        println!("  [{verdict}] {} — {} (target {})", row["metric"].as_str().unwrap_or(""), gate::row_text(row), row["target"].as_str().unwrap_or(""));
    }
}

#[derive(Debug)]
struct Options {
    out: PathBuf,
    scale: f64,
    quiet_wait: Duration,
    work_dir: Option<PathBuf>,
    verify: Option<gate::Mode>,
    artifact: PathBuf,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Res<Options> {
    let mut opts = Options {
        out: PathBuf::from(CANONICAL_OUT),
        scale: 1.0,
        quiet_wait: Duration::ZERO,
        work_dir: None,
        verify: None,
        artifact: PathBuf::from(CANONICAL_OUT),
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => opts.out = PathBuf::from(args.next().ok_or("--out needs a path")?),
            "--quick" => opts.scale = 0.2,
            "--scale" => {
                opts.scale = args.next().and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.0).ok_or("--scale needs a positive number")?
            }
            "--quiet-wait" => {
                let secs = args.next().and_then(|v| v.parse::<f64>().ok()).filter(|s| *s >= 0.0 && s.is_finite());
                opts.quiet_wait = Duration::from_secs_f64(secs.ok_or("--quiet-wait needs a number of seconds")?);
            }
            "--work-dir" => opts.work_dir = Some(PathBuf::from(args.next().ok_or("--work-dir needs a directory")?)),
            "--verify" => opts.verify = Some(gate::Mode::parse(&args.next().ok_or("--verify needs phase1 or gate")?)?),
            "--artifact" => opts.artifact = PathBuf::from(args.next().ok_or("--artifact needs a path")?),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(opts)
}

fn absolute(path: &Path, root: &Path) -> PathBuf {
    let path = if path.is_relative() { root.join(path) } else { path.to_path_buf() };
    // Resolve `..` and symlinks in the directory part so equal files compare equal.
    match (path.parent().and_then(|d| fs::canonicalize(d).ok()), path.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => path,
    }
}

/// Output path; a reduced-scale run never overwrites the canonical artifact.
fn resolve_out(out: &Path, scale: f64, root: &Path) -> Res<PathBuf> {
    let out = absolute(out, root);
    if scale < 1.0 && out == absolute(Path::new(CANONICAL_OUT), root) {
        return Err(format!(
            "a reduced-scale run (scale {scale}) must not overwrite {CANONICAL_OUT}; pass --out elsewhere, e.g. --out target/bench-smoke.json"
        )
        .into());
    }
    Ok(out)
}

/// Fixture directory path: `target/aegis-bench.noindex`, or `aegis-bench-<pid>.noindex` inside
/// `--work-dir`. Both end in `.noindex` so Spotlight does not index the fixtures.
fn work_dir_path(work_dir: Option<&Path>, root: &Path, pid: u32) -> PathBuf {
    match work_dir {
        None => root.join(DEFAULT_WORK_DIR),
        Some(base) => absolute(base, root).join(format!("aegis-bench-{pid}{NOINDEX_SUFFIX}")),
    }
}

/// Fixture directory: the default one (recreated), or a new per-process directory inside
/// `--work-dir` (never an existing one, so nothing outside the benchmark is removed).
fn prepare_work_dir(work_dir: Option<&Path>, root: &Path) -> Res<PathBuf> {
    let dir = work_dir_path(work_dir, root, std::process::id());
    if dir.exists() {
        if work_dir.is_some() {
            return Err(format!("{} already exists", dir.display()).into());
        }
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn main() -> Res<()> {
    let opts = parse_args(std::env::args().skip(1))?;
    let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))?;
    if let Some(mode) = opts.verify {
        return gate::verify(mode, &absolute(&opts.artifact, &root), &root);
    }
    let out = resolve_out(&opts.out, opts.scale, &root)?;

    // Configuration first: copy .env-only values into the process environment before any thread
    // (pool, reaper, CoreML) exists. Values are never printed.
    let config = match aegis_config::Settings::load_default() {
        Ok(settings) => {
            let exported = aegis_config::export_to_process_env(&settings);
            json!({"dotenv_path": settings.path(), "dotenv_keys_exported": exported.len(), "error": null})
        }
        Err(e) => json!({"dotenv_path": aegis_config::default_dotenv_path(), "dotenv_keys_exported": 0, "error": e.to_string()}),
    };
    eprintln!("aegis-bench: .env keys exported: {} (values not shown)", config["dotenv_keys_exported"]);

    let busy_threshold = performance_cores().map(f64::from);
    let quiet_wait = conditions::wait_for_quiet(busy_threshold, opts.quiet_wait);
    let sources_start = source_fingerprint(&root)?;
    let load_start = load_average();
    let host = host_info(&root);
    let sampler = external::Sampler::start(external::SAMPLE_INTERVAL);
    let started = Instant::now();
    let fixtures = prepare_work_dir(opts.work_dir.as_deref(), &root)?;
    let scale = opts.scale;

    eprintln!("aegis-bench: APFS (cold clonefile, kernel time, WarmBranchPool, commit, rollback, reaper)");
    let apfs = apfs::bench_apfs(&fixtures, &root, scale)?;
    eprintln!("aegis-bench: CoreML window models (back-to-back and isolated calls) + MLComputePlan");
    let ane = ane::bench_ane(&root, scale)?;
    let scorer = Arc::new(AneScorer::open_default()?);
    let gov = Arc::new(GovernanceEngine::load_default()?);
    eprintln!("aegis-bench: Cedar");
    let cedar = flows::bench_cedar(&gov, scale)?;
    eprintln!("aegis-bench: end-to-end transactions");
    let e2e = flows::bench_end_to_end(&fixtures, &scorer, &gov, scale)?;
    eprintln!("aegis-bench: PayPal receipts");
    let (paypal, rows) = flows::bench_paypal(scorer.clone(), gov.clone(), scale)?;
    eprintln!("aegis-bench: tier-0 routing + MCP");
    let router = flows::bench_router(scorer.clone(), gov.clone(), scale)?;
    let mcp = flows::bench_mcp(scorer, gov, scale)?;
    let benchmark_seconds = round6(started.elapsed().as_secs_f64());
    let external = sampler.finish();
    let load_end = load_average();
    let sources_end = source_fingerprint(&root)?;
    let run = conditions::evaluate(&Conditions {
        scale,
        busy_threshold,
        load_start,
        load_end,
        quiet_wait,
        sources_start: sources_start.clone(),
        sources_end,
        external,
    });

    let artifacts = out.parent().ok_or("output path has no parent")?.to_path_buf();
    fs::create_dir_all(&artifacts)?;
    fs::write(artifacts.join("paypal_aggrid_rows.json"), serde_json::to_string_pretty(&aggrid_export(&rows))? + "\n")?;
    fs::write(artifacts.join("paypal_bryntum_store.json"), serde_json::to_string_pretty(&bryntum_export(&rows))? + "\n")?;

    let mut summary = summarize(&apfs, &ane, &router, &e2e, &host);
    if summary.gated.len() != GATED_ROWS {
        return Err(format!("internal: {} gated rows instead of {GATED_ROWS}", summary.gated.len()).into());
    }
    summary.note.push_str(&conditions::note_sentence(&run));
    let doc = json!({
        "schema": gate::SCHEMA,
        "phase": "phase-2",
        "generated_at": utc_timestamp(),
        "git": git_info(&root),
        "sources": sources_start,
        "host": host,
        "config": config,
        "live_mode": flows::live_mode(),
        "benchmark_seconds": benchmark_seconds,
        "scale": scale,
        "preliminary": run["preliminary"],
        "run_conditions": run,
        "targets": {
            "branch_acquire_ms": TARGET_BRANCH_ACQUIRE_MS,
            "settled_acquire_hit_rate": TARGET_SETTLED_HIT_RATE,
            "renameatx_swap_ms": TARGET_RENAMEATX_SWAP_MS,
            "rollback_caller_ms": TARGET_ROLLBACK_CALLER_MS,
            "ane_inference_ms": TARGET_ANE_INFERENCE_MS,
            "ane_speedup": TARGET_ANE_SPEEDUP,
            "tier0_route_ms": TARGET_TIER0_ROUTE_MS,
            "clonefile_reference_ms": REFERENCE_CLONEFILE_MS,
        },
        "apfs": apfs,
        "ane": ane,
        "cedar": cedar,
        "end_to_end": e2e,
        "paypal_receipt": paypal,
        "nebius_router": router,
        "mcp": mcp,
        "external_services": {
            "nebius": "see nebius_router.live_calls",
            "paypal_sandbox": "see paypal_receipt.sandbox_submission",
            "chat_webhooks": "see mcp.webhooks",
        },
        "targets_summary": summary.gated,
        "informational": summary.informational,
        "targets_note": summary.note,
    });
    fs::write(&out, serde_json::to_string_pretty(&doc)? + "\n")?;
    fs::remove_dir_all(&fixtures)?;

    println!("wrote {}", out.display());
    let gated = doc["targets_summary"].as_array().cloned().unwrap_or_default();
    let passed = gated.iter().filter(|r| r["pass"] == true).count();
    let label = if doc["preliminary"] == true { " (PRELIMINARY run, see run_conditions)" } else { "" };
    print_rows(&format!("gated targets: {passed}/{} passed{label}", gated.len()), &gated, true);
    print_rows("informational (not gated):", doc["informational"].as_array().map(Vec::as_slice).unwrap_or(&[]), false);
    println!("note: {}", doc["targets_note"].as_str().unwrap_or(""));
    if !summary.integrity_ok {
        // The artifact is written (the evidence stays), but a broken live tree is an error, not a
        // missed target.
        return Err("live-tree integrity failed (see the informational integrity row and apfs.fixtures[].integrity)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use util::stats;

    /// Samples whose p50 sits at `fraction` of `limit` (synthetic inputs derived from the targets).
    fn at(limit: f64, fraction: f64) -> Value {
        stats(vec![limit * fraction; 6])
    }

    fn fixture(name: &str, hits: u64, misses: u64) -> Value {
        json!({
            "name": name,
            "files": 3,
            "clonefile_ms": at(REFERENCE_CLONEFILE_MS, 2.0),
            "renameatx_swap_ms": at(TARGET_RENAMEATX_SWAP_MS, 0.25),
            "commit_caller_ms": at(TARGET_RENAMEATX_SWAP_MS, 0.5),
            "pool": {
                "acquire_hit_ms": at(TARGET_BRANCH_ACQUIRE_MS, 0.25),
                "acquire_miss_ms": if misses > 0 { at(REFERENCE_CLONEFILE_MS, 2.0) } else { stats(Vec::new()) },
                "hits": hits,
                "misses": misses,
                "rollback_caller_ms": at(TARGET_ROLLBACK_CALLER_MS, 0.5),
            },
            "integrity": {"checks": 7, "passed": 7},
            "store_after_pool_drop": {"clean": true},
        })
    }

    struct Inputs {
        apfs: Value,
        ane: Value,
        router: Value,
        e2e: Value,
        host: Value,
    }

    fn inputs() -> Inputs {
        let plan = json!({"operations": 4, "preferred": {"neural_engine": 4, "cpu": 0, "gpu": 0}});
        let ane_runs = json!({
            "cpu_and_neural_engine": at(TARGET_ANE_INFERENCE_MS, 0.25),
            "cpu_only": at(TARGET_ANE_INFERENCE_MS, 0.5),
            "all": at(TARGET_ANE_INFERENCE_MS, 0.25),
        });
        Inputs {
            apfs: json!({"fixtures": gate::FIXTURE_NAMES.iter().map(|n| fixture(n, 6, 0)).collect::<Vec<_>>()}),
            ane: json!({
                "runs": ane_runs.clone(),
                "speedup_cpu_only_over_ane_p50": 2.0,
                "compute_plan": plan.clone(),
                "batch": {"compute_plan": plan},
                "isolated": {"method": {"gap_ms": util::ms(Duration::from_millis(3))}, "busy_gap": {"runs": ane_runs.clone(), "speedup_cpu_only_over_ane_p50": 2.0},
                             "idle_gap": {"runs": ane_runs, "speedup_cpu_only_over_ane_p50": 2.0}},
            }),
            router: json!({"tier0_route_us": at(TARGET_TIER0_ROUTE_MS * 1000.0, 0.25)}),
            e2e: json!({"integrity": {"checks": 2, "passed": 2}, "stage_us": {"ane_score": at(TARGET_ANE_INFERENCE_MS * 1000.0, 0.5)}}),
            host: json!({"cpu": "test host"}),
        }
    }

    fn run(i: &Inputs) -> Summary {
        summarize(&i.apfs, &i.ane, &i.router, &i.e2e, &i.host)
    }

    #[test]
    fn all_targets_met_gives_eleven_passing_rows_with_n_and_p95() {
        let s = run(&inputs());
        assert_eq!(s.gated.len(), GATED_ROWS);
        assert!(s.integrity_ok);
        for row in &s.gated {
            assert_eq!(row["pass"], true, "{row}");
            if row.get("measured_p50_ms").is_some() {
                assert_eq!(row["n"], 6, "latency rows carry n: {row}");
                assert!(row["p95_ms"].is_number(), "latency rows carry p95: {row}");
            }
        }
        let tier0 = s.gated.iter().find(|r| r["metric"].as_str().unwrap().starts_with("tier-0")).unwrap();
        assert_eq!(tier0["measured_p50_ms"].as_f64(), Some(round6(TARGET_TIER0_ROUTE_MS * 0.25)), "us converted to ms");
        assert!(s.gated.iter().all(|r| r["measured_hit_rate"].is_null() || r["measured_hit_rate"] == 1.0));
        let integrity = s.informational.iter().find(|r| r["section"] == "integrity").unwrap();
        assert_eq!(integrity["pass"], true);
    }

    #[test]
    fn a_miss_under_a_settled_pool_fails_the_acquire_row() {
        let mut i = inputs();
        i.apfs["fixtures"][2] = fixture("medium", 1, 59);
        let s = run(&i);
        let medium = s.gated.iter().find(|r| r["fixture"] == "medium").unwrap();
        assert!(medium["measured_p50_ms"].as_f64().unwrap() < TARGET_BRANCH_ACQUIRE_MS, "warm hits alone are fast");
        assert_eq!(medium["pass"], false, "a miss fails the row: {medium}");
        assert!(medium["measured_hit_rate"].as_f64().unwrap() < TARGET_SETTLED_HIT_RATE);
        assert!(medium["miss_p50_ms"].is_number());
        assert!(gate::row_text(medium).contains("hit rate"));
        assert_eq!(s.gated.iter().filter(|r| r["pass"] == true).count(), GATED_ROWS - 1);
    }

    #[test]
    fn integrity_failure_fails_the_apfs_gated_rows() {
        let mut i = inputs();
        i.apfs["fixtures"][1]["integrity"]["passed"] = json!(6);
        let s = run(&i);
        assert!(!s.integrity_ok);
        let apfs: Vec<&Value> = s.gated.iter().filter(|r| r["section"] == "apfs").collect();
        assert_eq!(apfs.len(), 6, "4 acquire rows, commit, rollback");
        assert!(apfs.iter().all(|r| r["pass"] == false && r["live_tree_integrity"] == false), "{apfs:?}");
        assert!(s.gated.iter().filter(|r| r["section"] != "apfs").all(|r| r["pass"] == true));
        assert!(s.note.contains("FAILED"));

        let mut dirty_store = inputs();
        dirty_store.apfs["fixtures"][0]["store_after_pool_drop"]["clean"] = json!(false);
        let s = run(&dirty_store);
        assert!(!s.integrity_ok);
        let rollback = s.gated.iter().find(|r| r["metric"].as_str().unwrap().starts_with("rollback")).unwrap();
        assert_eq!(rollback["pass"], false);
    }

    #[test]
    fn missing_measurement_stays_null() {
        let mut i = inputs();
        i.router["tier0_route_us"] = stats(Vec::new());
        i.apfs["fixtures"][0]["pool"]["acquire_hit_ms"] = stats(Vec::new());
        i.apfs["fixtures"][0]["pool"]["hits"] = json!(0);
        let s = run(&i);
        let tier0 = s.gated.iter().find(|r| r["metric"].as_str().unwrap().starts_with("tier-0")).unwrap();
        assert!(tier0["pass"].is_null() && tier0["n"] == 0);
        assert!(s.gated[0]["pass"].is_null());
    }

    #[test]
    fn isolated_and_caller_visible_rows_are_informational() {
        let s = run(&inputs());
        let metrics: Vec<&str> = s.informational.iter().filter_map(|r| r["metric"].as_str()).collect();
        assert!(metrics.iter().any(|m| m.contains("isolated calls") && m.contains("busy CPU")));
        assert!(metrics.iter().any(|m| m.contains("isolated calls") && m.contains("idle CPU")));
        assert!(metrics.iter().any(|m| m.contains("one call per transaction")));
        assert!(metrics.iter().filter(|m| m.starts_with("commit · caller-visible")).count() == gate::FIXTURE_NAMES.len());
        assert!(s.gated.iter().any(|r| r["metric"].as_str().unwrap().contains("(syscall only)")));
        assert!(s.gated.iter().filter(|r| r["section"] == "ane").any(|r| r["metric"].as_str().unwrap().contains("back-to-back")));
        let e2e_row = s.informational.iter().find(|r| r["metric"].as_str().unwrap().contains("one call per transaction")).unwrap();
        assert_eq!(e2e_row["section"], "ane");
        assert_eq!(e2e_row["measured_p50_ms"].as_f64(), Some(round6(TARGET_ANE_INFERENCE_MS * 0.5)));
        assert!(s.note.contains("idle"), "{}", s.note);
    }

    #[test]
    fn reduced_scale_cannot_overwrite_the_canonical_artifact() {
        let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
        let canonical = Path::new(CANONICAL_OUT);
        assert!(resolve_out(canonical, 0.2, &root).is_err());
        assert!(resolve_out(&root.join("artifacts/../artifacts/aegis_phase1_telemetry.json"), 0.1, &root).is_err());
        assert_eq!(resolve_out(canonical, 1.0, &root).unwrap(), root.join(CANONICAL_OUT));
        assert_eq!(resolve_out(Path::new("target/bench-smoke.json"), 0.1, &root).unwrap(), root.join("target/bench-smoke.json"));

        let quick = parse_args(["--quick".to_string()]).unwrap();
        assert!(resolve_out(&quick.out, quick.scale, &root).is_err(), "--quick without --out is refused");
    }

    #[test]
    fn work_dirs_end_in_noindex_so_spotlight_skips_them() {
        let root = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
        let default = work_dir_path(None, &root, 7);
        assert_eq!(default, root.join("target/aegis-bench.noindex"));
        let base = std::env::temp_dir().join(format!("aegis-bench-workdir-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let per_process = work_dir_path(Some(&base), &root, 7);
        assert_eq!(per_process.file_name().unwrap(), "aegis-bench-7.noindex");
        for dir in [&default, &per_process] {
            assert!(dir.extension().is_some_and(|e| e == "noindex"), "{}", dir.display());
        }
        // A per-process directory is created fresh and never reused.
        let made = prepare_work_dir(Some(&base), &root).unwrap();
        assert!(made.is_dir() && made.to_string_lossy().ends_with(NOINDEX_SUFFIX), "{}", made.display());
        assert!(prepare_work_dir(Some(&base), &root).is_err(), "an existing per-process directory is refused");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn parses_verify_and_run_options() {
        let opts = parse_args(["--verify", "gate", "--artifact", "x.json"].map(String::from)).unwrap();
        assert_eq!(opts.verify, Some(gate::Mode::Gate));
        assert_eq!(opts.artifact, PathBuf::from("x.json"));
        let opts = parse_args(["--quiet-wait", "30", "--scale", "0.5", "--work-dir", "w"].map(String::from)).unwrap();
        assert_eq!(opts.quiet_wait, Duration::from_secs(30));
        assert_eq!(opts.scale, 0.5);
        assert_eq!(opts.work_dir, Some(PathBuf::from("w")));
        assert!(parse_args(["--verify", "everything"].map(String::from)).is_err());
        assert!(parse_args(["--quiet-wait", "-1"].map(String::from)).is_err());
    }
}
