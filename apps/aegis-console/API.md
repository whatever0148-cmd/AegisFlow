# Aegis Console ↔ aegis-daemon contract

The console (`apps/aegis-console`, static HTML/CSS/JS, no build step) is served by
`crates/aegis-daemon` (Axum + Tokio) on `http://127.0.0.1:4173`. This file is the single
source of truth for both sides; change it first when the API changes.

Example values below are type placeholders (`<number>`, `<integer>`), never measurements.

## Ground rules

- Every number the console shows comes from the daemon, which measures it from a real
  operation (or reads it from the benchmark artifact). The console never hardcodes a latency,
  score, count or target result. Labels distinguish **live** (this daemon's own operations)
  from **bench** (`artifacts/aegis_phase1_telemetry.json`).
- External services (the Nemotron provider, Tavily, PayPal Sandbox, Mattermost, Slack) are used only when
  their credentials are configured. When they are not, the daemon reports
  `enabled: false` with a reason and the console shows that state; it never simulates a
  remote reply. The one emulation is the daemon's own local PayPal Orders v2 gateway
  (`/paypal-sandbox`, see "Local PayPal Orders v2 gateway"): it is labelled `mode: "local"`
  everywhere, marks every response and order as local, and never presents itself as PayPal.
- Secret values are write-only: no endpoint ever returns a secret value, masked or not.

## Security

- The daemon binds `127.0.0.1:4173`, and `[::1]:4173` when it can: browsers try `[::1]` first
  for `localhost`, and an unbound IPv6 loopback port could be taken by another local process. If
  `[::1]:4173` is already taken the daemon logs a warning and serves IPv4 only. Open
  `http://127.0.0.1:4173` (what `harness/console.sh` and the startup message use).
- Every request must carry `Host: 127.0.0.1:4173`, `localhost:4173` or `[::1]:4173`
  (DNS-rebinding guard). If an `Origin` header is present it must be `http://` plus one of those.
  Anything else gets `403`.
- The daemon generates a random token per process and substitutes it for the literal
  `%%AEGIS_TOKEN%%` in `index.html` (`<meta name="aegis-token" content="%%AEGIS_TOKEN%%">`).
  Every `POST` must send `X-Aegis-Token: <token>` and `Content-Type: application/json`;
  the WebSocket connects to `/ws?token=<token>`. Missing or wrong token → `401`.
- Exception: `/paypal-sandbox/*` (the local PayPal Orders v2 gateway) is not behind the browser
  token or the JSON-only rule, because the PayPal adapter calls it like the real API (OAuth2
  `Basic` client credentials with a form body, then `Bearer` tokens). It keeps the `Host` and
  `Origin` checks, is served only on the daemon's loopback listener, and answers errors in PayPal's
  shape (below), not `{"error"}`.
- The token is a CSRF and DNS-rebinding defence for browsers. It does **not** authenticate local
  processes: `GET /` hands it to any client that sends an allowed `Host`. So any local process
  that can reach loopback can drive this API, so the setting that decides where a stored secret
  is sent is restricted over HTTP. `POST /api/config` sets `NEBIUS_BASE_URL` only to one of the
  allowlisted Nemotron providers below (the `choices` of that key in `GET /api/config`); a
  loopback test endpoint or any other value → `400` (set loopback with
  `aegis-daemon config set NEBIUS_BASE_URL`, `harness/configure_keys.sh`, or the environment).
  A save that changes the effective provider while `NEBIUS_API_KEY` is set must also set (or
  clear, with `""`) `NEBIUS_API_KEY` in the same request → else `400`, and is refused when the key
  comes from the daemon's environment; so a stored key never follows a provider change it was not
  entered for. Unsetting `NEBIUS_BASE_URL` selects the default provider (Nebius Token Factory) and
  counts as a change. Keys carry `console_writable` in `GET /api/config` (all `true` today).
- Nemotron providers (`aegis_config::NEMOTRON_PROVIDERS`, matched exactly after removing one
  trailing `/`; scheme, host, case and path must match):

  | id | label | `NEBIUS_BASE_URL` | keys |
  |---|---|---|---|
  | `nebius` | Nebius Token Factory (default) | `https://api.tokenfactory.nebius.com/v1` | https://tokenfactory.nebius.com/ |
  | `nvidia-nim` | NVIDIA NIM | `https://integrate.api.nvidia.com/v1` | https://build.nvidia.com (`nvapi-…`) |
  | `openrouter` | OpenRouter | `https://openrouter.ai/api/v1` | https://openrouter.ai/keys |

  `NEBIUS_API_KEY` (the key for the selected provider; the name is kept for compatibility) is
  only sent to one of these or to `http(s)://127.0.0.1|localhost|[::1][:port]/…` (local test
  endpoints, accepted from `.env`, the CLI or the environment). Any other value, from `.env` or the
  environment (look-alike hosts, `http://` to a provider host, user info, other paths), disables the
  remote recovery tiers with a reason that lists the allowed providers, and the key is not sent.
- Tier models: `NEBIUS_TIER1_MODEL` / `NEBIUS_TIER2_MODEL`, else the provider's default where one
  is confirmed: Nebius `nvidia/Llama-3_3-Nemotron-Super-49B-v1` / `nvidia/Llama-3_1-Nemotron-Ultra-253B-v1`
  (the ids used since phase 1); NVIDIA NIM tier 2 `nvidia/llama-3.1-nemotron-ultra-253b-v1` (listed by
  its public `GET /models`, which does not list a Llama-Nemotron Super 49B, so tier 1 is required);
  OpenRouter none (its public `GET /models` lists neither, so both are required). A missing tier model disables the remote
  tiers (`services.nebius.reason` names it) and `tier1_model` / `tier2_model` are `null`.
- Error bodies are `{"error": "<message>"}` with a 4xx/5xx status (except under `/paypal-sandbox`).

## Shared shapes

```jsonc
// Stat: rolling statistics over the daemon's own samples (last 512)
{ "n": <integer>, "p50": <number>, "p95": <number>, "last": <number> }   // null fields when n == 0

// LiveStats
{
  "samples": {
    "branch_acquire_ms": Stat,  // WarmBranchPool acquire (or cold fallback) as seen by the caller
    "branch_cold_ms": Stat,     // cold clonefile when the pool missed
    "edit_ms": Stat, "diff_ms": Stat,
    "ane_score_ms": Stat,       // CoreML prediction, context-window model
    "cedar_eval_us": Stat,      // Authorizer::is_authorized
    "commit_swap_ms": Stat,     // renameatx_np RENAME_SWAP
    "rollback_detach_ms": Stat, // synchronous caller-visible rollback
    "reap_ms": Stat,            // background reaper, per detached tree
    "paypal_total_us": Stat, "paypal_verify_us": Stat
  },
  "counters": {
    "runs": <integer>, "commits": <integer>, "rollbacks": <integer>, "approvals_pending": <integer>,
    "orders": <integer>, "orders_blocked": <integer>, "orders_sent": <integer>,
    "pool_hits": <integer>, "pool_misses": <integer>
  }
  // ane_score_ms has every real prediction as measured, cold ones included; keep-warm pings
  // (see GET /api/health) are never samples.
}

// Approval
{
  "id": "a-000001", "run_id": "r-000003" /* or null */, "kind": "workspace" | "paypal",
  "title": "Mass delete in demo workspace", "summary": "…", "risk": <number 0-100>,
  "files_changed": <integer>, "excerpt": "redacted diff excerpt",
  "determining_policies": ["forbid-high-impact-without-team-approval"],
  "created_at": "<ISO8601>", "status": "pending" | "approved" | "rejected",
  "resolved_by": null, "resolved_at": null,
  "result": null,          // after resolution: { "action": "commit"|"rollback", "ms": <number|null>, "detail": {...} }
  "cards": { "mattermost": "disabled" | "sent" | "error: …", "slack": "…" }
}

// RunRecord (GET /api/runs): one workspace run as this daemon broadcast it
{
  "run_id": "r-000003", "scenario": "mass_delete", "title": "Mass delete",
  "started_at": "<ISO8601>",             // ts of its run.started
  "outcome": "committed" | … | null,     // latest run.finished outcome; null while in progress
  "total_ms": <number|null>,             // latest run.finished total_ms
  "queued_ms": <number|null>,            // the non-null queued_ms (only the first run.finished has one)
  "stages": [ /* run.stage messages exactly as broadcast, in order:
                 {type:"run.stage", ts, run_id, seq, stage, status, ms, detail} */ ],
  "finished": [ /* run.finished messages exactly as broadcast: {type, ts, run_id, seq, outcome,
                   total_ms, queued_ms}; two after an approval resolution */ ],
  "last_seq": <integer>                  // highest seq broadcast for this run
}

// PayPalService: GET /api/config services.paypal and GET /api/paypal/scenarios sandbox (same fields)
{
  "enabled": <boolean>,                       // mode != "disabled"
  "mode": "local" | "remote" | "disabled",    // where Cedar-allowed orders are submitted
  "reason": "…",                              // why this mode (which keys are set, or what is missing)
  "base_url": "http://127.0.0.1:4173/paypal-sandbox" | "https://api-m.sandbox.paypal.com",
  "submitted": <integer>, "cap": <integer>,   // remote Sandbox submissions this run / per-run cap
  "local_submitted": <integer>,               // orders created + captured by the local gateway this run (no cap)
  "gateway": null | { "base_url", "orders_stored", "orders_kept", "tokens_active", "tokens_issued",
                      "create_requests", "orders_created", "orders_captured", "requests_rejected",
                      "receipts_verified", "receipts_absent" }   // local mode only
}
// mode: PAYPAL_CLIENT_ID and PAYPAL_CLIENT_SECRET both unset, or both the literal "local-sandbox" →
// "local" (the daemon points the middleware at its own /paypal-sandbox with the local credentials);
// both set to anything else → "remote" (PayPal Sandbox, unchanged); one missing, or "local-sandbox"
// mixed with a real value → "disabled".

// OrderRow: exactly adapter_paypal::TelemetryRow (see crates/adapter-paypal/src/lib.rs). Submission
// fields: submission ("sent" | "blocked" | "not_sent" | "error"), submission_detail (order status
// after the last call, e.g. "COMPLETED", or the reason), paypal_order_id, submission_target
// ("local" | "remote" | null), paypal_capture_id, submit_us (sum of the HTTP calls, null without
// calls), submit_calls: [{step: "oauth" | "create" | "capture", method, path, status, duration_us}],
// each call timed with Instant around the request and the read of its body.
// TimelineEvent (Bryntum Scheduler event-store shape)
{ "id": "…", "resourceId": "redact" | "score" | "policy" | "sign" | "verify" | "submit",
  "name": "bench-042 redact", "startDate": "ISO8601 with ms", "endDate": "ISO8601 with ms",
  "durationUs": <number>, "orderId": "…", "cls": "ok" | "blocked" | "error" }
// Optional on TimelineEvent: "startOffsetUs" (number), the µs from the order's first span start to this
// span's start. Dates carry only ms, so without it the console packs spans in startDate order.
// aegis-daemon always sends it: spans are laid out back to back in execution order (redact, sign,
// verify, score, policy, then submit) from the row's measured durations; untimed glue between
// stages is not drawn. "submit" spans exist only when a submission was attempted: one per HTTP call
// (id "<orderId>:submit:<step>", name "<reference> submit <step>") from OrderRow.submit_calls, or a
// single "<orderId>:submit" span over a failed attempt.
```

## HTTP

| Method & path | Body | Response |
|---|---|---|
| `GET /` | — | `index.html` with the token substituted; static files under `/assets/*`, `/vendor/*` |
| `GET /api/health` | — | `{ok, mock:false, version, git_commit, started_at, workspace, compute_units, model:{name, input_shape, compute_plan:{neural_engine, cpu, gpu}}, ane_keepwarm:{enabled, interval_ms, pings, models:{workspace, paypal:{pings, failures, last_ping_ms}}}}` |
| `GET /api/telemetry` | — | `{bench: <artifact JSON or null>, bench_path, bench_generated_at, live: LiveStats}` |
| `GET /api/config` | — | `{path, keys:[{name, set, source:"env"\|"dotenv"\|null, secret, console_writable, hint, signup_url, choices?}], nemotron_providers:[{id, label, base_url, signup_url, key_hint, tier1_model\|null, tier2_model\|null}], services:{nebius:{enabled, reason, provider:"<label>"\|"loopback test endpoint"\|null, base_url, tier1_model\|null, tier2_model\|null}, tavily:{enabled, reason}, paypal:PayPalService, mattermost:{enabled, reason}, slack:{enabled, reason}}}`. Only `NEBIUS_BASE_URL` has `choices:[{value, label}]` (the allowlisted provider URLs); `NEBIUS_API_KEY`'s `signup_url` is the selected provider's |
| `POST /api/config` | `{values:{NAME:"value"}}` (whitelisted names; only `""` unsets; a whitespace-only value → `400`; a non-empty value for a key with `console_writable:false` → `400`; `NEBIUS_BASE_URL` other than a provider URL, or a provider change without `NEBIUS_API_KEY` in the same save → `400`, see "Security") | same as `GET /api/config` |
| `POST /api/config/test` | `{service:"nebius"\|"tavily"\|"paypal"}` | `{service, ok, latency_ms, detail}` (a real call; `ok:false, detail:"not configured"` otherwise). `nebius` tests the selected Nemotron provider (Nebius, NVIDIA NIM or OpenRouter: `GET {base}/models`, `{data:[{id,…}]}`) and is `ok` only when the tier-1 and tier-2 model ids are configured and listed; otherwise `detail` names the unset or unlisted ones and the Nemotron ids the provider lists. `paypal` in local mode requests an OAuth2 token from the local gateway (nothing is sent to PayPal); `local-sandbox` credentials are never sent to PayPal |
| `GET /api/workspace` | — | `{live, files, pool:{ready, capacity, hits, misses, stale_discards, last_replenish_ms}, reaper:{queued, reaped, last_reap_ms}, versions:[{number, created_at}], scenarios:[{id, title, description, expected}]}` |
| `POST /api/workspace/run` | `{scenario}` | `{run_id}`; progress arrives on the WebSocket |
| `POST /api/workspace/reset` | `{}` | `GET /api/workspace` shape |
| `GET /api/runs` | — | `{runs:[RunRecord], limit}`: the newest `limit` (10) workspace runs, newest first (see "Run history") |
| `GET /api/approvals` | — | `{pending:[Approval], resolved:[Approval]}` (resolved: newest 50) |
| `POST /api/approvals/{id}/approve` | `{by}` | `Approval` |
| `POST /api/approvals/{id}/reject` | `{by}` | `Approval` |
| `GET /api/paypal/orders` | — | `{rows:[OrderRow], columnDefs:[…], timeline:{resources:[{id,name}], events:[TimelineEvent]}, limit}`: the newest `limit` (20) orders, oldest first, and the timeline events of exactly those orders |
| `POST /api/paypal/order` | `{scenario}` (ids from `GET /api/paypal/scenarios`) | `{order_id}`; result arrives on the WebSocket |
| `GET /api/paypal/scenarios` | — | `{scenarios:[{id, title, description}], stream:{enabled, interval_ms}, sandbox:PayPalService}` |
| `POST /api/paypal/stream` | `{enabled, interval_ms}` | `{enabled, interval_ms}` |

Scenario ids are defined by the daemon; the console renders whatever the list endpoints
return. Expected workspace scenarios: `safe_refactor` (multi-file, low risk → commit),
`secret_leak` (unformatted secret → rollback), `mass_delete` (high impact → team approval),
`build_break` (verification fails → recovery tiers → patched commit or rollback).

`mass_delete` reaches team approval because of modelling choices made after measuring, and its
`description` says so: the demo crate's fixture count (`demo::FIXTURES`) was picked so the change
crosses the high-impact threshold, and the cleanup agent's deletes are modelled as separate tool
calls (`demo::DELETE_BATCH` fixtures each, then `docs/`, then `README.md`), each one event in its
context window. Its `score` stage also carries `single_event_risk` and `single_event_score_ms`:
the same change scored as one event with no history, measured in that run, for comparison.

aegis-daemon's PayPal scenarios are the four benchmark cases as simulated agent intents:
`known_payee_42`, `known_payee_120`, `new_payee_480`, `redaction_disabled_25`.

Errors: `400` invalid JSON body, unknown scenario, service or setting, a value with a line
break, a whitespace-only config value, a refused `NEBIUS_BASE_URL` (not an allowlisted provider, or a provider change without its key), or a key that is not `console_writable`; `401` token; `403`
Host/Origin; `404` unknown approval (an id this daemon never issued) or path; `409` approval
already resolved, including ids no longer in the newest-50 `resolved` list (the message then says
`resolved` instead of `approved`/`rejected`); `413` body over 256 KB; `415` a POST whose
`Content-Type` is not `application/json`.

`POST /api/config` saves are serialised: concurrent saves (two tabs, a retry, or
`aegis-daemon config set` while the daemon runs) are applied one after another and none is lost.
A `.env` that is a symbolic link is not replaced (`400`; point `AEGIS_DOTENV` at the target).

Limits: `POST /api/paypal/stream` clamps `interval_ms` to 1000–600000 and answers with the value
in effect; the stream cycles through the scenarios. Only Cedar-allowed orders are submitted. In
`remote` mode at most `sandbox.cap` (25) orders per daemon run go to the PayPal Sandbox (create
only, as before). In `local` mode every allowed order does OAuth2 → create → capture over HTTP
against the local gateway, uncapped (the stream clamp bounds the rate); a sent local order has
`submission_detail: "COMPLETED"` and `submission_target: "local"`. `OrderRow.submission` is
`"error"` (detail in `submission_detail`) when a call failed. Successful submissions (local or
remote) feed the middleware's order-velocity features, so later orders in the same hour score higher.

## Local PayPal Orders v2 gateway (`/paypal-sandbox`)

An in-process emulation of PayPal's REST OAuth2 token endpoint and Orders v2 create / show /
capture, on the daemon's own loopback listener, so the PayPal pipeline runs a real end-to-end HTTP
submission without PayPal credentials. Shapes follow developer.paypal.com ("Authentication",
the Orders v2 reference and OpenAPI schema, "Idempotency", "API responses", the Orders v2 error
messages, currency codes). It is **not PayPal**: every response has the header
`x-aegis-local-sandbox: aegis-daemon local emulation of PayPal Orders v2; not PayPal`, every order
and error body has `"aegis_local_sandbox": true`, tokens start with `aegis-local-`, order ids with
`LOCAL-`, capture ids with `LOCALCAP-`, `debug_id`s with `local-`. Nothing leaves the process.

| Method & path | Request | Response |
|---|---|---|
| `POST /paypal-sandbox/v1/oauth2/token` | `Authorization: Basic base64(local-sandbox:local-sandbox)`, form body `grant_type=client_credentials` | `200 {scope, access_token, token_type:"Bearer", app_id, expires_in, nonce, aegis_local_sandbox:true}`; wrong or missing credentials → `401 {error:"invalid_client", error_description}`; missing or other grant → `400 {error:"unsupported_grant_type", …}` |
| `POST /paypal-sandbox/v2/checkout/orders` | `Authorization: Bearer <token>`, `Content-Type: application/json`, optional `PayPal-Request-Id`, optional `Prefer: return=minimal\|representation`; Orders v2 body `{intent, purchase_units:[…]}` | `201` Order (below); a replayed `PayPal-Request-Id` with the same body → `200` with the stored order |
| `GET /paypal-sandbox/v2/checkout/orders/{id}` | `Authorization: Bearer <token>` | `200` Order; unknown id → `404 RESOURCE_NOT_FOUND` (`INVALID_RESOURCE_ID`) |
| `POST /paypal-sandbox/v2/checkout/orders/{id}/capture` | `Authorization: Bearer <token>`, optional `PayPal-Request-Id`, optional JSON body | `201` Order with `status:"COMPLETED"` and `purchase_units[].payments.captures:[{id, status:"COMPLETED", amount, final_capture:true, custom_id?, create_time, update_time, links:[up]}]`; the same `PayPal-Request-Id` again → `200` with the same order |
| `GET /paypal-sandbox/checkoutnow?token={id}` | — (the `approve` link) | `200 {id, status, message, aegis_local_sandbox:true}`: approval is implied, see below |

Order: `{id, intent, status:"CREATED"|"COMPLETED", purchase_units, create_time, update_time,
links, aegis_local_sandbox:true, aegis_receipt:{scheme:"c2pa:ed25519", present, verified,
payload_sha256, public_key, detail}}`. `links` while `CREATED`: `self` (GET), `approve` (GET
`…/checkoutnow?token=`), `update` (PATCH; not implemented → `405 METHOD_NOT_SUPPORTED`), and
`capture` (POST, `CAPTURE` intent only); after capture only `self`. All hrefs point at
`http://127.0.0.1:<port>/paypal-sandbox`. With `Prefer: return=minimal` create and capture return
`{id, status, links, aegis_local_sandbox}`; without `Prefer` the full order (PayPal's default is
minimal).

Errors use PayPal's error JSON: `{name, message, debug_id, details:[{field, location, issue,
description}], aegis_local_sandbox:true}` (`field` is a JSON pointer into the body, or a header
name). Token endpoint errors use PayPal's identity shape `{error, error_description}`.

- `401 AUTHENTICATION_FAILURE` (`INVALID_TOKEN`): no `Bearer` header, a token this gateway did not
  issue, or an expired one. Tokens are random, kept in memory (newest 256) and expire after
  `expires_in` seconds.
- `400 INVALID_REQUEST`: `MALFORMED_REQUEST_JSON`, `MISSING_REQUIRED_PARAMETER`,
  `INVALID_PARAMETER_VALUE` (`intent` not `CAPTURE`/`AUTHORIZE`, a money object in another
  currency than its purchase unit, a bad `PayPal-Request-Id`), `INVALID_PARAMETER_SYNTAX` (money
  `value` not a decimal string, `quantity` not a whole number), `INVALID_STRING_LENGTH`
  (`custom_id` over 127 characters, `reference_id` over 256, `description` / item `name` over 3000;
  stored values are cut to 127 as PayPal reflects them), `INVALID_ARRAY_MIN_ITEMS` /
  `INVALID_ARRAY_MAX_ITEMS` (1–10 purchase units).
- `422 UNPROCESSABLE_ENTITY`: `AMOUNT_MISMATCH` (amount ≠ item_total + tax_total + shipping +
  handling + insurance − shipping_discount − discount), `ITEM_TOTAL_REQUIRED`,
  `ITEM_TOTAL_MISMATCH`, `TAX_TOTAL_REQUIRED`, `TAX_TOTAL_MISMATCH`, `DECIMAL_PRECISION` (two
  decimals; none for HUF, JPY, TWD), `CANNOT_BE_ZERO_OR_NEGATIVE`, `CANNOT_BE_NEGATIVE`,
  `MAX_VALUE_EXCEEDED`, `INVALID_CURRENCY_CODE`, `REFERENCE_ID_REQUIRED` / `DUPLICATE_REFERENCE_ID`
  (several purchase units), `ORDER_ALREADY_CAPTURED`, `ACTION_DOES_NOT_MATCH_INTENT` (capture of an
  `AUTHORIZE` order; authorize is not implemented).
- Local rules, also `422`: `DUPLICATE_REQUEST_ID` (a `PayPal-Request-Id` reused with a different
  body; PayPal documents returning the earlier result instead), `AEGIS_UNREDACTED_SENSITIVE_DATA`
  (a string still holds data the middleware's detector flags — secrets, card numbers, SSNs, e-mail
  addresses, phone numbers; `details` name the fields and counts, never the values; a
  `c2pa:ed25519:` receipt in `custom_id` is a signature and is not scanned),
  `AEGIS_RECEIPT_SIGNATURE_INVALID` (a `c2pa:ed25519:` receipt in `purchase_units[0].custom_id` that
  does not verify against the daemon's receipt key over the body without that `custom_id`).
  An order without such a receipt is accepted with `aegis_receipt.verified: false`.
- `413` body over 64 KiB, `415` create without `Content-Type: application/json`, `404` unknown path,
  `405 METHOD_NOT_SUPPORTED`.

Approval is implied: there is no buyer in the local sandbox, so `capture` completes a `CREATED`
order directly and the `approve` link only says so. Orders live in memory (newest 256) and do not
survive a restart. `/paypal-sandbox` is always routed, but the daemon only points the middleware at
it in `local` mode.

CoreML keep-warm: the first prediction after the daemon has been idle can be much slower than
warm ones. With `AEGIS_ANE_KEEPWARM_MS` (environment, default `1000`, `0` disables) the daemon
runs one prediction on an all-zero window for each model (`workspace`, `paypal`) that has not
predicted for that long. `health.ane_keepwarm` reports `enabled`, `interval_ms` (`null` when
disabled), the total `pings`, and per model `pings`, `failures` and the measured `last_ping_ms`.

Optional extra fields the daemon sends: `health.model.{path, layout, operations_dispatched}`,
`pool.{replenishing, replenished, fs_events, fs_events_ignored, watching_fs_events,
recent_fs_changes:[{path, flags}], last_error}` (FSEvents diagnostics: paths relative to live, raw
flag bits in hex), `reaper.{failed, background}`.

## WebSocket `GET /ws?token=…`

Server → client JSON messages; every message has `type` and `ts` (ISO8601).

| type | fields |
|---|---|
| `hello` | `health`, `live` (LiveStats) |
| `run.started` | `run_id`, `scenario`, `title`, `seq` (always `1`) |
| `run.stage` | `run_id`, `stage` (`branch`\|`edit`\|`diff`\|`features`\|`score`\|`policy`\|`verify`\|`recover`\|`approval`\|`commit`\|`rollback`\|`reap`), `status` (`ok`\|`fail`\|`skip`\|`pending`), `ms` (number or null), `detail` (object; e.g. branch `{source:"pool"\|"cold"}`, score `{risk, units}`, policy `{allowed, determining_policies, eval_us}`, recover `{tier, model, enabled, reason, patch_applied}`), `seq` |
| `run.finished` | `run_id`, `outcome` (`committed`\|`rolled_back`\|`pending_approval`\|`recovered`\|`error`), `total_ms`, `queued_ms` (number, or `null` after an approval resolution), `seq` |
| `approval.created` / `approval.resolved` | `approval` |
| `paypal.order` | `row` (OrderRow), `events` ([TimelineEvent]) |
| `telemetry.live` | `live` (LiveStats), sent about once per second |
| `config.changed` | `config` (same as `GET /api/config`) |
| `log` | `level` (`info`\|`warn`\|`error`), `message`; `dropped` (integer) only on the broadcast-lag notice |

Client → server: nothing (all actions are HTTP POSTs).

`seq` (optional for clients; added with `GET /api/runs`): a per-run counter over that run's
`run.started` (`1`), `run.stage` and `run.finished` messages, increasing by 1 per message in
broadcast order, so `run_id` + `seq` identifies a message. It is absent on messages for a run the
history no longer keeps (older than the newest 10). Clients written before it ignore it.

Broadcast lag: a client that falls too far behind the broadcast channel misses messages. It then
gets a `log` with `level: "warn"`, a human-readable `message`, and `dropped`: the number of
messages it missed. Treat `dropped` as the signal (older daemons sent only the message text,
`this client fell behind; <n> messages were dropped`) and reload from the GET endpoints.

### Run lifecycle (aegis-daemon)

- Scenario runs are queued: one runs at a time. `run.started` is sent when the run is requested;
  `queued_ms` is the time it then waited behind earlier runs (or a reset), and `total_ms` counts
  from when it started running. Approval resolutions do not wait for a running scenario.
- Stages arrive in pipeline order: `branch`, `edit`, `diff`, `features`, `score`, `policy`,
  `verify`, then `recover` events (build failures), `approval`, `commit` or `rollback`, `reap`.
  `diff`, `score`, `policy` and `verify` repeat after a remote recovery patch.
- Commit conflict: a branch whose base is no longer the live tree (another commit landed while it
  was pending or running) gets `commit` `fail` with `detail.conflict: true`, then a normal
  `rollback` (detach time measured, counted in `rollback_detach_ms`), then `reap`; the outcome is
  `rolled_back`. If a conflict is only found inside the storage commit itself (a race with another
  process), `commit` `fail` carries `detail.rollback` saying the branch was discarded there and
  there is no `rollback` stage.
- `reap` is the background reaper deleting the run's detached tree; it usually arrives after
  `run.finished`. A commit has no `reap` unless it pruned old versions.
- A run that ends with `run.finished` `pending_approval` keeps its branch. When the approval is
  resolved (or the workspace is reset) the same `run_id` gets `approval` (`ok` approved, `fail`
  rejected, `skip` reset), for Approve a second `policy` with `team_approval_granted: true`, then
  `commit` or `rollback`, `approval.resolved`, and a second `run.finished` whose `total_ms`
  includes the wait (counted from when the original run started running).
- Pending approvals live in memory. On SIGINT or SIGTERM the daemon rolls their branches back;
  branches left behind by a daemon that was killed outright are discarded at the next start.
- Run history: the daemon keeps its newest 10 workspace runs in memory (`GET /api/runs`), each
  with its `run.stage` and `run.finished` messages exactly as broadcast, including the
  post-approval continuation. Records are ordered by run id (request order). The oldest run is
  dropped first, except that a run whose latest outcome is `pending_approval` is kept while any
  other run can be dropped, so its continuation lands in the same record. Nothing survives a daemon
  restart.
- Reloading the console: on page load, WebSocket reconnect and after a lag notice the console reads
  `GET /api/runs`, `GET /api/approvals` and `GET /api/paypal/orders`, and merges live messages by
  `run_id` + `seq` (a message already in the history is not applied twice). A run whose history
  starts at `seq` 1 and has no missing `seq` is fully observed.
- `recover`: tier 0 (`RecoveryRouter::triage`, local CoreML + Cedar) always runs and is `ok` when
  the triage completed. Tiers 1 and 2 are `skip` with `enabled: false` and a `reason` while
  `NEBIUS_API_KEY` is not set, `NEBIUS_BASE_URL` is refused, or a tier model is unset (`model`
  is then `null`); tier-2 events carry `detail.tavily: {enabled, reason}`.
- Every `ms` is the duration of the real call the stage reports (`Instant` around it); `commit`
  reports the `RENAME_SWAP` time and `rollback` the detach time, with totals in `detail`.
