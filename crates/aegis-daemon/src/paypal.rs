//! Panel 2: simulated agent purchase intents processed by the real `adapter_paypal` pipeline
//! (redaction, CoreML score, Cedar budget rules, ed25519 receipt sign + verify). Cedar-allowed
//! orders are then submitted according to [`SandboxMode`]:
//!
//! * `local` — `PAYPAL_CLIENT_ID` and `PAYPAL_CLIENT_SECRET` unset, or both the literal
//!   `local-sandbox`, and this process serves `/paypal-sandbox`: OAuth2 → create → capture over HTTP
//!   against the daemon's own local Orders v2 emulation ([`crate::paypal_sandbox`]), uncapped (the
//!   stream interval clamp bounds the rate). Nothing is sent to PayPal.
//! * `remote` — real credentials: the PayPal Sandbox, unchanged, at most `sandbox_cap` per run.
//! * `disabled` — anything else (one credential missing, a mix of `local-sandbox` and a real value,
//!   or no local gateway in this process); nothing is sent.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use adapter_paypal::{
    aggrid_export, telemetry_row, LineItem, OrderIntent, PayPalConfig, PayPalMiddleware, PreparedOrder, Submission, TelemetryRow,
    LOCAL_SANDBOX_CREDENTIAL, SANDBOX_BASE_URL,
};
use aegis_ane_scorer::AneScorer;
use aegis_config::Settings;
use aegis_governance::{GovernanceEngine, ReceiptSigner};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::paypal_sandbox::LocalGateway;
use crate::services::{paypal_config, scrub, PAYPAL_AGENT};
use crate::state::{local_gateway_url, lock, AppState, DaemonOptions};
use crate::util::{iso_ms, round6};

/// Order rows kept for `GET /api/paypal/orders` (newest; their timeline events are kept with them).
pub const ORDERS_KEPT: usize = 20;
pub const DEFAULT_STREAM_INTERVAL_MS: u64 = 2000;
pub const MAX_STREAM_INTERVAL_MS: u64 = 600_000;
pub const RESOURCES: [&str; 6] = ["redact", "score", "policy", "sign", "verify", "submit"];

/// The four phase-1 benchmark order cases, as simulated agent intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderScenario {
    KnownPayee42,
    KnownPayee120,
    NewPayee480,
    RedactionDisabled25,
}

impl OrderScenario {
    pub const ALL: [OrderScenario; 4] =
        [OrderScenario::KnownPayee42, OrderScenario::KnownPayee120, OrderScenario::NewPayee480, OrderScenario::RedactionDisabled25];

    pub fn id(self) -> &'static str {
        match self {
            OrderScenario::KnownPayee42 => "known_payee_42",
            OrderScenario::KnownPayee120 => "known_payee_120",
            OrderScenario::NewPayee480 => "new_payee_480",
            OrderScenario::RedactionDisabled25 => "redaction_disabled_25",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            OrderScenario::KnownPayee42 => "$42.00 known payee",
            OrderScenario::KnownPayee120 => "$120.00 known payee",
            OrderScenario::NewPayee480 => "$480.00 new payee, high-risk country",
            OrderScenario::RedactionDisabled25 => "$25.00 with redaction disabled",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            OrderScenario::KnownPayee42 => "Simulated agent purchase intent: 2 × Thunderbolt 4 cable at $21.00 from a known merchant; the customer note carries an e-mail address and phone number that are redacted before signing.",
            OrderScenario::KnownPayee120 => "Simulated agent purchase intent: 2 × $60.00 from the same known merchant; above $50, so Cedar needs the verified ed25519 receipt.",
            OrderScenario::NewPayee480 => "Simulated agent purchase intent: 8 × $60.00 to a new payee in a high-risk country.",
            OrderScenario::RedactionDisabled25 => "Simulated agent purchase intent through a middleware with redaction switched off: a card number stays unformatted in the payload.",
        }
    }

    fn redacting(self) -> bool {
        self != OrderScenario::RedactionDisabled25
    }

    /// Same parameters as `aegis-bench`'s PayPal cases.
    pub fn intent(self, reference: &str) -> OrderIntent {
        let (quantity, unit_cents, note, new_payee, country_risk) = match self {
            OrderScenario::KnownPayee42 => (2, 2100, "Deliver to jane.doe@example.com, call +1 (415) 555-0132", false, 0.05),
            OrderScenario::KnownPayee120 => (2, 6000, "Ship with the next batch", false, 0.05),
            OrderScenario::NewPayee480 => (8, 6000, "Urgent", true, 0.9),
            OrderScenario::RedactionDisabled25 => (1, 2500, "Card 4111 1111 1111 1111", false, 0.05),
        };
        OrderIntent {
            reference: reference.to_string(),
            merchant_id: "sandbox-lab-supplies".to_string(),
            currency: "USD".to_string(),
            description: "Build-lab hardware for CI runners".to_string(),
            customer_note: note.to_string(),
            items: vec![LineItem { name: "Thunderbolt 4 cable".into(), quantity, unit_amount_cents: unit_cents, sku: Some("TB4-1M".into()) }],
            new_payee,
            payee_country_risk: country_risk,
            customer_pii_fields: 1,
            team_approval_granted: false,
        }
    }
}

/// Where Cedar-allowed orders are submitted (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    Local,
    Remote,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRoute {
    pub mode: SandboxMode,
    pub reason: String,
    pub base_url: String,
}

fn setting(settings: &Settings, name: &str) -> Option<String> {
    settings.get(name).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

/// The route the PayPal credentials select. `local_gateway` is the base URL of this process's
/// `/paypal-sandbox` listener, if it serves one.
pub fn sandbox_route(settings: &Settings, local_gateway: Option<&str>) -> SandboxRoute {
    let id = setting(settings, "PAYPAL_CLIENT_ID");
    let secret = setting(settings, "PAYPAL_CLIENT_SECRET");
    let local = |v: &Option<String>| v.as_deref() == Some(LOCAL_SANDBOX_CREDENTIAL);
    let disabled = |reason: String| SandboxRoute { mode: SandboxMode::Disabled, reason, base_url: SANDBOX_BASE_URL.to_string() };
    let local_reason = match (&id, &secret) {
        (None, None) => "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set".to_string(),
        (Some(_), Some(_)) if local(&id) && local(&secret) => format!("PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET = {LOCAL_SANDBOX_CREDENTIAL}"),
        (Some(_), Some(_)) if local(&id) || local(&secret) => {
            return disabled(format!(
                "set both PAYPAL_CLIENT_ID and PAYPAL_CLIENT_SECRET to {LOCAL_SANDBOX_CREDENTIAL} (local gateway), or both to PayPal Sandbox credentials"
            ))
        }
        (Some(_), Some(_)) => {
            let sources: Vec<String> = ["PAYPAL_CLIENT_ID", "PAYPAL_CLIENT_SECRET"]
                .iter()
                .map(|n| format!("{n} set ({})", settings.source(n).map(|s| s.name()).unwrap_or("unknown")))
                .collect();
            return SandboxRoute { mode: SandboxMode::Remote, reason: sources.join(", "), base_url: SANDBOX_BASE_URL.to_string() };
        }
        (None, Some(_)) => return disabled("PAYPAL_CLIENT_ID not set".to_string()),
        (Some(_), None) => return disabled("PAYPAL_CLIENT_SECRET not set".to_string()),
    };
    match local_gateway {
        Some(base) => SandboxRoute {
            mode: SandboxMode::Local,
            reason: format!("{local_reason}: orders go to the daemon's local Orders v2 gateway (an emulation; nothing is sent to PayPal)"),
            base_url: base.to_string(),
        },
        None => disabled(format!("{local_reason}; this daemon instance does not serve the local Orders v2 gateway")),
    }
}

struct Middlewares {
    redacting: PayPalMiddleware,
    unredacted: PayPalMiddleware,
    route: SandboxRoute,
}

#[derive(Default)]
struct Store {
    rows: VecDeque<TelemetryRow>,
    events: VecDeque<Value>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct StreamState {
    pub enabled: bool,
    pub interval_ms: u64,
}

pub struct PayPalEngine {
    gov: Arc<GovernanceEngine>,
    scorer: Arc<AneScorer>,
    seed: std::path::PathBuf,
    /// Base URL of this process's `/paypal-sandbox`, when it serves one.
    local_gateway: Option<String>,
    gateway: Arc<LocalGateway>,
    mw: RwLock<Arc<Middlewares>>,
    store: Mutex<Store>,
    seq: AtomicU64,
    /// Remote Sandbox submissions this run (counted against `cap`).
    submitted: AtomicU32,
    /// Orders created and captured through the local gateway this run (no cap).
    local_submitted: AtomicU64,
    cap: u32,
    pub min_interval_ms: u64,
    stream: Mutex<StreamState>,
    /// Bumped whenever the stream settings change; a running stream loop exits when it differs.
    pub stream_epoch: AtomicU64,
}

/// A prepared order waiting for submission.
pub struct Prepared {
    pub order_id: String,
    prepared: PreparedOrder,
    scenario: OrderScenario,
    mw: Arc<Middlewares>,
    started_at: SystemTime,
}

/// The middleware config for `route`: the remote config unchanged, the local gateway with the
/// local credentials, or no credentials at all.
fn route_config(settings: &Settings, route: &SandboxRoute, redact: bool) -> PayPalConfig {
    let remote = paypal_config(settings, redact);
    match route.mode {
        SandboxMode::Remote => remote,
        SandboxMode::Local => PayPalConfig {
            client_id: Some(LOCAL_SANDBOX_CREDENTIAL.to_string()),
            client_secret: Some(LOCAL_SANDBOX_CREDENTIAL.to_string()),
            base_url: route.base_url.clone(),
            ..remote
        },
        SandboxMode::Disabled => PayPalConfig { client_id: None, client_secret: None, ..remote },
    }
}

fn build(
    settings: &Settings,
    seed: &std::path::Path,
    gov: &Arc<GovernanceEngine>,
    scorer: &Arc<AneScorer>,
    local_gateway: Option<&str>,
) -> Result<Middlewares, String> {
    let signer = || ReceiptSigner::load_or_create(seed).map_err(|e| format!("receipt signing key {}: {e}", seed.display()));
    let route = sandbox_route(settings, local_gateway);
    Ok(Middlewares {
        redacting: PayPalMiddleware::new(route_config(settings, &route, true), gov.clone(), scorer.clone(), signer()?),
        unredacted: PayPalMiddleware::new(route_config(settings, &route, false), gov.clone(), scorer.clone(), signer()?),
        route,
    })
}

impl PayPalEngine {
    pub fn new(
        opts: &DaemonOptions,
        settings: &Settings,
        gov: Arc<GovernanceEngine>,
        scorer: Arc<AneScorer>,
        gateway: Arc<LocalGateway>,
    ) -> Result<Self, String> {
        let local_gateway = opts.local_paypal_sandbox.then(|| local_gateway_url(opts.port));
        let mw = build(settings, &opts.signing_seed, &gov, &scorer, local_gateway.as_deref())?;
        Ok(Self {
            gov,
            scorer,
            seed: opts.signing_seed.clone(),
            local_gateway,
            gateway,
            mw: RwLock::new(Arc::new(mw)),
            store: Mutex::new(Store::default()),
            seq: AtomicU64::new(0),
            submitted: AtomicU32::new(0),
            local_submitted: AtomicU64::new(0),
            cap: opts.sandbox_cap,
            min_interval_ms: opts.min_stream_interval_ms,
            stream: Mutex::new(StreamState { enabled: false, interval_ms: DEFAULT_STREAM_INTERVAL_MS.max(opts.min_stream_interval_ms) }),
            stream_epoch: AtomicU64::new(0),
        })
    }

    /// Rebuilds the middlewares after a configuration change (credentials are read at build time).
    pub fn reconfigure(&self, settings: &Settings) -> Result<(), String> {
        let mw = build(settings, &self.seed, &self.gov, &self.scorer, self.local_gateway.as_deref())?;
        *self.mw.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(mw);
        Ok(())
    }

    fn middlewares(&self) -> Arc<Middlewares> {
        self.mw.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn route(&self) -> SandboxRoute {
        self.middlewares().route.clone()
    }

    pub fn mode(&self) -> SandboxMode {
        self.middlewares().route.mode
    }

    /// `GET /api/paypal/scenarios` `sandbox` and `GET /api/config` `services.paypal`: the mode, why,
    /// where orders go, and the counts (`submitted` / `cap`: remote Sandbox; `local_submitted`:
    /// local gateway orders created and captured; `gateway`: the local gateway's own counters in
    /// local mode, else null).
    pub fn service_json(&self) -> Value {
        let route = self.route();
        json!({
            "enabled": route.mode != SandboxMode::Disabled,
            "mode": route.mode,
            "reason": route.reason,
            "base_url": route.base_url,
            "submitted": self.submitted.load(Ordering::Relaxed),
            "cap": self.cap,
            "local_submitted": self.local_submitted.load(Ordering::Relaxed),
            "gateway": if route.mode == SandboxMode::Local { self.gateway.stats_json() } else { Value::Null },
        })
    }

    pub fn scenarios_json(&self) -> Value {
        let scenarios: Vec<Value> =
            OrderScenario::ALL.iter().map(|s| json!({"id": s.id(), "title": s.title(), "description": s.description()})).collect();
        json!({
            "scenarios": scenarios,
            "stream": self.stream_state(),
            "sandbox": self.service_json(),
        })
    }

    /// `POST /api/config/test` for `paypal` in local mode: one OAuth2 client-credentials call to the
    /// local gateway. Blocking.
    pub fn test_local(&self) -> Value {
        let route = self.route();
        if route.mode != SandboxMode::Local {
            return json!({"service": "paypal", "ok": false, "latency_ms": null, "detail": "not configured"});
        }
        let url = format!("{}/v1/oauth2/token", route.base_url.trim_end_matches('/'));
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
        let started = Instant::now();
        let response = agent
            .post(&url)
            .set("Authorization", &format!("Basic {}", BASE64.encode(format!("{LOCAL_SANDBOX_CREDENTIAL}:{LOCAL_SANDBOX_CREDENTIAL}"))))
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string("grant_type=client_credentials");
        let (ok, detail) = match response {
            Ok(r) => {
                let body = r.into_json::<Value>().unwrap_or(Value::Null);
                if body["access_token"].is_string() && body["aegis_local_sandbox"] == true {
                    let expires = body["expires_in"].as_u64().map(|s| format!(", expires in {s} s")).unwrap_or_default();
                    (true, format!("local Orders v2 gateway {url}: OAuth2 client-credentials token issued{expires}; nothing is sent to PayPal"))
                } else {
                    (false, format!("{url} answered without a local-sandbox access_token"))
                }
            }
            Err(ureq::Error::Status(status, _)) => (false, format!("{url}: HTTP {status}")),
            Err(ureq::Error::Transport(t)) => (false, format!("{url}: transport error: {t}")),
        };
        json!({"service": "paypal", "ok": ok, "latency_ms": round6(started.elapsed().as_secs_f64() * 1e3), "detail": detail})
    }

    pub fn orders_json(&self) -> Value {
        let store = lock(&self.store);
        let rows: Vec<&TelemetryRow> = store.rows.iter().collect();
        let resources: Vec<Value> = RESOURCES.iter().map(|r| json!({"id": r, "name": r})).collect();
        json!({
            "rows": rows,
            "columnDefs": aggrid_export(&[])["columnDefs"],
            "timeline": {"resources": resources, "events": store.events},
            "limit": ORDERS_KEPT,
        })
    }

    pub fn stream_state(&self) -> StreamState {
        *lock(&self.stream)
    }

    /// Applies new stream settings (interval clamped to `[min_interval_ms, MAX]`) and returns them.
    pub fn set_stream(&self, enabled: bool, interval_ms: u64) -> (StreamState, u64) {
        let state = StreamState { enabled, interval_ms: interval_ms.clamp(self.min_interval_ms, MAX_STREAM_INTERVAL_MS) };
        *lock(&self.stream) = state;
        let epoch = self.stream_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        (state, epoch)
    }

    /// Redaction, scoring, Cedar and receipt signing + verification. Blocking.
    pub fn prepare(&self, scenario: OrderScenario) -> Result<Prepared, String> {
        let mw = self.middlewares();
        let n = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let reference = format!("agent-{n:04}");
        let intent = scenario.intent(&reference);
        let started_at = SystemTime::now();
        let middleware = if scenario.redacting() { &mw.redacting } else { &mw.unredacted };
        let prepared = middleware.prepare(&intent).map_err(|e| e.to_string())?;
        let order_id = telemetry_row(&prepared, &Submission::NotSent { reason: String::new() }, PAYPAL_AGENT).id;
        Ok(Prepared { order_id, prepared, scenario, mw, started_at })
    }

    /// Submission (local gateway: OAuth2 → create → capture; remote Sandbox: under the cap), the
    /// dashboard row and its timeline events. Blocking (calls the gateway or the Sandbox).
    pub fn complete(&self, state: &AppState, p: Prepared) -> (TelemetryRow, Vec<Value>) {
        let middleware = if p.scenario.redacting() { &p.mw.redacting } else { &p.mw.unredacted };
        let route = &p.mw.route;
        let mut submit_span: Option<(Duration, bool)> = None;
        let mut error: Option<String> = None;
        let submission = if !p.prepared.decision.allowed {
            middleware.submit(&p.prepared).unwrap_or_else(|e| Submission::Blocked { reason: e.to_string() })
        } else if route.mode == SandboxMode::Disabled || !middleware.config().has_credentials() {
            Submission::NotSent { reason: route.reason.clone() }
        } else if route.mode == SandboxMode::Remote && !self.reserve_submission() {
            Submission::NotSent { reason: format!("Sandbox submission cap reached ({} per daemon run)", self.cap) }
        } else {
            let started = Instant::now();
            let result =
                if route.mode == SandboxMode::Local { middleware.submit_and_capture(&p.prepared) } else { middleware.submit(&p.prepared) };
            match result {
                Ok(s) => {
                    submit_span = Some((started.elapsed(), true));
                    if route.mode == SandboxMode::Local && matches!(s, Submission::Sent { .. }) {
                        self.local_submitted.fetch_add(1, Ordering::Relaxed);
                    }
                    s
                }
                Err(e) => {
                    submit_span = Some((started.elapsed(), false));
                    let text = scrub(&state.settings(), &e.to_string());
                    error = Some(text.clone());
                    Submission::NotSent { reason: text }
                }
            }
        };
        let mut row = telemetry_row(&p.prepared, &submission, PAYPAL_AGENT);
        if let Some(text) = error {
            row.submission = "error".to_string();
            row.submission_detail = text;
        }
        let events = timeline_events(&row, p.started_at, submit_span);
        {
            let mut live = lock(&state.live);
            live.record("paypal_total_us", row.total_us);
            live.record("paypal_verify_us", row.verify_us);
            live.counters.orders += 1;
            if row.decision != "Allow" {
                live.counters.orders_blocked += 1;
            }
            if row.submission == "sent" {
                live.counters.orders_sent += 1;
            }
        }
        {
            let mut store = lock(&self.store);
            store.rows.push_back(row.clone());
            store.events.extend(events.iter().cloned());
            while store.rows.len() > ORDERS_KEPT {
                if let Some(old) = store.rows.pop_front() {
                    store.events.retain(|e| e["orderId"] != old.id.as_str());
                }
            }
        }
        state.events.send("paypal.order", json!({"row": row, "events": events}));
        (row, events)
    }

    /// Takes one of the per-run Sandbox submission slots; false once the cap is reached.
    fn reserve_submission(&self) -> bool {
        let mut current = self.submitted.load(Ordering::SeqCst);
        loop {
            if current >= self.cap {
                return false;
            }
            match self.submitted.compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    pub fn row_by_id(&self, id: &str) -> Option<TelemetryRow> {
        lock(&self.store).rows.iter().find(|r| r.id == id).cloned()
    }

    /// Re-verifies a prepared order's detached receipt (used by tests).
    pub fn reverify(p: &Prepared) -> bool {
        p.prepared.verify().is_ok()
    }

    pub fn decision(p: &Prepared) -> (bool, Vec<String>) {
        (p.prepared.decision.allowed, p.prepared.decision.determining_policies.clone())
    }
}

/// One span per pipeline stage, laid out in execution order (redact, sign, verify, score, policy,
/// then submit) from the row's measured durations. A submission that made HTTP calls gets one
/// `submit` span per call (`oauth`, `create`, `capture`) from `row.submit_calls`; a failed one gets a
/// single `submit` span over the whole attempt. Untimed glue between stages is not drawn.
pub fn timeline_events(row: &TelemetryRow, started_at: SystemTime, submit: Option<(Duration, bool)>) -> Vec<Value> {
    let blocked = row.decision != "Allow";
    let mut spans: Vec<(&str, f64, &str, Option<&str>)> = vec![
        ("redact", row.redact_us, "ok", None),
        ("sign", row.sign_us, "ok", None),
        ("verify", row.verify_us, if row.receipt_verified { "ok" } else { "error" }, None),
        ("score", row.score_us, "ok", None),
        ("policy", row.cedar_us, if blocked { "blocked" } else { "ok" }, None),
    ];
    if !row.submit_calls.is_empty() {
        for call in &row.submit_calls {
            spans.push(("submit", call.duration_us, if call.status < 400 { "ok" } else { "error" }, Some(call.step.as_str())));
        }
    } else if let Some((d, ok)) = submit {
        spans.push(("submit", d.as_secs_f64() * 1e6, if ok { "ok" } else { "error" }, None));
    }
    let mut offset_us = 0.0;
    spans
        .into_iter()
        .map(|(resource, duration_us, cls, step)| {
            let start = started_at + Duration::from_secs_f64(offset_us / 1e6);
            let end = started_at + Duration::from_secs_f64((offset_us + duration_us) / 1e6);
            let (id, name) = match step {
                Some(step) => (format!("{}:{resource}:{step}", row.id), format!("{} {resource} {step}", row.reference)),
                None => (format!("{}:{resource}", row.id), format!("{} {resource}", row.reference)),
            };
            let event = json!({
                "id": id,
                "resourceId": resource,
                "name": name,
                "startDate": iso_ms(start),
                "endDate": iso_ms(end),
                "durationUs": (duration_us * 100.0).round() / 100.0,
                "startOffsetUs": (offset_us * 100.0).round() / 100.0,
                "orderId": row.id,
                "cls": cls,
            });
            offset_us += duration_us;
            event
        })
        .collect()
}
