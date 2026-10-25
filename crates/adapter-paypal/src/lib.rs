//! PayPal Orders v2 middleware for autonomous procurement agents.
//!
//! [`PayPalMiddleware::prepare`] builds the Orders v2 body, formats sensitive customer strings as
//! `[REDACTED]`, signs the canonical body with Ed25519 (receipt checked immediately), scores the
//! order with the CoreML model and evaluates Cedar's `paypal::create_order` rules. The signature is
//! attached as `purchase_units[0].custom_id` (`c2pa:ed25519:...`, detached: it covers the body
//! without `custom_id`).
//!
//! [`PayPalMiddleware::submit`] sends nothing unless Cedar allowed the order **and** Sandbox
//! credentials (`PAYPAL_CLIENT_ID`, `PAYPAL_CLIENT_SECRET`) are present: it then fetches an OAuth2
//! client-credentials token and POSTs `/v2/checkout/orders` with `PayPal-Request-Id` = payload digest.
//! [`PayPalMiddleware::submit_and_capture`] also POSTs `/v2/checkout/orders/{id}/capture`; it is
//! meant for aegis-daemon's local Orders v2 gateway (`/paypal-sandbox`), where payer approval is
//! implied. The literal [`LOCAL_SANDBOX_CREDENTIAL`] credentials are only ever sent to a loopback
//! `http://` base URL, so they can never reach PayPal. Every HTTP call is timed ([`HttpCall`]).
//!
//! Each order yields a [`TelemetryRow`]; [`aggrid_export`] and [`bryntum_export`] wrap rows in the
//! shapes AG Grid (`columnDefs` + `rowData`) and Bryntum stores (`{success, data}`) load directly.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aegis_ane_scorer::{commerce_features, AneScorer, CommerceContext};
use aegis_governance::receipt::{verify_json, ReceiptError};
use aegis_governance::time::utc_timestamp;
use aegis_governance::{risk_to_context, scanner, Action, AuthzRequest, Decision, GovernanceEngine, PolicyContext, Receipt, ReceiptSigner};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};

pub const SANDBOX_BASE_URL: &str = "https://api-m.sandbox.paypal.com";
/// Client id and secret of aegis-daemon's local Orders v2 gateway. Setting both
/// `PAYPAL_CLIENT_ID` and `PAYPAL_CLIENT_SECRET` to this value selects that gateway.
pub const LOCAL_SANDBOX_CREDENTIAL: &str = "local-sandbox";
pub const DEFAULT_BUDGET_CENTS: u64 = 20_000;
/// PayPal's limit for free-text fields such as `description`, item `name` and `custom_id`.
const PAYPAL_TEXT_LIMIT: usize = 127;

#[derive(Debug, thiserror::Error)]
pub enum PayPalError {
    #[error(transparent)]
    Governance(#[from] aegis_governance::GovernanceError),
    #[error(transparent)]
    Ane(#[from] aegis_ane_scorer::AneError),
    #[error("HTTP {status} from {url}: {body}")]
    Http { status: u16, url: String, body: String },
    #[error("transport error calling {url}: {message}")]
    Transport { url: String, message: String },
    #[error("unexpected response from {url}: {message}")]
    Response { url: String, message: String },
    #[error("invalid order: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct PayPalConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub base_url: String,
    pub budget_cents: u64,
    pub agent_id: String,
    /// Format sensitive strings as `[REDACTED]` before signing (disable only to test the policy).
    pub redact: bool,
    pub timeout: Duration,
}

impl PayPalConfig {
    /// `PAYPAL_CLIENT_ID`, `PAYPAL_CLIENT_SECRET`, `PAYPAL_BASE_URL` (Sandbox by default),
    /// `AEGIS_ORDER_BUDGET_CENTS`, `AEGIS_AGENT_ID`.
    pub fn from_env() -> Self {
        let var = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        Self {
            client_id: var("PAYPAL_CLIENT_ID"),
            client_secret: var("PAYPAL_CLIENT_SECRET"),
            base_url: var("PAYPAL_BASE_URL").unwrap_or_else(|| SANDBOX_BASE_URL.to_string()),
            budget_cents: var("AEGIS_ORDER_BUDGET_CENTS").and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_BUDGET_CENTS),
            agent_id: var("AEGIS_AGENT_ID").unwrap_or_else(|| "procurement-agent".to_string()),
            redact: true,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn has_credentials(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }

    /// Either credential is the literal [`LOCAL_SANDBOX_CREDENTIAL`].
    pub fn uses_local_sandbox_credentials(&self) -> bool {
        [&self.client_id, &self.client_secret].iter().any(|v| v.as_deref() == Some(LOCAL_SANDBOX_CREDENTIAL))
    }
}

/// True for `http://127.0.0.1[:port]`, `http://localhost[:port]` and `http://[::1][:port]`, with
/// any path: the only base URLs the local-sandbox credentials are sent to.
pub fn is_loopback_http(base_url: &str) -> bool {
    let Some(rest) = base_url.strip_prefix("http://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if authority.starts_with('[') {
        match authority.find(']') {
            Some(end) if authority[end + 1..].is_empty() || authority[end + 1..].starts_with(':') => &authority[..=end],
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let port_ok = authority[host.len()..].strip_prefix(':').is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok && ["127.0.0.1", "localhost", "[::1]"].contains(&host.to_ascii_lowercase().as_str())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LineItem {
    pub name: String,
    pub quantity: u32,
    pub unit_amount_cents: u64,
    pub sku: Option<String>,
}

/// What the agent wants to buy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrderIntent {
    pub reference: String,
    pub merchant_id: String,
    pub currency: String,
    pub description: String,
    pub customer_note: String,
    pub items: Vec<LineItem>,
    pub new_payee: bool,
    pub payee_country_risk: f64,
    /// Customer PII fields the agent handled while preparing the order.
    pub customer_pii_fields: u32,
    pub team_approval_granted: bool,
}

impl OrderIntent {
    pub fn amount_cents(&self) -> u64 {
        self.items.iter().map(|i| u64::from(i.quantity) * i.unit_amount_cents).sum()
    }
}

/// Microseconds spent in each `prepare` stage.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StageTimings {
    pub redact_us: f64,
    pub sign_us: f64,
    pub verify_us: f64,
    pub score_us: f64,
    pub cedar_us: f64,
    pub total_us: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PreparedOrder {
    pub reference: String,
    pub merchant_id: String,
    pub amount_cents: u64,
    pub currency: String,
    /// Orders v2 request body, redacted, with the signature in `purchase_units[0].custom_id`.
    pub body: Value,
    pub receipt: Receipt,
    pub receipt_verified: bool,
    pub risk_score: f32,
    pub decision: Decision,
    pub redactions: usize,
    pub has_unformatted_sensitive_fields: bool,
    pub timings: StageTimings,
}

impl PreparedOrder {
    /// Re-checks the detached receipt against the body as it is now.
    pub fn verify(&self) -> Result<(), ReceiptError> {
        let mut unsigned = self.body.clone();
        let attached = unsigned["purchase_units"][0]
            .as_object_mut()
            .and_then(|unit| unit.remove("custom_id"))
            .and_then(|v| v.as_str().map(str::to_owned));
        if attached.as_deref() != Some(self.receipt.signature.as_str()) {
            return Err(ReceiptError::Signature);
        }
        verify_json(&self.receipt, &unsigned)
    }
}

/// One HTTP call made while submitting an order, timed with `Instant` around the request and the
/// read of its response body.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HttpCall {
    /// `oauth`, `create` or `capture`.
    pub step: String,
    pub method: String,
    /// URL path (no scheme, host or query).
    pub path: String,
    pub status: u16,
    pub duration_us: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Submission {
    Sent {
        order_id: String,
        /// Status after the last call: `CREATED` (create only) or the capture's order status.
        status: String,
        /// Duration of the create call.
        latency_ms: f64,
        /// `purchase_units[0].payments.captures[0].id` when the order was captured.
        capture_id: Option<String>,
        /// The gateway marked its order JSON `"aegis_local_sandbox": true` (aegis-daemon's local
        /// emulation, not PayPal).
        local_sandbox: bool,
        calls: Vec<HttpCall>,
    },
    Blocked { reason: String },
    NotSent { reason: String },
}

/// Order velocity used for features (successful submissions only).
#[derive(Default)]
struct SpendHistory {
    entries: VecDeque<(Instant, u64)>,
}

impl SpendHistory {
    fn window(&mut self) -> (u32, u64) {
        let now = Instant::now();
        while self.entries.front().is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(86_400)) {
            self.entries.pop_front();
        }
        let last_hour = self.entries.iter().filter(|(t, _)| now.duration_since(*t) <= Duration::from_secs(3_600)).count();
        (last_hour as u32, self.entries.iter().map(|(_, c)| c).sum())
    }

    fn record(&mut self, cents: u64) {
        self.entries.push_back((Instant::now(), cents));
    }
}

pub struct PayPalMiddleware {
    cfg: PayPalConfig,
    gov: Arc<GovernanceEngine>,
    scorer: Arc<AneScorer>,
    signer: ReceiptSigner,
    http: ureq::Agent,
    history: Mutex<SpendHistory>,
}

fn money(cents: u64) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn micros(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e6
}

fn truncate(value: &mut Value) {
    if let Some(text) = value.as_str() {
        if text.chars().count() > PAYPAL_TEXT_LIMIT {
            *value = Value::String(text.chars().take(PAYPAL_TEXT_LIMIT).collect());
        }
    }
}

fn order_body(intent: &OrderIntent) -> Value {
    let total = money(intent.amount_cents());
    let items: Vec<Value> = intent
        .items
        .iter()
        .map(|item| {
            let mut v = json!({
                "name": item.name,
                "quantity": item.quantity.to_string(),
                "unit_amount": {"currency_code": intent.currency, "value": money(item.unit_amount_cents)},
            });
            if let Some(sku) = &item.sku {
                v["sku"] = json!(sku);
            }
            v
        })
        .collect();
    json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "reference_id": intent.reference,
            "description": format!("{} | {}", intent.description, intent.customer_note),
            "amount": {
                "currency_code": intent.currency,
                "value": total,
                "breakdown": {"item_total": {"currency_code": intent.currency, "value": total}},
            },
            "items": items,
        }],
    })
}

/// Applies PayPal's text limits after redaction, so a cut can never split a sensitive value.
fn fit_limits(body: &mut Value) {
    let unit = &mut body["purchase_units"][0];
    truncate(&mut unit["description"]);
    if let Some(items) = unit["items"].as_array_mut() {
        for item in items {
            truncate(&mut item["name"]);
        }
    }
}

impl PayPalMiddleware {
    pub fn new(cfg: PayPalConfig, gov: Arc<GovernanceEngine>, scorer: Arc<AneScorer>, signer: ReceiptSigner) -> Self {
        let http = ureq::AgentBuilder::new().timeout(cfg.timeout).build();
        Self { cfg, gov, scorer, signer, http, history: Mutex::new(SpendHistory::default()) }
    }

    pub fn config(&self) -> &PayPalConfig {
        &self.cfg
    }

    pub fn prepare(&self, intent: &OrderIntent) -> Result<PreparedOrder, PayPalError> {
        let started = Instant::now();
        let amount = intent.amount_cents();
        if intent.items.is_empty() || amount == 0 {
            return Err(PayPalError::Invalid("an order needs at least one item with a positive amount".into()));
        }
        let mut timings = StageTimings::default();

        let t = Instant::now();
        let mut body = order_body(intent);
        let redactions = if self.cfg.redact { scanner().redact_json(&mut body).total() } else { 0 };
        fit_limits(&mut body);
        let has_unformatted = scanner().scan_json(&body).any();
        timings.redact_us = micros(t);

        let t = Instant::now();
        let receipt = self.signer.sign_json(&body);
        timings.sign_us = micros(t);
        let t = Instant::now();
        let receipt_verified = verify_json(&receipt, &body).is_ok();
        timings.verify_us = micros(t);

        let t = Instant::now();
        let (orders_last_hour, spend_last_24h_cents) = self.history.lock().expect("history lock").window();
        let features = commerce_features(
            &CommerceContext {
                amount_cents: amount,
                budget_cents: self.cfg.budget_cents,
                currency: intent.currency.clone(),
                new_payee: intent.new_payee,
                payee_country_risk: intent.payee_country_risk,
                line_items: intent.items.len() as u32,
                quantity_total: intent.items.iter().map(|i| i.quantity).sum(),
                receipt_verified,
                orders_last_hour,
                spend_last_24h_cents,
                customer_pii_fields: intent.customer_pii_fields,
                unformatted_sensitive_fields: has_unformatted,
                ..Default::default()
            },
            None,
        );
        let risk_score = self.scorer.score(&features)?;
        timings.score_us = micros(t);

        let t = Instant::now();
        let decision = self.decide(intent, amount, risk_score, has_unformatted, receipt_verified)?;
        timings.cedar_us = micros(t);

        body["purchase_units"][0]["custom_id"] = json!(receipt.signature);
        timings.total_us = micros(started);
        Ok(PreparedOrder {
            reference: intent.reference.clone(),
            merchant_id: intent.merchant_id.clone(),
            amount_cents: amount,
            currency: intent.currency.clone(),
            body,
            receipt,
            receipt_verified,
            risk_score,
            decision,
            redactions,
            has_unformatted_sensitive_fields: has_unformatted,
            timings,
        })
    }

    fn decide(&self, intent: &OrderIntent, amount: u64, risk: f32, unformatted: bool, receipt_verified: bool) -> Result<Decision, PayPalError> {
        Ok(self.gov.authorize(&AuthzRequest {
            principal: self.cfg.agent_id.clone(),
            action: Action::CreateOrder,
            resource: intent.merchant_id.clone(),
            context: PolicyContext {
                ane_risk_score: risk_to_context(risk),
                has_unformatted_sensitive_fields: unformatted,
                team_approval_granted: intent.team_approval_granted,
                ed25519_receipt_verified: receipt_verified,
                amount_cents: amount as i64,
            },
        })?)
    }

    /// OAuth2 token, then `POST /v2/checkout/orders` (see the module docs).
    pub fn submit(&self, prepared: &PreparedOrder) -> Result<Submission, PayPalError> {
        self.send(prepared, false)
    }

    /// [`submit`](Self::submit), then `POST /v2/checkout/orders/{id}/capture`. For aegis-daemon's
    /// local Orders v2 gateway, where payer approval is implied; a real PayPal order needs buyer
    /// approval before it can be captured.
    pub fn submit_and_capture(&self, prepared: &PreparedOrder) -> Result<Submission, PayPalError> {
        self.send(prepared, true)
    }

    fn send(&self, prepared: &PreparedOrder, capture: bool) -> Result<Submission, PayPalError> {
        if !prepared.decision.allowed {
            let policies = if prepared.decision.determining_policies.is_empty() {
                "default deny (no permit matched)".to_string()
            } else {
                prepared.decision.determining_policies.join(", ")
            };
            return Ok(Submission::Blocked { reason: format!("Cedar denied: {policies}") });
        }
        let (Some(id), Some(secret)) = (&self.cfg.client_id, &self.cfg.client_secret) else {
            return Ok(Submission::NotSent { reason: "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set".to_string() });
        };
        if self.cfg.uses_local_sandbox_credentials() && !is_loopback_http(&self.cfg.base_url) {
            return Ok(Submission::NotSent {
                reason: format!(
                    "{LOCAL_SANDBOX_CREDENTIAL} credentials select aegis-daemon's local Orders v2 gateway; they are only sent to a loopback http:// base URL, and PAYPAL_BASE_URL is not one"
                ),
            });
        }
        let base = self.cfg.base_url.trim_end_matches('/');
        let mut calls = Vec::new();
        let token_url = format!("{base}/v1/oauth2/token");
        let started = Instant::now();
        let token = read_json(
            self.http
                .post(&token_url)
                .set("Authorization", &format!("Basic {}", BASE64.encode(format!("{id}:{secret}"))))
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string("grant_type=client_credentials"),
            &token_url,
        );
        let token = record(&mut calls, "oauth", &token_url, started, token)?;
        let access = token["access_token"]
            .as_str()
            .ok_or_else(|| PayPalError::Response { url: token_url.clone(), message: "no access_token".into() })?;
        let orders_url = format!("{base}/v2/checkout/orders");
        let started = Instant::now();
        let order = read_json(
            self.http
                .post(&orders_url)
                .set("Authorization", &format!("Bearer {access}"))
                .set("Content-Type", "application/json")
                .set("PayPal-Request-Id", &prepared.receipt.payload_sha256)
                .set("Prefer", "return=minimal")
                .send_json(prepared.body.clone()),
            &orders_url,
        );
        let latency_ms = started.elapsed().as_secs_f64() * 1e3;
        let order = record(&mut calls, "create", &orders_url, started, order)?;
        self.history.lock().expect("history lock").record(prepared.amount_cents);
        let order_id = order["id"].as_str().unwrap_or_default().to_string();
        let local_sandbox = order["aegis_local_sandbox"] == Value::Bool(true);
        let mut status = order["status"].as_str().unwrap_or_default().to_string();
        let mut capture_id = None;
        if capture {
            if order_id.is_empty() || !order_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                return Err(PayPalError::Response { url: orders_url, message: "order id missing or not URL-safe; not capturing".into() });
            }
            let capture_url = format!("{orders_url}/{order_id}/capture");
            let started = Instant::now();
            let captured = read_json(
                self.http
                    .post(&capture_url)
                    .set("Authorization", &format!("Bearer {access}"))
                    .set("Content-Type", "application/json")
                    .set("PayPal-Request-Id", &prepared.receipt.payload_sha256)
                    .set("Prefer", "return=representation")
                    .send_string("{}"),
                &capture_url,
            );
            let captured = record(&mut calls, "capture", &capture_url, started, captured)?;
            status = captured["status"].as_str().unwrap_or_default().to_string();
            capture_id = captured["purchase_units"][0]["payments"]["captures"][0]["id"].as_str().map(str::to_owned);
        }
        Ok(Submission::Sent { order_id, status, latency_ms, capture_id, local_sandbox, calls })
    }
}

/// URL path of `url` (no scheme, host or query).
fn url_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |i| &rest[i..]);
    path.split(['?', '#']).next().unwrap_or("/").to_string()
}

/// Appends the timed call (successful or HTTP error) and passes the result through.
fn record(calls: &mut Vec<HttpCall>, step: &str, url: &str, started: Instant, result: Result<(u16, Value), PayPalError>) -> Result<Value, PayPalError> {
    let duration_us = micros(started);
    let status = match &result {
        Ok((status, _)) => Some(*status),
        Err(PayPalError::Http { status, .. }) => Some(*status),
        Err(_) => None,
    };
    if let Some(status) = status {
        calls.push(HttpCall { step: step.to_string(), method: "POST".to_string(), path: url_path(url), status, duration_us });
    }
    result.map(|(_, body)| body)
}

fn read_json(response: Result<ureq::Response, ureq::Error>, url: &str) -> Result<(u16, Value), PayPalError> {
    match response {
        Ok(r) => {
            let status = r.status();
            r.into_json::<Value>().map(|v| (status, v)).map_err(|e| PayPalError::Response { url: url.to_string(), message: e.to_string() })
        }
        Err(ureq::Error::Status(status, r)) => Err(PayPalError::Http {
            status,
            url: url.to_string(),
            body: r.into_string().unwrap_or_default().chars().take(500).collect(),
        }),
        Err(ureq::Error::Transport(t)) => Err(PayPalError::Transport { url: url.to_string(), message: t.to_string() }),
    }
}

/// One dashboard row per prepared order.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TelemetryRow {
    pub id: String,
    pub timestamp: String,
    pub reference: String,
    pub merchant: String,
    pub agent: String,
    pub amount_cents: u64,
    pub amount: String,
    pub currency: String,
    pub risk_score: f64,
    pub decision: String,
    pub policies: String,
    pub receipt_sha256: String,
    pub signature: String,
    pub receipt_verified: bool,
    pub redactions: usize,
    pub unformatted_sensitive_fields: bool,
    pub submission: String,
    pub submission_detail: String,
    pub paypal_order_id: Option<String>,
    /// `"local"` (aegis-daemon's local Orders v2 emulation) or `"remote"` for a sent order.
    pub submission_target: Option<String>,
    pub paypal_capture_id: Option<String>,
    /// Sum of the submission's HTTP call durations (`submit_calls`), when calls were made.
    pub submit_us: Option<f64>,
    pub submit_calls: Vec<HttpCall>,
    pub redact_us: f64,
    pub sign_us: f64,
    pub verify_us: f64,
    pub score_us: f64,
    pub cedar_us: f64,
    pub total_us: f64,
}

pub fn telemetry_row(prepared: &PreparedOrder, submission: &Submission, agent: &str) -> TelemetryRow {
    let (kind, detail, order_id) = match submission {
        Submission::Sent { order_id, status, .. } => ("sent", status.clone(), Some(order_id.clone())),
        Submission::Blocked { reason } => ("blocked", reason.clone(), None),
        Submission::NotSent { reason } => ("not_sent", reason.clone(), None),
    };
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let (target, capture_id, calls) = match submission {
        Submission::Sent { local_sandbox, capture_id, calls, .. } => {
            (Some(if *local_sandbox { "local" } else { "remote" }.to_string()), capture_id.clone(), calls.clone())
        }
        _ => (None, None, Vec::new()),
    };
    let submit_us = (!calls.is_empty()).then(|| round(calls.iter().map(|c| c.duration_us).sum()));
    let calls: Vec<HttpCall> = calls.into_iter().map(|c| HttpCall { duration_us: round(c.duration_us), ..c }).collect();
    TelemetryRow {
        id: format!("{}-{}", prepared.reference, &prepared.receipt.payload_sha256[..12]),
        timestamp: utc_timestamp(),
        reference: prepared.reference.clone(),
        merchant: prepared.merchant_id.clone(),
        agent: agent.to_string(),
        amount_cents: prepared.amount_cents,
        amount: money(prepared.amount_cents),
        currency: prepared.currency.clone(),
        risk_score: round(f64::from(prepared.risk_score)),
        decision: if prepared.decision.allowed { "Allow" } else { "Deny" }.to_string(),
        policies: prepared.decision.determining_policies.join(", "),
        receipt_sha256: prepared.receipt.payload_sha256.clone(),
        signature: prepared.receipt.signature.clone(),
        receipt_verified: prepared.receipt_verified,
        redactions: prepared.redactions,
        unformatted_sensitive_fields: prepared.has_unformatted_sensitive_fields,
        submission: kind.to_string(),
        submission_detail: detail,
        paypal_order_id: order_id,
        submission_target: target,
        paypal_capture_id: capture_id,
        submit_us,
        submit_calls: calls,
        redact_us: round(prepared.timings.redact_us),
        sign_us: round(prepared.timings.sign_us),
        verify_us: round(prepared.timings.verify_us),
        score_us: round(prepared.timings.score_us),
        cedar_us: round(prepared.timings.cedar_us),
        total_us: round(prepared.timings.total_us),
    }
}

/// AG Grid grid options fragment: `{ columnDefs, rowData }`.
pub fn aggrid_export(rows: &[TelemetryRow]) -> Value {
    let col = |field: &str, header: &str, numeric: bool| {
        let mut c = json!({"field": field, "headerName": header, "sortable": true, "filter": true});
        if numeric {
            c["type"] = json!("numericColumn");
        }
        c
    };
    json!({
        "columnDefs": [
            col("timestamp", "Time (UTC)", false),
            col("reference", "Order", false),
            col("merchant", "Merchant", false),
            col("amount", "Amount", true),
            col("currency", "Ccy", false),
            col("risk_score", "ANE risk", true),
            col("decision", "Cedar", false),
            col("policies", "Determining policies", false),
            col("receipt_verified", "Receipt verified", false),
            col("redactions", "Redactions", true),
            col("submission", "Submission", false),
            col("total_us", "Pipeline µs", true),
            col("submit_us", "Submit µs", true),
        ],
        "rowData": rows,
    })
}

/// Bryntum store load response: `{ success, total, data }` (rows carry an `id` field).
pub fn bryntum_export(rows: &[TelemetryRow]) -> Value {
    json!({"success": true, "total": rows.len(), "data": rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    fn middleware(cfg: PayPalConfig) -> PayPalMiddleware {
        PayPalMiddleware::new(
            cfg,
            Arc::new(GovernanceEngine::load_default().unwrap()),
            Arc::new(AneScorer::open_default().unwrap()),
            ReceiptSigner::generate(),
        )
    }

    fn offline() -> PayPalConfig {
        PayPalConfig { client_id: None, client_secret: None, ..PayPalConfig::from_env() }
    }

    fn intent(reference: &str, unit_cents: u64, note: &str) -> OrderIntent {
        OrderIntent {
            reference: reference.into(),
            merchant_id: "sandbox-office-supplies".into(),
            currency: "USD".into(),
            description: "USB-C cables for the build lab".into(),
            customer_note: note.into(),
            items: vec![LineItem { name: "USB-C cable 2m".into(), quantity: 2, unit_amount_cents: unit_cents, sku: Some("USBC-2M".into()) }],
            new_payee: false,
            payee_country_risk: 0.05,
            customer_pii_fields: 1,
            team_approval_granted: false,
        }
    }

    /// Serves `responses` to consecutive connections; returns the raw requests.
    fn serve(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut request, mut length) = (String::new(), 0usize);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut payload = vec![0u8; length];
                reader.read_exact(&mut payload).unwrap();
                request.push_str(&String::from_utf8_lossy(&payload));
                let mut stream = stream;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                requests.push(request);
            }
            requests
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn sensitive_customer_strings_are_redacted_before_signing() {
        let mw = middleware(offline());
        let p = mw.prepare(&intent("ord-redact", 2100, "Deliver to jane.doe@example.com, card 4111 1111 1111 1111, call +1 (415) 555-0132")).unwrap();
        assert_eq!(p.redactions, 3);
        assert!(!p.has_unformatted_sensitive_fields);
        let text = p.body.to_string();
        assert!(text.contains("[REDACTED]") && !text.contains("jane.doe") && !text.contains("4111"));
        assert!(p.body["purchase_units"][0]["description"].as_str().unwrap().chars().count() <= PAYPAL_TEXT_LIMIT);
    }

    #[test]
    fn custom_id_signature_verifies_and_detects_tampering() {
        let mw = middleware(offline());
        let mut p = mw.prepare(&intent("ord-sig", 2100, "leave at reception")).unwrap();
        assert!(p.receipt_verified);
        assert!(p.body["purchase_units"][0]["custom_id"].as_str().unwrap().starts_with("c2pa:ed25519:"));
        assert_eq!(p.verify(), Ok(()));
        p.body["purchase_units"][0]["amount"]["value"] = json!("4200.00");
        assert_eq!(p.verify(), Err(ReceiptError::Digest));
    }

    #[test]
    fn standard_order_follows_risk_policy() {
        let mw = middleware(offline());
        let p = mw.prepare(&intent("ord-42", 2100, "leave at reception")).unwrap();
        assert_eq!(p.amount_cents, 4200);
        assert_eq!(p.decision.allowed, p.risk_score < 34.5, "risk {} decision {:?}", p.risk_score, p.decision);
        if p.decision.allowed {
            assert_eq!(p.decision.determining_policies, vec!["permit-low-risk"]);
        }
    }

    #[test]
    fn over_fifty_dollars_without_verified_receipt_is_forbidden() {
        let mw = middleware(offline());
        let order = intent("ord-120", 6000, "standard delivery");
        let denied = mw.decide(&order, order.amount_cents(), 10.0, false, false).unwrap();
        assert!(!denied.allowed);
        assert_eq!(denied.determining_policies, vec!["forbid-order-over-50-usd-without-receipt"]);
        assert!(mw.decide(&order, order.amount_cents(), 10.0, false, true).unwrap().allowed);
    }

    #[test]
    fn unredacted_payload_is_blocked() {
        let mw = middleware(PayPalConfig { redact: false, ..offline() });
        let p = mw.prepare(&intent("ord-pii", 1000, "email jane.doe@example.com")).unwrap();
        assert!(p.has_unformatted_sensitive_fields);
        assert!(!p.decision.allowed);
        assert!(matches!(mw.submit(&p).unwrap(), Submission::Blocked { .. }));
    }

    #[test]
    fn nothing_is_sent_without_credentials() {
        let mw = middleware(offline());
        let p = mw.prepare(&intent("ord-dry", 1000, "")).unwrap();
        if p.decision.allowed {
            assert!(matches!(mw.submit(&p).unwrap(), Submission::NotSent { .. }));
        }
    }

    #[test]
    fn submits_to_local_orders_v2_mock() {
        let (base_url, server) = serve(vec![
            json!({"access_token": "A21AA-test", "token_type": "Bearer", "expires_in": 32400}).to_string(),
            json!({"id": "MOCK-ORDER-1", "status": "CREATED"}).to_string(),
        ]);
        let cfg = PayPalConfig { client_id: Some("client".into()), client_secret: Some("secret".into()), base_url, ..offline() };
        let mw = middleware(cfg);
        let p = mw.prepare(&intent("ord-mock", 1000, "")).unwrap();
        assert!(p.decision.allowed, "$20 low-risk order should be allowed: {:?}", p.decision);
        let sent = mw.submit(&p).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /v1/oauth2/token") && requests[0].contains("grant_type=client_credentials"));
        assert!(requests[1].starts_with("POST /v2/checkout/orders"));
        assert!(requests[1].to_ascii_lowercase().contains(&format!("paypal-request-id: {}", p.receipt.payload_sha256)));
        assert!(requests[1].contains("c2pa:ed25519:"));
        match sent {
            Submission::Sent { order_id, status, .. } => assert_eq!((order_id.as_str(), status.as_str()), ("MOCK-ORDER-1", "CREATED")),
            other => panic!("expected Sent, got {other:?}"),
        }
    }

    #[test]
    fn loopback_base_urls() {
        for ok in ["http://127.0.0.1:4173/paypal-sandbox", "http://localhost:9", "http://[::1]:4173", "http://LOCALHOST/x", "http://127.0.0.1"] {
            assert!(is_loopback_http(ok), "{ok}");
        }
        for bad in [SANDBOX_BASE_URL, "https://127.0.0.1:4173", "http://127.0.0.1.evil.example", "http://user@127.0.0.1:1", "http://[::1]x:1", "http://127.0.0.1:", "http://10.0.0.1:4173", "127.0.0.1:4173"] {
            assert!(!is_loopback_http(bad), "{bad}");
        }
    }

    #[test]
    fn local_sandbox_credentials_never_leave_loopback() {
        let local = Some(LOCAL_SANDBOX_CREDENTIAL.to_string());
        for base_url in [SANDBOX_BASE_URL.to_string(), "http://192.0.2.1:4173/paypal-sandbox".to_string()] {
            let cfg = PayPalConfig { client_id: local.clone(), client_secret: local.clone(), base_url, ..offline() };
            assert!(cfg.uses_local_sandbox_credentials());
            let mw = middleware(cfg);
            let p = mw.prepare(&intent("ord-local", 1000, "")).unwrap();
            assert!(p.decision.allowed, "{:?}", p.decision);
            match mw.submit_and_capture(&p).unwrap() {
                Submission::NotSent { reason } => assert!(reason.contains("loopback"), "{reason}"),
                other => panic!("expected NotSent, got {other:?}"),
            }
        }
    }

    #[test]
    fn submit_and_capture_times_each_call() {
        let (base_url, server) = serve(vec![
            json!({"access_token": "local-token", "token_type": "Bearer", "expires_in": 600}).to_string(),
            json!({"id": "LOCAL-ORDER-1", "status": "CREATED", "aegis_local_sandbox": true}).to_string(),
            json!({"id": "LOCAL-ORDER-1", "status": "COMPLETED", "aegis_local_sandbox": true,
                   "purchase_units": [{"payments": {"captures": [{"id": "LOCALCAP-1", "status": "COMPLETED"}]}}]}).to_string(),
        ]);
        let local = Some(LOCAL_SANDBOX_CREDENTIAL.to_string());
        let cfg = PayPalConfig { client_id: local.clone(), client_secret: local, base_url: format!("{base_url}/paypal-sandbox"), ..offline() };
        let mw = middleware(cfg);
        let p = mw.prepare(&intent("ord-capture", 1000, "")).unwrap();
        assert!(p.decision.allowed, "{:?}", p.decision);
        let sent = mw.submit_and_capture(&p).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /paypal-sandbox/v1/oauth2/token"));
        assert!(requests[1].starts_with("POST /paypal-sandbox/v2/checkout/orders "));
        assert!(requests[2].starts_with("POST /paypal-sandbox/v2/checkout/orders/LOCAL-ORDER-1/capture "));
        let row = telemetry_row(&p, &sent, "procurement-agent");
        match sent {
            Submission::Sent { order_id, status, capture_id, local_sandbox, calls, .. } => {
                assert_eq!((order_id.as_str(), status.as_str(), capture_id.as_deref()), ("LOCAL-ORDER-1", "COMPLETED", Some("LOCALCAP-1")));
                assert!(local_sandbox);
                let steps: Vec<&str> = calls.iter().map(|c| c.step.as_str()).collect();
                assert_eq!(steps, ["oauth", "create", "capture"]);
                assert!(calls.iter().all(|c| c.status == 200 && c.duration_us > 0.0));
                assert_eq!(calls[2].path, "/paypal-sandbox/v2/checkout/orders/LOCAL-ORDER-1/capture");
            }
            other => panic!("expected Sent, got {other:?}"),
        }
        assert_eq!(row.submission, "sent");
        assert_eq!(row.submission_detail, "COMPLETED");
        assert_eq!(row.submission_target.as_deref(), Some("local"));
        assert_eq!(row.paypal_capture_id.as_deref(), Some("LOCALCAP-1"));
        assert_eq!(row.submit_calls.len(), 3);
        assert!(row.submit_us.unwrap() > 0.0);
    }

    #[test]
    fn dashboard_exports_have_expected_shapes() {
        let mw = middleware(offline());
        let p = mw.prepare(&intent("ord-row", 1500, "")).unwrap();
        let row = telemetry_row(&p, &mw.submit(&p).unwrap(), "procurement-agent");
        let grid = aggrid_export(std::slice::from_ref(&row));
        assert_eq!(grid["rowData"][0]["reference"], "ord-row");
        assert!(grid["columnDefs"].as_array().unwrap().iter().any(|c| c["field"] == "risk_score"));
        let store = bryntum_export(&[row]);
        assert_eq!(store["success"], true);
        assert_eq!(store["data"][0]["amount"], "30.00");
    }
}
