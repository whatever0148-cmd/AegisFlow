//! Routes (API.md "HTTP" and "WebSocket"), static files and background tasks.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tower_http::services::ServeDir;

use crate::keepwarm::Model;
use crate::paypal::OrderScenario;
use crate::security::{error, guard};
use crate::services::{config_json, console_writable, test_service};
use crate::state::{lock, AppState, Events};
use crate::util::clean_text;
use crate::workspace::{self, ResolveError, Scenario};

const BODY_LIMIT: usize = 256 * 1024;

pub fn router(state: Arc<AppState>) -> Router {
    let console = state.opts.console_dir.clone();
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/api/health", get(health))
        .route("/api/telemetry", get(telemetry))
        .route("/api/config", get(get_config).post(post_config))
        .route("/api/config/test", post(post_config_test))
        .route("/api/workspace", get(get_workspace))
        .route("/api/workspace/run", post(post_run))
        .route("/api/workspace/reset", post(post_reset))
        .route("/api/approvals", get(get_approvals))
        .route("/api/approvals/{id}/approve", post(post_approve))
        .route("/api/approvals/{id}/reject", post(post_reject))
        .route("/api/paypal/orders", get(get_orders))
        .route("/api/paypal/order", post(post_order))
        .route("/api/paypal/scenarios", get(get_paypal_scenarios))
        .route("/api/paypal/stream", post(post_stream))
        .route("/ws", get(ws_upgrade))
        // Only these two directories are served: never dev/, API.md or anything else in the app.
        .nest_service("/assets", ServeDir::new(console.join("assets")))
        .nest_service("/vendor/ag-grid-community", ServeDir::new(console.join("vendor/ag-grid-community")))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "not found")
}

fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    let text = if body.is_empty() { &b"{}"[..] } else { &body[..] };
    serde_json::from_slice(text).map_err(|e| error(StatusCode::BAD_REQUEST, format!("invalid JSON body: {e}")))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, Response> {
    tokio::task::spawn_blocking(f).await.map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, format!("worker failed: {e}")))
}

// ---------------------------------------------------------------------------------------------
// Static files.

async fn index(State(state): State<Arc<AppState>>) -> Response {
    let path = state.opts.console_dir.join("index.html");
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => {
            let body = text.replace("%%AEGIS_TOKEN%%", &state.token);
            let mut response = Response::new(Body::from(body));
            let h = response.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("reading index.html: {e}")),
    }
}

// ---------------------------------------------------------------------------------------------
// Health, telemetry, config.

async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.health_json())
}

async fn telemetry(State(state): State<Arc<AppState>>) -> Response {
    let path = state.opts.bench_path.clone();
    let bench = match tokio::fs::read(&path).await {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => v,
            Err(e) => {
                state.events.log("warn", format!("bench artifact {} is not valid JSON: {e}", path.display()));
                Value::Null
            }
        },
        Err(_) => Value::Null,
    };
    let generated_at = bench.get("generated_at").cloned().unwrap_or(Value::Null);
    Json(json!({"bench": bench, "bench_path": path.display().to_string(), "bench_generated_at": generated_at, "live": state.live_json()}))
        .into_response()
}

async fn get_config(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(config_json(&state.settings()))
}

#[derive(serde::Deserialize)]
struct ConfigBody {
    values: serde_json::Map<String, Value>,
}

async fn post_config(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let body: ConfigBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mut updates: Vec<(String, Option<String>)> = Vec::new();
    for (name, value) in body.values {
        if aegis_config::spec(&name).is_none() {
            return error(StatusCode::BAD_REQUEST, format!("unknown setting {name}"));
        }
        // Only "" unsets (API.md). A whitespace-only value is refused rather than read as "unset",
        // so a stray space cannot delete a stored key.
        let value = match value {
            Value::String(s) if s.is_empty() => None,
            Value::String(s) if s.trim().is_empty() => {
                return error(StatusCode::BAD_REQUEST, format!("{name}: value contains only whitespace; send \"\" to unset"))
            }
            Value::String(s) => Some(s),
            Value::Null => None,
            _ => return error(StatusCode::BAD_REQUEST, format!("{name}: value must be a string")),
        };
        // The session token stops browsers, not other local processes, so a setting that decides
        // where a stored secret is sent cannot be set over HTTP (unsetting restores the default).
        if value.is_some() && !console_writable(&name) {
            return error(
                StatusCode::BAD_REQUEST,
                format!("{name} cannot be set from the console; use `aegis-daemon config set {name}` (harness/configure_keys.sh) or the environment"),
            );
        }
        updates.push((name, value));
    }
    if updates.is_empty() {
        return error(StatusCode::BAD_REQUEST, "values is empty");
    }
    let st = state.clone();
    let names: Vec<String> = updates.iter().map(|(n, _)| n.clone()).collect();
    let result = blocking(move || -> Result<Value, (StatusCode, String)> {
        // One save at a time: the file write, the reload and the in-memory swap happen in the same
        // order for every request (write_dotenv also locks against other processes).
        let _saving = lock(&st.config_lock);
        let path = st.opts.dotenv_path.clone();
        let refs: Vec<(&str, Option<&str>)> = updates.iter().map(|(n, v)| (n.as_str(), v.as_deref())).collect();
        aegis_config::write_dotenv(&path, &refs).map_err(|e| {
            let status = if e.kind() == std::io::ErrorKind::InvalidInput { StatusCode::BAD_REQUEST } else { StatusCode::INTERNAL_SERVER_ERROR };
            (status, e.to_string())
        })?;
        let settings = st.load_settings().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if let Err(e) = st.paypal.reconfigure(&settings) {
            st.events.log("error", format!("PayPal middleware not rebuilt: {e}"));
        }
        let config = config_json(&settings);
        *st.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
        Ok(config)
    })
    .await;
    match result {
        Ok(Ok(config)) => {
            state.events.send("config.changed", json!({"config": config}));
            state.events.log("info", format!("configuration updated: {} (values are never shown)", names.join(", ")));
            Json(config).into_response()
        }
        Ok(Err((status, message))) => error(status, message),
        Err(r) => r,
    }
}

#[derive(serde::Deserialize)]
struct TestBody {
    service: String,
}

async fn post_config_test(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let body: TestBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    if !["nebius", "tavily", "paypal"].contains(&body.service.as_str()) {
        return error(StatusCode::BAD_REQUEST, format!("unknown service {}", clean_text(&body.service, 32)));
    }
    let settings = state.settings();
    match blocking(move || test_service(&settings, &body.service)).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

// ---------------------------------------------------------------------------------------------
// Workspace and approvals.

async fn get_workspace(State(state): State<Arc<AppState>>) -> Response {
    match blocking(move || state.workspace.workspace_json()).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => r,
    }
}

#[derive(serde::Deserialize)]
struct RunBody {
    scenario: String,
}

async fn post_run(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let body: RunBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(scenario) = Scenario::parse(&body.scenario) else {
        return error(StatusCode::BAD_REQUEST, format!("unknown scenario {}", clean_text(&body.scenario, 64)));
    };
    let run_id = state.next_run_id();
    let id = run_id.clone();
    tokio::task::spawn_blocking(move || workspace::run_scenario(&state, &id, scenario));
    Json(json!({"run_id": run_id})).into_response()
}

async fn post_reset(State(state): State<Arc<AppState>>) -> Response {
    let st = state.clone();
    match blocking(move || workspace::reset(&st).map(|_| st.workspace.workspace_json())).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
        Err(r) => r,
    }
}

async fn get_approvals(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(lock(&state.approvals).list_json())
}

#[derive(serde::Deserialize, Default)]
struct ResolveBody {
    #[serde(default)]
    by: String,
}

async fn resolve(state: Arc<AppState>, id: String, body: Bytes, approve: bool) -> Response {
    let body: ResolveBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let by = clean_text(&body.by, 64);
    if by.is_empty() {
        return error(StatusCode::BAD_REQUEST, "by must name the approver");
    }
    let id = clean_text(&id, 32);
    match blocking(move || workspace::resolve_approval(&state, &id, approve, &by)).await {
        Ok(Ok(approval)) => Json(approval).into_response(),
        Ok(Err(ResolveError::NotFound(m))) => error(StatusCode::NOT_FOUND, m),
        Ok(Err(ResolveError::Conflict(m))) => error(StatusCode::CONFLICT, m),
        Ok(Err(ResolveError::Internal(m))) => error(StatusCode::INTERNAL_SERVER_ERROR, m),
        Err(r) => r,
    }
}

async fn post_approve(State(state): State<Arc<AppState>>, Path(id): Path<String>, body: Bytes) -> Response {
    resolve(state, id, body, true).await
}

async fn post_reject(State(state): State<Arc<AppState>>, Path(id): Path<String>, body: Bytes) -> Response {
    resolve(state, id, body, false).await
}

// ---------------------------------------------------------------------------------------------
// PayPal.

async fn get_orders(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.paypal.orders_json())
}

async fn get_paypal_scenarios(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(state.paypal.scenarios_json())
}

/// Prepares the order (fast, local), answers with its id, and finishes submission and the
/// `paypal.order` broadcast in the background.
pub async fn place_order(state: Arc<AppState>, scenario: OrderScenario) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || match state.paypal.prepare(scenario) {
        Ok(prepared) => {
            state.keepwarm.touch(Model::PayPal);
            let _ = tx.send(Ok(prepared.order_id.clone()));
            state.paypal.complete(&state, prepared);
        }
        Err(e) => {
            state.events.log("error", format!("order {} failed: {e}", scenario.id()));
            let _ = tx.send(Err(e));
        }
    });
    rx.await.map_err(|e| format!("order worker failed: {e}"))?
}

#[derive(serde::Deserialize)]
struct OrderBody {
    scenario: String,
}

async fn post_order(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let body: OrderBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let Some(scenario) = OrderScenario::parse(&body.scenario) else {
        return error(StatusCode::BAD_REQUEST, format!("unknown scenario {}", clean_text(&body.scenario, 64)));
    };
    match place_order(state, scenario).await {
        Ok(order_id) => Json(json!({"order_id": order_id})).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

#[derive(serde::Deserialize)]
struct StreamBody {
    enabled: bool,
    interval_ms: Option<f64>,
}

async fn post_stream(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let body: StreamBody = match parse(&body) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let current = state.paypal.stream_state();
    let requested = body.interval_ms.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.round() as u64).unwrap_or(current.interval_ms);
    let (stream, epoch) = state.paypal.set_stream(body.enabled, requested);
    if stream.enabled {
        tokio::spawn(stream_loop(state.clone(), epoch, stream.interval_ms));
    }
    state.events.log(
        "info",
        format!("PayPal order stream {} (interval {} ms)", if stream.enabled { "on" } else { "off" }, stream.interval_ms),
    );
    Json(json!(stream)).into_response()
}

/// Places one simulated order per interval, cycling through the scenarios, until the stream
/// settings change or the daemon shuts down.
async fn stream_loop(state: Arc<AppState>, epoch: u64, interval_ms: u64) {
    let mut shutdown = state.shutdown.subscribe();
    let mut next = 0usize;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
            _ = shutdown.changed() => return,
        }
        if state.paypal.stream_epoch.load(Ordering::SeqCst) != epoch || *shutdown.borrow() {
            return;
        }
        let scenario = OrderScenario::ALL[next % OrderScenario::ALL.len()];
        next += 1;
        if let Err(e) = place_order(state.clone(), scenario).await {
            state.events.log("error", format!("stream order failed: {e}"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// WebSocket.

async fn ws_upgrade(State(state): State<Arc<AppState>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| client(state, socket))
}

async fn client(state: Arc<AppState>, mut socket: WebSocket) {
    let mut rx = state.events.subscribe();
    let mut shutdown = state.shutdown.subscribe();
    let hello = Events::message("hello", json!({"health": state.health_json(), "live": state.live_json()}));
    if socket.send(Message::Text(hello.as_ref().into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    if socket.send(Message::Text(text.as_ref().into())).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    let note = Events::message("log", json!({"level": "warn", "message": format!("this client fell behind; {skipped} messages were dropped")}));
                    if socket.send(Message::Text(note.as_ref().into())).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
            _ = shutdown.changed() => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Background tasks.

/// Sleeps until the next model is due (idle for the keep-warm interval) and pings it.
async fn keepwarm_loop(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.subscribe();
    loop {
        let Some(wait) = Model::ALL.iter().filter_map(|m| state.keepwarm.due_in(*m)).min() else { return };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = shutdown.changed() => return,
        }
        if *shutdown.borrow() {
            return;
        }
        for model in Model::ALL {
            if state.keepwarm.due_in(model) != Some(Duration::ZERO) {
                continue;
            }
            let st = state.clone();
            match tokio::task::spawn_blocking(move || st.keepwarm.ping(model)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => state.events.log("warn", format!("ANE keep-warm ping ({}) failed: {e}", model.name())),
                Err(e) => state.events.log("warn", format!("ANE keep-warm worker failed: {e}")),
            }
        }
    }
}

/// `telemetry.live` about once per second, reaper polling every 100 ms, and the CoreML keep-warm
/// when it is enabled.
pub fn spawn_background(state: &Arc<AppState>) {
    if state.keepwarm.interval().is_some() {
        tokio::spawn(keepwarm_loop(state.clone()));
    }
    let st = state.clone();
    tokio::spawn(async move {
        let mut shutdown = st.shutdown.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => st.events.send("telemetry.live", json!({"live": st.live_json()})),
                _ = shutdown.changed() => return,
            }
        }
    });
    let st = state.clone();
    tokio::spawn(async move {
        let mut shutdown = st.shutdown.subscribe();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = tick.tick() => workspace::poll_reaps(&st),
                _ = shutdown.changed() => return,
            }
        }
    });
}
