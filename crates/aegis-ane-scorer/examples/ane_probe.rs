//! Placement and latency probe for compiled AegisFlow risk models, through the same Swift bridge
//! the daemon uses.
//!
//! ```text
//! cargo run --release -p aegis-ane-scorer --example ane_probe -- MODEL.mlmodelc [MODEL2.mlmodelc ...]
//!     [--rounds 40] [--per-round 60] [--warmup 300]
//! ```
//!
//! For every model it prints one JSON line with
//! * the `MLComputePlan` placement for `.cpuAndNeuralEngine` (preferred device per dispatched op),
//! * per-prediction latency for `.cpuAndNeuralEngine` and `.cpuOnly`, measured in alternating rounds
//!   (the unit that goes first alternates every round) so background load hits both units alike.
//!
//! Window models are fed a full window of deterministic pseudo-random sparse events; batch models
//! a full batch of such windows per prediction. Latencies are wall-clock times of
//! `AneScorer::score_windows` and include the FFI call, the FP16 input fill and CoreML.

use std::path::PathBuf;
use std::time::Instant;

use aegis_ane_scorer::{compute_plan, AneScorer, ComputeUnits, ContextWindow, FeatureVector, FEATURE_COUNT};
use serde_json::{json, Value};

struct Lcg(u64);

impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

fn sample_window(rng: &mut Lcg, width: usize) -> ContextWindow {
    let mut window = ContextWindow::new(width);
    for _ in 0..width {
        let mut v = FeatureVector::default();
        for slot in v.0.iter_mut() {
            if rng.next_f32() < 0.12 {
                *slot = rng.next_f32();
            }
        }
        window.push(v);
    }
    window
}

fn stats(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let pick = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let round = |x: f64| (x * 1e6).round() / 1e6;
    json!({
        "n": samples.len(),
        "p50": round(pick(0.5)),
        "p95": round(pick(0.95)),
        "p99": round(pick(0.99)),
        "min": round(samples[0]),
        "mean": round(mean),
    })
}

fn probe(model: &PathBuf, rounds: usize, per_round: usize, warmup: usize) -> Result<Value, Box<dyn std::error::Error>> {
    let plan = compute_plan(model, ComputeUnits::CpuAndNeuralEngine)?;
    let ane = AneScorer::open(model, ComputeUnits::CpuAndNeuralEngine)?;
    let cpu = AneScorer::open(model, ComputeUnits::CpuOnly)?;
    let mut rng = Lcg(0x5eed_a11c_e5);
    let windows: Vec<ContextWindow> = (0..ane.batch_size()).map(|_| sample_window(&mut rng, ane.window_len())).collect();
    for _ in 0..warmup {
        ane.score_windows(&windows)?;
        cpu.score_windows(&windows)?;
    }
    let (mut ane_ms, mut cpu_ms) = (Vec::with_capacity(rounds * per_round), Vec::with_capacity(rounds * per_round));
    for round in 0..rounds {
        let order: [(&AneScorer, bool); 2] = if round % 2 == 0 { [(&ane, true), (&cpu, false)] } else { [(&cpu, false), (&ane, true)] };
        for (scorer, is_ane) in order {
            let sink = if is_ane { &mut ane_ms } else { &mut cpu_ms };
            for _ in 0..per_round {
                let t = Instant::now();
                scorer.score_windows(&windows)?;
                sink.push(t.elapsed().as_secs_f64() * 1e3);
            }
        }
    }
    let a = ane.score_windows(&windows)?;
    let c = cpu.score_windows(&windows)?;
    let max_unit_diff = a.iter().zip(&c).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    let ane_stats = stats(ane_ms);
    let cpu_stats = stats(cpu_ms);
    let speedup = cpu_stats["p50"].as_f64().unwrap_or(0.0) / ane_stats["p50"].as_f64().unwrap_or(f64::NAN);
    let off_ane: Vec<Value> = plan
        .dispatched_ops()
        .filter(|op| op.preferred != "neural_engine")
        .map(|op| json!({"operator": op.operator, "preferred": op.preferred, "outputs": op.outputs}))
        .collect();
    Ok(json!({
        "model": model,
        "layout": ane.layout().name(),
        "feature_count": FEATURE_COUNT,
        "window_len": ane.window_len(),
        "batch_size": ane.batch_size(),
        "plan": {
            "compute_units": plan.compute_units,
            "operations_total": plan.operations_total,
            "operations_dispatched": plan.operations_dispatched,
            "neural_engine": plan.preferred.get("neural_engine").copied().unwrap_or(0),
            "cpu": plan.preferred.get("cpu").copied().unwrap_or(0),
            "gpu": plan.preferred.get("gpu").copied().unwrap_or(0),
            "all_on_neural_engine": plan.all_dispatched_on_neural_engine(),
            "not_on_neural_engine": off_ane,
        },
        "latency_ms": {"cpu_and_neural_engine": ane_stats, "cpu_only": cpu_stats},
        "speedup_p50_cpu_only_over_ane": (speedup * 1e4).round() / 1e4,
        "rounds": rounds,
        "per_round": per_round,
        "warmup": warmup,
        "max_abs_diff_between_units_points": max_unit_diff,
    }))
}

fn main() {
    let mut models = Vec::new();
    let (mut rounds, mut per_round, mut warmup) = (40usize, 60usize, 300usize);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut number = |name: &str| -> usize {
            args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                eprintln!("{name} needs a positive integer");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--rounds" => rounds = number("--rounds"),
            "--per-round" => per_round = number("--per-round"),
            "--warmup" => warmup = number("--warmup"),
            _ => models.push(PathBuf::from(arg)),
        }
    }
    if models.is_empty() || rounds == 0 || per_round == 0 {
        eprintln!("usage: ane_probe MODEL.mlmodelc [...] [--rounds N] [--per-round N] [--warmup N]");
        std::process::exit(2);
    }
    let mut failed = false;
    for model in &models {
        match probe(model, rounds, per_round, warmup) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                failed = true;
                println!("{}", json!({"model": model, "error": e.to_string()}));
            }
        }
    }
    std::process::exit(if failed { 1 } else { 0 });
}
