//! Daemon state shared by every handler and background task.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use aegis_ane_scorer::{compute_plan, AneScorer, ComputeUnits};
use aegis_config::Settings;
use aegis_governance::GovernanceEngine;
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, watch};

use crate::approvals::ApprovalStore;
use crate::keepwarm::KeepWarm;
use crate::live::LiveStats;
use crate::paypal::PayPalEngine;
use crate::runs::RunHistory;
use crate::util::{now_iso, random_token};
use crate::workspace::WorkspaceEngine;

pub const DEFAULT_PORT: u16 = 4173;
/// Default of `AEGIS_ANE_KEEPWARM_MS`.
pub const DEFAULT_KEEPWARM_INTERVAL_MS: u64 = 1000;

/// Everything the daemon needs to know about its environment. `from_env` is what the binary
/// uses; tests build one pointing at temporary directories.
#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub port: u16,
    pub repo_root: PathBuf,
    /// `apps/aegis-console` (served statically, except `dev/` and `API.md`).
    pub console_dir: PathBuf,
    /// Holds `live/` (the demo crate), its `.aegis-live` store and scratch directories.
    pub demo_root: PathBuf,
    pub dotenv_path: PathBuf,
    /// Benchmark artifact read by `GET /api/telemetry` at request time.
    pub bench_path: PathBuf,
    /// Receipt signing seed (`ReceiptSigner::load_or_create`).
    pub signing_seed: PathBuf,
    pub pool_capacity: usize,
    /// Timeout of the fixed verification command run inside a branch.
    pub verify_timeout: Duration,
    /// PayPal Sandbox submissions allowed per daemon run.
    pub sandbox_cap: u32,
    /// Lower bound of the auto-stream interval.
    pub min_stream_interval_ms: u64,
    /// CoreML keep-warm interval (`AEGIS_ANE_KEEPWARM_MS`); 0 disables it.
    pub keepwarm_interval_ms: u64,
    /// True (the binary): the process environment overrides `.env`, as documented. Tests set it
    /// false so credentials exported in the developer's shell can never reach the daemon under test.
    pub env_overrides_dotenv: bool,
}

impl DaemonOptions {
    /// `AEGIS_CONSOLE_PORT`, `AEGIS_CONSOLE_DIR`, `AEGIS_CONSOLE_DEMO_DIR`, `AEGIS_DOTENV`,
    /// `AEGIS_ROOT`, `AEGIS_ANE_KEEPWARM_MS`; defaults relative to the repository this crate was
    /// built in.
    pub fn from_env() -> Self {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        let repo_root = canonical(aegis_config::repo_root());
        let console_dir = canonical(var("AEGIS_CONSOLE_DIR").map(PathBuf::from).unwrap_or_else(|| repo_root.join("apps/aegis-console")));
        Self {
            port: var("AEGIS_CONSOLE_PORT").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT),
            console_dir,
            demo_root: var("AEGIS_CONSOLE_DEMO_DIR").map(PathBuf::from).unwrap_or_else(|| repo_root.join("target/aegis-console/demo")),
            dotenv_path: aegis_config::default_dotenv_path(),
            bench_path: repo_root.join("artifacts/aegis_phase1_telemetry.json"),
            signing_seed: repo_root.join("artifacts/keys/console_receipt_ed25519.seed"),
            pool_capacity: 2,
            verify_timeout: Duration::from_secs(60),
            sandbox_cap: 25,
            min_stream_interval_ms: 1000,
            keepwarm_interval_ms: var("AEGIS_ANE_KEEPWARM_MS").and_then(|v| v.trim().parse().ok()).unwrap_or(DEFAULT_KEEPWARM_INTERVAL_MS),
            env_overrides_dotenv: true,
            repo_root,
        }
    }
}

/// `path` without `..` components when it exists (display and comparisons), else unchanged.
fn canonical(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// WebSocket fan-out. Every message is a JSON object with `type` and `ts`. Run messages
/// (`run.started`, `run.stage`, `run.finished`) also get their per-run `seq` and are recorded in
/// the run history (`GET /api/runs`) under the same lock as the broadcast, so history order and
/// `seq` order are broadcast order.
#[derive(Debug, Clone)]
pub struct Events {
    tx: broadcast::Sender<Arc<str>>,
    history: Arc<Mutex<RunHistory>>,
}

impl Events {
    pub fn new(capacity: usize) -> Self {
        Self { tx: broadcast::channel(capacity).0, history: Arc::new(Mutex::new(RunHistory::default())) }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.tx.subscribe()
    }

    fn object(kind: &str, fields: Value) -> Map<String, Value> {
        let mut obj = match fields {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        obj.insert("type".into(), json!(kind));
        obj.insert("ts".into(), json!(now_iso()));
        obj
    }

    /// Serialises `{type, ts, ...fields}` (never recorded in the run history; see [`Events::send`]).
    pub fn message(kind: &str, fields: Value) -> Arc<str> {
        Arc::from(Value::Object(Self::object(kind, fields)).to_string())
    }

    /// The notice a WebSocket client gets after the broadcast channel dropped `dropped` messages
    /// for it: a `log` warning whose `dropped` field is the machine-readable count (API.md).
    pub fn lag_notice(dropped: u64) -> Arc<str> {
        Self::message(
            "log",
            json!({"level": "warn", "message": format!("this client fell behind; {dropped} messages were dropped"), "dropped": dropped}),
        )
    }

    /// Broadcasts `{type, ts, ...fields}`. Never blocks on receivers: slow ones lag, senders do not wait.
    pub fn send(&self, kind: &str, fields: Value) {
        let mut obj = Self::object(kind, fields);
        // Held until the message is on the channel: two senders for the same run cannot broadcast
        // in an order different from their `seq` order.
        let _history = RunHistory::tracks(kind).then(|| {
            let mut history = lock(&self.history);
            history.observe(&mut obj);
            history
        });
        let _ = self.tx.send(Arc::from(Value::Object(obj).to_string()));
    }

    pub fn log(&self, level: &str, message: impl Into<String>) {
        self.send("log", json!({"level": level, "message": message.into()}));
    }

    pub fn stage(&self, run_id: &str, stage: &str, status: &str, ms: Option<f64>, detail: Value) {
        self.send("run.stage", json!({"run_id": run_id, "stage": stage, "status": status, "ms": ms, "detail": detail}));
    }

    /// `GET /api/runs`.
    pub fn runs_json(&self) -> Value {
        lock(&self.history).list_json()
    }
}

/// Locks a std mutex, recovering the data if another thread panicked while holding it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct AppState {
    pub opts: DaemonOptions,
    pub token: String,
    pub started_at: String,
    /// Static part of `GET /api/health` (model placement is read once at startup).
    pub health: Value,
    pub events: Events,
    pub live: Mutex<LiveStats>,
    pub settings: RwLock<Settings>,
    pub scorer: Arc<AneScorer>,
    pub gov: Arc<GovernanceEngine>,
    pub workspace: WorkspaceEngine,
    pub approvals: Mutex<ApprovalStore>,
    pub paypal: PayPalEngine,
    pub keepwarm: KeepWarm,
    /// Held across write `.env` -> reload -> swap `settings` in `POST /api/config`, so concurrent
    /// saves are applied (and become visible) in one order.
    pub config_lock: Mutex<()>,
    pub shutdown: watch::Sender<bool>,
    run_seq: AtomicU64,
}

impl AppState {
    /// Opens the CoreML scorers, Cedar policies, the demo workspace with its warm pool, and the
    /// PayPal middleware. Blocking; call from `spawn_blocking` or before starting the runtime.
    pub fn open(opts: DaemonOptions) -> Result<Arc<Self>, String> {
        let settings = load_settings(&opts).map_err(|e| format!("reading {}: {e}", opts.dotenv_path.display()))?;
        let scorer = Arc::new(AneScorer::open_default().map_err(|e| format!("opening the CoreML model: {e}"))?);
        let paypal_scorer = Arc::new(AneScorer::open_default().map_err(|e| format!("opening the CoreML model: {e}"))?);
        let gov = Arc::new(GovernanceEngine::load_default().map_err(|e| format!("loading Cedar policies: {e}"))?);
        let plan = compute_plan(scorer.model_path(), ComputeUnits::CpuAndNeuralEngine)
            .map_err(|e| format!("reading MLComputePlan: {e}"))?;
        let workspace = WorkspaceEngine::open(&opts.demo_root, opts.pool_capacity).map_err(|e| format!("demo workspace: {e}"))?;
        let keepwarm = KeepWarm::new(opts.keepwarm_interval_ms, scorer.clone(), paypal_scorer.clone());
        let paypal = PayPalEngine::new(&opts, &settings, gov.clone(), paypal_scorer)?;
        let started_at = now_iso();
        let model_name = scorer.model_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let health = json!({
            "ok": true,
            "mock": false,
            "version": env!("CARGO_PKG_VERSION"),
            "git_commit": git_commit(&opts.repo_root),
            "started_at": started_at,
            "workspace": canonical(workspace.live_path()).display().to_string(),
            "compute_units": scorer.units().name(),
            "model": {
                "name": model_name,
                "path": canonical(scorer.model_path().to_path_buf()).display().to_string(),
                "layout": scorer.layout().name(),
                "input_shape": [1, scorer.feature_count(), 1, scorer.window_len()],
                "compute_plan": {
                    "neural_engine": plan.preferred.get("neural_engine").copied().unwrap_or(0),
                    "cpu": plan.preferred.get("cpu").copied().unwrap_or(0),
                    "gpu": plan.preferred.get("gpu").copied().unwrap_or(0),
                },
                "operations_dispatched": plan.operations_dispatched,
            },
        });
        Ok(Arc::new(Self {
            token: random_token(),
            started_at,
            health,
            events: Events::new(1024),
            live: Mutex::new(LiveStats::default()),
            settings: RwLock::new(settings),
            scorer,
            gov,
            workspace,
            approvals: Mutex::new(ApprovalStore::default()),
            paypal,
            keepwarm,
            config_lock: Mutex::new(()),
            shutdown: watch::channel(false).0,
            run_seq: AtomicU64::new(0),
            opts,
        }))
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Reads the settings again from `.env` (and the environment unless isolated).
    pub fn load_settings(&self) -> std::io::Result<Settings> {
        load_settings(&self.opts)
    }

    pub fn next_run_id(&self) -> String {
        format!("r-{:06}", self.run_seq.fetch_add(1, Ordering::Relaxed) + 1)
    }

    pub fn record(&self, key: &str, value: f64) {
        lock(&self.live).record(key, value);
    }

    pub fn live_json(&self) -> Value {
        let pending = lock(&self.approvals).pending_count();
        lock(&self.live).snapshot(pending)
    }

    /// Static health plus the keep-warm counters.
    pub fn health_json(&self) -> Value {
        let mut health = self.health.clone();
        health["ane_keepwarm"] = self.keepwarm.json();
        health
    }
}

fn load_settings(opts: &DaemonOptions) -> std::io::Result<Settings> {
    if opts.env_overrides_dotenv {
        Settings::load(&opts.dotenv_path)
    } else {
        Settings::load_isolated(&opts.dotenv_path)
    }
}

/// Short commit hash of the repository (read-only `git rev-parse`), or `null`.
fn git_commit(repo: &Path) -> Value {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map_or(Value::Null, Value::String)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_notice_carries_the_dropped_count_as_a_field() {
        let dropped = u64::from(std::process::id() % 97) + 1;
        let notice: Value = serde_json::from_str(&Events::lag_notice(dropped)).unwrap();
        assert_eq!(notice["type"], "log");
        assert_eq!(notice["level"], "warn");
        assert_eq!(notice["dropped"], dropped);
        assert!(notice["message"].as_str().unwrap().contains(&format!("{dropped} messages were dropped")));
        assert!(notice["ts"].is_string());
    }

    #[test]
    fn run_messages_get_seq_and_are_recorded_other_messages_are_not() {
        let events = Events::new(16);
        let mut rx = events.subscribe();
        events.send("run.started", json!({"run_id": "r-000001", "scenario": "secret_leak", "title": "t"}));
        events.stage("r-000001", "branch", "ok", None, json!({}));
        events.log("info", "not a run message");
        let started: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        let stage: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        let log: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!((started["seq"].clone(), stage["seq"].clone()), (json!(1), json!(2)));
        assert!(log.get("seq").is_none());
        let runs = events.runs_json();
        assert_eq!(runs["runs"][0]["stages"], json!([stage]));
        assert_eq!(runs["runs"][0]["started_at"], started["ts"]);
    }
}
