//! CPU contention during a benchmark run (P2-R6.5).
//!
//! The 1-minute load average is an EWMA over runnable threads, so at the end of a run it mostly
//! reflects the benchmark's own APFS, reaper and CoreML threads. A background thread samples
//! about once per second and records two measures per interval:
//!
//! * **Attributable contention** (the rule's input): CPU time of the other processes this
//!   benchmark can read (`proc_listallpids` + `proc_pid_rusage`; in practice the processes of the
//!   same user, which is where real interference shows up: compiles, training, browsers, IDEs),
//!   excluding this process and its descendants (`bench_child`), divided by the interval. Each
//!   process is keyed by (pid, start time) and counted from its start when it started during the
//!   run. A process that starts and exits between two listings is counted through its parent's
//!   reaped-children CPU (`ri_child_user_time` + `ri_child_system_time`) when the parent is
//!   readable and reaps it during the run; for a tracked child that exits, its parent's
//!   children-CPU rise is matched against the child's last cumulative CPU first, so nothing is
//!   counted twice. Not attributed: processes of other users (root daemons, unreadable), CPU a
//!   process spends after the last listing that saw it unless a readable parent reaps it, and
//!   short processes whose parent is unreadable (launchd) or reaps them after the run.
//! * **Total external CPU** (informational): system-wide busy cores from
//!   `host_statistics(HOST_CPU_LOAD_INFO)` tick deltas (user + system + nice over all ticks,
//!   times the logical CPU count) minus this process's own CPU (`getrusage(RUSAGE_SELF)`),
//!   clamped at 0. It includes unattributable root and system services (`kernel_task`, `mds`,
//!   `fseventsd`, `aned`, `ANECompilerService`) that mostly respond to the benchmark's own
//!   filesystem and CoreML activity, so it measures OS work induced by the benchmark more than
//!   interference. The unattributed remainder (total minus the CPU of all readable processes,
//!   benchmark descendants included) is recorded.

use std::collections::{HashMap, HashSet};
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::util::{command_line, percentile, round6};

/// Time between two samples.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// A run is busy when the mean attributable contention reaches this fraction of the logical CPUs.
pub const BUSY_FRACTION_OF_LOGICAL_CPUS: f64 = 0.25;
/// Intervals shorter than this are merged into the next one (tick resolution).
const MIN_SAMPLE_SECONDS: f64 = 0.1;
/// Number of processes listed by CPU time.
const TOP_PROCESSES: usize = 8;
/// Extra pid slots on top of the count the kernel reported, for processes started in between.
const PID_SLACK: usize = 64;

/// `struct mach_timebase_info` from `<mach/mach_time.h>`.
#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

// Mach symbols from libSystem, declared here because the `libc` crate deprecates its copies in
// favour of the `mach2` crate (and has no `mach_port_deallocate`).
extern "C" {
    /// The task's own port name; `mach_task_self()` in C is a macro reading it.
    static mach_task_self_: libc::mach_port_t;
    fn mach_host_self() -> libc::mach_port_t;
    fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> libc::kern_return_t;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> libc::kern_return_t;
    fn mach_absolute_time() -> u64;
}

/// One sampling interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub seconds: f64,
    pub system_cores: f64,
    pub own_cores: f64,
    /// max(0, system - own): informational.
    pub external_cores: f64,
    /// CPU of readable non-benchmark processes over the interval / interval; None when a process
    /// listing that falls into the interval failed.
    pub user_contention_cores: Option<f64>,
}

/// CPU contention over a run: the input of the run-conditions rule.
#[derive(Clone, Debug)]
pub struct ExternalCpu {
    pub logical_cpus: Option<u32>,
    pub intervals: Vec<Interval>,
    /// Why sampling did not start or stopped early; any error makes the run preliminary.
    pub error: Option<String>,
    /// Successful process listings (the baseline plus one per sample).
    pub listings: usize,
    /// Distinct errors of failed process listings; any makes the run preliminary.
    pub listing_errors: Vec<String>,
    /// CPU of the benchmark's descendants (`cp -Rc`, ...): outside `RUSAGE_SELF`, so part of the
    /// total external figure, but never contention.
    pub bench_child_core_seconds: f64,
    /// Per-process attribution.
    pub processes: Value,
}

impl ExternalCpu {
    pub fn failed(logical_cpus: Option<u32>, error: impl Into<String>) -> Self {
        Self {
            logical_cpus,
            intervals: Vec::new(),
            error: Some(error.into()),
            listings: 0,
            listing_errors: Vec::new(),
            bench_child_core_seconds: 0.0,
            processes: Value::Null,
        }
    }

    pub fn samples(&self) -> usize {
        self.intervals.len()
    }

    pub fn sampled_seconds(&self) -> f64 {
        self.intervals.iter().map(|i| i.seconds).sum()
    }

    /// Time-weighted mean of `pick` over the sampled intervals.
    fn weighted_mean(&self, pick: impl Fn(&Interval) -> f64) -> Option<f64> {
        let seconds = self.sampled_seconds();
        (seconds > 0.0).then(|| self.intervals.iter().map(|i| pick(i) * i.seconds).sum::<f64>() / seconds)
    }

    pub fn busy_threshold_cores(&self) -> Option<f64> {
        self.logical_cpus.map(|n| f64::from(n) * BUSY_FRACTION_OF_LOGICAL_CPUS)
    }

    // ----- total external CPU (system minus own), informational -----

    /// Mean external cores over the run (core-seconds of external CPU / sampled seconds).
    pub fn mean_cores(&self) -> Option<f64> {
        self.weighted_mean(|i| i.external_cores)
    }

    pub fn p95_cores(&self) -> Option<f64> {
        percentile(&self.intervals.iter().map(|i| i.external_cores).collect::<Vec<_>>(), 95.0)
    }

    pub fn max_cores(&self) -> Option<f64> {
        self.intervals.iter().map(|i| i.external_cores).max_by(f64::total_cmp)
    }

    pub fn external_core_seconds(&self) -> f64 {
        self.intervals.iter().map(|i| i.external_cores * i.seconds).sum()
    }

    // ----- attributable contention, the rule's input -----

    /// Why attributable contention is unavailable: no listing succeeded, a listing failed, or an
    /// interval has no attribution. None when every interval is attributed.
    pub fn attribution_problem(&self) -> Option<String> {
        if self.listings == 0 {
            return Some("no process listing succeeded".to_string());
        }
        if !self.listing_errors.is_empty() {
            return Some(format!("process listing failed: {}", self.listing_errors.join("; ")));
        }
        let missing = self.intervals.iter().filter(|i| i.user_contention_cores.is_none()).count();
        (missing > 0).then(|| format!("{missing} of {} intervals have no process attribution", self.samples()))
    }

    /// Per-interval attributable contention, only when every interval is attributed.
    fn user_series(&self) -> Option<Vec<f64>> {
        self.intervals.iter().map(|i| i.user_contention_cores).collect()
    }

    /// Mean attributable contention: attributable core-seconds / sampled seconds.
    pub fn user_contention_mean(&self) -> Option<f64> {
        self.user_series()?;
        self.weighted_mean(|i| i.user_contention_cores.unwrap_or_default())
    }

    pub fn user_contention_p95(&self) -> Option<f64> {
        percentile(&self.user_series()?, 95.0)
    }

    pub fn user_contention_max(&self) -> Option<f64> {
        self.user_series()?.into_iter().max_by(f64::total_cmp)
    }

    pub fn user_contention_core_seconds(&self) -> Option<f64> {
        self.user_series()?;
        Some(self.intervals.iter().map(|i| i.user_contention_cores.unwrap_or_default() * i.seconds).sum())
    }

    /// `run_conditions.user_contention` (the rule's input; `busy` is added by the rule).
    pub fn contention_json(&self) -> Value {
        let r = |v: Option<f64>| v.map(round6);
        json!({
            "method": "background sampler, one process listing per interval: proc_listallpids + proc_pid_rusage(RUSAGE_INFO_V1) per (pid, start time); per interval, the user+system CPU delta of every readable process (in practice the processes of the same user) plus the rise of its reaped-children CPU (ri_child_user_time + ri_child_system_time; a tracked child's last cumulative CPU is matched first, so nothing is counted twice), excluding this benchmark process and its descendants (bench_child), divided by the interval; a process that started during the run counts from its start; mean is time-weighted (attributable core-seconds / sampled seconds), p95 and max are over the per-interval values; a reaped child's CPU is credited in the interval its parent reaps it",
            "not_attributed": "processes of other users (root and system daemons such as kernel_task, mds, fseventsd, aned, ANECompilerService are unreadable; see external_cpu_total); CPU a process spends after the last listing that saw it unless a readable parent reaps it; short processes whose parent is unreadable (launchd) or reaps them after the run",
            "interval_s": SAMPLE_INTERVAL.as_secs_f64(),
            "logical_cpus": self.logical_cpus,
            "busy_fraction_of_logical_cpus": BUSY_FRACTION_OF_LOGICAL_CPUS,
            "busy_threshold_cores": self.busy_threshold_cores(),
            "samples": self.samples(),
            "attributed_samples": self.intervals.iter().filter(|i| i.user_contention_cores.is_some()).count(),
            "sampled_seconds": round6(self.sampled_seconds()),
            "mean_cores": r(self.user_contention_mean()),
            "p95_cores": r(self.user_contention_p95()),
            "max_cores": r(self.user_contention_max()),
            "core_seconds": r(self.user_contention_core_seconds()),
            "listings": self.listings,
            "listing_errors": self.listing_errors,
            "attribution_error": self.attribution_problem(),
            "sampler_error": self.error,
            "processes": self.processes,
        })
    }

    /// `run_conditions.external_cpu_total` (informational, never part of the rule).
    pub fn total_json(&self) -> Value {
        let r = |v: Option<f64>| v.map(round6);
        let external = self.external_core_seconds();
        let user = self.user_contention_core_seconds();
        let attributed = user.map(|u| u + self.bench_child_core_seconds);
        json!({
            "note": "informational, not part of the rule: system busy cores minus this process's own cores includes unattributable root and system services (kernel_task, mds, fseventsd, aned, ANECompilerService), which mostly respond to the benchmark's own filesystem (clonefile trees, deletions, FSEvents) and CoreML activity; user_contention measures interference from other programs",
            "method": "system busy cores = (user+system+nice tick delta / all-tick delta) x logical CPUs from host_statistics(HOST_CPU_LOAD_INFO); own cores = getrusage(RUSAGE_SELF) user+system CPU delta / interval; external cores = max(0, system - own); mean is time-weighted over the sampled intervals; unattributed core-seconds = external core-seconds - attributed core-seconds (readable processes including the benchmark's descendants; may be slightly negative from tick vs rusage accounting)",
            "interval_s": SAMPLE_INTERVAL.as_secs_f64(),
            "logical_cpus": self.logical_cpus,
            "samples": self.samples(),
            "sampled_seconds": round6(self.sampled_seconds()),
            "mean_cores": r(self.mean_cores()),
            "p95_cores": r(self.p95_cores()),
            "max_cores": r(self.max_cores()),
            "mean_system_busy_cores": r(self.weighted_mean(|i| i.system_cores)),
            "mean_own_cores": r(self.weighted_mean(|i| i.own_cores)),
            "external_core_seconds": round6(external),
            "attributed_core_seconds": r(attributed),
            "user_contention_core_seconds": r(user),
            "bench_child_core_seconds": round6(self.bench_child_core_seconds),
            "unattributed_core_seconds": r(attributed.map(|a| external - a)),
            "error": self.error,
        })
    }
}

/// One interval from two snapshots. `ticks` are cumulative `cpu_ticks` indexed by `CPU_STATE_*`
/// (32-bit counters that may wrap); `own_cpu_seconds` is this process's CPU time over the
/// interval. None when the interval is too short or no tick advanced. Attribution is filled in
/// by the caller.
pub fn interval_between(seconds: f64, ticks_before: [u32; 4], ticks_after: [u32; 4], own_cpu_seconds: f64, logical_cpus: f64) -> Option<Interval> {
    let delta = |state: libc::c_int| f64::from(ticks_after[state as usize].wrapping_sub(ticks_before[state as usize]));
    let busy = delta(libc::CPU_STATE_USER) + delta(libc::CPU_STATE_SYSTEM) + delta(libc::CPU_STATE_NICE);
    let total = busy + delta(libc::CPU_STATE_IDLE);
    if seconds < MIN_SAMPLE_SECONDS || total <= 0.0 {
        return None;
    }
    let system_cores = busy / total * logical_cpus;
    let own_cores = own_cpu_seconds.max(0.0) / seconds;
    Some(Interval { seconds, system_cores, own_cores, external_cores: (system_cores - own_cores).max(0.0), user_contention_cores: None })
}

/// Logical CPU count (`hw.logicalcpu`).
pub fn logical_cpus() -> Option<u32> {
    command_line("sysctl", &["-n", "hw.logicalcpu"]).and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0)
}

/// Background sampler; `finish` stops it and returns what it measured.
pub struct Sampler {
    stop: Sender<()>,
    handle: Result<JoinHandle<ExternalCpu>, String>,
    logical_cpus: Option<u32>,
}

impl Sampler {
    pub fn start(interval: Duration) -> Self {
        let logical_cpus = logical_cpus();
        let (stop, stopped) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("aegis-bench-cpu-sampler".to_string())
            .spawn(move || record(interval, logical_cpus, stopped))
            .map_err(|e| format!("sampler thread not started: {e}"));
        Self { stop, handle, logical_cpus }
    }

    pub fn finish(self) -> ExternalCpu {
        // A send error means the thread already ended (setup or sampling error); join reports it.
        let _ = self.stop.send(());
        match self.handle {
            Ok(handle) => handle.join().unwrap_or_else(|_| ExternalCpu::failed(self.logical_cpus, "sampler thread panicked")),
            Err(e) => ExternalCpu::failed(self.logical_cpus, e),
        }
    }
}

struct Snapshot {
    at: Instant,
    ticks: [u32; 4],
    own_cpu_seconds: f64,
}

impl Snapshot {
    fn take(host: &HostPort) -> io::Result<Self> {
        let ticks = host.cpu_ticks()?;
        let own_cpu_seconds = own_cpu_seconds()?;
        Ok(Self { at: Instant::now(), ticks, own_cpu_seconds })
    }
}

/// Attributable CPU seconds accumulated since the last recorded interval; None once a listing
/// in that span failed.
fn accumulate(pending: Option<f64>, listed: Result<f64, String>) -> Option<f64> {
    match (pending, listed) {
        (Some(p), Ok(s)) => Some(p + s),
        _ => None,
    }
}

fn record(interval: Duration, logical_cpus: Option<u32>, stop: Receiver<()>) -> ExternalCpu {
    let Some(logical) = logical_cpus.map(f64::from) else {
        return ExternalCpu::failed(None, "logical CPU count unavailable (sysctl hw.logicalcpu)");
    };
    let setup = HostPort::open().and_then(|host| Ok((host, Timebase::query()?)));
    let (host, timebase) = match setup {
        Ok(v) => v,
        Err(e) => return ExternalCpu::failed(logical_cpus, format!("sampler setup: {e}")),
    };
    let mut processes = ProcessTable::new(timebase);
    let mut prev = match Snapshot::take(&host) {
        Ok(s) => s,
        Err(e) => return ExternalCpu::failed(logical_cpus, format!("first sample: {e}")),
    };
    // Baseline listing; CPU of processes that started in between folds into the first interval.
    let mut pending = accumulate(Some(0.0), processes.observe());
    let mut out = ExternalCpu {
        logical_cpus,
        intervals: Vec::new(),
        error: None,
        listings: 0,
        listing_errors: Vec::new(),
        bench_child_core_seconds: 0.0,
        processes: Value::Null,
    };
    loop {
        let last = !matches!(stop.recv_timeout(interval), Err(RecvTimeoutError::Timeout));
        let snapshot = Snapshot::take(&host);
        // The listing follows the tick snapshot directly, so both describe the same interval.
        pending = accumulate(pending, processes.observe());
        match snapshot {
            Ok(now) => {
                let seconds = now.at.duration_since(prev.at).as_secs_f64();
                if let Some(mut i) = interval_between(seconds, prev.ticks, now.ticks, now.own_cpu_seconds - prev.own_cpu_seconds, logical) {
                    i.user_contention_cores = pending.map(|cpu| cpu / i.seconds);
                    out.intervals.push(i);
                    pending = Some(0.0);
                    prev = now;
                }
            }
            Err(e) => {
                out.error = Some(format!("sample {} failed: {e}", out.intervals.len() + 1));
                break;
            }
        }
        if last {
            break;
        }
    }
    out.listings = processes.listings;
    out.listing_errors = processes.list_errors.clone();
    out.bench_child_core_seconds = processes.bench_child_seconds();
    out.processes = processes.report(out.sampled_seconds());
    out
}

/// A send right to the host port, released on drop.
struct HostPort(libc::mach_port_t);

impl HostPort {
    fn open() -> io::Result<Self> {
        // SAFETY: mach_host_self takes no arguments and returns a send right to the host port
        // (MACH_PORT_NULL on failure) whose user reference the caller owns; `Drop` releases it.
        let port = unsafe { mach_host_self() };
        if port == 0 {
            return Err(io::Error::other("mach_host_self returned MACH_PORT_NULL"));
        }
        Ok(Self(port))
    }

    /// Cumulative CPU ticks over all CPUs, indexed by `CPU_STATE_*`. Mach calls return a
    /// `kern_return_t` and do not set errno, so the code itself is the error.
    fn cpu_ticks(&self) -> io::Result<[u32; 4]> {
        let mut info = libc::host_cpu_load_info { cpu_ticks: [0; libc::CPU_STATE_MAX as usize] };
        let mut count: libc::mach_msg_type_number_t = libc::HOST_CPU_LOAD_INFO_COUNT;
        // SAFETY: `info` is a host_cpu_load_info, exactly HOST_CPU_LOAD_INFO_COUNT integer_t
        // words, and `count` passes that capacity in and receives the number of words written;
        // both outlive the call. `self.0` is a live host send right owned by this value.
        let kr = unsafe {
            libc::host_statistics(self.0, libc::HOST_CPU_LOAD_INFO, (&mut info as *mut libc::host_cpu_load_info).cast(), &mut count)
        };
        if kr != libc::KERN_SUCCESS {
            return Err(io::Error::other(format!("host_statistics(HOST_CPU_LOAD_INFO) returned kern_return_t {kr}")));
        }
        if count != libc::HOST_CPU_LOAD_INFO_COUNT {
            return Err(io::Error::other(format!("host_statistics(HOST_CPU_LOAD_INFO) wrote {count} words, expected {}", libc::HOST_CPU_LOAD_INFO_COUNT)));
        }
        Ok(info.cpu_ticks)
    }
}

impl Drop for HostPort {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the send right returned by mach_host_self and is released exactly
        // once, here. `mach_task_self_` is set by libSystem before main and never written after.
        let kr = unsafe { mach_port_deallocate(mach_task_self_, self.0) };
        if kr != libc::KERN_SUCCESS {
            eprintln!("aegis-bench: mach_port_deallocate(host port) returned kern_return_t {kr}");
        }
    }
}

/// Mach absolute time -> seconds (`proc_pid_rusage` reports CPU and start times in these units).
#[derive(Clone, Copy)]
struct Timebase {
    numer: u32,
    denom: u32,
}

impl Timebase {
    fn query() -> io::Result<Self> {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid, writable mach_timebase_info (same layout, repr(C)) for the
        // duration of the call.
        let kr = unsafe { mach_timebase_info(&mut info) };
        if kr != libc::KERN_SUCCESS || info.denom == 0 {
            return Err(io::Error::other(format!("mach_timebase_info returned kern_return_t {kr} (denominator {})", info.denom)));
        }
        Ok(Self { numer: info.numer, denom: info.denom })
    }

    fn seconds(self, abs: u64) -> f64 {
        abs as f64 * f64::from(self.numer) / f64::from(self.denom) / 1e9
    }
}

fn timeval_seconds(tv: libc::timeval) -> f64 {
    tv.tv_sec as f64 + f64::from(tv.tv_usec) / 1e6
}

/// User + system CPU time of this process (all threads), in seconds.
pub fn own_cpu_seconds() -> io::Result<f64> {
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable storage for one rusage, which getrusage fills on success.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: getrusage returned 0, so the struct is initialised (it was also zeroed before).
    let usage = unsafe { usage.assume_init() };
    Ok(timeval_seconds(usage.ru_utime) + timeval_seconds(usage.ru_stime))
}

/// Every pid on the system.
fn all_pids() -> io::Result<Vec<libc::pid_t>> {
    // SAFETY: a null buffer of size 0 only asks for the number of pids; nothing is written.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids: Vec<libc::pid_t> = vec![0; count as usize + PID_SLACK];
    let bytes = libc::c_int::try_from(pids.len() * size_of::<libc::pid_t>()).map_err(io::Error::other)?;
    // SAFETY: `pids` owns `bytes` writable bytes; proc_listallpids writes at most that many and
    // returns the number of pids written (or -1 with errno set).
    let written = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    if written < 0 {
        return Err(io::Error::last_os_error());
    }
    pids.truncate((written as usize).min(pids.len()));
    Ok(pids)
}

/// Cumulative CPU of one readable process at one listing, in Mach absolute time units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub pid: libc::pid_t,
    /// `ri_proc_start_abstime`; with the pid it identifies a process across pid reuse.
    pub start: u64,
    /// User + system CPU of the process itself.
    pub own: u64,
    /// User + system CPU of the terminated children it waited for (recursively accumulated).
    pub children: u64,
}

/// `proc_pid_rusage(RUSAGE_INFO_V1)` of `pid`.
fn process_cpu(pid: libc::pid_t) -> io::Result<Reading> {
    let mut info = MaybeUninit::<libc::rusage_info_v1>::zeroed();
    // SAFETY: flavor RUSAGE_INFO_V1 makes the kernel write exactly one rusage_info_v1 into the
    // buffer, which `info` provides; the pointer is only used during the call.
    let rc = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V1, info.as_mut_ptr().cast()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: proc_pid_rusage returned 0, so the struct is initialised (it was also zeroed).
    let info = unsafe { info.assume_init() };
    Ok(Reading {
        pid,
        start: info.ri_proc_start_abstime,
        own: info.ri_user_time.saturating_add(info.ri_system_time),
        children: info.ri_child_user_time.saturating_add(info.ri_child_system_time),
    })
}

/// `proc_name`, else the file name of `proc_pidpath` (which also works for root daemons such as
/// `aned`, where `proc_name` returns nothing).
fn process_name(pid: libc::pid_t) -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is writable for `buf.len()` bytes; proc_name writes at most that many and
    // returns the name length (0 or less on failure).
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n > 0 {
        return Some(String::from_utf8_lossy(&buf[..(n as usize).min(buf.len())]).into_owned());
    }
    let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `path` is writable for its full length (PROC_PIDPATHINFO_MAXSIZE bytes, the size
    // proc_pidpath requires); it writes at most that many and returns the path length (0 or less
    // on failure).
    let n = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    if n <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&path[..(n as usize).min(path.len())]).into_owned();
    path.rsplit('/').next().filter(|name| !name.is_empty()).map(str::to_owned)
}

fn parent_pid(pid: libc::pid_t) -> Option<libc::pid_t> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = libc::c_int::try_from(size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: `info` provides `size` writable bytes for one proc_bsdinfo; proc_pidinfo writes at
    // most `size` bytes and returns how many it wrote.
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if n != size {
        return None;
    }
    // SAFETY: proc_pidinfo wrote the whole struct (it was also zeroed).
    libc::pid_t::try_from(unsafe { info.assume_init() }.pbi_ppid).ok()
}

/// System daemons CoreML / the Neural Engine use; listed, never excluded.
pub fn is_coreml_daemon(name: &str) -> bool {
    name == "aned" || name.starts_with("ANE") || name.to_ascii_lowercase().contains("coreml")
}

/// (pid, start time).
type Key = (libc::pid_t, u64);

struct Tracked {
    pid: libc::pid_t,
    name: Option<String>,
    parent: Option<Key>,
    /// A descendant of the benchmark process: never contention.
    bench_child: bool,
    alive: bool,
    last_own: u64,
    last_children: u64,
    /// Own CPU credited during the run.
    own_cpu: u64,
    /// Reaped-children CPU credited during the run (after matching tracked children).
    children_cpu: u64,
    /// Last cumulative CPU of tracked children that exited, not yet matched by a rise of
    /// `children` (their CPU up to that listing was already credited to them).
    pending_children: u64,
    /// What this process added to its parent's `pending_children` when it vanished.
    handed_to_parent: u64,
}

impl Tracked {
    fn credited(&self) -> u64 {
        self.own_cpu.saturating_add(self.children_cpu)
    }
}

/// CPU accounting over successive process listings, in Mach absolute time units. Pure: the
/// readings come from `ProcessTable`, so the bookkeeping is unit-tested without syscalls.
struct Accounting {
    started_abs: u64,
    own_pid: libc::pid_t,
    tracked: HashMap<Key, Tracked>,
    /// Pid -> key of the live process with that pid.
    live: HashMap<libc::pid_t, Key>,
}

impl Accounting {
    fn new(started_abs: u64, own_pid: libc::pid_t) -> Self {
        Self { started_abs, own_pid, tracked: HashMap::new(), live: HashMap::new() }
    }

    /// Accounts one listing (`readings`: every readable process except this one) and returns the
    /// attributable CPU since the previous listing. `describe` gives name and parent pid of a
    /// process seen for the first time.
    fn observe(&mut self, readings: &[Reading], mut describe: impl FnMut(libc::pid_t) -> (Option<String>, Option<libc::pid_t>)) -> u64 {
        let present: HashSet<Key> = readings.iter().map(|r| (r.pid, r.start)).collect();

        // 1. Processes that vanished: their parent's next children-CPU rise starts with their
        //    cumulative CPU, which was credited to them already.
        let vanished: Vec<Key> = self.tracked.iter().filter(|(k, t)| t.alive && !present.contains(*k)).map(|(k, _)| *k).collect();
        for key in vanished {
            let Some(t) = self.tracked.get_mut(&key) else { continue };
            t.alive = false;
            let (parent, total) = (t.parent, t.last_own.saturating_add(t.last_children));
            if self.live.get(&key.0) == Some(&key) {
                self.live.remove(&key.0);
            }
            let handed = match parent.and_then(|p| self.tracked.get_mut(&p)).filter(|p| p.alive) {
                Some(p) => {
                    p.pending_children = p.pending_children.saturating_add(total);
                    total
                }
                None => 0,
            };
            if let Some(t) = self.tracked.get_mut(&key) {
                t.handed_to_parent = handed;
            }
        }

        // 2. New processes start from zero when they started during the run, else from their
        //    current values; a process that reappears takes its hand-over back.
        let mut new_keys = Vec::new();
        for r in readings {
            let key = (r.pid, r.start);
            self.live.insert(r.pid, key);
            match self.tracked.get_mut(&key) {
                None => {
                    let during_run = r.start >= self.started_abs;
                    self.tracked.insert(
                        key,
                        Tracked {
                            pid: r.pid,
                            name: None,
                            parent: None,
                            bench_child: false,
                            alive: true,
                            last_own: if during_run { 0 } else { r.own },
                            last_children: if during_run { 0 } else { r.children },
                            own_cpu: 0,
                            children_cpu: 0,
                            pending_children: 0,
                            handed_to_parent: 0,
                        },
                    );
                    new_keys.push(key);
                }
                Some(t) if !t.alive => {
                    t.alive = true;
                    let (parent, handed) = (t.parent, std::mem::take(&mut t.handed_to_parent));
                    if let Some(p) = parent.and_then(|p| self.tracked.get_mut(&p)) {
                        p.pending_children = p.pending_children.saturating_sub(handed);
                    }
                }
                Some(_) => {}
            }
        }
        // Parents start before their children: resolve in start order so a new child sees its
        // (possibly also new) parent's bench_child flag.
        new_keys.sort_by_key(|k| (k.1, k.0));
        for key in new_keys {
            let (name, ppid) = describe(key.0);
            let parent = ppid.and_then(|pp| self.live.get(&pp).copied()).filter(|p| *p != key);
            let bench_child = ppid == Some(self.own_pid) || parent.and_then(|p| self.tracked.get(&p)).is_some_and(|p| p.bench_child);
            if let Some(t) = self.tracked.get_mut(&key) {
                t.name = name;
                t.parent = parent;
                t.bench_child = bench_child;
            }
        }

        // 3. Deltas since the previous listing.
        let mut attributable = 0u64;
        for r in readings {
            let Some(t) = self.tracked.get_mut(&(r.pid, r.start)) else { continue };
            let own = r.own.saturating_sub(t.last_own);
            let rise = r.children.saturating_sub(t.last_children);
            let matched = rise.min(t.pending_children);
            t.pending_children -= matched;
            let children = rise - matched;
            t.last_own = t.last_own.max(r.own);
            t.last_children = t.last_children.max(r.children);
            t.own_cpu = t.own_cpu.saturating_add(own);
            t.children_cpu = t.children_cpu.saturating_add(children);
            if !t.bench_child {
                attributable = attributable.saturating_add(own).saturating_add(children);
            }
        }
        attributable
    }

    fn bench_child_cpu(&self) -> u64 {
        self.tracked.values().filter(|t| t.bench_child).map(Tracked::credited).sum()
    }

    fn user_cpu(&self) -> u64 {
        self.tracked.values().filter(|t| !t.bench_child).map(Tracked::credited).sum()
    }
}

/// Process listings for the sampler: readings from the kernel, bookkeeping in `Accounting`.
struct ProcessTable {
    timebase: Timebase,
    acct: Accounting,
    unreadable: HashMap<libc::pid_t, Option<String>>,
    listings: usize,
    list_errors: Vec<String>,
}

impl ProcessTable {
    fn new(timebase: Timebase) -> Self {
        // SAFETY: mach_absolute_time has no arguments and no failure mode.
        let started_abs = unsafe { mach_absolute_time() };
        Self {
            timebase,
            acct: Accounting::new(started_abs, std::process::id() as libc::pid_t),
            unreadable: HashMap::new(),
            listings: 0,
            list_errors: Vec::new(),
        }
    }

    /// One listing; attributable CPU seconds since the previous one, Err when the listing failed.
    fn observe(&mut self) -> Result<f64, String> {
        let pids = match all_pids() {
            Ok(p) => p,
            Err(e) => {
                let message = format!("proc_listallpids: {e}");
                if !self.list_errors.contains(&message) {
                    self.list_errors.push(message.clone());
                }
                return Err(message);
            }
        };
        self.listings += 1;
        let mut readings = Vec::with_capacity(pids.len());
        for pid in pids {
            if pid == self.acct.own_pid {
                continue;
            }
            match process_cpu(pid) {
                Ok(r) => readings.push(r),
                // Exited between the listing and the query.
                Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
                Err(_) => {
                    self.unreadable.entry(pid).or_insert_with(|| process_name(pid));
                }
            }
        }
        // `describe` runs only for processes seen for the first time.
        let cpu = self.acct.observe(&readings, |pid| (process_name(pid), parent_pid(pid)));
        Ok(self.timebase.seconds(cpu))
    }

    fn bench_child_seconds(&self) -> f64 {
        self.timebase.seconds(self.acct.bench_child_cpu())
    }

    fn report(&self, sampled_seconds: f64) -> Value {
        let seconds = |t: &Tracked| self.timebase.seconds(t.credited());
        let mut rows: Vec<&Tracked> = self.acct.tracked.values().collect();
        rows.sort_by(|a, b| seconds(b).total_cmp(&seconds(a)));
        let cores = |cpu: f64| (sampled_seconds > 0.0).then(|| round6(cpu / sampled_seconds));
        let row = |t: &Tracked| {
            let cpu = seconds(t);
            json!({
                "pid": t.pid, "name": t.name, "cpu_seconds": round6(cpu), "mean_cores": cores(cpu),
                "own_cpu_seconds": round6(self.timebase.seconds(t.own_cpu)),
                "reaped_children_cpu_seconds": round6(self.timebase.seconds(t.children_cpu)),
                "bench_child": t.bench_child,
            })
        };
        let top: Vec<Value> = rows.iter().filter(|t| !t.bench_child && seconds(t) > 0.0).take(TOP_PROCESSES).map(|t| row(t)).collect();
        let bench_children: Vec<Value> = rows.iter().filter(|t| t.bench_child && seconds(t) > 0.0).take(TOP_PROCESSES).map(|t| row(t)).collect();
        let mut coreml: Vec<Value> = rows
            .iter()
            .filter(|t| t.name.as_deref().is_some_and(is_coreml_daemon))
            .map(|t| {
                let cpu = seconds(t);
                json!({"pid": t.pid, "name": t.name, "readable": true, "cpu_seconds": round6(cpu), "mean_cores": cores(cpu)})
            })
            .collect();
        let mut unreadable_coreml: Vec<(&libc::pid_t, &String)> =
            self.unreadable.iter().filter_map(|(pid, name)| name.as_ref().filter(|n| is_coreml_daemon(n)).map(|n| (pid, n))).collect();
        unreadable_coreml.sort();
        coreml.extend(
            unreadable_coreml
                .into_iter()
                .map(|(pid, name)| json!({"pid": pid, "name": name, "readable": false, "cpu_seconds": null, "mean_cores": null})),
        );
        json!({
            "method": "proc_listallpids + proc_pid_rusage(RUSAGE_INFO_V1) per (pid, start time) at every sample, names from proc_name (else proc_pidpath); cpu_seconds = own CPU + reaped-children CPU credited during the run; processes of other users are usually unreadable and are counted, not attributed",
            "top_contending": top,
            "bench_children": bench_children,
            "coreml_daemons": coreml,
            "readable_processes": self.acct.tracked.len(),
            "unreadable_processes": self.unreadable.len(),
            "user_cpu_seconds": round6(self.timebase.seconds(self.acct.user_cpu())),
            "bench_child_cpu_seconds": round6(self.bench_child_seconds()),
            "listings": self.listings,
            "list_errors": self.list_errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(user: u32, system: u32, idle: u32, nice: u32) -> [u32; 4] {
        let mut t = [0; 4];
        t[libc::CPU_STATE_USER as usize] = user;
        t[libc::CPU_STATE_SYSTEM as usize] = system;
        t[libc::CPU_STATE_IDLE as usize] = idle;
        t[libc::CPU_STATE_NICE as usize] = nice;
        t
    }

    fn sampled(intervals: Vec<Interval>) -> ExternalCpu {
        ExternalCpu { intervals, listings: 3, error: None, ..ExternalCpu::failed(Some(8), "") }
    }

    #[test]
    fn interval_subtracts_own_cpu_and_clamps_at_zero() {
        let logical = 8.0;
        // Half of all ticks busy -> 4 busy cores; this process used 1.5 cores of them.
        let i = interval_between(2.0, ticks(0, 0, 0, 0), ticks(300, 80, 400, 20), 3.0, logical).unwrap();
        assert!((i.system_cores - 4.0).abs() < 1e-9, "{i:?}");
        assert!((i.own_cores - 1.5).abs() < 1e-9, "{i:?}");
        assert!((i.external_cores - 2.5).abs() < 1e-9, "{i:?}");
        assert_eq!(i.user_contention_cores, None, "attribution is filled in by the sampler");
        // Own CPU above the tick estimate never makes external load negative.
        let clamped = interval_between(1.0, ticks(0, 0, 0, 0), ticks(10, 0, 90, 0), 2.0, logical).unwrap();
        assert_eq!(clamped.external_cores, 0.0);
        // Counters wrap.
        let wrapped = interval_between(1.0, ticks(u32::MAX, 0, 0, 0), ticks(99, 0, 100, 0), 0.0, logical).unwrap();
        assert!((wrapped.system_cores - 4.0).abs() < 1e-9, "{wrapped:?}");
        // Too short, or no tick progress: no sample.
        assert!(interval_between(MIN_SAMPLE_SECONDS / 2.0, ticks(0, 0, 0, 0), ticks(1, 0, 1, 0), 0.0, logical).is_none());
        assert!(interval_between(1.0, ticks(5, 5, 5, 5), ticks(5, 5, 5, 5), 0.0, logical).is_none());
    }

    #[test]
    fn summaries_are_time_weighted_with_p95_and_max() {
        let iv = |seconds: f64, external: f64, user: f64| Interval {
            seconds,
            system_cores: external,
            own_cores: 0.0,
            external_cores: external,
            user_contention_cores: Some(user),
        };
        let ext = sampled(vec![iv(1.0, 1.0, 0.5), iv(3.0, 3.0, 1.5)]);
        assert_eq!(ext.samples(), 2);
        assert!((ext.mean_cores().unwrap() - 2.5).abs() < 1e-9);
        assert_eq!(ext.max_cores(), Some(3.0));
        assert_eq!(ext.p95_cores(), Some(3.0));
        assert!((ext.user_contention_mean().unwrap() - 1.25).abs() < 1e-9);
        assert!((ext.user_contention_core_seconds().unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(ext.user_contention_max(), Some(1.5));
        assert_eq!(ext.user_contention_p95(), Some(1.5));
        assert_eq!(ext.attribution_problem(), None);
        assert_eq!(ext.busy_threshold_cores(), Some(8.0 * BUSY_FRACTION_OF_LOGICAL_CPUS));
        let total = ext.total_json();
        let unattributed = ext.external_core_seconds() - ext.user_contention_core_seconds().unwrap();
        assert!((total["unattributed_core_seconds"].as_f64().unwrap() - round6(unattributed)).abs() < 1e-9, "{total}");
        assert!(total["note"].as_str().unwrap().contains("informational"));
        let empty = ExternalCpu::failed(Some(8), "none");
        assert_eq!((empty.mean_cores(), empty.p95_cores(), empty.max_cores()), (None, None, None));
        assert_eq!(empty.user_contention_mean(), None);
    }

    #[test]
    fn missing_attribution_has_no_contention_figures() {
        let iv = |user: Option<f64>| Interval { seconds: 1.0, system_cores: 1.0, own_cores: 0.0, external_cores: 1.0, user_contention_cores: user };
        let gap = sampled(vec![iv(Some(0.1)), iv(None)]);
        assert!(gap.attribution_problem().unwrap().contains("1 of 2 intervals"));
        assert_eq!((gap.user_contention_mean(), gap.user_contention_p95(), gap.user_contention_max()), (None, None, None));
        assert!(gap.total_json()["unattributed_core_seconds"].is_null());
        let mut no_listing = sampled(vec![iv(Some(0.1))]);
        no_listing.listings = 0;
        assert!(no_listing.attribution_problem().unwrap().contains("no process listing"));
        let mut failed_listing = sampled(vec![iv(Some(0.1))]);
        failed_listing.listing_errors = vec!["proc_listallpids: denied".to_string()];
        assert!(failed_listing.attribution_problem().unwrap().contains("listing failed"));
        assert_eq!(accumulate(Some(1.0), Err("x".to_string())), None);
        assert_eq!(accumulate(None, Ok(1.0)), None);
        assert_eq!(accumulate(Some(1.0), Ok(2.0)), Some(3.0));
    }

    const OWN: libc::pid_t = 500;
    const RUN_START: u64 = 1_000;

    fn reading(pid: libc::pid_t, start: u64, own: u64, children: u64) -> Reading {
        Reading { pid, start, own, children }
    }

    /// Parent pids for `describe`.
    fn parents(map: &[(libc::pid_t, libc::pid_t)]) -> impl FnMut(libc::pid_t) -> (Option<String>, Option<libc::pid_t>) + '_ {
        move |pid| (Some(format!("p{pid}")), map.iter().find(|(c, _)| *c == pid).map(|(_, p)| *p))
    }

    #[test]
    fn a_long_lived_process_counts_its_cpu_during_the_run_only() {
        let mut a = Accounting::new(RUN_START, OWN);
        let tree = [(10, 1)];
        // Started before the run with CPU already spent: the baseline is not counted.
        assert_eq!(a.observe(&[reading(10, 1, 700, 40)], parents(&tree)), 0);
        assert_eq!(a.observe(&[reading(10, 1, 900, 40)], parents(&tree)), 200);
        assert_eq!(a.observe(&[reading(10, 1, 1_000, 40)], parents(&tree)), 100);
        assert_eq!(a.user_cpu(), 300);
    }

    #[test]
    fn processes_that_start_and_exit_during_the_run_are_counted() {
        let mut a = Accounting::new(RUN_START, OWN);
        let tree = [(10, 1), (11, 10), (12, 10)];
        assert_eq!(a.observe(&[reading(10, 1, 0, 0)], parents(&tree)), 0);
        // 11 starts during the run: everything it used counts, from its start.
        assert_eq!(a.observe(&[reading(10, 1, 0, 0), reading(11, RUN_START + 5, 300, 0)], parents(&tree)), 300);
        assert_eq!(a.observe(&[reading(10, 1, 0, 0), reading(11, RUN_START + 5, 450, 0)], parents(&tree)), 150);
        // 11 exits after 500 in total and its parent reaps it; 12 runs entirely between two
        // listings (never seen) and is reaped too: 11's unseen tail and all of 12 count once.
        let unseen_short_child = 80;
        assert_eq!(a.observe(&[reading(10, 1, 0, 500 + unseen_short_child)], parents(&tree)), 50 + unseen_short_child);
        assert_eq!(a.user_cpu(), 500 + unseen_short_child);
        // A reused pid with a new start time is a new process.
        assert_eq!(a.observe(&[reading(10, 1, 0, 580), reading(11, RUN_START + 99, 20, 0)], parents(&tree)), 20);
    }

    #[test]
    fn a_child_reaped_after_its_exit_listing_is_not_double_counted() {
        let mut a = Accounting::new(RUN_START, OWN);
        let tree = [(10, 1), (11, 10)];
        a.observe(&[reading(10, 1, 0, 0), reading(11, RUN_START + 1, 100, 0)], parents(&tree));
        // 11 vanished, parent has not reaped it yet (zombie not readable).
        assert_eq!(a.observe(&[reading(10, 1, 0, 0)], parents(&tree)), 0);
        // Reaped later: only the 20 it used after the last listing that saw it are new.
        assert_eq!(a.observe(&[reading(10, 1, 0, 120)], parents(&tree)), 20);
        assert_eq!(a.user_cpu(), 120);
    }

    #[test]
    fn the_benchmark_and_its_descendants_are_excluded() {
        let mut a = Accounting::new(RUN_START, OWN);
        // 20 is a child of the benchmark (`cp -Rc`), 21 its child; 30 is unrelated.
        let tree = [(20, OWN), (21, 20), (30, 1)];
        let first = [reading(20, RUN_START + 1, 100, 0), reading(21, RUN_START + 2, 50, 0), reading(30, RUN_START + 3, 7, 0)];
        assert_eq!(a.observe(&first, parents(&tree)), 7);
        assert_eq!(a.bench_child_cpu(), 150);
        assert_eq!(a.user_cpu(), 7);
        assert!(a.tracked[&(21, RUN_START + 2)].bench_child);
    }

    /// Smoke test: the sampler runs briefly, takes samples and attributes every interval.
    #[test]
    fn sampler_records_samples_briefly() {
        let interval = Duration::from_millis(200);
        let sampler = Sampler::start(interval);
        // A short-lived child process of this test is a bench child, never contention.
        let status = std::process::Command::new("/bin/sh").args(["-c", "i=0; while [ $i -lt 20000 ]; do i=$((i+1)); done"]).status().unwrap();
        assert!(status.success());
        // Generous headroom: parallel tests and a freshly built binary can delay the sampler thread.
        thread::sleep(interval * 8);
        let ext = sampler.finish();
        assert!(ext.error.is_none(), "{:?}", ext.error);
        assert!(ext.logical_cpus.is_some_and(|n| n > 0));
        assert!(ext.samples() >= 3, "{} samples", ext.samples());
        assert_eq!(ext.attribution_problem(), None, "{:?}", ext.listing_errors);
        let n = f64::from(ext.logical_cpus.unwrap());
        for i in &ext.intervals {
            assert!(i.seconds >= MIN_SAMPLE_SECONDS);
            assert!((0.0..=n + 1e-9).contains(&i.system_cores), "{i:?}");
            assert!(i.external_cores >= 0.0 && i.external_cores <= i.system_cores + 1e-9, "{i:?}");
            assert!(i.user_contention_cores.is_some_and(|u| u >= 0.0), "{i:?}");
        }
        let mean = ext.mean_cores().unwrap();
        assert!(mean >= 0.0 && mean <= n);
        assert!(ext.user_contention_mean().is_some());
        assert!(ext.processes["readable_processes"].as_u64().unwrap() > 0, "{}", ext.processes);
        assert!(ext.listings >= 2);
        let doc = ext.contention_json();
        assert_eq!(doc["samples"], ext.samples());
        assert!(doc["mean_cores"].is_number() && doc["p95_cores"].is_number() && doc["max_cores"].is_number());
        assert!(ext.total_json()["unattributed_core_seconds"].is_number());
    }

    #[test]
    fn coreml_daemons_are_recognised() {
        for name in ["aned", "ANECompilerService", "ANEStorageMaintainer", "com.apple.CoreMLModelSecurityService"] {
            assert!(is_coreml_daemon(name), "{name}");
        }
        for name in ["launchd", "planed", "cargo"] {
            assert!(!is_coreml_daemon(name), "{name}");
        }
    }

    /// launchd (pid 1, root) is unreadable for `proc_name` from a user process; the path fallback names it.
    #[test]
    fn processes_of_other_users_get_a_name() {
        assert_eq!(process_name(1).as_deref(), Some("launchd"));
        let own = process_name(std::process::id() as libc::pid_t);
        assert!(own.is_some_and(|n| !n.is_empty()));
    }

    /// The rusage clock of `proc_pid_rusage` (Mach units) agrees with getrusage for this process.
    #[test]
    fn proc_pid_rusage_and_getrusage_agree_for_self() {
        let timebase = Timebase::query().unwrap();
        let busy_until = Instant::now() + Duration::from_millis(300);
        let mut x = 0u64;
        while Instant::now() < busy_until {
            x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
        }
        let own = own_cpu_seconds().unwrap();
        let r = process_cpu(std::process::id() as libc::pid_t).unwrap();
        let via_proc = timebase.seconds(r.own);
        assert!(r.start > 0);
        // Both read the same counters a moment apart; allow scheduling slack.
        assert!((via_proc - own).abs() < 0.25 * own.max(0.2), "proc_pid_rusage {via_proc} s vs getrusage {own} s");
    }

    /// A reaped child's CPU shows up in the parent's `ri_child_*` fields.
    #[test]
    fn reaped_children_cpu_is_reported() {
        let timebase = Timebase::query().unwrap();
        let before = process_cpu(std::process::id() as libc::pid_t).unwrap();
        let status = std::process::Command::new("/bin/sh").args(["-c", "i=0; while [ $i -lt 200000 ]; do i=$((i+1)); done"]).status().unwrap();
        assert!(status.success());
        let after = process_cpu(std::process::id() as libc::pid_t).unwrap();
        assert!(timebase.seconds(after.children.saturating_sub(before.children)) > 0.0, "{before:?} -> {after:?}");
    }
}
