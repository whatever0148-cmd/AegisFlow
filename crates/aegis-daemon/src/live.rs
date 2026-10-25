//! `LiveStats`: rolling statistics over this daemon's own measured samples (API.md "Shared shapes").

use std::collections::{BTreeMap, VecDeque};

use serde_json::{json, Map, Value};

/// Samples kept per key.
pub const WINDOW: usize = 512;

/// Sample keys in API.md order; every value recorded under one of these is a measurement.
pub const SAMPLE_KEYS: [&str; 11] = [
    "branch_acquire_ms",
    "branch_cold_ms",
    "edit_ms",
    "diff_ms",
    "ane_score_ms",
    "cedar_eval_us",
    "commit_swap_ms",
    "rollback_detach_ms",
    "reap_ms",
    "paypal_total_us",
    "paypal_verify_us",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Counters {
    pub runs: u64,
    pub commits: u64,
    pub rollbacks: u64,
    pub approvals_pending: u64,
    pub orders: u64,
    pub orders_blocked: u64,
    pub orders_sent: u64,
    pub pool_hits: u64,
    pub pool_misses: u64,
}

#[derive(Debug)]
pub struct LiveStats {
    samples: BTreeMap<&'static str, VecDeque<f64>>,
    pub counters: Counters,
}

impl Default for LiveStats {
    fn default() -> Self {
        Self { samples: SAMPLE_KEYS.iter().map(|k| (*k, VecDeque::with_capacity(WINDOW))).collect(), counters: Counters::default() }
    }
}

/// Nearest-rank percentile of an ascending slice.
fn rank(sorted: &[f64], p: f64) -> f64 {
    let idx = ((p * sorted.len() as f64).ceil() as usize).max(1) - 1;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn stat(values: &VecDeque<f64>) -> Value {
    if values.is_empty() {
        return json!({"n": 0, "p50": null, "p95": null, "last": null});
    }
    let mut sorted: Vec<f64> = values.iter().copied().collect();
    sorted.sort_by(f64::total_cmp);
    json!({"n": values.len(), "p50": rank(&sorted, 0.50), "p95": rank(&sorted, 0.95), "last": values.back()})
}

impl LiveStats {
    /// Records one measured value. Unknown keys are ignored (they would not be rendered anyway).
    pub fn record(&mut self, key: &str, value: f64) {
        if !value.is_finite() {
            return;
        }
        if let Some(list) = self.samples.get_mut(key) {
            if list.len() == WINDOW {
                list.pop_front();
            }
            list.push_back(value);
        }
    }

    pub fn count(&self, key: &str) -> usize {
        self.samples.get(key).map_or(0, VecDeque::len)
    }

    /// `LiveStats` JSON; `approvals_pending` is supplied by the caller (it lives in the approval store).
    pub fn snapshot(&self, approvals_pending: usize) -> Value {
        let mut samples = Map::new();
        for key in SAMPLE_KEYS {
            samples.insert(key.to_string(), stat(&self.samples[key]));
        }
        let counters = Counters { approvals_pending: approvals_pending as u64, ..self.counters };
        json!({"samples": samples, "counters": counters})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stat_has_null_fields() {
        let live = LiveStats::default();
        let snap = live.snapshot(0);
        assert_eq!(snap["samples"]["edit_ms"], json!({"n": 0, "p50": null, "p95": null, "last": null}));
        assert_eq!(snap["counters"]["runs"], 0);
        assert_eq!(snap["samples"].as_object().unwrap().len(), SAMPLE_KEYS.len());
    }

    #[test]
    fn nearest_rank_percentiles_and_window() {
        let mut live = LiveStats::default();
        for i in 1..=(WINDOW + 10) {
            live.record("diff_ms", i as f64);
        }
        live.record("not_a_key", 1.0);
        live.record("diff_ms", f64::NAN);
        let snap = live.snapshot(3);
        let s = &snap["samples"]["diff_ms"];
        assert_eq!(s["n"], WINDOW);
        assert_eq!(s["last"], (WINDOW + 10) as f64);
        // Window holds 11..=522: p50 is the 256th value, p95 the 487th.
        assert_eq!(s["p50"], 266.0);
        assert_eq!(s["p95"], 497.0);
        assert_eq!(snap["counters"]["approvals_pending"], 3);
    }
}
