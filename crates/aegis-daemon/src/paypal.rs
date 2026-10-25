//! Panel 2: simulated agent purchase intents processed by the real `adapter_paypal` pipeline
//! (redaction, CoreML score, Cedar budget rules, ed25519 receipt sign + verify), with PayPal
//! Sandbox submission only when `PAYPAL_CLIENT_ID` and `PAYPAL_CLIENT_SECRET` are configured, at
//! most `sandbox_cap` times per daemon run.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use adapter_paypal::{aggrid_export, telemetry_row, LineItem, OrderIntent, PayPalMiddleware, PreparedOrder, Submission, TelemetryRow};
use aegis_ane_scorer::AneScorer;
use aegis_config::Settings;
use aegis_governance::{GovernanceEngine, ReceiptSigner};
use serde_json::{json, Value};

use crate::services::{paypal_config, scrub, PAYPAL_AGENT};
use crate::state::{lock, AppState, DaemonOptions};
use crate::util::iso_ms;

const ROWS_KEPT: usize = 500;
const EVENTS_KEPT: usize = 3000;
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

struct Middlewares {
    redacting: PayPalMiddleware,
    unredacted: PayPalMiddleware,
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
    mw: RwLock<Arc<Middlewares>>,
    store: Mutex<Store>,
    seq: AtomicU64,
    submitted: AtomicU32,
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

fn build(settings: &Settings, seed: &std::path::Path, gov: &Arc<GovernanceEngine>, scorer: &Arc<AneScorer>) -> Result<Middlewares, String> {
    let signer = || ReceiptSigner::load_or_create(seed).map_err(|e| format!("receipt signing key {}: {e}", seed.display()));
    Ok(Middlewares {
        redacting: PayPalMiddleware::new(paypal_config(settings, true), gov.clone(), scorer.clone(), signer()?),
        unredacted: PayPalMiddleware::new(paypal_config(settings, false), gov.clone(), scorer.clone(), signer()?),
    })
}

impl PayPalEngine {
    pub fn new(opts: &DaemonOptions, settings: &Settings, gov: Arc<GovernanceEngine>, scorer: Arc<AneScorer>) -> Result<Self, String> {
        let mw = build(settings, &opts.signing_seed, &gov, &scorer)?;
        Ok(Self {
            gov,
            scorer,
            seed: opts.signing_seed.clone(),
            mw: RwLock::new(Arc::new(mw)),
            store: Mutex::new(Store::default()),
            seq: AtomicU64::new(0),
            submitted: AtomicU32::new(0),
            cap: opts.sandbox_cap,
            min_interval_ms: opts.min_stream_interval_ms,
            stream: Mutex::new(StreamState { enabled: false, interval_ms: DEFAULT_STREAM_INTERVAL_MS.max(opts.min_stream_interval_ms) }),
            stream_epoch: AtomicU64::new(0),
        })
    }

    /// Rebuilds the middlewares after a configuration change (credentials are read at build time).
    pub fn reconfigure(&self, settings: &Settings) -> Result<(), String> {
        let mw = build(settings, &self.seed, &self.gov, &self.scorer)?;
        *self.mw.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(mw);
        Ok(())
    }

    fn middlewares(&self) -> Arc<Middlewares> {
        self.mw.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn has_credentials(&self) -> bool {
        self.middlewares().redacting.config().has_credentials()
    }

    pub fn scenarios_json(&self) -> Value {
        let scenarios: Vec<Value> =
            OrderScenario::ALL.iter().map(|s| json!({"id": s.id(), "title": s.title(), "description": s.description()})).collect();
        json!({
            "scenarios": scenarios,
            "stream": self.stream_state(),
            "sandbox": {"enabled": self.has_credentials(), "submitted": self.submitted.load(Ordering::Relaxed), "cap": self.cap},
        })
    }

    pub fn orders_json(&self) -> Value {
        let store = lock(&self.store);
        let rows: Vec<&TelemetryRow> = store.rows.iter().collect();
        let resources: Vec<Value> = RESOURCES.iter().map(|r| json!({"id": r, "name": r})).collect();
        json!({
            "rows": rows,
            "columnDefs": aggrid_export(&[])["columnDefs"],
            "timeline": {"resources": resources, "events": store.events},
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

    /// Submission (Sandbox only when configured and under the cap), the dashboard row and its
    /// timeline events. Blocking (may call the Sandbox).
    pub fn complete(&self, state: &AppState, p: Prepared) -> (TelemetryRow, Vec<Value>) {
        let middleware = if p.scenario.redacting() { &p.mw.redacting } else { &p.mw.unredacted };
        let mut submit_span: Option<(Duration, bool)> = None;
        let mut error: Option<String> = None;
        let submission = if !p.prepared.decision.allowed {
            middleware.submit(&p.prepared).unwrap_or_else(|e| Submission::Blocked { reason: e.to_string() })
        } else if !middleware.config().has_credentials() {
            Submission::NotSent { reason: "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set".to_string() }
        } else if !self.reserve_submission() {
            Submission::NotSent { reason: format!("Sandbox submission cap reached ({} per daemon run)", self.cap) }
        } else {
            let started = Instant::now();
            match middleware.submit(&p.prepared) {
                Ok(s) => {
                    submit_span = Some((started.elapsed(), true));
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
            while store.rows.len() > ROWS_KEPT {
                store.rows.pop_front();
            }
            store.events.extend(events.iter().cloned());
            while store.events.len() > EVENTS_KEPT {
                store.events.pop_front();
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
/// then submit) from the row's measured durations. Untimed glue between stages is not drawn.
pub fn timeline_events(row: &TelemetryRow, started_at: SystemTime, submit: Option<(Duration, bool)>) -> Vec<Value> {
    let blocked = row.decision != "Allow";
    let mut spans: Vec<(&str, f64, &str)> = vec![
        ("redact", row.redact_us, "ok"),
        ("sign", row.sign_us, "ok"),
        ("verify", row.verify_us, if row.receipt_verified { "ok" } else { "error" }),
        ("score", row.score_us, "ok"),
        ("policy", row.cedar_us, if blocked { "blocked" } else { "ok" }),
    ];
    if let Some((d, ok)) = submit {
        spans.push(("submit", d.as_secs_f64() * 1e6, if ok { "ok" } else { "error" }));
    }
    let mut offset_us = 0.0;
    spans
        .into_iter()
        .map(|(resource, duration_us, cls)| {
            let start = started_at + Duration::from_secs_f64(offset_us / 1e6);
            let end = started_at + Duration::from_secs_f64((offset_us + duration_us) / 1e6);
            let event = json!({
                "id": format!("{}:{resource}", row.id),
                "resourceId": resource,
                "name": format!("{} {resource}", row.reference),
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
