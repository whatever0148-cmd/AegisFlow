//! Settings-derived configuration: `GET /api/config`, adapter configs built explicitly from
//! [`aegis_config::Settings`] (the daemon never mutates the process environment), and the
//! `POST /api/config/test` connectivity checks.
//!
//! Secret values leave this module only inside adapter configs and outbound request headers;
//! every string that goes back to the console passes through [`scrub`].

use std::time::{Duration, Instant};

use adapter_nebius_nemotron::{list_models, missing_models, RouterConfig, DEFAULT_BASE_URL, TAVILY_URL};
use adapter_paypal::{PayPalConfig, DEFAULT_BUDGET_CENTS, SANDBOX_BASE_URL};
use adapter_switch_mcp::ChatConfig;
use aegis_config::{Endpoint, Settings, Source, KEYS, NEMOTRON_PROVIDERS};
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

/// Settings the console may write with `POST /api/config`: every known one. `NEBIUS_BASE_URL`
/// decides where the stored `NEBIUS_API_KEY` is sent and the session token does not authenticate
/// local processes, so over HTTP it accepts only the allowlisted providers, and a provider change
/// must carry that provider's key ([`console_update_problem`]; API.md "Security").
pub fn console_writable(name: &str) -> bool {
    aegis_config::spec(name).is_some()
}

/// The extra rules `POST /api/config` applies on top of [`aegis_config::write_dotenv`] (the CLI and
/// the environment are not bound by them), checked against the settings `current` before the save:
///
/// * `NEBIUS_BASE_URL` may only be set to one of [`NEMOTRON_PROVIDERS`]; a loopback test endpoint
///   needs `aegis-daemon config set` or the environment.
/// * A save that changes the effective provider while `NEBIUS_API_KEY` is set must also set or
///   clear `NEBIUS_API_KEY` (and cannot do so when the key comes from the environment), so a key
///   is never sent to a provider it was not entered for.
///
/// Returns the 400 message, which never repeats a submitted value.
pub fn console_update_problem(current: &Settings, updates: &[(String, Option<String>)]) -> Option<String> {
    let (_, base) = updates.iter().rev().find(|(name, _)| name == "NEBIUS_BASE_URL")?;
    let next = match base {
        Some(value) => match aegis_config::nemotron_provider(value) {
            Some(provider) => provider,
            None => {
                return Some(format!(
                    "NEBIUS_BASE_URL from the console must be one of the Nemotron providers: {}; a loopback test endpoint can \
                     only be set with `aegis-daemon config set NEBIUS_BASE_URL` (harness/configure_keys.sh) or the environment",
                    aegis_config::allowed_providers_text()
                ))
            }
        },
        None => &NEMOTRON_PROVIDERS[0],
    };
    // A NEBIUS_BASE_URL from the environment wins over .env, so the save cannot change the provider.
    if current.source("NEBIUS_BASE_URL") == Some(Source::Env) || get(current, "NEBIUS_API_KEY").is_none() {
        return None;
    }
    let now = nebius_base_url(current);
    if aegis_config::nemotron_provider(&now).is_some_and(|p| p.id == next.id) {
        return None;
    }
    let label = next.label;
    if current.source("NEBIUS_API_KEY") == Some(Source::Env) {
        return Some(format!(
            "NEBIUS_API_KEY comes from the daemon's environment, so the console cannot switch the Nemotron provider to {label} \
             (the environment key would be sent to it); change NEBIUS_BASE_URL and NEBIUS_API_KEY in the environment instead"
        ));
    }
    if !updates.iter().any(|(name, _)| name == "NEBIUS_API_KEY") {
        return Some(format!(
            "switching the Nemotron provider to {label} needs that provider's API key in NEBIUS_API_KEY in the same save \
             (or clear NEBIUS_API_KEY), so the stored key is never sent to a different provider"
        ));
    }
    None
}

/// Why the configured Nemotron endpoint will not be given the API key, if it will not
/// ([`aegis_config::validate_base_url`]: an allowlisted provider, or http(s) on a loopback host).
/// The value is never repeated, only the rule and the allowed providers.
pub fn nebius_endpoint_problem(settings: &Settings) -> Option<String> {
    let source = settings.source("NEBIUS_BASE_URL").map(|s| s.name()).unwrap_or("default");
    aegis_config::validate_base_url(&nebius_base_url(settings)).err().map(|why| format!("NEBIUS_BASE_URL ({source}) rejected: {why}"))
}

/// The provider label for `services.nebius.provider`: an allowlisted provider, a loopback test
/// endpoint, or `None` for a refused URL.
fn nebius_provider_label(settings: &Settings) -> Option<&'static str> {
    match aegis_config::validate_base_url(&nebius_base_url(settings)) {
        Ok(Endpoint::Provider(p)) => Some(p.label),
        Ok(Endpoint::Loopback) => Some("loopback test endpoint"),
        Err(_) => None,
    }
}

/// `(enabled, reason)` for the remote recovery tiers: endpoint rule, then key, then tier models.
pub fn nebius_status(settings: &Settings) -> (bool, String) {
    if let Some(problem) = nebius_endpoint_problem(settings) {
        return (false, problem);
    }
    let (enabled, reason) = needs(settings, &["NEBIUS_API_KEY"]);
    if !enabled {
        return (enabled, reason);
    }
    let router = router_config(settings, Duration::from_secs(60));
    match missing_models(&router.base_url, router.tier1_model.as_deref(), router.tier2_model.as_deref()) {
        Some(missing) => (false, missing),
        None => (true, reason),
    }
}

/// The API key is only handed to the router when the endpoint passes the base-URL rule. Tier
/// models: `NEBIUS_TIER{1,2}_MODEL`, else the provider's confirmed default, else none.
pub fn router_config(settings: &Settings, verify_timeout: Duration) -> RouterConfig {
    let base_url = nebius_base_url(settings);
    let (tier1, tier2) = aegis_config::default_tier_models(&base_url);
    RouterConfig {
        nebius_api_key: get(settings, "NEBIUS_API_KEY").filter(|_| nebius_endpoint_problem(settings).is_none()),
        tier1_model: get(settings, "NEBIUS_TIER1_MODEL").or_else(|| tier1.map(str::to_string)),
        tier2_model: get(settings, "NEBIUS_TIER2_MODEL").or_else(|| tier2.map(str::to_string)),
        base_url,
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
            "provider": nebius_provider_label(settings),
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

/// The allowlisted Nemotron providers (`GET /api/config` `nemotron_providers`).
pub fn nemotron_providers_json() -> Value {
    NEMOTRON_PROVIDERS
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "label": p.label,
                "base_url": p.base_url,
                "signup_url": p.signup_url,
                "key_hint": p.key_hint,
                "tier1_model": p.tier1_model,
                "tier2_model": p.tier2_model,
            })
        })
        .collect()
}

/// `GET /api/config`: key presence and source only, never a value. `NEBIUS_BASE_URL` carries the
/// provider `choices` the console may set, and `NEBIUS_API_KEY` the selected provider's signup URL.
pub fn config_json(settings: &Settings) -> Value {
    let provider = aegis_config::nemotron_provider(&nebius_base_url(settings));
    let keys: Vec<Value> = settings
        .describe()
        .into_iter()
        .map(|status| {
            let signup_url = match (status.spec.name, provider) {
                ("NEBIUS_API_KEY", Some(p)) => Some(p.signup_url),
                _ => status.spec.signup_url,
            };
            let mut key = json!({
                "name": status.spec.name,
                "set": status.source.is_some(),
                "source": status.source.map(|s| s.name()),
                "secret": status.spec.secret,
                "console_writable": console_writable(status.spec.name),
                "hint": status.spec.hint,
                "signup_url": signup_url,
            });
            if status.spec.name == "NEBIUS_BASE_URL" {
                key["choices"] = NEMOTRON_PROVIDERS.iter().map(|p| json!({"value": p.base_url, "label": p.label})).collect();
            }
            key
        })
        .collect();
    json!({
        "path": settings.path().display().to_string(),
        "keys": keys,
        "nemotron_providers": nemotron_providers_json(),
        "services": services_json(settings),
    })
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
                    let listed = format!("GET {}/models: {} models listed", base.trim().trim_end_matches('/'), models.len());
                    let unset = missing_models(&base, router.tier1_model.as_deref(), router.tier2_model.as_deref());
                    let missing: Vec<String> = [(1, &router.tier1_model), (2, &router.tier2_model)]
                        .into_iter()
                        .filter_map(|(tier, id)| id.as_ref().map(|id| (tier, id)))
                        .filter(|(_, id)| !models.iter().any(|m| m == *id))
                        .map(|(tier, id)| format!("tier {tier} model {id}"))
                        .collect();
                    if missing.is_empty() && unset.is_none() {
                        (true, started, format!("{listed}; tier 1 and tier 2 models are served"))
                    } else {
                        // Name what the provider does serve, so the tier ids can be picked from it.
                        let nemotron: Vec<&str> =
                            models.iter().map(String::as_str).filter(|m| m.to_ascii_lowercase().contains("nemotron")).take(24).collect();
                        let mut parts = vec![listed];
                        parts.extend(unset);
                        if !missing.is_empty() {
                            parts.push(format!("not served: {}", missing.join(", ")));
                        }
                        parts.push(format!("Nemotron models listed: {}", if nemotron.is_empty() { "none".to_string() } else { nemotron.join(", ") }));
                        (false, started, parts.join("; "))
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
            // The local-sandbox credentials belong to the daemon's own gateway (api.rs tests it in
            // local mode); they are never sent to PayPal.
            if [&id, &secret].iter().any(|v| v.as_str() == adapter_paypal::LOCAL_SANDBOX_CREDENTIAL) {
                let detail = "local-sandbox credentials select the daemon's local Orders v2 gateway; set both to local-sandbox (or both to Sandbox credentials). Nothing was sent to PayPal";
                return json!({"service": service, "ok": false, "latency_ms": null, "detail": detail});
            }
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
        // Writable over HTTP, but only to the allowlisted providers (console_update_problem).
        assert_eq!(base["console_writable"], true);
        let choices: Vec<&str> = base["choices"].as_array().unwrap().iter().map(|c| c["value"].as_str().unwrap()).collect();
        assert_eq!(choices, ["https://api.tokenfactory.nebius.com/v1", "https://integrate.api.nvidia.com/v1", "https://openrouter.ai/api/v1"]);
        assert!(services_json(&s)["nebius"]["provider"].is_null(), "a refused URL has no provider");
    }

    #[test]
    fn tier_models_default_per_provider_and_are_required_where_unconfirmed() {
        let s = settings_from("NEBIUS_API_KEY=nb-local-only\nNEBIUS_BASE_URL=https://integrate.api.nvidia.com/v1\n", "nim");
        let router = router_config(&s, TEST_TIMEOUT);
        assert_eq!(router.nebius_api_key.as_deref(), Some("nb-local-only"), "an allowlisted provider gets the key");
        assert_eq!((router.tier1_model.as_deref(), router.tier2_model.as_deref()), (None, Some("nvidia/llama-3.1-nemotron-ultra-253b-v1")));
        let (enabled, reason) = nebius_status(&s);
        assert!(!enabled && reason.starts_with("NEBIUS_TIER1_MODEL not set") && reason.contains("NVIDIA NIM"), "{reason}");
        let config = config_json(&s);
        assert_eq!(config["services"]["nebius"]["provider"], "NVIDIA NIM");
        assert_eq!(config["services"]["nebius"]["enabled"], false);
        let key = config["keys"].as_array().unwrap().iter().find(|k| k["name"] == "NEBIUS_API_KEY").unwrap().clone();
        assert_eq!(key["signup_url"], "https://build.nvidia.com", "the key link follows the selected provider");
        assert_eq!(config["nemotron_providers"].as_array().unwrap().len(), 3);
        assert!(!config.to_string().contains("nb-local-only"));

        let s = settings_from("NEBIUS_API_KEY=nb-local-only\nNEBIUS_BASE_URL=https://openrouter.ai/api/v1/\n", "openrouter-none");
        let (enabled, reason) = nebius_status(&s);
        assert!(!enabled && reason.starts_with("NEBIUS_TIER1_MODEL / NEBIUS_TIER2_MODEL not set") && reason.contains("OpenRouter"), "{reason}");
        let s = settings_from(
            "NEBIUS_API_KEY=nb-local-only\nNEBIUS_BASE_URL=https://openrouter.ai/api/v1\nNEBIUS_TIER1_MODEL=vendor/a\nNEBIUS_TIER2_MODEL=vendor/b\n",
            "openrouter-set",
        );
        assert!(nebius_status(&s).0);
        assert_eq!(services_json(&s)["nebius"]["provider"], "OpenRouter");

        let s = settings_from("NEBIUS_API_KEY=nb-local-only\n", "nebius-default");
        let router = router_config(&s, TEST_TIMEOUT);
        assert_eq!((router.tier1_model.as_deref(), router.tier2_model.as_deref()), (Some(aegis_config::NEBIUS_TIER1_MODEL_ID), Some(aegis_config::NEBIUS_TIER2_MODEL_ID)));
        assert!(nebius_status(&s).0);
    }

    #[test]
    fn console_updates_accept_only_providers_and_pair_a_provider_change_with_its_key() {
        let up = |pairs: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
            pairs.iter().map(|(n, v)| (n.to_string(), v.map(str::to_string))).collect()
        };
        let empty = settings_from("", "update-empty");
        for bad in ["http://127.0.0.1:9/v1", "https://integrate.api.nvidia.com.evil.example/v1", "https://x@openrouter.ai/api/v1", "http://integrate.api.nvidia.com/v1"] {
            let problem = console_update_problem(&empty, &up(&[("NEBIUS_BASE_URL", Some(bad))])).expect(bad);
            assert!(problem.contains("https://openrouter.ai/api/v1") && !problem.contains(bad), "{problem}");
        }
        for provider in NEMOTRON_PROVIDERS {
            assert_eq!(console_update_problem(&empty, &up(&[("NEBIUS_BASE_URL", Some(provider.base_url))])), None, "{}", provider.id);
        }
        assert_eq!(console_update_problem(&empty, &up(&[("TAVILY_API_KEY", Some("tvly-x"))])), None);

        // A stored key may not be redirected to another provider by a save that does not replace it.
        let stored = settings_from("NEBIUS_API_KEY=nb-stored-key-5e6f\n", "update-stored");
        let problem = console_update_problem(&stored, &up(&[("NEBIUS_BASE_URL", Some("https://openrouter.ai/api/v1"))])).expect("needs the key");
        assert!(problem.contains("OpenRouter") && problem.contains("NEBIUS_API_KEY") && !problem.contains("nb-stored-key"), "{problem}");
        for key in [Some("sk-new-key-for-openrouter"), None] {
            let pairs = up(&[("NEBIUS_BASE_URL", Some("https://openrouter.ai/api/v1")), ("NEBIUS_API_KEY", key)]);
            assert_eq!(console_update_problem(&stored, &pairs), None, "{key:?}");
        }
        // Same provider (trailing slash) or the default again: no change, no key needed.
        assert_eq!(console_update_problem(&stored, &up(&[("NEBIUS_BASE_URL", Some("https://api.tokenfactory.nebius.com/v1/"))])), None);
        assert_eq!(console_update_problem(&stored, &up(&[("NEBIUS_BASE_URL", None)])), None);
        // Back to the default from another provider is a change too.
        let nim = settings_from("NEBIUS_API_KEY=nb-stored-key-5e6f\nNEBIUS_BASE_URL=https://integrate.api.nvidia.com/v1\n", "update-nim");
        let problem = console_update_problem(&nim, &up(&[("NEBIUS_BASE_URL", None)])).expect("default is a different provider");
        assert!(problem.contains("Nebius Token Factory"), "{problem}");
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
