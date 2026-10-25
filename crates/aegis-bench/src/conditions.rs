//! Run conditions. P2-R6.5: numbers measured on a shared machine are labelled preliminary, and
//! only a quiet full-scale run decides the phase result. A run is preliminary when
//!
//! * it is reduced-scale;
//! * the 1-minute load average at its start (after `--quiet-wait`) is above the busy threshold
//!   (the performance-core count);
//! * other programs contended for the CPU during it: the time-weighted mean attributable
//!   contention (CPU of the other processes the benchmark can read, i.e. same-user processes,
//!   excluding the benchmark and its descendants, sampled about once per second, see
//!   `external`) is at or above 25% of the logical CPUs;
//! * that contention could not be measured: the CPU sampler failed or recorded no samples, no
//!   process listing succeeded, a listing failed, or an interval has no attribution;
//! * or the measured sources changed while it ran.
//!
//! Informational only: the load average at the end of the run (after minutes of tight APFS /
//! reaper / CoreML loops the 1-minute EWMA mostly reflects the benchmark's own threads) and the
//! total external CPU (system busy cores minus own cores), which includes unattributable root
//! and system services (`kernel_task`, `mds`, `fseventsd`, `aned`) that mostly respond to the
//! benchmark's own filesystem and CoreML activity.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::external::{ExternalCpu, BUSY_FRACTION_OF_LOGICAL_CPUS};
use crate::util::{load_average, round6};

/// Poll interval while waiting for a quiet machine.
const QUIET_POLL: Duration = Duration::from_secs(5);

pub const RULE: &str = "preliminary when reduced-scale; when the 1-min load average at the start (after --quiet-wait) is above the busy threshold (performance-core count); when the time-weighted mean attributable CPU contention over the run is at or above 25% of the logical CPUs (attributable contention = user+system CPU of the other processes the benchmark can read, in practice same-user processes, from proc_listallpids + proc_pid_rusage per (pid, start time) about once per second, including CPU of children they reaped, excluding the benchmark process and its descendants, divided by the sampled seconds); when that contention could not be measured (CPU sampler failed or recorded no samples, no process listing succeeded, a listing failed, or an interval has no attribution); or when the measured sources changed during the run. Informational only: the end-of-run load average (it mostly reflects the benchmark's own threads) and external_cpu_total (system busy cores from host_statistics minus this process's getrusage), which includes unattributable root and system services (kernel_task, mds, fseventsd, aned) that mostly respond to the benchmark's own filesystem and CoreML activity. Only a run that is not preliminary decides the phase result (T18)";

pub struct Conditions {
    pub scale: f64,
    /// 1-minute load average above which the machine counts as busy at the start.
    pub busy_threshold: Option<f64>,
    pub load_start: Option<[f64; 3]>,
    /// Informational only.
    pub load_end: Option<[f64; 3]>,
    pub quiet_wait: Value,
    pub sources_start: Value,
    pub sources_end: Value,
    pub external: ExternalCpu,
}

/// Whether attributable contention made the run busy: Some(busy) when it was measured, None when
/// data is missing.
pub fn contention_busy(e: &ExternalCpu) -> Option<bool> {
    if e.error.is_some() || e.attribution_problem().is_some() {
        return None;
    }
    Some(e.user_contention_mean()? >= e.busy_threshold_cores()?)
}

/// Why CPU contention makes the run preliminary: missing data, or a mean attributable contention
/// at or above `BUSY_FRACTION_OF_LOGICAL_CPUS` of the logical CPUs. None for a quiet run. The
/// total external CPU (system minus own) never decides.
pub fn contention_reason(e: &ExternalCpu) -> Option<String> {
    if let Some(error) = &e.error {
        return Some(format!("CPU contention not measured: the CPU sampler has no complete data: {error}"));
    }
    let Some(threshold) = e.busy_threshold_cores() else {
        return Some("CPU contention not measured: logical CPU count unavailable".to_string());
    };
    if e.samples() == 0 {
        return Some("CPU contention not measured: the CPU sampler recorded no samples".to_string());
    }
    if let Some(problem) = e.attribution_problem() {
        return Some(format!("CPU contention not measured: attributable contention unavailable ({problem})"));
    }
    let Some(mean) = e.user_contention_mean() else {
        return Some("CPU contention not measured: attributable contention unavailable".to_string());
    };
    (mean >= threshold).then(|| {
        format!(
            "mean attributable CPU contention {mean:.2} cores over {} samples at or above the busy threshold {threshold:.2} cores ({:.0}% of {} logical CPUs)",
            e.samples(),
            BUSY_FRACTION_OF_LOGICAL_CPUS * 100.0,
            e.logical_cpus.unwrap_or_default()
        )
    })
}

/// The `run_conditions` object of the artifact.
pub fn evaluate(c: &Conditions) -> Value {
    let mut reasons = Vec::new();
    if c.scale < 1.0 {
        reasons.push(format!("reduced scale {}: iteration counts scaled down (at least 5 samples each)", c.scale));
    }
    match (c.busy_threshold, c.load_start) {
        (None, _) => reasons.push("busy threshold unknown (core count unavailable)".to_string()),
        (Some(_), None) => reasons.push("load average unavailable at start".to_string()),
        (Some(threshold), Some(l)) if l[0] > threshold => {
            reasons.push(format!("1-min load average {:.2} at start above the busy threshold {threshold}", l[0]))
        }
        (Some(_), Some(_)) => {}
    }
    reasons.extend(contention_reason(&c.external));
    let start = c.sources_start["sha256"].as_str();
    if start.is_none() || start != c.sources_end["sha256"].as_str() {
        reasons.push("measured sources changed during the run (or could not be fingerprinted)".to_string());
    }
    let mut user_contention = c.external.contention_json();
    user_contention["busy"] = json!(contention_busy(&c.external));
    json!({
        "preliminary": !reasons.is_empty(),
        "reasons": reasons,
        "scale": c.scale,
        "full_scale": c.scale >= 1.0,
        "busy_threshold_load1": c.busy_threshold,
        "busy_threshold_basis": "performance-core count (hw.perflevel0.physicalcpu, else hw.physicalcpu); applied to the 1-min load average at the start only",
        "load_average_start": c.load_start,
        "load_average_end": c.load_end,
        "load_average_end_note": "informational only, not part of the rule: the 1-min load average is an EWMA of runnable threads, and after minutes of tight APFS / reaper / CoreML loops it mostly reflects the benchmark's own threads; user_contention measures other programs",
        "user_contention": user_contention,
        "external_cpu_total": c.external.total_json(),
        "quiet_wait": c.quiet_wait,
        "sources_unchanged_during_run": start.is_some() && start == c.sources_end["sha256"].as_str(),
        "rule": RULE,
    })
}

/// Sentence appended to `targets_note`.
pub fn note_sentence(run: &Value) -> String {
    let reasons: Vec<&str> = run["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let total = &run["external_cpu_total"];
    let num = |v: &Value| v.as_f64().map_or_else(|| v.to_string(), |x| format!("{x:.2}"));
    let informational = format!(
        "the end-of-run load average and the total external CPU (system minus own: mean {} cores, {} core-seconds unattributed, mostly root and system services responding to the benchmark's own filesystem and CoreML activity) are informational",
        num(&total["mean_cores"]),
        num(&total["unattributed_core_seconds"])
    );
    if run["preliminary"] == true {
        format!(" This run is PRELIMINARY ({}); only a quiet full-scale run decides the phase result; {informational}.", reasons.join("; "))
    } else {
        let user = &run["user_contention"];
        format!(
            " Run conditions: full scale, 1-min load average at start below the busy threshold, mean attributable CPU contention from other programs {} cores (p95 {}, max {}, {} samples) below the busy threshold of {} cores, sources unchanged during the run; {informational}.",
            num(&user["mean_cores"]),
            num(&user["p95_cores"]),
            num(&user["max_cores"]),
            user["samples"],
            num(&user["busy_threshold_cores"])
        )
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
    use crate::external::Interval;
    use crate::util::parse_loadavg;

    const LOGICAL: u32 = 8;

    fn threshold() -> f64 {
        f64::from(LOGICAL) * BUSY_FRACTION_OF_LOGICAL_CPUS
    }

    /// Attributable contention at `user` and total external CPU at `total` times the busy
    /// threshold, over `samples` one-second intervals.
    fn contention(user: f64, total: f64, samples: usize) -> ExternalCpu {
        let own = 1.0;
        let iv = Interval {
            seconds: 1.0,
            system_cores: threshold() * total + own,
            own_cores: own,
            external_cores: threshold() * total,
            user_contention_cores: Some(threshold() * user),
        };
        ExternalCpu {
            logical_cpus: Some(LOGICAL),
            intervals: vec![iv; samples],
            error: None,
            listings: samples + 1,
            listing_errors: Vec::new(),
            bench_child_core_seconds: 0.0,
            processes: Value::Null,
        }
    }

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
            external: contention(0.25, 0.5, 10),
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
        assert_eq!(run["user_contention"]["busy"], false, "{run}");
        assert_eq!(run["user_contention"]["samples"], 10);
        assert!(run["user_contention"]["p95_cores"].is_number() && run["user_contention"]["max_cores"].is_number());
        assert!(run["external_cpu_total"]["mean_cores"].is_number());
        assert!(run["external_cpu_total"]["unattributed_core_seconds"].is_number());
        let note = note_sentence(&run);
        for part in ["full scale", "attributable CPU contention", "informational", "unattributed"] {
            assert!(note.contains(part), "{note} lacks {part}");
        }
    }

    #[test]
    fn end_load_average_is_informational() {
        // The benchmark's own threads drive the 1-min EWMA far above the threshold by the end.
        let mut own_load = quiet(4.0);
        own_load.load_end = Some([4.0 * 3.0, 6.0, 2.0]);
        let run = evaluate(&own_load);
        assert_eq!(run["preliminary"], false, "{run}");
        let mut unknown_end = quiet(4.0);
        unknown_end.load_end = None;
        assert_eq!(evaluate(&unknown_end)["preliminary"], false);
    }

    #[test]
    fn attributable_contention_at_or_above_a_quarter_of_logical_cpus_is_busy() {
        assert_eq!(contention_reason(&contention(0.99, 0.99, 10)), None);
        assert_eq!(contention_busy(&contention(0.99, 0.99, 10)), Some(false));
        for fraction in [1.0, 1.5] {
            let mut busy = quiet(4.0);
            busy.external = contention(fraction, fraction, 10);
            let run = evaluate(&busy);
            assert_eq!(run["preliminary"], true, "{fraction}: {run}");
            assert_eq!(run["user_contention"]["busy"], true);
            assert!(run["reasons"][0].as_str().unwrap().contains("attributable CPU contention"), "{run}");
        }
        // The mean decides, not a single spike.
        let mut spiky = contention(0.2, 0.2, 20);
        spiky.intervals[0].user_contention_cores = Some(f64::from(LOGICAL));
        assert_eq!(contention_reason(&spiky), None, "mean {:?}", spiky.user_contention_mean());
    }

    #[test]
    fn total_external_above_threshold_with_attributable_below_is_not_busy() {
        // System services reacting to the benchmark push system-minus-own over the threshold;
        // other programs used little.
        let mut c = quiet(4.0);
        c.external = contention(0.1, 1.7, 30);
        assert!(c.external.mean_cores().unwrap() >= c.external.busy_threshold_cores().unwrap());
        let run = evaluate(&c);
        assert_eq!(run["preliminary"], false, "{run}");
        assert_eq!(run["user_contention"]["busy"], false);
        assert!(run["external_cpu_total"]["mean_cores"].as_f64().unwrap() >= threshold());
        assert!(run["external_cpu_total"]["note"].as_str().unwrap().contains("informational"));
    }

    #[test]
    fn missing_data_is_preliminary_with_a_reason() {
        let no_samples = contention(0.0, 0.0, 0);
        let mut no_cpus = contention(0.0, 0.0, 10);
        no_cpus.logical_cpus = None;
        let failed = ExternalCpu::failed(Some(LOGICAL), "host_statistics unsupported");
        let mut stopped_early = contention(0.0, 0.0, 10);
        stopped_early.error = Some("sample 11 failed".to_string());
        let mut zero_listings = contention(0.0, 0.0, 10);
        zero_listings.listings = 0;
        let mut listing_failed = contention(0.0, 0.0, 10);
        listing_failed.listing_errors = vec!["proc_listallpids: Operation not permitted".to_string()];
        listing_failed.intervals[3].user_contention_cores = None;
        let mut unattributed_interval = contention(0.0, 0.0, 10);
        unattributed_interval.intervals[0].user_contention_cores = None;
        for (name, external) in [
            ("no samples", no_samples),
            ("no cpu count", no_cpus),
            ("failed", failed),
            ("stopped early", stopped_early),
            ("zero listings", zero_listings),
            ("listing failed", listing_failed),
            ("attribution unavailable", unattributed_interval),
        ] {
            let reason = contention_reason(&external);
            assert!(reason.as_deref().is_some_and(|r| r.contains("not measured")), "{name}: {reason:?}");
            assert_eq!(contention_busy(&external), None, "{name}");
            let mut c = quiet(4.0);
            c.external = external;
            let run = evaluate(&c);
            assert_eq!(run["preliminary"], true, "{name}: {run}");
            assert!(run["user_contention"]["busy"].is_null(), "{name}");
            assert!(note_sentence(&run).contains("PRELIMINARY"), "{name}");
        }
    }

    #[test]
    fn reduced_scale_busy_start_and_changed_sources_are_preliminary() {
        let mut reduced = quiet(4.0);
        reduced.scale = 0.1;
        let mut busy_start = quiet(4.0);
        busy_start.load_start = Some([8.0, 1.0, 1.0]);
        let mut unknown_load = quiet(4.0);
        unknown_load.load_start = None;
        let mut changed = quiet(4.0);
        changed.sources_end = json!({"sha256": "other"});
        let mut no_threshold = quiet(4.0);
        no_threshold.busy_threshold = None;
        for (name, c) in [
            ("reduced", reduced),
            ("busy start", busy_start),
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
