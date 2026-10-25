//! Local emulation of PayPal's REST OAuth2 token endpoint and Orders v2 create / show / capture,
//! mounted at [`PREFIX`] (`/paypal-sandbox`) on the daemon's own loopback listener, so the console's
//! PayPal pipeline can run a real end-to-end HTTP order submission without PayPal credentials.
//!
//! Request and response shapes follow developer.paypal.com: "Authentication"
//! (`POST /v1/oauth2/token`), the Orders v2 reference and its OpenAPI schema (`/api/orders/v2`,
//! `schema.json`), "Idempotency" (`PayPal-Request-Id`), "API responses" (error JSON, identity
//! errors) and the Orders v2 error messages (issue names), plus the currency-codes reference.
//!
//! It is not PayPal and never presents itself as PayPal: every response carries [`MARKER_HEADER`],
//! every order and error body carries `"aegis_local_sandbox": true`, tokens start with
//! `aegis-local-`, order and capture ids with `LOCAL`, debug ids with `local-`, and nothing leaves the
//! process. Local choices (also in `apps/aegis-console/API.md`):
//!
//! * Only the client id / secret [`LOCAL_SANDBOX_CREDENTIAL`] / [`LOCAL_SANDBOX_CREDENTIAL`] get a
//!   token. Tokens are random, kept in memory (at most `tokens_kept`) and expire after `token_ttl`.
//! * Payer approval is implied: there is no buyer, so `capture` completes a `CREATED` order. The
//!   `approve` link answers with a JSON note saying so; `update` (`PATCH`) is not implemented (405).
//! * Without a `Prefer` header the full representation is returned (PayPal's default is
//!   `return=minimal`); `Prefer: return=minimal` returns `{id, status, links}` plus the marker.
//! * The same `PayPal-Request-Id` with the same body returns the stored order (`200`); with a
//!   different body it is refused with `422` and the local issue `DUPLICATE_REQUEST_ID` (PayPal
//!   documents returning the earlier result instead).
//! * `custom_id` is limited to 127 characters (the limit the middleware targets; the current schema
//!   allows 255).
//! * Strings that still contain sensitive data (the detector the middleware uses,
//!   [`aegis_governance::scanner`]) are refused with `422` / `AEGIS_UNREDACTED_SENSITIVE_DATA`; the
//!   error names the fields, never the values.
//! * A `c2pa:ed25519:` receipt in `purchase_units[0].custom_id` must verify against the daemon's
//!   receipt key (detached: it covers the body without that `custom_id`), else `422` /
//!   `AEGIS_RECEIPT_SIGNATURE_INVALID`. An order without such a receipt is accepted and stored with
//!   `aegis_receipt.verified: false`.
//! * Orders live in memory (newest `orders_kept`); a body over [`BODY_LIMIT`] is refused with `413`.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub use adapter_paypal::LOCAL_SANDBOX_CREDENTIAL;
use aegis_governance::receipt::{digest_json, verify_json, SIGNATURE_PREFIX};
use aegis_governance::{scanner, Receipt, ScanReport};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::map_response;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Map, Value};

use crate::state::lock;
use crate::util::{constant_time_eq, iso_ms, random_alnum, random_token};

pub const PREFIX: &str = "/paypal-sandbox";
/// Response header on everything the gateway answers.
pub const MARKER_HEADER: &str = "x-aegis-local-sandbox";
pub const MARKER_VALUE: &str = "aegis-daemon local emulation of PayPal Orders v2; not PayPal";
pub const BODY_LIMIT: usize = 64 * 1024;
pub const DEFAULT_ORDERS_KEPT: usize = 256;
pub const DEFAULT_TOKENS_KEPT: usize = 256;
pub const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(9 * 3600);
pub const CUSTOM_ID_LIMIT: usize = 127;
/// PayPal accepts up to 3000 characters in `description` and item `name` and truncates them to 127
/// in the representation.
const TEXT_ACCEPTED: usize = 3000;
const TEXT_KEPT: usize = 127;
const REFERENCE_ID_LIMIT: usize = 256;
const MAX_PURCHASE_UNITS: usize = 10;
const REQUEST_ID_LIMIT: usize = 10_000;
/// `money.value`: up to 15 digits before the decimal point.
const MAX_INTEGER_DIGITS: usize = 15;
/// developer.paypal.com currency codes; HUF, JPY and TWD do not support decimals.
const CURRENCIES: [&str; 25] = [
    "AUD", "BRL", "CAD", "CNY", "CZK", "DKK", "EUR", "HKD", "HUF", "ILS", "JPY", "MYR", "MXN", "TWD", "NZD", "NOK", "PHP", "PLN", "GBP",
    "RUB", "SGD", "SEK", "CHF", "THB", "USD",
];
const ZERO_DECIMAL: [&str; 3] = ["HUF", "JPY", "TWD"];
const BREAKDOWN_ADD: [&str; 5] = ["item_total", "tax_total", "shipping", "handling", "insurance"];
const BREAKDOWN_SUB: [&str; 2] = ["shipping_discount", "discount"];

const INVALID_REQUEST: &str = "Request is not well-formed, syntactically incorrect, or violates schema.";
const UNPROCESSABLE: &str = "The requested action could not be performed, semantically incorrect, or failed business validation.";

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// `http://127.0.0.1:<port>/paypal-sandbox`: HATEOAS links point here.
    pub base_url: String,
    /// Hex Ed25519 verifying key of the daemon's receipt signer.
    pub receipt_public_key: String,
    pub token_ttl: Duration,
    pub orders_kept: usize,
    pub tokens_kept: usize,
}

impl GatewayConfig {
    pub fn new(base_url: String, receipt_public_key: String) -> Self {
        Self {
            base_url,
            receipt_public_key,
            token_ttl: DEFAULT_TOKEN_TTL,
            orders_kept: DEFAULT_ORDERS_KEPT,
            tokens_kept: DEFAULT_TOKENS_KEPT,
        }
    }
}

#[derive(Default)]
struct Stats {
    tokens_issued: AtomicU64,
    create_requests: AtomicU64,
    orders_created: AtomicU64,
    orders_captured: AtomicU64,
    requests_rejected: AtomicU64,
    receipts_verified: AtomicU64,
    receipts_absent: AtomicU64,
}

struct StoredOrder {
    order: Value,
    request_digest: String,
    create_request_id: Option<String>,
    capture_request_id: Option<String>,
}

#[derive(Default)]
struct Store {
    /// Access token → expiry.
    tokens: HashMap<String, Instant>,
    orders: HashMap<String, StoredOrder>,
    /// Order ids, oldest first (eviction order).
    order_ids: VecDeque<String>,
    /// Create `PayPal-Request-Id` → order id.
    create_ids: HashMap<String, String>,
}

pub struct LocalGateway {
    cfg: GatewayConfig,
    store: Mutex<Store>,
    stats: Stats,
}

/// One entry of an error's `details[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub field: String,
    pub location: &'static str,
    pub issue: &'static str,
    pub description: String,
    /// `true`: schema violation (`400 INVALID_REQUEST`); `false`: business rule (`422`).
    pub schema: bool,
}

impl Issue {
    fn schema(field: impl Into<String>, issue: &'static str, description: impl Into<String>) -> Self {
        Self { field: field.into(), location: "body", issue, description: description.into(), schema: true }
    }

    fn business(field: impl Into<String>, issue: &'static str, description: impl Into<String>) -> Self {
        Self { field: field.into(), location: "body", issue, description: description.into(), schema: false }
    }

    fn at(mut self, location: &'static str) -> Self {
        self.location = location;
        self
    }

    fn json(&self) -> Value {
        json!({"field": self.field, "location": self.location, "issue": self.issue, "description": self.description})
    }
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

fn debug_id() -> String {
    format!("local-{}", &random_token()[..13])
}

/// `YYYY-MM-DDTHH:MM:SSZ`, PayPal's timestamp format.
fn iso_seconds() -> String {
    let ms = iso_ms(SystemTime::now());
    format!("{}Z", &ms[..19])
}

fn random_id(prefix: &str) -> String {
    format!("{prefix}{}", random_token()[..16].to_ascii_uppercase())
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// PayPal error JSON (`name`, `message`, `debug_id`, `details[]`), marked as local.
pub fn paypal_error(status: StatusCode, name: &str, message: &str, details: &[Issue]) -> Response {
    let details: Vec<Value> = details.iter().map(Issue::json).collect();
    let body = json!({"name": name, "message": message, "debug_id": debug_id(), "details": details, "aegis_local_sandbox": true});
    no_store((status, Json(body)).into_response())
}

/// PayPal's identity-error JSON (OAuth2 token endpoint), marked as local.
fn identity_error(status: StatusCode, error: &str, description: &str) -> Response {
    no_store((status, Json(json!({"error": error, "error_description": description, "aegis_local_sandbox": true}))).into_response())
}

fn invalid(issues: &[Issue]) -> Response {
    paypal_error(StatusCode::BAD_REQUEST, "INVALID_REQUEST", INVALID_REQUEST, issues)
}

fn unprocessable(issues: &[Issue]) -> Response {
    paypal_error(StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY", UNPROCESSABLE, issues)
}

fn not_found_order(id: &str) -> Response {
    let shown: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
    let issue = Issue::business("order_id", "INVALID_RESOURCE_ID", format!("Specified resource ID {shown} does not exist.")).at("path");
    paypal_error(StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND", "The specified resource does not exist.", &[issue])
}

fn auth_failure(description: &str) -> Response {
    let issue = Issue::schema("Authorization", "INVALID_TOKEN", description).at("header");
    paypal_error(
        StatusCode::UNAUTHORIZED,
        "AUTHENTICATION_FAILURE",
        "Authentication failed due to missing Authorization header, or invalid authentication credentials.",
        &[issue],
    )
}

async fn mark(mut response: Response) -> Response {
    response.headers_mut().insert(MARKER_HEADER, HeaderValue::from_static(MARKER_VALUE));
    response
}

async fn read_body(body: Body) -> Result<Bytes, Response> {
    to_bytes(body, BODY_LIMIT).await.map_err(|_| {
        let issue = Issue::schema("", "INVALID_REQUEST_BODY", format!("request body unreadable or over the local gateway's {} KiB limit", BODY_LIMIT / 1024));
        paypal_error(StatusCode::PAYLOAD_TOO_LARGE, "INVALID_REQUEST", INVALID_REQUEST, &[issue])
    })
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.trim().to_ascii_lowercase().starts_with("application/json"))
}

fn unsupported_media() -> Response {
    let issue = Issue::schema("Content-Type", "UNSUPPORTED_MEDIA_TYPE", "send Content-Type: application/json").at("header");
    paypal_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE", "The server does not support the request payload's media type.", &[issue])
}

fn parse_json(bytes: &[u8]) -> Result<Value, Response> {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| invalid(&[Issue::schema("", "MALFORMED_REQUEST_JSON", "The request JSON is not well formed.")]))
}

/// `Basic base64(id:secret)` from the `Authorization` header.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = String::from_utf8(BASE64.decode(encoded.trim()).ok()?).ok()?;
    let (id, secret) = decoded.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
}

/// Optional `PayPal-Request-Id` (1 to 10000 visible characters).
fn request_id(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(value) = headers.get("paypal-request-id") else { return Ok(None) };
    match value.to_str() {
        Ok(v) if !v.is_empty() && v.len() <= REQUEST_ID_LIMIT => Ok(Some(v.to_string())),
        _ => Err(invalid(&[Issue::schema("PayPal-Request-Id", "INVALID_PARAMETER_VALUE", "1 to 10000 visible ASCII characters").at("header")])),
    }
}

/// `Prefer: return=minimal` → `true`. Without the header the full representation is returned.
fn wants_minimal(headers: &HeaderMap) -> bool {
    headers
        .get("prefer")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|p| p.trim().eq_ignore_ascii_case("return=minimal")))
}

fn representation(order: &Value, minimal: bool) -> Value {
    if minimal {
        json!({"id": order["id"], "status": order["status"], "links": order["links"], "aegis_local_sandbox": true})
    } else {
        order.clone()
    }
}

fn respond(status: StatusCode, order: &Value, minimal: bool) -> Response {
    no_store((status, Json(representation(order, minimal))).into_response())
}

// ---------------------------------------------------------------------------------------------
// Validation (Orders v2 `order_request` schema and business rules).

/// Minor units of a `money.value` string for `currency`, or the issue.
pub fn parse_value(value: &str, currency: &str, field: &str) -> Result<i128, Issue> {
    let syntax = || Issue::schema(field, "INVALID_PARAMETER_SYNTAX", "The value of a field does not conform to the expected format.");
    if value.is_empty() || value.len() > 32 {
        return Err(syntax());
    }
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    let (int, frac) = match digits.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (digits, None),
    };
    let numeric = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let well_formed = match frac {
        None => !int.is_empty() && numeric(int),
        Some(f) => !f.is_empty() && numeric(f) && numeric(int),
    };
    if !well_formed {
        return Err(syntax());
    }
    let scale: usize = if ZERO_DECIMAL.contains(&currency) { 0 } else { 2 };
    let frac = frac.unwrap_or("");
    if frac.len() > scale {
        let rule = if scale == 0 { "This currency does not support decimals." } else { "Only two decimal place precision is supported." };
        return Err(Issue::business(field, "DECIMAL_PRECISION", rule));
    }
    let int = int.trim_start_matches('0');
    if int.len() > MAX_INTEGER_DIGITS {
        return Err(Issue::business(field, "MAX_VALUE_EXCEEDED", "Should be less than or equal to 999999999999999.99."));
    }
    let padded = format!("{int}{frac:0<scale$}");
    let minor: i128 = if padded.is_empty() { 0 } else { padded.parse().map_err(|_| syntax())? };
    Ok(if negative { -minor } else { minor })
}

/// A `money` object: `(currency_code, minor units)`. `currency` (the unit's) must match when given.
fn money(v: &Value, field: &str, currency: Option<&str>, issues: &mut Vec<Issue>) -> Option<(String, i128)> {
    let Some(obj) = v.as_object() else {
        issues.push(if v.is_null() {
            Issue::schema(field, "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing.")
        } else {
            Issue::schema(field, "INVALID_PARAMETER_SYNTAX", "must be a money object {currency_code, value}")
        });
        return None;
    };
    let code_field = format!("{field}/currency_code");
    let code = match obj.get("currency_code") {
        None | Some(Value::Null) => {
            issues.push(Issue::schema(&code_field, "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing."));
            return None;
        }
        Some(Value::String(c)) if c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()) => c.clone(),
        Some(_) => {
            issues.push(Issue::schema(&code_field, "INVALID_PARAMETER_SYNTAX", "a three-character ISO-4217 code"));
            return None;
        }
    };
    if !CURRENCIES.contains(&code.as_str()) {
        issues.push(Issue::business(&code_field, "INVALID_CURRENCY_CODE", "Currency code is invalid or is not currently supported."));
        return None;
    }
    if let Some(expected) = currency {
        if code != expected {
            issues.push(Issue::schema(&code_field, "INVALID_PARAMETER_VALUE", format!("must equal the purchase unit's currency_code {expected}")));
            return None;
        }
    }
    let value_field = format!("{field}/value");
    let value = match obj.get("value") {
        None | Some(Value::Null) => {
            issues.push(Issue::schema(&value_field, "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing."));
            return None;
        }
        Some(Value::String(v)) => v,
        Some(_) => {
            issues.push(Issue::schema(&value_field, "INVALID_PARAMETER_SYNTAX", "value must be a decimal string"));
            return None;
        }
    };
    match parse_value(value, &code, &value_field) {
        Ok(minor) => Some((code, minor)),
        Err(issue) => {
            issues.push(issue);
            None
        }
    }
}

fn non_negative(minor: i128, field: &str, issues: &mut Vec<Issue>) {
    if minor < 0 {
        issues.push(Issue::business(format!("{field}/value"), "CANNOT_BE_NEGATIVE", "Must be greater than or equal to 0."));
    }
}

fn text(unit: &Map<String, Value>, key: &str, field: &str, max: usize, issues: &mut Vec<Issue>) -> Option<String> {
    match unit.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if (1..=max).contains(&s.chars().count()) => Some(s.clone()),
        Some(Value::String(_)) => {
            issues.push(Issue::schema(field, "INVALID_STRING_LENGTH", format!("The value of a field is either too short or too long (1 to {max} characters).")));
            None
        }
        Some(_) => {
            issues.push(Issue::schema(field, "INVALID_PARAMETER_SYNTAX", "must be a string"));
            None
        }
    }
}

fn validate_unit(i: usize, unit: &Value, issues: &mut Vec<Issue>) -> Option<String> {
    let base = format!("/purchase_units/{i}");
    let Some(unit) = unit.as_object() else {
        issues.push(Issue::schema(&base, "INVALID_PARAMETER_SYNTAX", "a purchase unit must be an object"));
        return None;
    };
    let reference = text(unit, "reference_id", &format!("{base}/reference_id"), REFERENCE_ID_LIMIT, issues);
    text(unit, "custom_id", &format!("{base}/custom_id"), CUSTOM_ID_LIMIT, issues);
    text(unit, "description", &format!("{base}/description"), TEXT_ACCEPTED, issues);
    text(unit, "invoice_id", &format!("{base}/invoice_id"), 127, issues);

    let amount_field = format!("{base}/amount");
    let amount = unit.get("amount").unwrap_or(&Value::Null);
    let Some((currency, total)) = money(amount, &amount_field, None, issues) else { return reference };
    if total <= 0 {
        issues.push(Issue::business(format!("{amount_field}/value"), "CANNOT_BE_ZERO_OR_NEGATIVE", "Must be greater than zero."));
    }

    let breakdown = amount.get("breakdown").filter(|b| !b.is_null());
    let mut parts: HashMap<&str, i128> = HashMap::new();
    if let Some(b) = breakdown {
        if !b.is_object() {
            issues.push(Issue::schema(format!("{amount_field}/breakdown"), "INVALID_PARAMETER_SYNTAX", "breakdown must be an object"));
        } else {
            for key in BREAKDOWN_ADD.iter().chain(BREAKDOWN_SUB.iter()) {
                let field = format!("{amount_field}/breakdown/{key}");
                if let Some(v) = b.get(*key).filter(|v| !v.is_null()) {
                    if let Some((_, minor)) = money(v, &field, Some(&currency), issues) {
                        non_negative(minor, &field, issues);
                        parts.insert(key, minor);
                    }
                }
            }
        }
    }

    let (mut item_sum, mut tax_sum, mut has_items, mut has_tax) = (0i128, 0i128, false, false);
    match unit.get("items") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (j, item) in items.iter().enumerate() {
                let field = format!("{base}/items/{j}");
                let Some(item) = item.as_object() else {
                    issues.push(Issue::schema(&field, "INVALID_PARAMETER_SYNTAX", "an item must be an object"));
                    continue;
                };
                if item.get("name").is_none_or(Value::is_null) {
                    issues.push(Issue::schema(format!("{field}/name"), "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing."));
                } else {
                    text(item, "name", &format!("{field}/name"), TEXT_ACCEPTED, issues);
                }
                let quantity = match item.get("quantity") {
                    Some(Value::String(q)) if (1..=10).contains(&q.len()) && !q.starts_with('0') && q.bytes().all(|b| b.is_ascii_digit()) => {
                        q.parse::<i128>().ok()
                    }
                    None | Some(Value::Null) => {
                        issues.push(Issue::schema(format!("{field}/quantity"), "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing."));
                        None
                    }
                    Some(_) => {
                        issues.push(Issue::schema(format!("{field}/quantity"), "INVALID_PARAMETER_SYNTAX", "a whole number as a string (1 to 9999999999)"));
                        None
                    }
                };
                let unit_field = format!("{field}/unit_amount");
                let unit_amount = money(item.get("unit_amount").unwrap_or(&Value::Null), &unit_field, Some(&currency), issues);
                if let Some((_, minor)) = unit_amount {
                    non_negative(minor, &unit_field, issues);
                    has_items = true;
                    item_sum += minor * quantity.unwrap_or(0);
                }
                if let Some(tax) = item.get("tax").filter(|t| !t.is_null()) {
                    let tax_field = format!("{field}/tax");
                    if let Some((_, minor)) = money(tax, &tax_field, Some(&currency), issues) {
                        non_negative(minor, &tax_field, issues);
                        has_tax = true;
                        tax_sum += minor * quantity.unwrap_or(0);
                    }
                }
            }
        }
        Some(_) => issues.push(Issue::schema(format!("{base}/items"), "INVALID_PARAMETER_SYNTAX", "items must be an array")),
    }

    let breakdown_field = format!("{amount_field}/breakdown");
    if has_items {
        match parts.get("item_total") {
            None => issues.push(Issue::business(format!("{breakdown_field}/item_total"), "ITEM_TOTAL_REQUIRED", "If item details are specified (items.unit_amount and items.quantity) corresponding amount.breakdown.item_total is required.")),
            Some(total) if *total != item_sum => issues.push(Issue::business(format!("{breakdown_field}/item_total"), "ITEM_TOTAL_MISMATCH", "Should equal sum of (unit_amount * quantity) across all items for a given purchase_unit.")),
            Some(_) => {}
        }
    }
    if has_tax {
        match parts.get("tax_total") {
            None => issues.push(Issue::business(format!("{breakdown_field}/tax_total"), "TAX_TOTAL_REQUIRED", "If item details are specified (items.tax and items.quantity) corresponding amount.breakdown.tax_total is required.")),
            Some(total) if *total != tax_sum => issues.push(Issue::business(format!("{breakdown_field}/tax_total"), "TAX_TOTAL_MISMATCH", "Should equal sum of (tax * quantity) across all items for a given purchase_unit.")),
            Some(_) => {}
        }
    }
    if breakdown.is_some_and(Value::is_object) {
        let sum: i128 = BREAKDOWN_ADD.iter().filter_map(|k| parts.get(k)).sum::<i128>() - BREAKDOWN_SUB.iter().filter_map(|k| parts.get(k)).sum::<i128>();
        if sum != total {
            issues.push(Issue::business(format!("{amount_field}/value"), "AMOUNT_MISMATCH", "Should equal item_total + tax_total + shipping + handling + insurance - shipping_discount - discount."));
        }
    }
    reference
}

/// Every schema and business-rule issue of an Orders v2 create body (empty when valid).
pub fn validate_order(body: &Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    match body.get("intent") {
        None | Some(Value::Null) => issues.push(Issue::schema("/intent", "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing.")),
        Some(Value::String(s)) if s == "CAPTURE" || s == "AUTHORIZE" => {}
        Some(_) => issues.push(Issue::schema("/intent", "INVALID_PARAMETER_VALUE", "intent must be CAPTURE or AUTHORIZE")),
    }
    match body.get("purchase_units") {
        None | Some(Value::Null) => issues.push(Issue::schema("/purchase_units", "MISSING_REQUIRED_PARAMETER", "A required field / parameter is missing.")),
        Some(Value::Array(units)) if units.is_empty() => {
            issues.push(Issue::schema("/purchase_units", "INVALID_ARRAY_MIN_ITEMS", "The number of items in an array parameter is too small."))
        }
        Some(Value::Array(units)) if units.len() > MAX_PURCHASE_UNITS => {
            issues.push(Issue::schema("/purchase_units", "INVALID_ARRAY_MAX_ITEMS", "The number of items in an array parameter is too large."))
        }
        Some(Value::Array(units)) => {
            let references: Vec<Option<String>> = units.iter().enumerate().map(|(i, u)| validate_unit(i, u, &mut issues)).collect();
            if units.len() > 1 {
                for (i, reference) in references.iter().enumerate() {
                    match reference {
                        None => issues.push(Issue::business(format!("/purchase_units/{i}/reference_id"), "REFERENCE_ID_REQUIRED", "'reference_id' is required for each 'purchase_unit' if multiple 'purchase_unit' are provided.")),
                        Some(r) if references[..i].iter().any(|o| o.as_deref() == Some(r.as_str())) => issues.push(Issue::business(format!("/purchase_units/{i}/reference_id"), "DUPLICATE_REFERENCE_ID", "reference_id must be unique if multiple purchase_unit are provided.")),
                        Some(_) => {}
                    }
                }
            }
        }
        Some(_) => issues.push(Issue::schema("/purchase_units", "INVALID_PARAMETER_SYNTAX", "purchase_units must be an array")),
    }
    issues
}

/// `400` when any issue is a schema violation, else `422`.
fn validation_error(issues: &[Issue]) -> Response {
    let schema: Vec<Issue> = issues.iter().filter(|i| i.schema).cloned().collect();
    if schema.is_empty() {
        unprocessable(issues)
    } else {
        invalid(&schema)
    }
}

fn is_receipt(pointer: &str, s: &str) -> bool {
    s.starts_with(SIGNATURE_PREFIX) && pointer.starts_with("/purchase_units/") && pointer.ends_with("/custom_id")
}

fn scan_strings(value: &Value, pointer: &mut String, found: &mut Vec<(String, ScanReport)>) {
    match value {
        Value::String(s) if !is_receipt(pointer, s) => {
            let report = scanner().scan(s);
            if report.any() {
                found.push((pointer.clone(), report));
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let len = pointer.len();
                pointer.push_str(&format!("/{i}"));
                scan_strings(item, pointer, found);
                pointer.truncate(len);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                scan_strings(item, pointer, found);
                pointer.truncate(len);
            }
        }
        _ => {}
    }
}

/// Fields (JSON pointers) whose strings still hold sensitive data, with what was found. Detached
/// `c2pa:ed25519:` receipts in `custom_id` are signatures, not customer data, and are skipped.
pub fn unredacted_fields(body: &Value) -> Vec<Issue> {
    let mut found = Vec::new();
    scan_strings(body, &mut String::new(), &mut found);
    found
        .into_iter()
        .map(|(field, r)| {
            let kinds: Vec<String> = [("secret", r.secrets), ("card number", r.card_numbers), ("SSN", r.ssns), ("e-mail address", r.emails), ("phone number", r.phones)]
                .into_iter()
                .filter(|(_, n)| *n > 0)
                .map(|(kind, n)| format!("{n} {kind}"))
                .collect();
            Issue::business(field, "AEGIS_UNREDACTED_SENSITIVE_DATA", format!("{} not formatted as [REDACTED] (local sandbox rule; the value is not repeated)", kinds.join(", ")))
        })
        .collect()
}

/// Result of checking the detached receipt in `purchase_units[0].custom_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptCheck {
    pub present: bool,
    pub verified: bool,
    pub payload_sha256: Option<String>,
    pub detail: String,
}

pub fn check_receipt(body: &Value, public_key: &str) -> ReceiptCheck {
    let Some(signature) = body["purchase_units"][0]["custom_id"].as_str().filter(|s| s.starts_with(SIGNATURE_PREFIX)) else {
        return ReceiptCheck {
            present: false,
            verified: false,
            payload_sha256: None,
            detail: format!("no {SIGNATURE_PREFIX} receipt in purchase_units[0].custom_id"),
        };
    };
    let mut unsigned = body.clone();
    if let Some(unit) = unsigned["purchase_units"][0].as_object_mut() {
        unit.remove("custom_id");
    }
    let digest = digest_json(&unsigned);
    let receipt = Receipt { payload_sha256: digest.clone(), signature: signature.to_string(), public_key: public_key.to_string() };
    match verify_json(&receipt, &unsigned) {
        Ok(()) => ReceiptCheck {
            present: true,
            verified: true,
            payload_sha256: Some(digest),
            detail: "ed25519 signature over the body without purchase_units[0].custom_id verifies with the daemon's receipt key".to_string(),
        },
        Err(e) => ReceiptCheck { present: true, verified: false, payload_sha256: Some(digest), detail: e.to_string() },
    }
}

// ---------------------------------------------------------------------------------------------
// Gateway.

impl LocalGateway {
    pub fn new(cfg: GatewayConfig) -> Self {
        Self { cfg, store: Mutex::new(Store::default()), stats: Stats::default() }
    }

    pub fn base_url(&self) -> &str {
        &self.cfg.base_url
    }

    /// Counters for `GET /api/paypal/scenarios` and `GET /api/config`.
    pub fn stats_json(&self) -> Value {
        let (orders_stored, tokens_active) = {
            let store = lock(&self.store);
            let now = Instant::now();
            (store.orders.len(), store.tokens.values().filter(|exp| **exp > now).count())
        };
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        json!({
            "base_url": self.cfg.base_url,
            "orders_stored": orders_stored,
            "orders_kept": self.cfg.orders_kept,
            "tokens_active": tokens_active,
            "tokens_issued": get(&self.stats.tokens_issued),
            "create_requests": get(&self.stats.create_requests),
            "orders_created": get(&self.stats.orders_created),
            "orders_captured": get(&self.stats.orders_captured),
            "requests_rejected": get(&self.stats.requests_rejected),
            "receipts_verified": get(&self.stats.receipts_verified),
            "receipts_absent": get(&self.stats.receipts_absent),
        })
    }

    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/v1/oauth2/token", post(token))
            .route("/v2/checkout/orders", post(create_order))
            .route("/v2/checkout/orders/{id}", get(get_order))
            .route("/v2/checkout/orders/{id}/capture", post(capture_order))
            .route("/checkoutnow", get(checkoutnow))
            .fallback(fallback)
            .method_not_allowed_fallback(method_not_allowed)
            .layer(map_response(mark))
            .with_state(self.clone())
    }

    fn issue_token(&self) -> String {
        let token = format!("aegis-local-{}", random_token());
        let now = Instant::now();
        let mut store = lock(&self.store);
        store.tokens.retain(|_, exp| *exp > now);
        while store.tokens.len() >= self.cfg.tokens_kept.max(1) {
            let Some(oldest) = store.tokens.iter().min_by_key(|(_, exp)| **exp).map(|(t, _)| t.clone()) else { break };
            store.tokens.remove(&oldest);
        }
        store.tokens.insert(token.clone(), now + self.cfg.token_ttl);
        bump(&self.stats.tokens_issued);
        token
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), Response> {
        let value = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("").trim();
        let token = match value.split_once(' ') {
            Some((scheme, token)) if scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty() => token.trim(),
            _ => return Err(auth_failure("missing Authorization: Bearer <access_token> header")),
        };
        let mut store = lock(&self.store);
        match store.tokens.get(token).copied() {
            Some(expiry) if expiry > Instant::now() => Ok(()),
            Some(_) => {
                store.tokens.remove(token);
                Err(auth_failure("access token expired; request a new one from /v1/oauth2/token"))
            }
            None => Err(auth_failure("access token was not issued by this local gateway")),
        }
    }

    fn links(&self, id: &str, intent: &str, status: &str) -> Value {
        let base = self.cfg.base_url.trim_end_matches('/');
        let order_url = format!("{base}/v2/checkout/orders/{id}");
        let mut links = vec![json!({"href": order_url, "rel": "self", "method": "GET"})];
        if status == "CREATED" || status == "APPROVED" {
            links.push(json!({"href": format!("{base}/checkoutnow?token={id}"), "rel": "approve", "method": "GET"}));
            links.push(json!({"href": order_url, "rel": "update", "method": "PATCH"}));
            if intent == "CAPTURE" {
                links.push(json!({"href": format!("{order_url}/capture"), "rel": "capture", "method": "POST"}));
            }
        }
        Value::Array(links)
    }

    /// The order as stored: the request's purchase units (single unit without `reference_id` →
    /// `default`; `description` and item names truncated to 127 characters, as PayPal reflects
    /// them), status, times, links, the local marker and the receipt check.
    fn build_order(&self, id: &str, body: &Value, receipt: &ReceiptCheck) -> Value {
        let intent = body["intent"].as_str().unwrap_or_default().to_string();
        let mut units = body["purchase_units"].as_array().cloned().unwrap_or_default();
        let single = units.len() == 1;
        for unit in units.iter_mut() {
            if let Some(unit) = unit.as_object_mut() {
                if single && !unit.contains_key("reference_id") {
                    unit.insert("reference_id".into(), json!("default"));
                }
                if let Some(Value::String(d)) = unit.get_mut("description") {
                    *d = d.chars().take(TEXT_KEPT).collect();
                }
                if let Some(Value::Array(items)) = unit.get_mut("items") {
                    for item in items {
                        if let Some(Value::String(n)) = item.get_mut("name") {
                            *n = n.chars().take(TEXT_KEPT).collect();
                        }
                    }
                }
            }
        }
        let now = iso_seconds();
        json!({
            "id": id,
            "intent": intent,
            "status": "CREATED",
            "purchase_units": units,
            "create_time": now,
            "update_time": now,
            "links": self.links(id, &intent, "CREATED"),
            "aegis_local_sandbox": true,
            "aegis_receipt": {
                "scheme": "c2pa:ed25519",
                "present": receipt.present,
                "verified": receipt.verified,
                "payload_sha256": receipt.payload_sha256,
                "public_key": self.cfg.receipt_public_key,
                "detail": receipt.detail,
            },
        })
    }

    fn insert(&self, store: &mut Store, id: String, stored: StoredOrder) {
        while store.order_ids.len() >= self.cfg.orders_kept.max(1) {
            let Some(old) = store.order_ids.pop_front() else { break };
            if let Some(evicted) = store.orders.remove(&old) {
                if let Some(rid) = evicted.create_request_id {
                    store.create_ids.remove(&rid);
                }
            }
        }
        if let Some(rid) = &stored.create_request_id {
            store.create_ids.insert(rid.clone(), id.clone());
        }
        store.order_ids.push_back(id.clone());
        store.orders.insert(id, stored);
    }

    /// The stored order for a replayed create `PayPal-Request-Id`: `Ok(Some)` same body, `Err` different body.
    fn replay(store: &Store, rid: Option<&str>, digest: &str) -> Result<Option<Value>, Response> {
        let Some(order) = rid.and_then(|r| store.create_ids.get(r)).and_then(|id| store.orders.get(id)) else { return Ok(None) };
        if order.request_digest == digest {
            Ok(Some(order.order.clone()))
        } else {
            let issue = Issue::business("PayPal-Request-Id", "DUPLICATE_REQUEST_ID", "this PayPal-Request-Id was already used with a different request body (local sandbox rule)").at("header");
            Err(unprocessable(&[issue]))
        }
    }

    async fn create(&self, headers: &HeaderMap, body: Body) -> Result<Response, Response> {
        self.authorize(headers)?;
        if !is_json(headers) {
            return Err(unsupported_media());
        }
        let body = parse_json(&read_body(body).await?)?;
        let rid = request_id(headers)?;
        let minimal = wants_minimal(headers);
        let digest = digest_json(&body);
        if let Some(order) = Self::replay(&lock(&self.store), rid.as_deref(), &digest)? {
            return Ok(respond(StatusCode::OK, &order, minimal));
        }
        let leaks = unredacted_fields(&body);
        if !leaks.is_empty() {
            return Err(unprocessable(&leaks));
        }
        let issues = validate_order(&body);
        if !issues.is_empty() {
            return Err(validation_error(&issues));
        }
        let receipt = check_receipt(&body, &self.cfg.receipt_public_key);
        if receipt.present && !receipt.verified {
            let issue = Issue::business(
                "/purchase_units/0/custom_id",
                "AEGIS_RECEIPT_SIGNATURE_INVALID",
                format!("the c2pa:ed25519 receipt does not verify against the daemon's receipt key ({}); local sandbox rule", receipt.detail),
            );
            return Err(unprocessable(&[issue]));
        }
        let id = random_id("LOCAL-");
        let order = self.build_order(&id, &body, &receipt);
        {
            let mut store = lock(&self.store);
            // A concurrent request with the same PayPal-Request-Id may have landed meanwhile.
            if let Some(existing) = Self::replay(&store, rid.as_deref(), &digest)? {
                return Ok(respond(StatusCode::OK, &existing, minimal));
            }
            let stored = StoredOrder { order: order.clone(), request_digest: digest, create_request_id: rid, capture_request_id: None };
            self.insert(&mut store, id, stored);
        }
        bump(&self.stats.orders_created);
        bump(if receipt.verified { &self.stats.receipts_verified } else { &self.stats.receipts_absent });
        Ok(respond(StatusCode::CREATED, &order, minimal))
    }

    fn show(&self, headers: &HeaderMap, id: &str) -> Result<Response, Response> {
        self.authorize(headers)?;
        let store = lock(&self.store);
        let stored = store.orders.get(id).ok_or_else(|| not_found_order(id))?;
        Ok(respond(StatusCode::OK, &stored.order, false))
    }

    async fn capture(&self, headers: &HeaderMap, id: &str, body: Body) -> Result<Response, Response> {
        self.authorize(headers)?;
        let bytes = read_body(body).await?;
        if !bytes.iter().all(u8::is_ascii_whitespace) {
            if !is_json(headers) {
                return Err(unsupported_media());
            }
            parse_json(&bytes)?;
        }
        let rid = request_id(headers)?;
        let minimal = wants_minimal(headers);
        let mut store = lock(&self.store);
        let stored = store.orders.get_mut(id).ok_or_else(|| not_found_order(id))?;
        if rid.is_some() && stored.capture_request_id == rid {
            return Ok(respond(StatusCode::OK, &stored.order, minimal));
        }
        let order = &mut stored.order;
        if order["intent"] != "CAPTURE" {
            let issue = Issue::business("intent", "ACTION_DOES_NOT_MATCH_INTENT", "Order was created with an intent to 'AUTHORIZE'; the local sandbox does not implement authorize.");
            return Err(unprocessable(&[issue]));
        }
        if order["status"] == "COMPLETED" {
            let issue = Issue::business("", "ORDER_ALREADY_CAPTURED", "Order already captured. If 'intent=CAPTURE' only one capture per order is allowed.");
            return Err(unprocessable(&[issue]));
        }
        let now = iso_seconds();
        let order_url = format!("{}/v2/checkout/orders/{id}", self.cfg.base_url.trim_end_matches('/'));
        if let Some(units) = order["purchase_units"].as_array_mut() {
            for unit in units.iter_mut() {
                let mut capture = json!({
                    "id": random_id("LOCALCAP-"),
                    "status": "COMPLETED",
                    "amount": {"currency_code": unit["amount"]["currency_code"], "value": unit["amount"]["value"]},
                    "final_capture": true,
                    "create_time": now,
                    "update_time": now,
                    "links": [{"href": order_url, "rel": "up", "method": "GET"}],
                });
                if let Some(custom_id) = unit.get("custom_id") {
                    capture["custom_id"] = custom_id.clone();
                }
                unit["payments"] = json!({"captures": [capture]});
            }
        }
        order["status"] = json!("COMPLETED");
        order["update_time"] = json!(now);
        let intent = order["intent"].as_str().unwrap_or_default().to_string();
        order["links"] = self.links(id, &intent, "COMPLETED");
        stored.capture_request_id = rid;
        let response = respond(StatusCode::CREATED, &stored.order, minimal);
        drop(store);
        bump(&self.stats.orders_captured);
        Ok(response)
    }

    fn count(&self, result: Result<Response, Response>) -> Response {
        result.unwrap_or_else(|rejected| {
            bump(&self.stats.requests_rejected);
            rejected
        })
    }
}

type Gateway = State<Arc<LocalGateway>>;

async fn token(State(gw): Gateway, headers: HeaderMap, body: Body) -> Response {
    let result = async {
        let body = read_body(body).await?;
        let (id, secret) = basic_credentials(&headers).ok_or_else(|| identity_error(StatusCode::UNAUTHORIZED, "invalid_client", "Client Authentication failed"))?;
        let local = LOCAL_SANDBOX_CREDENTIAL.as_bytes();
        if !(constant_time_eq(id.as_bytes(), local) & constant_time_eq(secret.as_bytes(), local)) {
            return Err(identity_error(StatusCode::UNAUTHORIZED, "invalid_client", "Client Authentication failed"));
        }
        let form = String::from_utf8_lossy(&body);
        match form.split('&').filter_map(|pair| pair.split_once('=')).find(|(k, _)| k.trim() == "grant_type").map(|(_, v)| v.trim()) {
            Some("client_credentials") => {}
            None => return Err(identity_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "Grant Type is NULL")),
            Some(_) => return Err(identity_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "unsupported grant_type; use client_credentials")),
        }
        let access_token = gw.issue_token();
        let body = json!({
            "scope": "aegis-local-sandbox/v2/checkout/orders",
            "access_token": access_token,
            "token_type": "Bearer",
            "app_id": "APP-AEGIS-LOCAL-SANDBOX",
            "expires_in": gw.cfg.token_ttl.as_secs(),
            "nonce": format!("{}{}", iso_seconds(), random_alnum(43)),
            "aegis_local_sandbox": true,
        });
        Ok(no_store(Json(body).into_response()))
    }
    .await;
    gw.count(result)
}

async fn create_order(State(gw): Gateway, headers: HeaderMap, body: Body) -> Response {
    bump(&gw.stats.create_requests);
    let result = gw.create(&headers, body).await;
    gw.count(result)
}

async fn get_order(State(gw): Gateway, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let result = gw.show(&headers, &id);
    gw.count(result)
}

async fn capture_order(State(gw): Gateway, Path(id): Path<String>, headers: HeaderMap, body: Body) -> Response {
    let result = gw.capture(&headers, &id, body).await;
    gw.count(result)
}

/// Where the `approve` link points. There is no buyer in the local sandbox: approval is implied
/// and `capture` completes a `CREATED` order, so this only reports that.
async fn checkoutnow(State(gw): Gateway, Query(query): Query<HashMap<String, String>>) -> Response {
    let id = query.get("token").cloned().unwrap_or_default();
    let status = lock(&gw.store).orders.get(&id).map(|o| o.order["status"].clone());
    match status {
        Some(status) => no_store(
            Json(json!({
                "id": id,
                "status": status,
                "aegis_local_sandbox": true,
                "message": "Local sandbox: payer approval is implied; POST /v2/checkout/orders/{id}/capture completes the order. Nothing is sent to PayPal.",
            }))
            .into_response(),
        ),
        None => not_found_order(&id),
    }
}

async fn fallback() -> Response {
    paypal_error(StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND", "The specified resource does not exist.", &[])
}

async fn method_not_allowed() -> Response {
    let issue = Issue::schema("", "METHOD_NOT_SUPPORTED", "not implemented by the local sandbox (OAuth2 token, Orders v2 create, show and capture only)").at("path");
    paypal_error(StatusCode::METHOD_NOT_ALLOWED, "METHOD_NOT_SUPPORTED", "The server does not implement the requested HTTP method.", &[issue])
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_governance::ReceiptSigner;

    fn order() -> Value {
        json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "reference_id": "agent-0001",
                "description": "Build-lab hardware | Deliver to [REDACTED]",
                "amount": {"currency_code": "USD", "value": "42.00", "breakdown": {"item_total": {"currency_code": "USD", "value": "42.00"}}},
                "items": [{"name": "Thunderbolt 4 cable", "quantity": "2", "unit_amount": {"currency_code": "USD", "value": "21.00"}}],
            }],
        })
    }

    fn issues_of(body: &Value) -> Vec<&'static str> {
        validate_order(body).into_iter().map(|i| i.issue).collect()
    }

    #[test]
    fn values_parse_to_minor_units() {
        assert_eq!(parse_value("42.00", "USD", "/v"), Ok(4200));
        assert_eq!(parse_value("42", "USD", "/v"), Ok(4200));
        assert_eq!(parse_value(".5", "USD", "/v"), Ok(50));
        assert_eq!(parse_value("-1.25", "USD", "/v"), Ok(-125));
        assert_eq!(parse_value("1500", "JPY", "/v"), Ok(1500));
        assert_eq!(parse_value("1.5", "JPY", "/v").unwrap_err().issue, "DECIMAL_PRECISION");
        assert_eq!(parse_value("1.234", "USD", "/v").unwrap_err().issue, "DECIMAL_PRECISION");
        for bad in ["", "1.", "1e3", "abc", "1,00", "--1", " 1"] {
            assert_eq!(parse_value(bad, "USD", "/v").unwrap_err().issue, "INVALID_PARAMETER_SYNTAX", "{bad:?}");
        }
        assert_eq!(parse_value("1234567890123456", "USD", "/v").unwrap_err().issue, "MAX_VALUE_EXCEEDED");
    }

    #[test]
    fn a_consistent_order_has_no_issues() {
        assert_eq!(issues_of(&order()), Vec::<&str>::new());
    }

    #[test]
    fn schema_and_business_issues_are_reported() {
        assert_eq!(issues_of(&json!({})), ["MISSING_REQUIRED_PARAMETER", "MISSING_REQUIRED_PARAMETER"]);
        let mut b = order();
        b["intent"] = json!("SALE");
        assert_eq!(issues_of(&b), ["INVALID_PARAMETER_VALUE"]);
        let mut b = order();
        b["purchase_units"] = json!([]);
        assert_eq!(issues_of(&b), ["INVALID_ARRAY_MIN_ITEMS"]);
        let mut b = order();
        b["purchase_units"][0]["amount"]["value"] = json!("40.00");
        assert_eq!(issues_of(&b), ["AMOUNT_MISMATCH"]);
        let mut b = order();
        b["purchase_units"][0]["items"][0]["quantity"] = json!("3");
        assert_eq!(issues_of(&b), ["ITEM_TOTAL_MISMATCH"]);
        let mut b = order();
        b["purchase_units"][0]["amount"].as_object_mut().unwrap().remove("breakdown");
        assert_eq!(issues_of(&b), ["ITEM_TOTAL_REQUIRED"]);
        let mut b = order();
        b["purchase_units"][0]["amount"]["value"] = json!(42.0);
        assert!(issues_of(&b).contains(&"INVALID_PARAMETER_SYNTAX"));
        let mut b = order();
        b["purchase_units"][0]["items"][0]["unit_amount"]["currency_code"] = json!("EUR");
        assert!(issues_of(&b).contains(&"INVALID_PARAMETER_VALUE"));
        let mut b = order();
        b["purchase_units"][0]["amount"]["currency_code"] = json!("XYZ");
        assert_eq!(issues_of(&b), ["INVALID_CURRENCY_CODE"]);
        let mut b = order();
        b["purchase_units"][0]["custom_id"] = json!("x".repeat(CUSTOM_ID_LIMIT + 1));
        assert_eq!(issues_of(&b), ["INVALID_STRING_LENGTH"]);
        let mut b = order();
        b["purchase_units"][0]["items"][0]["quantity"] = json!("0");
        assert!(issues_of(&b).contains(&"INVALID_PARAMETER_SYNTAX"));
        let mut b = order();
        let unit = b["purchase_units"][0].clone();
        b["purchase_units"] = json!([unit.clone(), unit]);
        assert_eq!(issues_of(&b), ["DUPLICATE_REFERENCE_ID"]);
        b["purchase_units"][1].as_object_mut().unwrap().remove("reference_id");
        assert_eq!(issues_of(&b), ["REFERENCE_ID_REQUIRED"]);
        let mut b = order();
        b["purchase_units"][0]["amount"] = json!({"currency_code": "USD", "value": "0.00"});
        b["purchase_units"][0].as_object_mut().unwrap().remove("items");
        assert_eq!(issues_of(&b), ["CANNOT_BE_ZERO_OR_NEGATIVE"]);
    }

    #[test]
    fn unredacted_strings_are_found_by_field_without_their_value() {
        assert!(unredacted_fields(&order()).is_empty());
        let mut b = order();
        b["purchase_units"][0]["description"] = json!("call +1 (415) 555-0132, mail jane.doe@example.com");
        let found = unredacted_fields(&b);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].field, "/purchase_units/0/description");
        assert!(found[0].description.contains("e-mail") && found[0].description.contains("phone"));
        assert!(!found[0].description.contains("jane.doe"));
    }

    #[test]
    fn detached_receipt_verifies_only_with_the_daemon_key() {
        let signer = ReceiptSigner::generate();
        let mut body = order();
        let receipt = signer.sign_json(&body);
        body["purchase_units"][0]["custom_id"] = json!(receipt.signature);
        assert!(unredacted_fields(&body).is_empty(), "the signature is not customer data");
        let ok = check_receipt(&body, &signer.public_key_hex());
        assert!(ok.present && ok.verified, "{ok:?}");
        assert_eq!(ok.payload_sha256.as_deref(), Some(receipt.payload_sha256.as_str()));
        let other = check_receipt(&body, &ReceiptSigner::generate().public_key_hex());
        assert!(other.present && !other.verified);
        let mut tampered = body.clone();
        tampered["purchase_units"][0]["amount"]["value"] = json!("4200.00");
        assert!(!check_receipt(&tampered, &signer.public_key_hex()).verified);
        assert!(!check_receipt(&order(), &signer.public_key_hex()).present);
    }
}
