//! The local PayPal Orders v2 gateway (`/paypal-sandbox`) through the daemon's real router (Host /
//! Origin guard included) and, for token expiry and the order cap, a standalone gateway with small
//! limits. Nothing binds a port; nothing reaches PayPal.

mod common;

use std::sync::Arc;
use std::time::Duration;

use aegis_daemon::paypal_sandbox::{GatewayConfig, LocalGateway, BODY_LIMIT, MARKER_HEADER, MARKER_VALUE};
use aegis_governance::ReceiptSigner;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use common::Fixture;
use serde_json::{json, Value};
use tower::ServiceExt;

const BASIC_LOCAL: &str = "local-sandbox:local-sandbox";

/// A standalone gateway mounted where the daemon mounts it (no guard).
fn mounted(gw: &Arc<LocalGateway>) -> Router {
    Router::new().nest_service(aegis_daemon::paypal_sandbox::PREFIX, gw.router())
}

struct Reply {
    status: StatusCode,
    marker: Option<String>,
    body: Value,
}

async fn call(app: &Router, req: Request<Body>) -> Reply {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let marker = response.headers().get(MARKER_HEADER).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    Reply { status, marker, body }
}

fn token_request(host: &str, basic: Option<&str>, form: &str) -> Request<Body> {
    let mut b = Request::post("/paypal-sandbox/v1/oauth2/token").header("host", host).header("content-type", "application/x-www-form-urlencoded");
    if let Some(basic) = basic {
        b = b.header("authorization", format!("Basic {}", BASE64.encode(basic)));
    }
    b.body(Body::from(form.to_string())).unwrap()
}

async fn token(app: &Router, host: &str) -> String {
    let r = call(app, token_request(host, Some(BASIC_LOCAL), "grant_type=client_credentials")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    r.body["access_token"].as_str().unwrap().to_string()
}

fn order_request(host: &str, method: &str, uri: &str, token: Option<&str>, request_id: Option<&str>, body: Option<&Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header("host", host).header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    if let Some(id) = request_id {
        b = b.header("paypal-request-id", id);
    }
    b.body(body.map_or_else(Body::empty, |v| Body::from(v.to_string()))).unwrap()
}

fn create(host: &str, token: &str, request_id: Option<&str>, body: &Value) -> Request<Body> {
    order_request(host, "POST", "/paypal-sandbox/v2/checkout/orders", Some(token), request_id, Some(body))
}

fn order_body(reference: &str) -> Value {
    json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "reference_id": reference,
            "description": "Build-lab hardware for CI runners | Deliver to [REDACTED], call [REDACTED]",
            "amount": {"currency_code": "USD", "value": "42.00", "breakdown": {"item_total": {"currency_code": "USD", "value": "42.00"}}},
            "items": [{"name": "Thunderbolt 4 cable", "quantity": "2", "unit_amount": {"currency_code": "USD", "value": "21.00"}, "sku": "TB4-1M"}],
        }],
    })
}

/// `body` with a detached receipt in `purchase_units[0].custom_id`, signed by `signer`.
fn signed(signer: &ReceiptSigner, mut body: Value) -> Value {
    let receipt = signer.sign_json(&body);
    body["purchase_units"][0]["custom_id"] = json!(receipt.signature);
    body
}

fn issues(body: &Value) -> Vec<String> {
    body["details"].as_array().map(|d| d.iter().filter_map(|i| i["issue"].as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn assert_paypal_error(r: &Reply, status: StatusCode, name: &str) {
    assert_eq!(r.status, status, "{}", r.body);
    assert_eq!(r.body["name"], name, "{}", r.body);
    assert!(r.body["message"].is_string() && r.body["debug_id"].as_str().is_some_and(|d| d.starts_with("local-")), "{}", r.body);
    assert!(r.body["details"].is_array());
    assert_eq!(r.body["aegis_local_sandbox"], true);
    assert_eq!(r.marker.as_deref(), Some(MARKER_VALUE));
}

fn daemon_signer(fx: &Fixture) -> ReceiptSigner {
    ReceiptSigner::load_or_create(&fx.root.join("keys/receipt.seed")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn oauth_token_is_required_and_issued_only_for_local_credentials() {
    let fx = Fixture::new("sbx-auth", "").await;
    let (app, host) = (&fx.app, fx.host.as_str());

    // No browser token, a form body: the guard lets OAuth through; the Host check still applies.
    let r = call(app, token_request(host, Some(BASIC_LOCAL), "grant_type=client_credentials")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.marker.as_deref(), Some(MARKER_VALUE));
    for key in ["scope", "access_token", "token_type", "app_id", "expires_in", "nonce"] {
        assert!(!r.body[key].is_null(), "token response lacks {key}: {}", r.body);
    }
    assert_eq!(r.body["token_type"], "Bearer");
    assert_eq!(r.body["aegis_local_sandbox"], true);
    let first = r.body["access_token"].as_str().unwrap().to_string();
    assert!(first.starts_with("aegis-local-"));
    assert_ne!(first, token(app, host).await, "tokens are random");

    let r = call(app, token_request(host, None, "grant_type=client_credentials")).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_client")));
    let r = call(app, token_request(host, Some("client:secret"), "grant_type=client_credentials")).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_client")));
    let r = call(app, token_request(host, Some(BASIC_LOCAL), "")).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unsupported_grant_type")));
    let r = call(app, token_request(host, Some(BASIC_LOCAL), "grant_type=password")).await;
    assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("unsupported_grant_type")));

    for (host, origin) in [("evil.example:4173", None), ("127.0.0.1:9999", None), (host, Some("http://evil.example"))] {
        let mut req = token_request(host, Some(BASIC_LOCAL), "grant_type=client_credentials");
        if let Some(o) = origin {
            req.headers_mut().insert("origin", o.parse().unwrap());
        }
        assert_eq!(call(app, req).await.status, StatusCode::FORBIDDEN, "host {host} origin {origin:?}");
    }

    // Orders need a Bearer token this gateway issued.
    let body = order_body("auth-1");
    let r = call(app, order_request(host, "POST", "/paypal-sandbox/v2/checkout/orders", None, None, Some(&body))).await;
    assert_paypal_error(&r, StatusCode::UNAUTHORIZED, "AUTHENTICATION_FAILURE");
    let r = call(app, create(host, "aegis-local-forged", None, &body)).await;
    assert_paypal_error(&r, StatusCode::UNAUTHORIZED, "AUTHENTICATION_FAILURE");
    assert!(r.body["details"][0]["description"].as_str().unwrap().contains("not issued"));
    let r = call(app, order_request(host, "GET", "/paypal-sandbox/v2/checkout/orders/LOCAL-X", None, None, None)).await;
    assert_paypal_error(&r, StatusCode::UNAUTHORIZED, "AUTHENTICATION_FAILURE");
    // The console API itself still needs the browser token.
    let r = call(app, Request::post("/api/paypal/order").header("host", host).header("content-type", "application/json").body(Body::from("{}")).unwrap()).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.marker.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_tokens_are_refused_and_orders_are_capped() {
    let mut cfg = GatewayConfig::new("http://127.0.0.1:4199/paypal-sandbox".into(), ReceiptSigner::generate().public_key_hex());
    cfg.token_ttl = Duration::ZERO;
    cfg.orders_kept = 2;
    let gw = Arc::new(LocalGateway::new(cfg));
    let app = mounted(&gw);
    let host = "127.0.0.1:4199";
    let expired = token(&app, host).await;
    let r = call(&app, create(host, &expired, None, &order_body("exp-1"))).await;
    assert_paypal_error(&r, StatusCode::UNAUTHORIZED, "AUTHENTICATION_FAILURE");
    assert!(r.body["details"][0]["description"].as_str().unwrap().contains("expired"), "{}", r.body);

    let mut cfg = GatewayConfig::new("http://127.0.0.1:4199/paypal-sandbox".into(), ReceiptSigner::generate().public_key_hex());
    cfg.orders_kept = 2;
    let gw = Arc::new(LocalGateway::new(cfg));
    let app = mounted(&gw);
    let t = token(&app, host).await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let r = call(&app, create(host, &t, Some(&format!("req-{i}")), &order_body(&format!("cap-{i}")))).await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
        ids.push(r.body["id"].as_str().unwrap().to_string());
    }
    let get = |id: &str| order_request(host, "GET", &format!("/paypal-sandbox/v2/checkout/orders/{id}"), Some(&t), None, None);
    assert_paypal_error(&call(&app, get(&ids[0])).await, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND");
    assert_eq!(call(&app, get(&ids[2])).await.status, StatusCode::OK);
    assert_eq!(gw.stats_json()["orders_stored"], 2);
    // The evicted order's PayPal-Request-Id is forgotten with it: the replay creates a new order.
    let r = call(&app, create(host, &t, Some("req-0"), &order_body("cap-0"))).await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert_ne!(r.body["id"], ids[0].as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_show_capture_with_links_and_idempotency() {
    let fx = Fixture::new("sbx-flow", "").await;
    let (app, host) = (&fx.app, fx.host.as_str());
    let t = token(app, host).await;
    let body = signed(&daemon_signer(&fx), order_body("flow-1"));

    let r = call(app, create(host, &t, Some("flow-req-1"), &body)).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    assert_eq!(r.marker.as_deref(), Some(MARKER_VALUE));
    let order = r.body.clone();
    let id = order["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("LOCAL-"));
    assert_eq!(order["status"], "CREATED");
    assert_eq!(order["intent"], "CAPTURE");
    assert_eq!(order["aegis_local_sandbox"], true);
    assert_eq!(order["aegis_receipt"]["verified"], true, "{order}");
    assert_eq!(order["purchase_units"][0]["reference_id"], "flow-1");
    assert_eq!(order["purchase_units"][0]["amount"]["value"], "42.00");
    assert!(order["create_time"].as_str().unwrap().ends_with('Z'));
    let base = "http://127.0.0.1:4173/paypal-sandbox";
    let links: Vec<(String, String, String)> = order["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["rel"].as_str().unwrap().into(), l["method"].as_str().unwrap().into(), l["href"].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        links,
        [
            ("self".into(), "GET".into(), format!("{base}/v2/checkout/orders/{id}")),
            ("approve".into(), "GET".into(), format!("{base}/checkoutnow?token={id}")),
            ("update".into(), "PATCH".into(), format!("{base}/v2/checkout/orders/{id}")),
            ("capture".into(), "POST".into(), format!("{base}/v2/checkout/orders/{id}/capture")),
        ]
    );

    // Idempotency: same id + same body → the same order (200); same id + other body → 422.
    let r = call(app, create(host, &t, Some("flow-req-1"), &body)).await;
    assert_eq!((r.status, r.body["id"].as_str()), (StatusCode::OK, Some(id.as_str())));
    let other = signed(&daemon_signer(&fx), order_body("flow-2"));
    let r = call(app, create(host, &t, Some("flow-req-1"), &other)).await;
    assert_paypal_error(&r, StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY");
    assert_eq!(issues(&r.body), ["DUPLICATE_REQUEST_ID"]);
    // Without the header every request creates an order.
    let r = call(app, create(host, &t, None, &body)).await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert_ne!(r.body["id"], id.as_str());

    // Prefer: return=minimal.
    let mut req = create(host, &t, None, &other);
    req.headers_mut().insert("prefer", "return=minimal".parse().unwrap());
    let r = call(app, req).await;
    let mut keys: Vec<&str> = r.body.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["aegis_local_sandbox", "id", "links", "status"]);

    let uri = format!("/paypal-sandbox/v2/checkout/orders/{id}");
    let r = call(app, order_request(host, "GET", &uri, Some(&t), None, None)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, order);
    let r = call(app, order_request(host, "GET", &format!("/paypal-sandbox/checkoutnow?token={id}"), None, None, None)).await;
    assert_eq!((r.status, r.body["status"].as_str()), (StatusCode::OK, Some("CREATED")));
    assert!(r.body["message"].as_str().unwrap().contains("approval is implied"));
    let r = call(app, order_request(host, "PATCH", &uri, Some(&t), None, Some(&json!([])))).await;
    assert_paypal_error(&r, StatusCode::METHOD_NOT_ALLOWED, "METHOD_NOT_SUPPORTED");

    // Capture (approval implied) → COMPLETED with a capture; replay → 200; second capture → 422.
    let capture_uri = format!("{uri}/capture");
    let r = call(app, order_request(host, "POST", &capture_uri, Some(&t), Some("cap-req-1"), Some(&json!({})))).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    assert_eq!(r.body["status"], "COMPLETED");
    let capture = &r.body["purchase_units"][0]["payments"]["captures"][0];
    assert_eq!(capture["status"], "COMPLETED");
    assert!(capture["id"].as_str().unwrap().starts_with("LOCALCAP-"));
    assert_eq!(capture["amount"], json!({"currency_code": "USD", "value": "42.00"}));
    assert_eq!(capture["custom_id"], body["purchase_units"][0]["custom_id"]);
    let rels: Vec<&str> = r.body["links"].as_array().unwrap().iter().map(|l| l["rel"].as_str().unwrap()).collect();
    assert_eq!(rels, ["self"]);
    let completed = r.body.clone();
    let r = call(app, order_request(host, "POST", &capture_uri, Some(&t), Some("cap-req-1"), None)).await;
    assert_eq!((r.status, &r.body), (StatusCode::OK, &completed));
    let r = call(app, order_request(host, "POST", &capture_uri, Some(&t), Some("cap-req-2"), None)).await;
    assert_paypal_error(&r, StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY");
    assert_eq!(issues(&r.body), ["ORDER_ALREADY_CAPTURED"]);
    let r = call(app, order_request(host, "GET", &uri, Some(&t), None, None)).await;
    assert_eq!(r.body["status"], "COMPLETED");

    // AUTHORIZE orders are created but not captured; unknown ids are 404.
    let mut authorize = order_body("auth-intent");
    authorize["intent"] = json!("AUTHORIZE");
    let r = call(app, create(host, &t, None, &authorize)).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let rels: Vec<&str> = r.body["links"].as_array().unwrap().iter().map(|l| l["rel"].as_str().unwrap()).collect();
    assert_eq!(rels, ["self", "approve", "update"]);
    let r = call(app, order_request(host, "POST", &format!("/paypal-sandbox/v2/checkout/orders/{}/capture", r.body["id"].as_str().unwrap()), Some(&t), None, None)).await;
    assert_eq!(issues(&r.body), ["ACTION_DOES_NOT_MATCH_INTENT"]);
    let r = call(app, order_request(host, "POST", "/paypal-sandbox/v2/checkout/orders/LOCAL-NOPE/capture", Some(&t), None, None)).await;
    assert_paypal_error(&r, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND");
    assert_eq!(issues(&r.body), ["INVALID_RESOURCE_ID"]);
    let r = call(app, order_request(host, "GET", "/paypal-sandbox/v2/nothing-here", Some(&t), None, None)).await;
    assert_paypal_error(&r, StatusCode::NOT_FOUND, "RESOURCE_NOT_FOUND");

    let stats = fx.state.paypal_gateway.stats_json();
    assert_eq!(stats["orders_captured"], 1);
    assert_eq!(stats["receipts_verified"].as_u64().unwrap() + stats["receipts_absent"].as_u64().unwrap(), stats["orders_created"].as_u64().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_unredacted_or_unsigned_bodies_are_refused() {
    let fx = Fixture::new("sbx-validate", "").await;
    let (app, host) = (&fx.app, fx.host.as_str());
    let t = token(app, host).await;

    // Schema violations → 400 INVALID_REQUEST.
    let r = call(app, create(host, &t, None, &json!({"purchase_units": []}))).await;
    assert_paypal_error(&r, StatusCode::BAD_REQUEST, "INVALID_REQUEST");
    assert_eq!(issues(&r.body), ["MISSING_REQUIRED_PARAMETER", "INVALID_ARRAY_MIN_ITEMS"]);
    assert_eq!(r.body["details"][0]["field"], "/intent");
    let mut bad = order_body("v-1");
    bad["purchase_units"][0]["amount"]["value"] = json!("42.0.0");
    let r = call(app, create(host, &t, None, &bad)).await;
    assert_eq!(issues(&r.body), ["INVALID_PARAMETER_SYNTAX"]);
    assert_eq!(r.body["details"][0]["field"], "/purchase_units/0/amount/value");
    let mut bad = order_body("v-2");
    bad["purchase_units"][0]["custom_id"] = json!("c".repeat(128));
    assert_eq!(issues(&call(app, create(host, &t, None, &bad)).await.body), ["INVALID_STRING_LENGTH"]);
    let req = Request::post("/paypal-sandbox/v2/checkout/orders")
        .header("host", host)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {t}"))
        .body(Body::from("{\"intent\":"))
        .unwrap();
    let r = call(app, req).await;
    assert_paypal_error(&r, StatusCode::BAD_REQUEST, "INVALID_REQUEST");
    assert_eq!(issues(&r.body), ["MALFORMED_REQUEST_JSON"]);
    let mut req = create(host, &t, None, &order_body("v-3"));
    req.headers_mut().insert("content-type", "text/plain".parse().unwrap());
    assert_paypal_error(&call(app, req).await, StatusCode::UNSUPPORTED_MEDIA_TYPE, "UNSUPPORTED_MEDIA_TYPE");
    let huge = json!({"intent": "CAPTURE", "pad": "x".repeat(BODY_LIMIT)});
    assert_eq!(call(app, create(host, &t, None, &huge)).await.status, StatusCode::PAYLOAD_TOO_LARGE);

    // Business rules → 422 UNPROCESSABLE_ENTITY.
    let mut bad = order_body("v-4");
    bad["purchase_units"][0]["amount"]["value"] = json!("50.00");
    let r = call(app, create(host, &t, None, &bad)).await;
    assert_paypal_error(&r, StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY");
    assert_eq!(issues(&r.body), ["AMOUNT_MISMATCH"]);
    let mut bad = order_body("v-5");
    bad["purchase_units"][0]["items"][0]["quantity"] = json!("3");
    assert_eq!(issues(&call(app, create(host, &t, None, &bad)).await.body), ["ITEM_TOTAL_MISMATCH"]);
    let mut bad = order_body("v-6");
    bad["purchase_units"][0]["amount"]["value"] = json!("42.001");
    assert_eq!(issues(&call(app, create(host, &t, None, &bad)).await.body), ["DECIMAL_PRECISION"]);

    // Unredacted sensitive data → 422, fields named, values never echoed.
    let mut leak = order_body("v-7");
    leak["purchase_units"][0]["description"] = json!("Deliver to jane.doe@example.com, card 4111 1111 1111 1111");
    let r = call(app, create(host, &t, None, &leak)).await;
    assert_paypal_error(&r, StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY");
    assert_eq!(issues(&r.body), ["AEGIS_UNREDACTED_SENSITIVE_DATA"]);
    assert_eq!(r.body["details"][0]["field"], "/purchase_units/0/description");
    let text = r.body.to_string();
    assert!(!text.contains("jane.doe") && !text.contains("4111"), "{text}");

    // Receipts: the daemon's key verifies; another key or a changed body is refused; none is flagged.
    let signer = daemon_signer(&fx);
    let r = call(app, create(host, &t, None, &signed(&signer, order_body("s-1")))).await;
    assert_eq!((r.status, r.body["aegis_receipt"]["verified"].as_bool()), (StatusCode::CREATED, Some(true)), "{}", r.body);
    assert_eq!(r.body["aegis_receipt"]["public_key"], signer.public_key_hex().as_str());
    let r = call(app, create(host, &t, None, &signed(&ReceiptSigner::generate(), order_body("s-2")))).await;
    assert_paypal_error(&r, StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY");
    assert_eq!(issues(&r.body), ["AEGIS_RECEIPT_SIGNATURE_INVALID"]);
    let mut tampered = signed(&signer, order_body("s-3"));
    tampered["purchase_units"][0]["reference_id"] = json!("s-3-changed");
    assert_eq!(issues(&call(app, create(host, &t, None, &tampered)).await.body), ["AEGIS_RECEIPT_SIGNATURE_INVALID"]);
    let r = call(app, create(host, &t, None, &order_body("s-4"))).await;
    assert_eq!(r.status, StatusCode::CREATED);
    assert_eq!((r.body["aegis_receipt"]["present"].as_bool(), r.body["aegis_receipt"]["verified"].as_bool()), (Some(false), Some(false)));

    let stats = fx.state.paypal_gateway.stats_json();
    assert_eq!(stats["orders_created"], 2);
    assert_eq!((stats["receipts_verified"].as_u64(), stats["receipts_absent"].as_u64()), (Some(1), Some(1)));
    assert!(stats["requests_rejected"].as_u64().unwrap() >= 12);
}
