//! `GET /api/config` never contains a secret value. Its own test binary, because it points
//! `AEGIS_DOTENV` (read by `DaemonOptions::from_env`) at a temporary `.env` before anything else runs.

mod common;

use axum::http::StatusCode;
use common::Fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_config_never_contains_a_secret_value() {
    let names = ["TAVILY_API_KEY", "PAYPAL_CLIENT_SECRET", "AEGIS_SLACK_WEBHOOK_URL"];
    let root = Fixture::scratch("config-secret");
    let dotenv = root.join(".env");
    let tavily = "tvly-dotenv-secret-31c5e8a0";
    let paypal = "paypal-dotenv-secret-9be47d12";
    let slack = "https://hooks.slack.invalid/services/T000/B000/dotenvsecret";
    std::fs::write(&dotenv, format!("TAVILY_API_KEY={tavily}\nPAYPAL_CLIENT_SECRET={paypal}\nAEGIS_SLACK_WEBHOOK_URL={slack}\n")).unwrap();
    // Single test in this binary: nothing else reads the environment concurrently.
    std::env::set_var("AEGIS_DOTENV", &dotenv);
    let mut opts = aegis_daemon::DaemonOptions::from_env();
    assert_eq!(opts.dotenv_path, dotenv, "DaemonOptions::from_env reads AEGIS_DOTENV");
    let defaults = Fixture::options(&root);
    opts.demo_root = defaults.demo_root;
    opts.signing_seed = defaults.signing_seed;
    opts.bench_path = defaults.bench_path;
    opts.port = common::PORT;
    // from_env() lets the environment override .env (the binary's behaviour); this test is about
    // .env values, so it reads .env only and runs the same whatever the shell exports.
    assert!(opts.env_overrides_dotenv);
    opts.env_overrides_dotenv = false;
    opts.keepwarm_interval_ms = 0;
    let fx = Fixture::with_options(root, opts).await;

    let (status, text) = fx.send(fx.get("/api/config")).await;
    assert_eq!(status, StatusCode::OK);
    for secret in [tavily, paypal, slack, "dotenvsecret"] {
        assert!(!text.contains(secret), "GET /api/config contains a secret value");
    }
    let config: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(config["path"], dotenv.display().to_string());
    for name in names {
        let key = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == name).unwrap();
        assert_eq!(key["set"], true, "{name}");
        assert_eq!(key["source"], "dotenv", "{name}");
        assert_eq!(key["secret"], true, "{name}");
        assert!(key.get("value").is_none());
    }
    assert_eq!(config["services"]["slack"]["enabled"], true);
    assert_eq!(config["services"]["paypal"]["enabled"], false, "client id missing");
    // Health and telemetry do not carry them either.
    for uri in ["/api/health", "/api/telemetry", "/api/workspace", "/api/paypal/scenarios", "/"] {
        let (_, body) = fx.send(fx.get(uri)).await;
        for secret in [tavily, paypal, slack] {
            assert!(!body.contains(secret), "{uri} contains a secret value");
        }
    }
}
