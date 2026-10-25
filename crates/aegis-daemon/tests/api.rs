//! Security guard, configuration endpoints and static serving, through the real router.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Fixture, HOST};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_host_is_forbidden_everywhere() {
    let fx = Fixture::new("host", "").await;
    for uri in ["/api/health", "/api/runs", "/api/paypal/orders", "/api/approvals", "/", "/assets/app.js", "/ws", "/no-such-path"] {
        for host in ["evil.example:4173", "localhost", "127.0.0.1:9999", "localhost.evil.example:4173"] {
            let req = Request::get(uri).header("host", host).body(Body::empty()).unwrap();
            let (status, body) = fx.json(req).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri} with Host {host}");
            assert!(body["error"].as_str().is_some_and(|e| e.contains("Host")), "{body}");
        }
    }
    let mut req = fx.post("/api/workspace/run", &json!({"scenario": "safe_refactor"}));
    req.headers_mut().insert("host", "attacker.example:4173".parse().unwrap());
    assert_eq!(fx.send(req).await.0, StatusCode::FORBIDDEN);
    let req = Request::get("/api/health").header("host", "127.0.0.1:4173").body(Body::empty()).unwrap();
    assert_eq!(fx.send(req).await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn foreign_origin_is_forbidden() {
    let fx = Fixture::new("origin", "").await;
    for origin in ["http://evil.example", "https://localhost:4173", "null", "http://localhost:4174"] {
        let req = Request::get("/api/runs").header("host", HOST).header("origin", origin).body(Body::empty()).unwrap();
        assert_eq!(fx.send(req).await.0, StatusCode::FORBIDDEN, "GET /api/runs with Origin {origin}");
        let req = Request::get("/api/health").header("host", HOST).header("origin", origin).body(Body::empty()).unwrap();
        assert_eq!(fx.send(req).await.0, StatusCode::FORBIDDEN, "GET with Origin {origin}");
        let mut req = fx.post("/api/paypal/stream", &json!({"enabled": false, "interval_ms": 2000}));
        req.headers_mut().insert("origin", origin.parse().unwrap());
        assert_eq!(fx.send(req).await.0, StatusCode::FORBIDDEN, "POST with Origin {origin}");
    }
    for origin in ["http://localhost:4173", "http://127.0.0.1:4173"] {
        let req = Request::get("/api/health").header("host", HOST).header("origin", origin).body(Body::empty()).unwrap();
        assert_eq!(fx.send(req).await.0, StatusCode::OK, "Origin {origin}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_without_token_is_unauthorized() {
    let fx = Fixture::new("token", "").await;
    let body = json!({"scenario": "safe_refactor"}).to_string();
    let no_token = Request::post("/api/workspace/run").header("host", HOST).header("content-type", "application/json").body(Body::from(body.clone())).unwrap();
    let (status, err) = fx.json(no_token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(err["error"].is_string());
    let wrong = Request::post("/api/workspace/run")
        .header("host", HOST)
        .header("content-type", "application/json")
        .header("x-aegis-token", "0".repeat(64))
        .body(Body::from(body.clone()))
        .unwrap();
    assert_eq!(fx.send(wrong).await.0, StatusCode::UNAUTHORIZED);
    for uri in ["/api/config", "/api/config/test", "/api/approvals/a-000001/approve", "/api/paypal/order", "/api/workspace/reset"] {
        let req = Request::post(uri).header("host", HOST).header("content-type", "application/json").body(Body::from("{}")).unwrap();
        assert_eq!(fx.send(req).await.0, StatusCode::UNAUTHORIZED, "{uri}");
    }
    // Right token, wrong content type.
    let req = Request::post("/api/paypal/stream")
        .header("host", HOST)
        .header("content-type", "text/plain")
        .header("x-aegis-token", &fx.state.token)
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(fx.send(req).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    // WebSocket: no token and a wrong token are refused before the upgrade.
    for uri in ["/ws".to_string(), "/ws?token=nope".to_string()] {
        let req = Request::get(uri.as_str()).header("host", HOST).body(Body::empty()).unwrap();
        assert_eq!(fx.send(req).await.0, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let req = Request::get(format!("/ws?token={}", fx.state.token)).header("host", HOST).body(Body::empty()).unwrap();
    assert_ne!(fx.send(req).await.0, StatusCode::UNAUTHORIZED, "the right token passes the guard (then fails the upgrade: not a WS request)");
    // Nothing ran.
    assert_eq!(fx.state.live_json()["counters"]["runs"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn post_config_writes_0600_and_rejects_unknown_names() {
    let fx = Fixture::new("config", "# keep this comment\n").await;
    let dotenv = fx.root.join(".env");
    let (status, err) = fx.json(fx.post("/api/config", &json!({"values": {"NOT_A_KNOWN_KEY": "x"}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("NOT_A_KNOWN_KEY"));
    assert_eq!(std::fs::read_to_string(&dotenv).unwrap(), "# keep this comment\n", "nothing written on rejection");
    let (status, _) = fx.json(fx.post("/api/config", &json!({"values": {"NEBIUS_TIER1_MODEL": "a\nB=c"}}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "line breaks rejected");

    let mut rx = fx.subscribe();
    let secret = "tvly-unit-test-value-7f3a9c";
    let (status, config) = fx.json(fx.post("/api/config", &json!({"values": {"NEBIUS_TIER1_MODEL": "test/tier1-model", "TAVILY_API_KEY": secret}}))).await;
    assert_eq!(status, StatusCode::OK, "{config}");
    let mode = std::fs::metadata(&dotenv).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let text = std::fs::read_to_string(&dotenv).unwrap();
    assert!(text.starts_with("# keep this comment\n"));
    assert!(text.contains("NEBIUS_TIER1_MODEL=test/tier1-model") && text.contains(secret));
    assert!(!config.to_string().contains(secret), "POST /api/config response leaks the secret");
    let tavily = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == "TAVILY_API_KEY").unwrap();
    assert_eq!((tavily["set"].clone(), tavily["source"].clone(), tavily["secret"].clone()), (json!(true), json!("dotenv"), json!(true)));
    assert_eq!(config["services"]["nebius"]["tier1_model"], "test/tier1-model");
    assert_eq!(config["services"]["tavily"]["enabled"], true);
    let changed = common::wait_for(&mut rx, &mut Vec::new(), std::time::Duration::from_secs(5), |v| v["type"] == "config.changed").await;
    assert!(!changed.to_string().contains(secret), "config.changed leaks the secret");
    let (_, get) = fx.json(fx.get("/api/config")).await;
    assert!(!get.to_string().contains(secret), "GET /api/config leaks the secret");

    // A whitespace-only value is refused (400) and the stored key is kept; only "" unsets.
    for blank in [" ", "   ", "\t"] {
        let before = std::fs::read_to_string(&dotenv).unwrap();
        let (status, err) = fx.json(fx.post("/api/config", &json!({"values": {"TAVILY_API_KEY": blank}}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{blank:?}: {err}");
        assert!(err["error"].as_str().unwrap().contains("TAVILY_API_KEY"), "{err}");
        assert_eq!(std::fs::read_to_string(&dotenv).unwrap(), before, "{blank:?}: nothing written");
    }
    assert!(std::fs::read_to_string(&dotenv).unwrap().contains(secret), "the stored key survives");

    // "" unsets.
    let (status, _) = fx.json(fx.post("/api/config", &json!({"values": {"TAVILY_API_KEY": ""}}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!std::fs::read_to_string(&dotenv).unwrap().contains("TAVILY_API_KEY"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn config_test_reports_not_configured_without_keys() {
    let fx = Fixture::new("cfgtest", "").await;
    for service in ["nebius", "tavily", "paypal"] {
        let (status, body) = fx.json(fx.post("/api/config/test", &json!({"service": service}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"service": service, "ok": false, "latency_ms": null, "detail": "not configured"}));
    }
    let (status, _) = fx.json(fx.post("/api/config/test", &json!({"service": "mattermost"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn static_serving_substitutes_token_and_refuses_dev_api_md_and_traversal() {
    let fx = Fixture::new("static", "").await;
    let (status, html) = fx.send(fx.get("/")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("%%AEGIS_TOKEN%%"));
    assert!(html.contains(&format!("<meta name=\"aegis-token\" content=\"{}\">", fx.state.token)));
    let response = tower::ServiceExt::oneshot(fx.app.clone(), fx.get("/index.html")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let csp = response.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");

    for ok in ["/assets/app.js", "/assets/console.css", "/assets/favicon.svg", "/vendor/ag-grid-community/LICENSE.txt"] {
        assert_eq!(fx.send(fx.get(ok)).await.0, StatusCode::OK, "{ok}");
    }
    let response = tower::ServiceExt::oneshot(fx.app.clone(), fx.get("/assets/app.js")).await.unwrap();
    let ct = response.headers()["content-type"].to_str().unwrap();
    assert!(ct.contains("javascript"), "{ct}");

    for refused in [
        "/dev/contract-mock.mjs",
        "/API.md",
        "/apps/aegis-console/API.md",
        "/assets/../API.md",
        "/assets/../dev/contract-mock.mjs",
        "/assets/%2e%2e/API.md",
        "/assets/..%2fAPI.md",
        "/assets/%2e%2e%2fdev%2fcontract-mock.mjs",
        "/vendor/README.md",
        "/vendor/ag-grid-community/../../API.md",
        "/assets/../../../../etc/passwd",
    ] {
        let (status, body) = fx.send(fx.get(refused)).await;
        assert!(status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST, "{refused} -> {status}");
        assert!(!body.contains("Aegis Console ↔ aegis-daemon contract"), "{refused} served API.md");
        assert!(!body.contains("contract mock"), "{refused} served dev/");
        assert!(!body.contains("root:"), "{refused} escaped the console directory");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_and_telemetry_shapes() {
    let fx = Fixture::new("health", "").await;
    let (status, health) = fx.json(fx.get("/api/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["ok"], true);
    assert_eq!(health["mock"], false);
    assert_eq!(health["compute_units"], "cpuAndNeuralEngine");
    let plan = &health["model"]["compute_plan"];
    let total = plan["neural_engine"].as_u64().unwrap() + plan["cpu"].as_u64().unwrap() + plan["gpu"].as_u64().unwrap();
    assert_eq!(total, health["model"]["operations_dispatched"].as_u64().unwrap(), "placement counts come from MLComputePlan");
    assert_eq!(health["model"]["input_shape"].as_array().unwrap().len(), 4);

    // No artifact at bench_path: bench is null.
    let (_, t) = fx.json(fx.get("/api/telemetry")).await;
    assert!(t["bench"].is_null());
    assert!(t["bench_path"].as_str().unwrap().ends_with("bench.json"));
    assert_eq!(t["live"]["samples"].as_object().unwrap().len(), aegis_daemon::live::SAMPLE_KEYS.len());
    // An artifact appears: read at request time.
    std::fs::write(fx.root.join("bench.json"), json!({"generated_at": "2026-10-25T08:06:37Z", "targets_summary": []}).to_string()).unwrap();
    let (_, t) = fx.json(fx.get("/api/telemetry")).await;
    assert_eq!(t["bench_generated_at"], "2026-10-25T08:06:37Z");

    // The fixture disables the keep-warm; health says so.
    assert_eq!(health["ane_keepwarm"]["enabled"], false);
    assert_eq!(health["ane_keepwarm"]["pings"], 0);

    let (_, ws) = fx.json(fx.get("/api/workspace")).await;
    assert_eq!(ws["scenarios"].as_array().unwrap().len(), 4);
    let mass_delete = ws["scenarios"].as_array().unwrap().iter().find(|s| s["id"] == "mass_delete").unwrap();
    let description = mass_delete["description"].as_str().unwrap();
    assert!(description.contains(&format!("{} fixtures", aegis_daemon::demo::FIXTURES)), "{description}");
    assert!(description.contains("single_event_risk"), "{description}");
    assert!(ws["files"].as_u64().unwrap() > 30);
    assert!(ws["pool"]["capacity"].as_u64().unwrap() >= 1);
    let (_, sc) = fx.json(fx.get("/api/paypal/scenarios")).await;
    assert_eq!(sc["scenarios"].as_array().unwrap().len(), 4);
    assert_eq!(sc["sandbox"]["cap"], 25);
    let (_, orders) = fx.json(fx.get("/api/paypal/orders")).await;
    assert!(orders["columnDefs"].as_array().unwrap().iter().any(|c| c["field"] == "risk_score"));
    assert_eq!(orders["timeline"]["resources"].as_array().unwrap().len(), 6);
    assert_eq!(orders["limit"], aegis_daemon::paypal::ORDERS_KEPT);
    let (status, runs) = fx.json(fx.get("/api/runs")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs, json!({"runs": [], "limit": aegis_daemon::runs::RUNS_KEPT}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stream_interval_is_clamped() {
    let fx = Fixture::new("stream", "").await;
    let (status, body) = fx.json(fx.post("/api/paypal/stream", &json!({"enabled": false, "interval_ms": 5}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"enabled": false, "interval_ms": 1000}));
    let (_, body) = fx.json(fx.post("/api/paypal/stream", &json!({"enabled": false, "interval_ms": 2500}))).await;
    assert_eq!(body["interval_ms"], 2500);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_config_saves_all_land_intact() {
    let fx = Fixture::new("config-race", "# keep\n").await;
    let dotenv = fx.root.join(".env");
    let names = ["NEBIUS_TIER1_MODEL", "NEBIUS_TIER2_MODEL", "AEGIS_APPROVAL_CALLBACK_URL", "TAVILY_API_KEY"];
    for round in 0..6 {
        let values: Vec<String> = (0..names.len()).map(|i| format!("r{round}-{}", char::from(b'a' + i as u8).to_string().repeat(50_000))).collect();
        let barrier = Arc::new(tokio::sync::Barrier::new(names.len()));
        let mut saves = Vec::new();
        for (name, value) in names.iter().zip(&values) {
            let req = fx.post("/api/config", &json!({"values": {*name: value}}));
            let (app, barrier) = (fx.app.clone(), barrier.clone());
            saves.push(tokio::spawn(async move {
                barrier.wait().await;
                app.oneshot(req).await.unwrap().status()
            }));
        }
        for save in saves {
            assert_eq!(save.await.unwrap(), StatusCode::OK, "round {round}");
        }
        let text = std::fs::read_to_string(&dotenv).unwrap();
        assert_eq!(text.lines().count(), names.len() + 1, "round {round}: a stray or missing line");
        let parsed: std::collections::BTreeMap<String, String> = aegis_config::parse_dotenv(&text).into_iter().collect();
        for (name, value) in names.iter().zip(&values) {
            assert!(parsed.get(*name) == Some(value), "round {round}: {name} lost or damaged");
        }
        // The in-memory settings were swapped in the same order as the file writes.
        let (_, config) = fx.json(fx.get("/api/config")).await;
        for name in names {
            let key = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == name).unwrap();
            assert_eq!(key["set"], true, "round {round}: GET /api/config is stale for {name}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nebius_base_url_cannot_be_set_over_http() {
    let fx = Fixture::new("base-url", "NEBIUS_API_KEY=nb-FAKE-OWNER-KEY-0001\n").await;
    let dotenv = fx.root.join(".env");
    for values in [
        json!({"NEBIUS_BASE_URL": "http://127.0.0.1:9/v1"}),
        json!({"NEBIUS_BASE_URL": "https://api.tokenfactory.nebius.com/v1"}),
        json!({"NEBIUS_API_KEY": "nb-attacker", "NEBIUS_BASE_URL": "http://127.0.0.1:9/v1"}),
    ] {
        let (status, body) = fx.json(fx.post("/api/config", &json!({ "values": values }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{values}: {body}");
        assert!(body["error"].as_str().unwrap().contains("aegis-daemon config set NEBIUS_BASE_URL"), "{body}");
        assert_eq!(std::fs::read_to_string(&dotenv).unwrap(), "NEBIUS_API_KEY=nb-FAKE-OWNER-KEY-0001\n", "nothing written");
    }
    let (_, config) = fx.json(fx.get("/api/config")).await;
    let base = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == "NEBIUS_BASE_URL").unwrap();
    assert_eq!(base["console_writable"], false);
    let key = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == "NEBIUS_API_KEY").unwrap();
    assert_eq!(key["console_writable"], true);

    // Unsetting (back to the default https endpoint) is allowed.
    std::fs::write(&dotenv, "NEBIUS_API_KEY=nb-FAKE-OWNER-KEY-0001\nNEBIUS_BASE_URL=http://127.0.0.1:9/v1\n").unwrap();
    let (status, config) = fx.json(fx.post("/api/config", &json!({"values": {"NEBIUS_BASE_URL": ""}}))).await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert!(!std::fs::read_to_string(&dotenv).unwrap().contains("NEBIUS_BASE_URL"));
    assert!(config["services"]["nebius"]["base_url"].as_str().unwrap().starts_with("https://"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepwarm_pings_idle_models_and_reports_them_in_health_only() {
    let root = Fixture::scratch("keepwarm");
    std::fs::write(root.join(".env"), "").unwrap();
    let mut opts = Fixture::options(&root);
    opts.keepwarm_interval_ms = 20;
    let fx = Fixture::with_options(root, opts).await;
    let mut pings = fx.state.keepwarm.subscribe();
    // Both models are idle from the start, so every round pings both.
    tokio::time::timeout(Duration::from_secs(60), pings.wait_for(|n| *n >= 4)).await.expect("keep-warm pings within 60 s").unwrap();
    let (status, health) = fx.json(fx.get("/api/health")).await;
    assert_eq!(status, StatusCode::OK);
    let kw = &health["ane_keepwarm"];
    assert_eq!(kw["enabled"], true);
    assert_eq!(kw["interval_ms"], 20);
    assert!(kw["pings"].as_u64().unwrap() >= 4, "{kw}");
    for model in ["workspace", "paypal"] {
        assert!(kw["models"][model]["pings"].as_u64().unwrap() >= 1, "{kw}");
        assert!(kw["models"][model]["last_ping_ms"].as_f64().unwrap() > 0.0, "{kw}");
        assert_eq!(kw["models"][model]["failures"], 0);
    }
    // Pings are not samples.
    assert_eq!(fx.state.live_json()["samples"]["ane_score_ms"]["n"], 0);
}
