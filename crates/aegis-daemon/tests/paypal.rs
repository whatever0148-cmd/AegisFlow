//! PayPal pipeline through the daemon, offline: the fixture reads only its own empty `.env`, so no
//! Sandbox credentials are configured whatever the environment holds.

mod common;

use std::time::Duration;

use aegis_daemon::paypal::{OrderScenario, PayPalEngine};
use axum::http::StatusCode;
use common::{wait_for, Fixture};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_order_produces_a_verified_receipt_and_timeline() {
    let fx = Fixture::new("paypal-order", "").await;
    let mut rx = fx.subscribe();
    let (status, body) = fx.json(fx.post("/api/paypal/order", &json!({"scenario": "known_payee_42"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let order_id = body["order_id"].as_str().unwrap().to_string();
    let msg = wait_for(&mut rx, &mut Vec::new(), Duration::from_secs(30), |v| v["type"] == "paypal.order").await;
    let row = &msg["row"];
    assert_eq!(row["id"], order_id.as_str(), "order id == row id == timeline orderId");
    assert_eq!(row["receipt_verified"], true);
    assert!(row["signature"].as_str().unwrap().starts_with("c2pa:ed25519:"));
    assert_eq!(row["receipt_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(row["redactions"], 2);
    assert_eq!(row["unformatted_sensitive_fields"], false);
    assert_eq!(row["decision"], "Allow");
    assert_eq!(row["submission"], "not_sent");
    assert_eq!(row["submission_detail"], "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set");
    assert!(row["total_us"].as_f64().unwrap() > 0.0);
    let events = msg["events"].as_array().unwrap();
    let resources: Vec<&str> = events.iter().map(|e| e["resourceId"].as_str().unwrap()).collect();
    assert_eq!(resources, ["redact", "sign", "verify", "score", "policy"], "no submit span without a Sandbox call");
    assert!(events.iter().all(|e| e["orderId"] == order_id.as_str()));
    let offsets: Vec<f64> = events.iter().map(|e| e["startOffsetUs"].as_f64().unwrap()).collect();
    assert!(offsets.windows(2).all(|w| w[0] <= w[1]));
    let verify_span = events.iter().find(|e| e["resourceId"] == "verify").unwrap();
    assert_eq!(verify_span["durationUs"].as_f64().unwrap(), row["verify_us"].as_f64().unwrap());

    let (_, orders) = fx.json(fx.get("/api/paypal/orders")).await;
    assert_eq!(orders["rows"][0]["id"], order_id.as_str());
    assert_eq!(orders["timeline"]["events"].as_array().unwrap().len(), 5);
    let live = fx.state.live_json();
    assert_eq!(live["counters"]["orders"], 1);
    assert_eq!(live["samples"]["paypal_verify_us"]["n"], 1);
    let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
    assert_eq!(sc["sandbox"], json!({"enabled": false, "submitted": 0, "cap": 25}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bench_style_cases_keep_their_decisions() {
    let fx = Fixture::new("paypal-bench", "").await;
    // Phase-1 benchmark decisions (artifacts/aegis_phase1_telemetry.json, paypal_receipt.cases).
    let expected = [
        (OrderScenario::KnownPayee42, true, vec!["permit-low-risk"]),
        (OrderScenario::KnownPayee120, true, vec!["permit-low-risk"]),
        (OrderScenario::NewPayee480, false, vec![]),
        (OrderScenario::RedactionDisabled25, false, vec![]),
    ];
    let state = fx.state.clone();
    tokio::task::spawn_blocking(move || {
        for (scenario, allowed, policies) in expected {
            let prepared = state.paypal.prepare(scenario).unwrap();
            assert!(PayPalEngine::reverify(&prepared), "{scenario:?}: receipt does not re-verify");
            let (got_allowed, got_policies) = PayPalEngine::decision(&prepared);
            assert_eq!(got_allowed, allowed, "{scenario:?}");
            assert_eq!(got_policies, policies.iter().map(|p| p.to_string()).collect::<Vec<_>>(), "{scenario:?}");
            let (row, events) = state.paypal.complete(&state, prepared);
            assert_eq!(row.decision, if allowed { "Allow" } else { "Deny" });
            assert!(row.receipt_verified);
            assert_eq!(row.unformatted_sensitive_fields, scenario == OrderScenario::RedactionDisabled25);
            assert_eq!(row.submission, if allowed { "not_sent" } else { "blocked" }, "{scenario:?}: {}", row.submission_detail);
            let policy = events.iter().find(|e| e["resourceId"] == "policy").unwrap();
            assert_eq!(policy["cls"], if allowed { "ok" } else { "blocked" });
        }
    })
    .await
    .unwrap();
    let live = fx.state.live_json();
    assert_eq!(live["counters"]["orders"], 4);
    assert_eq!(live["counters"]["orders_blocked"], 2);
    assert_eq!(live["counters"]["orders_sent"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn orders_keep_the_newest_twenty_rows_with_their_timeline_events() {
    use aegis_daemon::paypal::ORDERS_KEPT;
    assert_eq!(ORDERS_KEPT, 20);
    let fx = Fixture::new("paypal-cap", "").await;
    let state = fx.state.clone();
    let total = ORDERS_KEPT + 5;
    let placed = tokio::task::spawn_blocking(move || {
        (0..total)
            .map(|i| {
                let scenario = OrderScenario::ALL[i % OrderScenario::ALL.len()];
                let prepared = state.paypal.prepare(scenario).unwrap();
                let (row, events) = state.paypal.complete(&state, prepared);
                (row.id, events.len())
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();

    let (status, orders) = fx.json(fx.get("/api/paypal/orders")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(orders["limit"], ORDERS_KEPT);
    let rows: Vec<&str> = orders["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    let kept: Vec<&str> = placed[total - ORDERS_KEPT..].iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(rows, kept, "the newest {ORDERS_KEPT} rows, oldest first");
    let events = orders["timeline"]["events"].as_array().unwrap();
    assert!(events.iter().all(|e| kept.contains(&e["orderId"].as_str().unwrap())), "timeline events of dropped rows remain");
    let expected: usize = placed[total - ORDERS_KEPT..].iter().map(|(_, n)| n).sum();
    assert_eq!(events.len(), expected, "every kept row keeps all of its timeline events");
    assert_eq!(fx.state.live_json()["counters"]["orders"], total as u64, "counters still count every order");
}
