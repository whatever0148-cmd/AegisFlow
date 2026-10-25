//! Settings-derived configuration: `GET /api/config`, adapter configs built explicitly from
//! [`aegis_config::Settings`] (the daemon never mutates the process environment), and the
//! `POST /api/config/test` connectivity checks.
//!
//! Secret values leave this module only inside adapter configs and outbound request headers;
//! every string that goes back to the console passes through [`scrub`].

use std::time::{Duration, Instant};

use adapter_nebius_nemotron::{list_models, RouterConfig, DEFAULT_BASE_URL, TAVILY_URL, TIER1_MODEL, TIER2_MODEL};
use adapter_paypal::{PayPalConfig, DEFAULT_BUDGET_CENTS, SANDBOX_BASE_URL};
use adapter_switch_mcp::ChatConfig;
use aegis_config::{Settings, KEYS};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::{json, Value};

use crate::util::round6;

pub const ROUTER_AGENT: &str = "build-recovery";
pub const PAYPAL_AGENT: &str = "procurement-agent";
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

fn get(settings: &Settings, name: &str) -> Option<String> {
    settings.get(name).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

pub fn nebius_base_url(settings: &Settings) -> String {
    get(settings, "NEBIUS_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Settings the console may write with `POST /api/config`. `NEBIUS_BASE_URL` decides where the
/// stored `NEBIUS_API_KEY` is sent and the session token does not authenticate local processes,
/// so it can only be set through the CLI or the environment (API.md "Security").
pub fn console_writable(name: &str) -> bool {
    name != "NEBIUS_BASE_URL"
}

/// Why the configured Nebius endpoint will not be given the API key, if it will not
/// ([`aegis_config::validate_base_url`]: https, or http on a loopback host). The value is never
/// repeated, only the rule.
pub fn nebius_endpoint_problem(settings: &Settings) -> Option<String> {
    let source = settings.source("NEBIUS_BASE_URL").map(|s| s.name()).unwrap_or("default");
    aegis_config::validate_base_url(&nebius_base_url(settings)).err().map(|why| format!("NEBIUS_BASE_URL ({source}) rejected: {why}"))
}

/// `(enabled, reason)` for the remote recovery tiers.
pub fn nebius_status(settings: &Settings) -> (bool, String) {
    match nebius_endpoint_problem(settings) {
        Some(problem) => (false, problem),
        None => needs(settings, &["NEBIUS_API_KEY"]),
    }
}

/// The API key is only handed to the router when the endpoint passes the base-URL rule.
pub fn router_config(settings: &Settings, verify_timeout: Duration) -> RouterConfig {
    RouterConfig {
        nebius_api_key: get(settings, "NEBIUS_API_KEY").filter(|_| nebius_endpoint_problem(settings).is_none()),
        base_url: nebius_base_url(settings),
        tier1_model: get(settings, "NEBIUS_TIER1_MODEL").unwrap_or_else(|| TIER1_MODEL.to_string()),
        tier2_model: get(settings, "NEBIUS_TIER2_MODEL").unwrap_or_else(|| TIER2_MODEL.to_string()),
        tavily_api_key: get(settings, "TAVILY_API_KEY"),
        tavily_url: TAVILY_URL.to_string(),
        agent_id: ROUTER_AGENT.to_string(),
        http_timeout: Duration::from_secs(120),
        verify_timeout,
    }
}

/// PayPal always targets the Sandbox base URL from this daemon.
pub fn paypal_config(settings: &Settings, redact: bool) -> PayPalConfig {
    PayPalConfig {
        client_id: get(settings, "PAYPAL_CLIENT_ID"),
        client_secret: get(settings, "PAYPAL_CLIENT_SECRET"),
        base_url: SANDBOX_BASE_URL.to_string(),
        budget_cents: DEFAULT_BUDGET_CENTS,
        agent_id: PAYPAL_AGENT.to_string(),
        redact,
        timeout: Duration::from_secs(30),
    }
}

pub fn chat_config(settings: &Settings) -> ChatConfig {
    ChatConfig {
        mattermost_webhook: get(settings, "AEGIS_MATTERMOST_WEBHOOK_URL"),
        slack_webhook: get(settings, "AEGIS_SLACK_WEBHOOK_URL"),
        callback_url: get(settings, "AEGIS_APPROVAL_CALLBACK_URL"),
    }
}

/// `{enabled, reason}` for a service that needs every name in `names`.
fn needs(settings: &Settings, names: &[&str]) -> (bool, String) {
    let missing: Vec<&str> = names.iter().copied().filter(|n| get(settings, n).is_none()).collect();
    if missing.is_empty() {
        let sources: Vec<String> = names
            .iter()
            .map(|n| format!("{n} set ({})", settings.source(n).map(|s| s.name()).unwrap_or("unknown")))
            .collect();
        (true, sources.join(", "))
    } else {
        (false, format!("{} not set", missing.join(" / ")))
    }
}

pub fn tavily_status(settings: &Settings) -> Value {
    let (enabled, reason) = needs(settings, &["TAVILY_API_KEY"]);
    json!({"enabled": enabled, "reason": reason})
}

pub fn services_json(settings: &Settings) -> Value {
    let (nebius, nebius_reason) = nebius_status(settings);
    let (paypal, paypal_reason) = needs(settings, &["PAYPAL_CLIENT_ID", "PAYPAL_CLIENT_SECRET"]);
    let (mattermost, mattermost_reason) = needs(settings, &["AEGIS_MATTERMOST_WEBHOOK_URL"]);
    let (slack, slack_reason) = needs(settings, &["AEGIS_SLACK_WEBHOOK_URL"]);
    let router = router_config(settings, Duration::from_secs(60));
    json!({
        "nebius": {
            "enabled": nebius,
            "reason": nebius_reason,
            "base_url": router.base_url,
            "tier1_model": router.tier1_model,
            "tier2_model": router.tier2_model,
        },
        "tavily": tavily_status(settings),
        "paypal": {"enabled": paypal, "reason": paypal_reason, "base_url": SANDBOX_BASE_URL},
        "mattermost": {"enabled": mattermost, "reason": mattermost_reason},
        "slack": {"enabled": slack, "reason": slack_reason},
    })
}

/// `GET /api/config`: key presence and source only, never a value.
pub fn config_json(settings: &Settings) -> Value {
    let keys: Vec<Value> = settings
        .describe()
        .into_iter()
        .map(|status| {
            json!({
                "name": status.spec.name,
                "set": status.source.is_some(),
                "source": status.source.map(|s| s.name()),
                "secret": status.spec.secret,
                "console_writable": console_writable(status.spec.name),
                "hint": status.spec.hint,
                "signup_url": status.spec.signup_url,
            })
        })
        .collect();
    json!({"path": settings.path().display().to_string(), "keys": keys, "services": services_json(settings)})
}

/// Replaces every configured secret value in `text` with `[REDACTED]`.
pub fn scrub(settings: &Settings, text: &str) -> String {
    let mut out = text.to_string();
    for spec in KEYS.iter().filter(|k| k.secret) {
        if let Some(value) = get(settings, spec.name) {
            if value.len() >= 4 {
                out = out.replace(&value, "[REDACTED]");
            }
        }
    }
    out
}

fn latency(started: Instant) -> f64 {
    round6(started.elapsed().as_secs_f64() * 1e3)
}

fn http_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(status, response) => {
            let body: String = response.into_string().unwrap_or_default().chars().take(200).collect();
            format!("HTTP {status}: {body}")
        }
        ureq::Error::Transport(t) => format!("transport error: {t}"),
    }
}

/// `POST /api/config/test`: one real, minimal call to a configured service. Blocking.
pub fn test_service(settings: &Settings, service: &str) -> Value {
    let not_configured = || json!({"service": service, "ok": false, "latency_ms": null, "detail": "not configured"});
    let (ok, started, detail) = match service {
        "nebius" => {
            let Some(key) = get(settings, "NEBIUS_API_KEY") else { return not_configured() };
            if let Some(problem) = nebius_endpoint_problem(settings) {
                return json!({"service": service, "ok": false, "latency_ms": null, "detail": format!("{problem}; the key was not sent")});
            }
            let base = nebius_base_url(settings);
            let router = router_config(settings, TEST_TIMEOUT);
            let started = Instant::now();
            match list_models(&base, &key, TEST_TIMEOUT) {
                Ok(models) => {
                    let listed = format!("GET {}/models: {} models listed", base.trim_end_matches('/'), models.len());
                    let missing: Vec<String> = [(1, &router.tier1_model), (2, &router.tier2_model)]
                        .into_iter()
                        .filter(|(_, id)| !models.iter().any(|m| m == *id))
                        .map(|(tier, id)| format!("tier {tier} model {id}"))
                        .collect();
                    if missing.is_empty() {
                        (true, started, format!("{listed}; tier 1 and tier 2 models are served"))
                    } else {
                        (false, started, format!("{listed}; not served: {}", missing.join(", ")))
                    }
                }
                Err(e) => (false, started, e.to_string()),
            }
        }
        "tavily" => {
            let Some(key) = get(settings, "TAVILY_API_KEY") else { return not_configured() };
            let agent = ureq::AgentBuilder::new().timeout(TEST_TIMEOUT).build();
            let started = Instant::now();
            let response = agent
                .post(TAVILY_URL)
                .set("Authorization", &format!("Bearer {key}"))
                .send_json(json!({"query": "AegisFlow connectivity check", "max_results": 1, "search_depth": "basic"}));
            match response {
                Ok(r) => {
                    let status = r.status();
                    let results = r.into_json::<Value>().ok().and_then(|v| v["results"].as_array().map(Vec::len)).unwrap_or(0);
                    (true, started, format!("POST {TAVILY_URL}: HTTP {status}, {results} result(s)"))
                }
                Err(e) => (false, started, http_error(e)),
            }
        }
        "paypal" => {
            let (Some(id), Some(secret)) = (get(settings, "PAYPAL_CLIENT_ID"), get(settings, "PAYPAL_CLIENT_SECRET")) else {
                return not_configured();
            };
            let url = format!("{SANDBOX_BASE_URL}/v1/oauth2/token");
            let agent = ureq::AgentBuilder::new().timeout(TEST_TIMEOUT).build();
            let started = Instant::now();
            let response = agent
                .post(&url)
                .set("Authorization", &format!("Basic {}", BASE64.encode(format!("{id}:{secret}"))))
                .set("Content-Type", "application/x-www-form-urlencoded")
                .send_string("grant_type=client_credentials");
            match response {
                Ok(r) => {
                    let body = r.into_json::<Value>().unwrap_or(Value::Null);
                    if body["access_token"].is_string() {
                        let expires = body["expires_in"].as_u64().map(|s| format!(", expires in {s} s")).unwrap_or_default();
                        (true, started, format!("Sandbox OAuth client-credentials token issued{expires}"))
                    } else {
                        (false, started, "Sandbox OAuth answered without an access_token".to_string())
                    }
                }
                Err(e) => (false, started, http_error(e)),
            }
        }
        _ => return json!({"service": service, "ok": false, "latency_ms": null, "detail": "unknown service"}),
    };
    json!({"service": service, "ok": ok, "latency_ms": latency(started), "detail": scrub(settings, &detail)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::mpsc;

    /// `.env`-only settings: credentials in the shell running the tests are never picked up.
    fn settings_from(text: &str, name: &str) -> Settings {
        let dir: PathBuf = std::env::temp_dir().join(format!("aegis-daemon-services-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        std::fs::write(&path, text).unwrap();
        Settings::load_isolated(&path).unwrap()
    }

    /// Answers `requests` GETs with an OpenAI-style model list; sends each request head back.
    fn models_endpoint(ids: &[&str], requests: usize) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let body = json!({"object": "list", "data": ids.iter().map(|id| json!({"id": id, "object": "model"})).collect::<Vec<_>>()}).to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..requests {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let mut stream = stream;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                let _ = tx.send(head);
            }
        });
        (base, rx)
    }

    #[test]
    fn unconfigured_services_report_reasons() {
        let s = settings_from("", "empty");
        let svc = services_json(&s);
        assert_eq!(svc["nebius"]["enabled"], false);
        assert_eq!(svc["nebius"]["reason"], "NEBIUS_API_KEY not set");
        assert!(svc["nebius"]["base_url"].as_str().is_some_and(|u| u.starts_with("https://")));
        assert_eq!(test_service(&s, "nebius")["detail"], "not configured");
        assert_eq!(svc["paypal"]["reason"], "PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set");
        assert_eq!(test_service(&s, "paypal")["ok"], false);
        assert_eq!(test_service(&s, "nope")["detail"], "unknown service");
    }

    #[test]
    fn scrub_removes_secret_values() {
        let s = settings_from("TAVILY_API_KEY=tvly-abcdef123456\n", "scrub");
        assert_eq!(scrub(&s, "bad key tvly-abcdef123456 rejected"), "bad key [REDACTED] rejected");
        let cfg = config_json(&s).to_string();
        assert!(!cfg.contains("tvly-abcdef123456"));
    }

    #[test]
    fn cleartext_remote_endpoint_never_receives_the_key() {
        let s = settings_from("NEBIUS_API_KEY=nb-owner-key-0001\nNEBIUS_BASE_URL=http://evil.example/v1\n", "cleartext");
        let (enabled, reason) = nebius_status(&s);
        assert!(!enabled);
        assert!(reason.starts_with("NEBIUS_BASE_URL (dotenv) rejected"), "{reason}");
        assert!(router_config(&s, TEST_TIMEOUT).nebius_api_key.is_none(), "the router must not get the key");
        let test = test_service(&s, "nebius");
        assert_eq!(test["ok"], false);
        assert!(test["latency_ms"].is_null(), "no call was made: {test}");
        assert!(!test.to_string().contains("evil.example") && !test.to_string().contains("nb-owner-key"));
        assert_eq!(services_json(&s)["nebius"]["enabled"], false);
        let keys = config_json(&s)["keys"].clone();
        let base = keys.as_array().unwrap().iter().find(|k| k["name"] == "NEBIUS_BASE_URL").unwrap();
        assert_eq!(base["console_writable"], false);
    }

    #[test]
    fn nebius_test_fails_when_a_tier_model_is_not_served() {
        let (base, requests) = models_endpoint(&["some-other/model-a", "test/tier1"], 2);
        let dotenv = format!("NEBIUS_API_KEY=nb-local-only\nNEBIUS_BASE_URL={base}\nNEBIUS_TIER1_MODEL=test/tier1\nNEBIUS_TIER2_MODEL=test/tier2\n");
        let s = settings_from(&dotenv, "models-missing");
        let result = test_service(&s, "nebius");
        assert_eq!(result["ok"], false, "{result}");
        let detail = result["detail"].as_str().unwrap();
        assert!(detail.contains("not served: tier 2 model test/tier2") && !detail.contains("tier 1 model"), "{detail}");
        assert!(result["latency_ms"].as_f64().is_some());
        let head = requests.recv().unwrap();
        assert!(head.starts_with("GET /v1/models"), "{head}");

        let s = settings_from(&format!("{dotenv}NEBIUS_TIER2_MODEL=some-other/model-a\n"), "models-served");
        let result = test_service(&s, "nebius");
        assert_eq!(result["ok"], true, "{result}");
        assert!(result["detail"].as_str().unwrap().contains("tier 1 and tier 2 models are served"));
        assert!(!result.to_string().contains("nb-local-only"));
        requests.recv().unwrap();
    }
}
