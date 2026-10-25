//! Test fixture: a daemon state over temporary directories, driven through the real router with
//! `tower::ServiceExt::oneshot`. Nothing here binds a port or calls an external service: settings
//! come from the fixture's own `.env` only (`env_overrides_dotenv: false`), so credentials exported
//! in the developer's shell never reach the daemon under test and no test needs to skip.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aegis_daemon::{router, spawn_background, workspace, AppState, DaemonOptions};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tokio::sync::broadcast;
use tower::ServiceExt;

pub const PORT: u16 = 4173;
pub const HOST: &str = "localhost:4173";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").components().collect()
}

pub struct Fixture {
    pub root: PathBuf,
    pub state: Arc<AppState>,
    pub app: Router,
}

impl Fixture {
    pub fn options(root: &Path) -> DaemonOptions {
        let repo = repo_root();
        DaemonOptions {
            port: PORT,
            console_dir: repo.join("apps/aegis-console"),
            demo_root: root.join("demo"),
            dotenv_path: root.join(".env"),
            bench_path: root.join("bench.json"),
            signing_seed: root.join("keys/receipt.seed"),
            pool_capacity: 1,
            verify_timeout: Duration::from_secs(120),
            sandbox_cap: 25,
            min_stream_interval_ms: 1000,
            // Off: tests assert on prediction counts; the keep-warm has its own test.
            keepwarm_interval_ms: 0,
            env_overrides_dotenv: false,
            repo_root: repo,
        }
    }

    pub fn scratch(name: &str) -> PathBuf {
        let root = repo_root().join("target/test-fixtures").join(format!("daemon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// New daemon state with an empty `.env` (or `dotenv` contents) under a fresh scratch directory.
    pub async fn new(name: &str, dotenv: &str) -> Self {
        let root = Self::scratch(name);
        std::fs::write(root.join(".env"), dotenv).unwrap();
        Self::with_options(root.clone(), Self::options(&root)).await
    }

    pub async fn with_options(root: PathBuf, opts: DaemonOptions) -> Self {
        let state = tokio::task::spawn_blocking(move || AppState::open(opts)).await.unwrap().expect("daemon state opens");
        spawn_background(&state);
        let app = router(state.clone());
        Self { root, state, app }
    }

    pub fn live(&self) -> PathBuf {
        self.root.join("demo/live")
    }

    pub fn get(&self, uri: &str) -> Request<Body> {
        Request::get(uri).header("host", HOST).body(Body::empty()).unwrap()
    }

    pub fn post(&self, uri: &str, body: &Value) -> Request<Body> {
        Request::post(uri)
            .header("host", HOST)
            .header("content-type", "application/json")
            .header("x-aegis-token", &self.state.token)
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    pub async fn send(&self, req: Request<Body>) -> (StatusCode, String) {
        let response = self.app.clone().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 64 * 1024 * 1024).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    pub async fn json(&self, req: Request<Body>) -> (StatusCode, Value) {
        let (status, text) = self.send(req).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.state.events.subscribe()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.state.shutdown.send(true);
        workspace::shutdown(&self.state, Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Waits for the first event satisfying `pred`, collecting every event seen on the way.
pub async fn wait_for(
    rx: &mut broadcast::Receiver<Arc<str>>,
    seen: &mut Vec<Value>,
    timeout: Duration,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let msg = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(m)) => m,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(e)) => panic!("event channel closed: {e}"),
            Err(_) => {
                let shown: Vec<String> = seen.iter().filter(|v| v["type"] != "telemetry.live").map(|v| v.to_string()).collect();
                panic!("timed out waiting for an event; non-telemetry events seen:\n{}", shown.join("\n"))
            }
        };
        let value: Value = serde_json::from_str(&msg).unwrap();
        seen.push(value.clone());
        if pred(&value) {
            return value;
        }
    }
}

pub fn is(kind: &str, run_id: &str) -> impl Fn(&Value) -> bool {
    let (kind, run_id) = (kind.to_string(), run_id.to_string());
    move |v: &Value| v["type"] == kind.as_str() && v["run_id"] == run_id.as_str()
}

/// `run.stage` events of one run, in arrival order.
pub fn stages<'a>(seen: &'a [Value], run_id: &str) -> Vec<&'a Value> {
    seen.iter().filter(|v| v["type"] == "run.stage" && v["run_id"] == run_id).collect()
}

pub fn stage<'a>(seen: &'a [Value], run_id: &str, name: &str) -> Option<&'a Value> {
    stages(seen, run_id).into_iter().find(|v| v["stage"] == name)
}
