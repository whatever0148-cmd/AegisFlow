//! CoreML section: the online context-window model (`AegisWorkflowFP16.mlmodelc`) and the batch
//! model (`AegisWorkflowFP16Batch.mlmodelc`).
//!
//! * Placement: `MLComputePlan` for `.cpuAndNeuralEngine`, counts and the per-op device list.
//! * Latency per compute unit, with the units interleaved in rounds (the unit that goes first
//!   rotates every round) so background load hits every unit alike. Each call scores one full
//!   window (`window_len()` events) through `AneScorer::score_window`, i.e. FFI + FP16 fill +
//!   CoreML prediction. Two call patterns: back-to-back calls (the gated rows), and isolated
//!   calls, one after a fixed gap in which the CPU either spins (as during diff and feature
//!   extraction) or sleeps (as while a transaction waits on I/O); a transaction makes one
//!   prediction, so the isolated pattern is the one the pipeline sees.
//! * Parity against `models/parity_vectors.json` (format 2): single events through `score()` and
//!   multi-event sessions through `score_window()`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use aegis_ane_scorer::{
    compute_plan, default_batch_model_path, default_model_path, neural_engine_cores, AneScorer, ComputePlanReport,
    ComputeUnits, ContextWindow, FeatureVector, FEATURE_COUNT,
};
use serde_json::{json, Value};

use crate::util::{ms, p50, round6, scaled, stats, Res};

/// Distinct full windows cycled through during latency runs.
const BENCH_WINDOWS: usize = 16;
const ONLINE_WARMUP: usize = 300;
const ONLINE_ROUNDS: usize = 40;
const ONLINE_PER_ROUND: usize = 60;
/// Isolated calls: one prediction after a fixed gap (method parameters, not measurements).
const ISOLATED_GAP: Duration = Duration::from_millis(3);
const ISOLATED_ROUNDS: usize = 20;
const ISOLATED_PER_ROUND: usize = 20;
const BATCH_WARMUP: usize = 20;
const BATCH_ROUNDS: usize = 20;
const BATCH_PER_ROUND: usize = 10;

pub struct ParityWindow {
    pub scenario: String,
    pub events: Vec<FeatureVector>,
    pub reference: f64,
    pub build_time_coreml: Option<f64>,
}

pub struct Parity {
    pub format: Option<u64>,
    pub tolerance: Option<f64>,
    pub vectors: Vec<FeatureVector>,
    pub vector_reference: Vec<f64>,
    pub windows: Vec<ParityWindow>,
}

fn dense(row: &Value) -> FeatureVector {
    let mut v = FeatureVector::default();
    for (slot, x) in v.0.iter_mut().zip(row.as_array().into_iter().flatten()) {
        *slot = x.as_f64().unwrap_or(0.0) as f32;
    }
    v
}

fn sparse(pairs: &Value) -> Res<FeatureVector> {
    let mut v = FeatureVector::default();
    for pair in pairs.as_array().ok_or("parity event is not a list")? {
        let idx = pair[0].as_u64().ok_or("parity event index missing")? as usize;
        let val = pair[1].as_f64().ok_or("parity event value missing")?;
        *v.0.get_mut(idx).ok_or("parity event index out of range")? = val as f32;
    }
    Ok(v)
}

pub fn load_parity(root: &Path) -> Res<Parity> {
    let doc: Value = serde_json::from_str(&fs::read_to_string(root.join("models/parity_vectors.json"))?)?;
    let vectors: Vec<FeatureVector> = doc["vectors"].as_array().ok_or("parity vectors missing")?.iter().map(dense).collect();
    let vector_reference: Vec<f64> =
        doc["reference_fp32"].as_array().ok_or("parity reference_fp32 missing")?.iter().filter_map(Value::as_f64).collect();
    if vector_reference.len() != vectors.len() {
        return Err(format!("parity: {} vectors but {} references", vectors.len(), vector_reference.len()).into());
    }
    let mut windows = Vec::new();
    for w in doc["windows"].as_array().into_iter().flatten() {
        let events = w["events"].as_array().ok_or("parity window events missing")?.iter().map(sparse).collect::<Res<Vec<_>>>()?;
        windows.push(ParityWindow {
            scenario: w["scenario"].as_str().unwrap_or("unknown").to_string(),
            events,
            reference: w["reference_fp32"].as_f64().ok_or("parity window reference_fp32 missing")?,
            build_time_coreml: w["coreml_fp16_cpu_and_ne"].as_f64(),
        });
    }
    Ok(Parity { format: doc["format"].as_u64(), tolerance: doc["tolerance_points"].as_f64(), vectors, vector_reference, windows })
}

/// Full windows of `width` consecutive events taken from the parity sessions (concatenated).
fn full_windows(parity: &Parity, width: usize, count: usize) -> Res<Vec<ContextWindow>> {
    let events: Vec<FeatureVector> = parity.windows.iter().flat_map(|w| w.events.iter().copied()).collect();
    if events.is_empty() {
        return Err("parity file has no window events".into());
    }
    let stride = (events.len() / count.max(1)).max(1);
    Ok((0..count).map(|k| ContextWindow::from_events(width, events.iter().cycle().skip(k * stride).take(width).copied())).collect())
}

pub fn plan_summary(plan: &ComputePlanReport) -> Value {
    let count = |k: &str| plan.preferred.get(k).copied().unwrap_or(0);
    json!({
        "compute_units": plan.compute_units,
        "operations": plan.operations_dispatched,
        "operations_total": plan.operations_total,
        "preferred": {"neural_engine": count("neural_engine"), "cpu": count("cpu"), "gpu": count("gpu")},
        "all_on_neural_engine": plan.all_dispatched_on_neural_engine(),
        "ops": plan.dispatched_ops().enumerate().map(|(i, o)| json!({
            "index": i,
            "operator": o.operator,
            "preferred": o.preferred,
            "supported": o.supported,
            "estimated_cost": o.estimated_cost,
        })).collect::<Vec<_>>(),
        "not_on_neural_engine": plan.dispatched_ops().filter(|o| o.preferred != "neural_engine").map(|o| json!({
            "operator": o.operator, "preferred": o.preferred, "outputs": o.outputs,
        })).collect::<Vec<_>>(),
    })
}

fn diff_summary(diffs: &[f64], tolerance: Option<f64>) -> Value {
    let n = diffs.len();
    json!({
        "n": n,
        "max_abs_diff": round6(diffs.iter().copied().fold(0.0, f64::max)),
        "mean_abs_diff": if n > 0 { round6(diffs.iter().sum::<f64>() / n as f64) } else { 0.0 },
        "within_tolerance": tolerance.map(|t| diffs.iter().filter(|d| **d <= t).count()),
    })
}

/// Interleaved latency: `rounds` rounds, each running every unit for `per_round` calls, the
/// starting unit rotating per round. `before` runs ahead of every call, outside the timed region.
/// Returns per-unit samples in ms.
fn interleaved<T>(
    units: &[T],
    rounds: usize,
    per_round: usize,
    mut before: impl FnMut(),
    mut call: impl FnMut(&T, usize) -> Res<()>,
) -> Res<Vec<Vec<f64>>> {
    let mut samples = vec![Vec::with_capacity(rounds * per_round); units.len()];
    let mut k = 0usize;
    for round in 0..rounds {
        for offset in 0..units.len() {
            let idx = (round + offset) % units.len();
            for _ in 0..per_round {
                before();
                let started = Instant::now();
                call(&units[idx], k)?;
                samples[idx].push(ms(started.elapsed()));
                k += 1;
            }
        }
    }
    Ok(samples)
}

/// What the CPU does in the gap before an isolated call.
#[derive(Clone, Copy)]
enum Gap {
    /// Spin on the calling core (CPU busy, as during diff and feature extraction).
    Busy,
    /// `thread::sleep` (CPU idle, as while a transaction waits on I/O or the pool).
    Idle,
}

fn gap(kind: Gap, length: Duration) {
    match kind {
        Gap::Busy => {
            let started = Instant::now();
            while started.elapsed() < length {
                std::hint::spin_loop();
            }
        }
        Gap::Idle => std::thread::sleep(length),
    }
}

fn speedup_of(runs: &Value) -> Option<f64> {
    p50(&runs["cpu_only"]).zip(p50(&runs["cpu_and_neural_engine"])).filter(|(_, a)| *a > 0.0).map(|(c, a)| round6(c / a))
}

/// Isolated calls on cpuAndNeuralEngine and cpuOnly (units interleaved in rounds), one prediction
/// after each gap.
fn isolated(scorers: &[AneScorer], windows: &[ContextWindow], kind: Gap, rounds: usize) -> Res<Value> {
    let samples = interleaved(scorers, rounds, ISOLATED_PER_ROUND, || gap(kind, ISOLATED_GAP), |s, k| {
        s.score_window(&windows[k % windows.len()])?;
        Ok(())
    })?;
    let mut runs = serde_json::Map::new();
    for (key, s) in ["cpu_and_neural_engine", "cpu_only"].iter().zip(samples) {
        runs.insert((*key).to_string(), stats(s));
    }
    let runs = Value::Object(runs);
    Ok(json!({"speedup_cpu_only_over_ane_p50": speedup_of(&runs), "runs": runs}))
}

pub fn bench_ane(root: &Path, scale: f64) -> Res<Value> {
    let parity = load_parity(root)?;
    let model = default_model_path();
    let plan = compute_plan(&model, ComputeUnits::CpuAndNeuralEngine)?;
    let plan_all = compute_plan(&model, ComputeUnits::All)?;
    let units = [
        ("cpu_and_neural_engine", ComputeUnits::CpuAndNeuralEngine),
        ("cpu_only", ComputeUnits::CpuOnly),
        ("all", ComputeUnits::All),
    ];
    let scorers = units.iter().map(|(_, u)| AneScorer::open(&model, *u)).collect::<Result<Vec<_>, _>>()?;
    let online = &scorers[0];
    let window_len = online.window_len();
    let windows = full_windows(&parity, window_len, BENCH_WINDOWS)?;

    // Latency, units interleaved.
    for scorer in &scorers {
        for i in 0..ONLINE_WARMUP {
            scorer.score_window(&windows[i % windows.len()])?;
        }
    }
    let rounds = scaled(ONLINE_ROUNDS, scale);
    let samples = interleaved(&scorers, rounds, ONLINE_PER_ROUND, || (), |s, k| {
        s.score_window(&windows[k % windows.len()])?;
        Ok(())
    })?;
    let mut runs = serde_json::Map::new();
    for ((key, _), s) in units.iter().zip(samples) {
        runs.insert((*key).to_string(), stats(s));
    }
    let runs = Value::Object(runs);
    let speedup = speedup_of(&runs);

    // Isolated calls (one prediction after a gap), cpuAndNeuralEngine and cpuOnly interleaved.
    let isolated_rounds = scaled(ISOLATED_ROUNDS, scale);
    let isolated_busy = isolated(&scorers[..2], &windows, Gap::Busy, isolated_rounds)?;
    let isolated_idle = isolated(&scorers[..2], &windows, Gap::Idle, isolated_rounds)?;

    // Agreement between compute units on the benchmark windows.
    let per_unit: Vec<Vec<f32>> = scorers.iter().map(|s| s.score_windows(&windows)).collect::<Result<_, _>>()?;
    let unit_diff = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| f64::from((x - y).abs())).fold(0.0, f64::max);

    // Parity: single events (score) and sessions (score_window), both units.
    let vector_diffs: Vec<f64> = parity
        .vectors
        .iter()
        .zip(&parity.vector_reference)
        .map(|(v, r)| online.score(v).map(|s| (f64::from(s) - r).abs()))
        .collect::<Result<_, _>>()?;
    let session_windows: Vec<ContextWindow> =
        parity.windows.iter().map(|w| ContextWindow::from_events(window_len, w.events.iter().copied())).collect();
    let mut window_diffs = Vec::new();
    let mut window_diffs_cpu = Vec::new();
    let mut build_time_diffs = Vec::new();
    let mut by_scenario: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    for (pw, cw) in parity.windows.iter().zip(&session_windows) {
        let ane = f64::from(online.score_window(cw)?);
        let cpu = f64::from(scorers[1].score_window(cw)?);
        let d = (ane - pw.reference).abs();
        window_diffs.push(d);
        window_diffs_cpu.push((cpu - pw.reference).abs());
        if let Some(b) = pw.build_time_coreml {
            build_time_diffs.push((ane - b).abs());
        }
        let e = by_scenario.entry(pw.scenario.clone()).or_insert((0, 0.0, 0.0));
        e.0 += 1;
        e.1 = e.1.max(d);
        e.2 += d;
    }
    let scenarios: serde_json::Map<String, Value> = by_scenario
        .into_iter()
        .map(|(k, (n, max, sum))| (k, json!({"n": n, "max_abs_diff": round6(max), "mean_abs_diff": round6(sum / n as f64)})))
        .collect();

    // Batch model, as left by the model build: B windows per prediction.
    let batch_model = default_batch_model_path();
    let batch_plan = compute_plan(&batch_model, ComputeUnits::CpuAndNeuralEngine)?;
    let batch_units = [("cpu_and_neural_engine", ComputeUnits::CpuAndNeuralEngine), ("cpu_only", ComputeUnits::CpuOnly)];
    let batch_scorers = batch_units.iter().map(|(_, u)| AneScorer::open(&batch_model, *u)).collect::<Result<Vec<_>, _>>()?;
    let batch_size = batch_scorers[0].batch_size();
    let full: Vec<ContextWindow> = (0..batch_size).map(|i| windows[i % windows.len()].clone()).collect();
    for scorer in &batch_scorers {
        for _ in 0..BATCH_WARMUP {
            scorer.score_windows(&full)?;
        }
    }
    let batch_rounds = scaled(BATCH_ROUNDS, scale);
    let batch_samples = interleaved(&batch_scorers, batch_rounds, BATCH_PER_ROUND, || (), |s, _| {
        s.score_windows(&full)?;
        Ok(())
    })?;
    let mut batch_runs = serde_json::Map::new();
    for ((key, _), s) in batch_units.iter().zip(batch_samples) {
        let summary = stats(s);
        let per_window = p50(&summary).map(|p| round6(p * 1000.0 / batch_size as f64));
        batch_runs.insert((*key).to_string(), json!({"batch_ms": summary, "amortized_us_per_window_p50": per_window}));
    }
    let batch_runs = Value::Object(batch_runs);
    let batch_speedup = p50(&batch_runs["cpu_only"]["batch_ms"])
        .zip(p50(&batch_runs["cpu_and_neural_engine"]["batch_ms"]))
        .filter(|(_, a)| *a > 0.0)
        .map(|(c, a)| round6(c / a));
    let batch_scores = batch_scorers[0].score_windows(&session_windows)?;
    let batch_diffs: Vec<f64> = batch_scores.iter().zip(&parity.windows).map(|(s, w)| (f64::from(*s) - w.reference).abs()).collect();
    let batch_vec = batch_scorers[0].score_batch(&parity.vectors)?;
    let batch_vec_diffs: Vec<f64> = batch_vec.iter().zip(&parity.vector_reference).map(|(s, r)| (f64::from(*s) - r).abs()).collect();

    let count = |p: &ComputePlanReport, k: &str| p.preferred.get(k).copied().unwrap_or(0);
    let note = format!(
        "MLComputePlan (.cpuAndNeuralEngine) placed the online window model ({} layout, {} events x {} features) as ANE {} / CPU {} / GPU {} of {} dispatched ops ({} including constants); the batch model ({} windows per prediction) as ANE {} / CPU {} / GPU {} of {}.",
        online.layout().name(),
        window_len,
        online.feature_count(),
        count(&plan, "neural_engine"),
        count(&plan, "cpu"),
        count(&plan, "gpu"),
        plan.operations_dispatched,
        plan.operations_total,
        batch_size,
        count(&batch_plan, "neural_engine"),
        count(&batch_plan, "cpu"),
        count(&batch_plan, "gpu"),
        batch_plan.operations_dispatched,
    );
    Ok(json!({
        "model": "models/AegisWorkflowFP16.mlmodelc",
        "precision": "float16",
        "layout": online.layout().name(),
        "window_len": window_len,
        "batch_size": online.batch_size(),
        "feature_count": online.feature_count(),
        "neural_engine_cores": neural_engine_cores()?,
        "method": {
            "call": "AneScorer::score_window on one full window per call (FFI + FP16 fill + CoreML prediction)",
            "pattern": "back-to-back: per_round consecutive calls per unit, no gap between calls",
            "interleaved": true,
            "rounds": rounds,
            "per_round": ONLINE_PER_ROUND,
            "warmup_per_unit": ONLINE_WARMUP,
            "distinct_windows": windows.len(),
        },
        "runs": runs,
        "speedup_cpu_only_over_ane_p50": speedup,
        "isolated": {
            "method": {
                "call": "one AneScorer::score_window per gap, units cpuAndNeuralEngine and cpuOnly interleaved in rounds; the gap is outside the timed region",
                "gap_ms": ms(ISOLATED_GAP),
                "busy_gap": "the calling thread spins for the gap (CPU busy, as during diff and feature extraction)",
                "idle_gap": "the calling thread sleeps for the gap (CPU idle, as while a transaction waits on I/O or the pool)",
                "rounds": isolated_rounds,
                "per_round": ISOLATED_PER_ROUND,
            },
            "busy_gap": isolated_busy,
            "idle_gap": isolated_idle,
        },
        "compute_plan": plan_summary(&plan),
        "compute_plan_all_units": {
            "compute_units": plan_all.compute_units,
            "preferred": {"neural_engine": count(&plan_all, "neural_engine"), "cpu": count(&plan_all, "cpu"), "gpu": count(&plan_all, "gpu")},
            "operations": plan_all.operations_dispatched,
        },
        "unit_agreement_max_abs_diff": {
            "cpu_only_vs_ane": round6(unit_diff(&per_unit[1], &per_unit[0])),
            "all_vs_ane": round6(unit_diff(&per_unit[2], &per_unit[0])),
        },
        "parity": {
            "format": parity.format,
            "tolerance_points": parity.tolerance,
            "reference": "reference_fp32 in models/parity_vectors.json: FP32 forward pass of the trained weights (PyTorch, per the model build)",
            "vectors": vector_diffs.len(),
            "max_abs_diff": round6(vector_diffs.iter().copied().fold(0.0, f64::max)),
            "mean_abs_diff": if vector_diffs.is_empty() { 0.0 } else { round6(vector_diffs.iter().sum::<f64>() / vector_diffs.len() as f64) },
            "single_events": diff_summary(&vector_diffs, parity.tolerance),
            "windows": diff_summary(&window_diffs, parity.tolerance),
            "windows_cpu_only": diff_summary(&window_diffs_cpu, parity.tolerance),
            "windows_by_scenario": scenarios,
            "windows_vs_build_time_coreml_max_abs_diff": round6(build_time_diffs.iter().copied().fold(0.0, f64::max)),
        },
        "batch": {
            "model": "models/AegisWorkflowFP16Batch.mlmodelc",
            "layout": batch_scorers[0].layout().name(),
            "batch_size": batch_size,
            "compute_plan": plan_summary(&batch_plan),
            "method": {"interleaved": true, "rounds": batch_rounds, "per_round": BATCH_PER_ROUND, "warmup_per_unit": BATCH_WARMUP},
            "runs": batch_runs,
            "speedup_cpu_only_over_ane_p50": batch_speedup,
            "parity_max_abs_diff": round6(batch_diffs.iter().copied().fold(0.0, f64::max)),
            "parity_windows": diff_summary(&batch_diffs, parity.tolerance),
            "parity_single_events": diff_summary(&batch_vec_diffs, parity.tolerance),
        },
        "feature_count_expected": FEATURE_COUNT,
        "placement_note": note,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn interleaved_rotates_units_and_runs_before_outside_each_call() {
        let units = ["a", "b", "c"];
        let log = RefCell::new(Vec::new());
        let samples = interleaved(
            &units,
            3,
            2,
            || log.borrow_mut().push("gap".to_string()),
            |u, _| {
                log.borrow_mut().push((*u).to_string());
                Ok(())
            },
        )
        .unwrap();
        assert!(samples.iter().all(|s| s.len() == 6), "every unit gets rounds x per_round samples");
        let log = log.into_inner();
        // Every call is preceded by exactly one `before`.
        assert_eq!(log.len(), 2 * 3 * 3 * 2);
        assert!(log.chunks(2).all(|pair| pair[0] == "gap" && pair[1] != "gap"));
        // The unit that goes first rotates per round.
        let calls: Vec<&str> = log.iter().filter(|e| *e != "gap").map(String::as_str).collect();
        let firsts: Vec<&str> = calls.chunks(units.len() * 2).map(|round| round[0]).collect();
        assert_eq!(firsts, ["a", "b", "c"]);
    }

    #[test]
    fn speedup_is_cpu_only_over_ane() {
        let ane = 1.0;
        let runs = json!({"cpu_and_neural_engine": stats(vec![ane; 3]), "cpu_only": stats(vec![ane * 2.0; 3])});
        assert_eq!(speedup_of(&runs), Some(2.0));
        assert_eq!(speedup_of(&json!({"cpu_only": stats(vec![ane])})), None);
    }
}
