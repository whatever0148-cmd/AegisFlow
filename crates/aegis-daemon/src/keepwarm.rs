//! Optional CoreML keep-warm (`AEGIS_ANE_KEEPWARM_MS`, default 1000, `0` disables).
//!
//! The first prediction after the daemon has been idle for a few seconds can take much longer
//! than the warm ones. When enabled, a background task runs one prediction on an all-zero window
//! for each model that has not predicted for `interval`. Pings are never recorded in `LiveStats`
//! (every real prediction still is, as measured); their count and the latency of the last one are
//! reported in `GET /api/health` under `ane_keepwarm`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aegis_ane_scorer::{AneError, AneScorer, ContextWindow, FeatureVector};
use serde_json::{json, Value};
use tokio::sync::watch;

use crate::state::lock;
use crate::util::ms;

/// The models the daemon predicts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Workspace scenarios and recovery triage (`AppState::scorer`).
    Workspace,
    /// The PayPal middleware's own scorer instance.
    PayPal,
}

impl Model {
    pub const ALL: [Model; 2] = [Model::Workspace, Model::PayPal];

    pub fn name(self) -> &'static str {
        match self {
            Model::Workspace => "workspace",
            Model::PayPal => "paypal",
        }
    }
}

struct Target {
    scorer: Arc<AneScorer>,
    /// Microseconds since `KeepWarm::origin` at the end of the last prediction (real or ping).
    last_used_us: AtomicU64,
    pings: AtomicU64,
    failures: AtomicU64,
    last_ping: Mutex<Option<Duration>>,
}

pub struct KeepWarm {
    interval: Option<Duration>,
    origin: Instant,
    targets: [Target; 2],
    /// Total successful pings, for waiters (tests).
    total: watch::Sender<u64>,
}

impl KeepWarm {
    /// `interval_ms == 0` disables pinging (usage is still tracked, which costs one atomic store).
    pub fn new(interval_ms: u64, workspace: Arc<AneScorer>, paypal: Arc<AneScorer>) -> Self {
        let target = |scorer| Target {
            scorer,
            last_used_us: AtomicU64::new(0),
            pings: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            last_ping: Mutex::new(None),
        };
        Self {
            interval: (interval_ms > 0).then(|| Duration::from_millis(interval_ms)),
            origin: Instant::now(),
            targets: [target(workspace), target(paypal)],
            total: watch::channel(0).0,
        }
    }

    /// Receiver of the total ping count (changes after every successful ping).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.total.subscribe()
    }

    fn target(&self, model: Model) -> &Target {
        &self.targets[model as usize]
    }

    pub fn interval(&self) -> Option<Duration> {
        self.interval
    }

    fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Records that `model` just finished a prediction.
    pub fn touch(&self, model: Model) {
        self.target(model).last_used_us.store(self.now_us(), Ordering::Relaxed);
    }

    /// How long until `model` is due for a ping (`Duration::ZERO` when it is due now).
    pub fn due_in(&self, model: Model) -> Option<Duration> {
        let interval = self.interval?;
        let idle = Duration::from_micros(self.now_us().saturating_sub(self.target(model).last_used_us.load(Ordering::Relaxed)));
        Some(interval.saturating_sub(idle))
    }

    /// One prediction on an all-zero window. Blocking. Not recorded in `LiveStats`.
    pub fn ping(&self, model: Model) -> Result<Duration, AneError> {
        let target = self.target(model);
        let window = ContextWindow::from_events(target.scorer.window_len(), [FeatureVector::default()]);
        let started = Instant::now();
        let result = target.scorer.score_window(&window);
        let elapsed = started.elapsed();
        self.touch(model);
        match result {
            Ok(_) => {
                target.pings.fetch_add(1, Ordering::Relaxed);
                *lock(&target.last_ping) = Some(elapsed);
                self.total.send_modify(|n| *n += 1);
                Ok(elapsed)
            }
            Err(e) => {
                target.failures.fetch_add(1, Ordering::Relaxed);
                Err(e)
            }
        }
    }

    /// `ane_keepwarm` in `GET /api/health`.
    pub fn json(&self) -> Value {
        let models: serde_json::Map<String, Value> = Model::ALL
            .iter()
            .map(|m| {
                let t = self.target(*m);
                let last = *lock(&t.last_ping);
                (
                    m.name().to_string(),
                    json!({
                        "pings": t.pings.load(Ordering::Relaxed),
                        "failures": t.failures.load(Ordering::Relaxed),
                        "last_ping_ms": last.map(ms),
                    }),
                )
            })
            .collect();
        let pings: u64 = self.targets.iter().map(|t| t.pings.load(Ordering::Relaxed)).sum();
        json!({
            "enabled": self.interval.is_some(),
            "interval_ms": self.interval.map(|d| d.as_millis() as u64),
            "pings": pings,
            "models": models,
        })
    }
}
