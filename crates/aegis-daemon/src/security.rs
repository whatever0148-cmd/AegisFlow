//! Request guard applied to every route, static files and `/ws` included (API.md "Security"):
//!
//! * `Host` must be `localhost:<port>`, `127.0.0.1:<port>` or `[::1]:<port>` (DNS-rebinding guard)
//!   → else 403.
//! * An `Origin`, if present, must be `http://` + one of those → else 403.
//! * `/ws` needs `?token=<token>`; every method other than GET/HEAD needs `X-Aegis-Token` → else 401,
//!   and a JSON `Content-Type` → else 415.
//! * Except under `/paypal-sandbox` (the local PayPal Orders v2 emulation, `paypal_sandbox`): the
//!   PayPal adapter calls it like a real API, with OAuth2 (`Basic` client credentials, then `Bearer`
//!   tokens) and a form-encoded token request, so it keeps the `Host` and `Origin` checks but not the
//!   browser token or the JSON-only rule. Its responses carry `x-aegis-local-sandbox`.
//!
//! The token is a CSRF / DNS-rebinding defence for browsers. `GET /` hands it to any client that
//! sends an allowed `Host`, so it does not authenticate other local processes; a setting that would
//! redirect a stored secret (`NEBIUS_BASE_URL`) only takes allowlisted providers over HTTP, and a
//! provider change must carry that provider's key (`services::console_update_problem`).
//!
//! Responses get `nosniff`, `no-referrer`, frame denial and the console's CSP.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::paypal_sandbox;
use crate::state::AppState;
use crate::util::constant_time_eq;

pub fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}

/// The policy the console is built for: AG Grid's Theming API injects `<style>` elements and uses
/// `data:` SVG icons; there are no inline scripts.
pub fn content_security_policy(port: u16) -> String {
    [
        "default-src 'self'".to_string(),
        "script-src 'self'".to_string(),
        "style-src 'self' 'unsafe-inline'".to_string(),
        "img-src 'self' data:".to_string(),
        format!("connect-src 'self' ws://127.0.0.1:{port} ws://localhost:{port} ws://[::1]:{port}"),
        "font-src 'self'".to_string(),
        "object-src 'none'".to_string(),
        "base-uri 'none'".to_string(),
        "form-action 'none'".to_string(),
        "frame-ancestors 'none'".to_string(),
    ]
    .join("; ")
}

/// Loopback names the daemon answers on (it binds 127.0.0.1 and, when available, [::1]).
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

fn host_allowed(host: &str, port: u16) -> bool {
    let host = host.trim().to_ascii_lowercase();
    LOOPBACK_HOSTS.iter().any(|h| host == format!("{h}:{port}"))
}

fn origin_allowed(origin: &str, port: u16) -> bool {
    LOOPBACK_HOSTS.iter().any(|h| origin == format!("http://{h}:{port}"))
}

/// `/paypal-sandbox` and everything below it.
pub fn is_sandbox_path(path: &str) -> bool {
    path.strip_prefix(paypal_sandbox::PREFIX).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|pair| pair.strip_prefix("token="))
}

pub async fn guard(State(state): State<Arc<AppState>>, req: Request<Body>, next: Next) -> Response {
    let port = state.opts.port;
    let headers = req.headers();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !host_allowed(host, port) {
        return error(StatusCode::FORBIDDEN, format!("Host must be 127.0.0.1:{port}, localhost:{port} or [::1]:{port}"));
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(|o| origin_allowed(o, port)) {
            return error(StatusCode::FORBIDDEN, format!("Origin must be http://127.0.0.1:{port}, http://localhost:{port} or http://[::1]:{port}"));
        }
    }
    let token = state.token.as_bytes();
    let sandbox = is_sandbox_path(req.uri().path());
    if sandbox {
        // OAuth2-protected like the API it emulates (see the module docs).
    } else if req.uri().path() == "/ws" {
        if !query_token(req.uri().query()).is_some_and(|t| constant_time_eq(t.as_bytes(), token)) {
            return error(StatusCode::UNAUTHORIZED, "missing or wrong token");
        }
    } else if req.method() != Method::GET && req.method() != Method::HEAD {
        let sent = headers.get("x-aegis-token").map(HeaderValue::as_bytes).unwrap_or_default();
        if !constant_time_eq(sent, token) {
            return error(StatusCode::UNAUTHORIZED, "missing or wrong X-Aegis-Token");
        }
        let json_body = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.trim().to_ascii_lowercase().starts_with("application/json"));
        if !json_body {
            return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/json");
        }
    }
    let is_api = req.uri().path().starts_with("/api/");
    let mut response = next.run(req).await;
    // Framework rejections (405, 413, a failed WS upgrade, ServeDir's 404) get the contract's
    // `{"error": ...}` body too.
    let status = response.status();
    let json_body = response.headers().get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"application/json"));
    if (status.is_client_error() || status.is_server_error()) && !json_body {
        response = error(status, status.canonical_reason().unwrap_or("request failed").to_lowercase());
    }
    let h = response.headers_mut();
    if sandbox {
        h.insert(paypal_sandbox::MARKER_HEADER, HeaderValue::from_static(paypal_sandbox::MARKER_VALUE));
    }
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if let Ok(csp) = HeaderValue::from_str(&content_security_policy(port)) {
        h.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    if is_api || !h.contains_key(header::CACHE_CONTROL) {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(if is_api { "no-store" } else { "no-cache" }));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_origin_rules() {
        assert!(host_allowed("localhost:4173", 4173));
        assert!(host_allowed("127.0.0.1:4173", 4173));
        assert!(host_allowed("LOCALHOST:4173", 4173));
        assert!(host_allowed("[::1]:4173", 4173));
        assert!(!host_allowed("[::1]", 4173));
        assert!(!host_allowed("[::2]:4173", 4173));
        assert!(!host_allowed("localhost", 4173));
        assert!(!host_allowed("evil.example:4173", 4173));
        assert!(!host_allowed("127.0.0.1:4174", 4173));
        assert!(origin_allowed("http://localhost:4173", 4173));
        assert!(origin_allowed("http://[::1]:4173", 4173));
        assert!(!origin_allowed("http://[::1]:4174", 4173));
        assert!(!origin_allowed("https://localhost:4173", 4173));
        assert!(!origin_allowed("null", 4173));
    }

    #[test]
    fn sandbox_paths() {
        assert!(is_sandbox_path("/paypal-sandbox"));
        assert!(is_sandbox_path("/paypal-sandbox/v2/checkout/orders"));
        assert!(!is_sandbox_path("/paypal-sandboxx/v1/oauth2/token"));
        assert!(!is_sandbox_path("/api/paypal-sandbox"));
    }

    #[test]
    fn token_from_query() {
        assert_eq!(query_token(Some("a=1&token=abc")), Some("abc"));
        assert_eq!(query_token(Some("tokens=abc")), None);
        assert_eq!(query_token(None), None);
    }
}
