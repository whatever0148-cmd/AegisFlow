//! PayPal pipeline through the daemon. The fixtures read only their own `.env`, so no Sandbox
//! credentials are configured whatever the environment holds. `Fixture::new` serves no local
//! gateway (PayPal reports `disabled`); `Fixture::serving` does, so allowed orders go over HTTP to the
//! daemon's own `/paypal-sandbox` (OAuth2 → create → capture). Nothing here reaches PayPal.

mod common;

use std::time::Duration;

use aegis_daemon::paypal::{OrderScenario, PayPalEngine};
use axum::http::StatusCode;
use common::{wait_for, Fixture};
use serde_json::{json, Value};

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
    let detail = row["submission_detail"].as_str().unwrap();
    assert!(detail.starts_with("PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set") && detail.contains("does not serve the local"), "{detail}");
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
    let sandbox = &sc["sandbox"];
    assert_eq!((sandbox["enabled"].as_bool(), sandbox["mode"].as_str()), (Some(false), Some("disabled")), "{sandbox}");
    assert_eq!((sandbox["submitted"].as_u64(), sandbox["cap"].as_u64(), sandbox["local_submitted"].as_u64()), (Some(0), Some(25), Some(0)));
    assert!(sandbox["gateway"].is_null());
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

fn submit_steps(row: &adapter_paypal::TelemetryRow) -> Vec<String> {
    row.submit_calls.iter().map(|c| c.step.clone()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_scenarios_submit_through_the_local_gateway_and_blocked_ones_never_reach_it() {
    let fx = Fixture::serving("paypal-local", "").await;
    let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
    assert_eq!(sc["sandbox"]["mode"], "local", "{}", sc["sandbox"]);
    assert_eq!(sc["sandbox"]["enabled"], true);
    assert_eq!(sc["sandbox"]["base_url"], format!("{}/paypal-sandbox", fx.base_url()));
    let (_, config) = fx.json(fx.get("/api/config")).await;
    assert_eq!(config["services"]["paypal"]["mode"], "local");

    let state = fx.state.clone();
    let rows = tokio::task::spawn_blocking(move || {
        OrderScenario::ALL.into_iter().map(|scenario| (scenario, state.paypal.complete(&state, state.paypal.prepare(scenario).unwrap()))).collect::<Vec<_>>()
    })
    .await
    .unwrap();
    let mut sent_ids = Vec::new();
    for (scenario, (row, events)) in &rows {
        // $42 comes first (empty spend history) and is allowed; $480 and the unredacted $25 are denied.
        // $120 is scored after $42 was really submitted, so the middleware's velocity features
        // (orders in the last hour, 24 h spend) count it and Cedar decides on that score.
        match scenario {
            OrderScenario::KnownPayee42 => assert_eq!(row.decision, "Allow"),
            OrderScenario::NewPayee480 | OrderScenario::RedactionDisabled25 => assert_eq!(row.decision, "Deny", "{scenario:?}"),
            OrderScenario::KnownPayee120 => eprintln!("$120 after a submitted $42: risk {} -> {}", row.risk_score, row.decision),
        }
        if row.decision == "Allow" {
            assert_eq!(row.submission, "sent", "{scenario:?}: {}", row.submission_detail);
            assert_eq!(row.submission_detail, "COMPLETED");
            assert_eq!(row.submission_target.as_deref(), Some("local"));
            let id = row.paypal_order_id.clone().unwrap();
            assert!(id.starts_with("LOCAL-"), "{id}");
            assert!(row.paypal_capture_id.as_deref().is_some_and(|c| c.starts_with("LOCALCAP-")));
            assert_eq!(submit_steps(row), ["oauth", "create", "capture"]);
            let statuses: Vec<u16> = row.submit_calls.iter().map(|c| c.status).collect();
            assert_eq!(statuses, [200, 201, 201]);
            assert!(row.submit_calls.iter().all(|c| c.duration_us > 0.0 && c.path.starts_with("/paypal-sandbox/")));
            let sum: f64 = row.submit_calls.iter().map(|c| c.duration_us).sum();
            assert!((row.submit_us.unwrap() - sum).abs() < 0.05, "submit_us is the sum of the calls");
            let submit: Vec<&Value> = events.iter().filter(|e| e["resourceId"] == "submit").collect();
            let names: Vec<&str> = submit.iter().map(|e| e["name"].as_str().unwrap()).collect();
            assert_eq!(names, [format!("{} submit oauth", row.reference), format!("{} submit create", row.reference), format!("{} submit capture", row.reference)]);
            assert!(submit.iter().all(|e| e["cls"] == "ok"));
            for (event, call) in submit.iter().zip(&row.submit_calls) {
                assert_eq!(event["durationUs"].as_f64().unwrap(), call.duration_us);
            }
            sent_ids.push(id);
        } else {
            assert_eq!(row.submission, "blocked", "{scenario:?}: Cedar blocks it before submission");
            assert!(row.submit_calls.is_empty() && row.paypal_order_id.is_none() && row.submission_target.is_none());
            assert!(events.iter().all(|e| e["resourceId"] != "submit"));
        }
    }

    // Only Cedar-allowed orders reached the gateway: one token and one create request each, nothing rejected.
    let sent = sent_ids.len() as u64;
    assert!(sent >= 1);
    let stats = fx.state.paypal_gateway.stats_json();
    assert_eq!(stats["tokens_issued"], sent, "{stats}");
    assert_eq!(stats["create_requests"], sent, "{stats}");
    assert_eq!(stats["orders_created"], sent);
    assert_eq!(stats["orders_captured"], sent);
    assert_eq!(stats["receipts_verified"], sent, "the middleware's custom_id receipts verify at the gateway");
    assert_eq!(stats["requests_rejected"], 0);

    let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
    assert_eq!((sc["sandbox"]["local_submitted"].as_u64(), sc["sandbox"]["submitted"].as_u64()), (Some(sent), Some(0)), "local orders use no remote slot");
    assert_eq!(sc["sandbox"]["gateway"]["orders_captured"], sent);
    assert_eq!(fx.state.live_json()["counters"]["orders_sent"], sent);

    // The stored order, read back over HTTP like any Orders v2 client would.
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let base = format!("{}/paypal-sandbox", fx.base_url());
    let id = sent_ids[0].clone();
    let order: Value = tokio::task::spawn_blocking(move || {
        let token: Value = agent
            .post(&format!("{base}/v1/oauth2/token"))
            .set("Authorization", "Basic bG9jYWwtc2FuZGJveDpsb2NhbC1zYW5kYm94")
            .send_string("grant_type=client_credentials")
            .unwrap()
            .into_json()
            .unwrap();
        let response = agent
            .get(&format!("{base}/v2/checkout/orders/{id}"))
            .set("Authorization", &format!("Bearer {}", token["access_token"].as_str().unwrap()))
            .call()
            .unwrap();
        assert!(response.header("x-aegis-local-sandbox").is_some());
        response.into_json().unwrap()
    })
    .await
    .unwrap();
    assert_eq!(order["status"], "COMPLETED");
    assert_eq!(order["aegis_local_sandbox"], true);
    assert_eq!(order["aegis_receipt"]["verified"], true);
    assert_eq!(order["purchase_units"][0]["amount"]["value"], "42.00");
    let description = order["purchase_units"][0]["description"].as_str().unwrap();
    assert!(description.contains("[REDACTED]") && !description.contains("jane.doe"), "{description}");
    eprintln!("sample local order: {}", serde_json::to_string_pretty(&order).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn literal_local_sandbox_credentials_select_the_gateway_over_the_console_api() {
    let fx = Fixture::serving("paypal-literal", "PAYPAL_CLIENT_ID=local-sandbox\nPAYPAL_CLIENT_SECRET=local-sandbox\n").await;
    assert_eq!(fx.state.paypal.mode(), aegis_daemon::paypal::SandboxMode::Local);
    let mut rx = fx.subscribe();
    let (status, body) = fx.json(fx.post("/api/paypal/order", &json!({"scenario": "known_payee_42"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let msg = wait_for(&mut rx, &mut Vec::new(), Duration::from_secs(30), |v| v["type"] == "paypal.order").await;
    assert_eq!(msg["row"]["submission"], "sent", "{}", msg["row"]);
    assert_eq!(msg["row"]["submission_target"], "local");
    assert_eq!(msg["row"]["submission_detail"], "COMPLETED");
    let (status, test) = fx.json(fx.post("/api/config/test", &json!({"service": "paypal"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(test["ok"], true, "{test}");
    assert!(test["detail"].as_str().unwrap().contains("local Orders v2 gateway"));
    assert!(test["latency_ms"].as_f64().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_or_mixed_credentials_select_remote_or_disabled() {
    // Remote: only the mode is checked; no order is placed, so nothing is sent to PayPal.
    let fx = Fixture::serving("paypal-remote", "PAYPAL_CLIENT_ID=sandbox-client-id-test\nPAYPAL_CLIENT_SECRET=sandbox-secret-test\n").await;
    let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
    assert_eq!(sc["sandbox"]["mode"], "remote", "{}", sc["sandbox"]);
    assert_eq!(sc["sandbox"]["base_url"], adapter_paypal::SANDBOX_BASE_URL);
    assert_eq!(sc["sandbox"]["cap"], 25);
    assert!(sc["sandbox"]["gateway"].is_null());
    let (_, config) = fx.json(fx.get("/api/config")).await;
    assert_eq!(config["services"]["paypal"]["mode"], "remote");
    assert!(!config.to_string().contains("sandbox-secret-test"));
    drop(fx);

    for (name, dotenv, reason) in [
        ("paypal-mixed", "PAYPAL_CLIENT_ID=local-sandbox\nPAYPAL_CLIENT_SECRET=sandbox-secret-test\n", "set both"),
        ("paypal-partial", "PAYPAL_CLIENT_SECRET=sandbox-secret-test\n", "PAYPAL_CLIENT_ID not set"),
    ] {
        let fx = Fixture::serving(name, dotenv).await;
        let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
        assert_eq!(sc["sandbox"]["mode"], "disabled", "{name}");
        assert!(sc["sandbox"]["reason"].as_str().unwrap().contains(reason), "{name}: {}", sc["sandbox"]);
        let state = fx.state.clone();
        let row = tokio::task::spawn_blocking(move || state.paypal.complete(&state, state.paypal.prepare(OrderScenario::KnownPayee42).unwrap()).0)
            .await
            .unwrap();
        assert_eq!(row.submission, "not_sent", "{name}");
        assert_eq!(fx.state.paypal_gateway.stats_json()["create_requests"], 0, "{name}");
    }
}
