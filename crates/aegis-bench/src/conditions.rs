//! Run conditions. P2-R6.5: numbers measured on a shared machine are labelled preliminary, and
//! only a quiet full-scale run decides the phase result. A run is preliminary when it is
//! reduced-scale, when the 1-minute load average at its start or end is above the busy threshold
//! (the performance-core count), or when the measured sources changed while it ran.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::util::{load_average, round6};

/// Poll interval while waiting for a quiet machine.
const QUIET_POLL: Duration = Duration::from_secs(5);

pub struct Conditions {
    pub scale: f64,
    /// 1-minute load average above which the machine counts as busy.
    pub busy_threshold: Option<f64>,
    pub load_start: Option<[f64; 3]>,
    pub load_end: Option<[f64; 3]>,
    pub quiet_wait: Value,
    pub sources_start: Value,
    pub sources_end: Value,
}

/// The `run_conditions` object of the artifact.
pub fn evaluate(c: &Conditions) -> Value {
    let mut reasons = Vec::new();
    if c.scale < 1.0 {
        reasons.push(format!("reduced scale {}: iteration counts scaled down (at least 5 samples each)", c.scale));
    }
    match c.busy_threshold {
        None => reasons.push("busy threshold unknown (core count unavailable)".to_string()),
        Some(threshold) => {
            for (when, load) in [("start", c.load_start), ("end", c.load_end)] {
                match load {
                    Some(l) if l[0] > threshold => {
                        reasons.push(format!("1-min load average {:.2} at {when} above the busy threshold {threshold}", l[0]))
                    }
                    Some(_) => {}
                    None => reasons.push(format!("load average unavailable at {when}")),
                }
            }
        }
    }
    let start = c.sources_start["sha256"].as_str();
    if start.is_none() || start != c.sources_end["sha256"].as_str() {
        reasons.push("measured sources changed during the run (or could not be fingerprinted)".to_string());
    }
    json!({
        "preliminary": !reasons.is_empty(),
        "reasons": reasons,
        "scale": c.scale,
        "full_scale": c.scale >= 1.0,
        "busy_threshold_load1": c.busy_threshold,
        "busy_threshold_basis": "performance-core count (hw.perflevel0.physicalcpu, else hw.physicalcpu)",
        "load_average_start": c.load_start,
        "load_average_end": c.load_end,
        "quiet_wait": c.quiet_wait,
        "sources_unchanged_during_run": start.is_some() && start == c.sources_end["sha256"].as_str(),
        "rule": "preliminary when reduced-scale, when the 1-min load average at start or end is above the busy threshold, or when the measured sources changed during the run; only a run that is not preliminary decides the phase result (T18)",
    })
}

/// Sentence appended to `targets_note`.
pub fn note_sentence(run: &Value) -> String {
    let reasons: Vec<&str> = run["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if run["preliminary"] == true {
        format!(" This run is PRELIMINARY ({}); only a quiet full-scale run decides the phase result.", reasons.join("; "))
    } else {
        " Run conditions: full scale, machine below the busy threshold at start and end, sources unchanged during the run.".to_string()
    }
}

/// Waits up to `max_wait` for the 1-minute load average to drop to `threshold` (no wait when
/// `max_wait` is zero or the threshold is unknown). Returns what happened, for the artifact.
pub fn wait_for_quiet(threshold: Option<f64>, max_wait: Duration) -> Value {
    let started = Instant::now();
    let mut last = load_average();
    if let Some(t) = threshold.filter(|_| !max_wait.is_zero()) {
        while last.is_some_and(|l| l[0] > t) && started.elapsed() < max_wait {
            if let Some(l) = last {
                eprintln!(
                    "aegis-bench: 1-min load average {:.2} above {t}; waiting for a quiet machine ({:.0} of {:.0} s)",
                    l[0],
                    started.elapsed().as_secs_f64(),
                    max_wait.as_secs_f64()
                );
            }
            std::thread::sleep(QUIET_POLL);
            last = load_average();
        }
    }
    json!({
        "max_wait_s": max_wait.as_secs_f64(),
        "waited_s": round6(started.elapsed().as_secs_f64()),
        "load_average_after_wait": last,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::parse_loadavg;

    fn quiet(threshold: f64) -> Conditions {
        let below = [threshold / 2.0; 3];
        let sources = json!({"sha256": "same"});
        Conditions {
            scale: 1.0,
            busy_threshold: Some(threshold),
            load_start: Some(below),
            load_end: Some(below),
            quiet_wait: Value::Null,
            sources_start: sources.clone(),
            sources_end: sources,
        }
    }

    #[test]
    fn parses_sysctl_loadavg() {
        let text = format!("{{ {} {} {} }}", 1.5, 2.25, 3.0);
        assert_eq!(parse_loadavg(&text), Some([1.5, 2.25, 3.0]));
        assert_eq!(parse_loadavg("{ }"), None);
    }

    #[test]
    fn quiet_full_scale_run_is_not_preliminary() {
        let run = evaluate(&quiet(4.0));
        assert_eq!(run["preliminary"], false, "{run}");
        assert!(note_sentence(&run).contains("full scale"));
    }

    #[test]
    fn reduced_scale_busy_machine_and_changed_sources_are_preliminary() {
        let mut reduced = quiet(4.0);
        reduced.scale = 0.1;
        let mut busy_start = quiet(4.0);
        busy_start.load_start = Some([8.0, 1.0, 1.0]);
        let mut busy_end = quiet(4.0);
        busy_end.load_end = Some([8.0, 1.0, 1.0]);
        let mut unknown_load = quiet(4.0);
        unknown_load.load_start = None;
        let mut changed = quiet(4.0);
        changed.sources_end = json!({"sha256": "other"});
        let mut no_threshold = quiet(4.0);
        no_threshold.busy_threshold = None;
        for (name, c) in [
            ("reduced", reduced),
            ("busy start", busy_start),
            ("busy end", busy_end),
            ("unknown load", unknown_load),
            ("changed sources", changed),
            ("no threshold", no_threshold),
        ] {
            let run = evaluate(&c);
            assert_eq!(run["preliminary"], true, "{name}: {run}");
            assert!(!run["reasons"].as_array().unwrap().is_empty(), "{name}");
            assert!(note_sentence(&run).contains("PRELIMINARY"), "{name}");
        }
    }

    #[test]
    fn no_wait_without_a_budget() {
        let waited = wait_for_quiet(Some(0.0), Duration::ZERO);
        assert_eq!(waited["max_wait_s"], 0.0);
    }
}
