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
- External services (Nebius, Tavily, PayPal Sandbox, Mattermost, Slack) are used only when
  their credentials are configured. When they are not, the daemon reports
  `enabled: false` with a reason and the console shows that state; it never simulates a
  remote reply.
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
- The token is a CSRF and DNS-rebinding defence for browsers. It does **not** authenticate local
  processes: `GET /` hands it to any client that sends an allowed `Host`. So any local process
  that can reach loopback can drive this API, and settings that decide where a stored secret is
  sent are not writable over HTTP: `POST /api/config` refuses to set `NEBIUS_BASE_URL` (`400`;
  unsetting it, which restores the default https endpoint, is allowed). Set it with
  `aegis-daemon config set NEBIUS_BASE_URL` (`harness/configure_keys.sh`) or the environment.
  Keys carry `console_writable` in `GET /api/config`.
- `NEBIUS_API_KEY` is only sent to an `https://` `NEBIUS_BASE_URL` (`http://` only for
  `127.0.0.1`, `localhost` or `[::1]`, i.e. local test endpoints). Any other value, from `.env` or
  the environment, disables the remote recovery tiers with a reason and the key is not sent.
- Error bodies are `{"error": "<message>"}` with a 4xx/5xx status.

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

// OrderRow: exactly adapter_paypal::TelemetryRow (see crates/adapter-paypal/src/lib.rs)
// TimelineEvent (Bryntum Scheduler event-store shape)
{ "id": "…", "resourceId": "redact" | "score" | "policy" | "sign" | "verify" | "submit",
  "name": "bench-042 redact", "startDate": "ISO8601 with ms", "endDate": "ISO8601 with ms",
  "durationUs": <number>, "orderId": "…", "cls": "ok" | "blocked" | "error" }
// Optional on TimelineEvent: "startOffsetUs" (number), the µs from the order's first span start to this
// span's start. Dates carry only ms, so without it the console packs spans in startDate order.
// aegis-daemon always sends it: spans are laid out back to back in execution order (redact, sign,
// verify, score, policy, then submit) from the row's measured durations; untimed glue between
// stages is not drawn. A "submit" span exists only when a Sandbox call was made.
```

## HTTP

| Method & path | Body | Response |
|---|---|---|
| `GET /` | — | `index.html` with the token substituted; static files under `/assets/*`, `/vendor/*` |
| `GET /api/health` | — | `{ok, mock:false, version, git_commit, started_at, workspace, compute_units, model:{name, input_shape, compute_plan:{neural_engine, cpu, gpu}}, ane_keepwarm:{enabled, interval_ms, pings, models:{workspace, paypal:{pings, failures, last_ping_ms}}}}` |
| `GET /api/telemetry` | — | `{bench: <artifact JSON or null>, bench_path, bench_generated_at, live: LiveStats}` |
| `GET /api/config` | — | `{path, keys:[{name, set, source:"env"\|"dotenv"\|null, secret, console_writable, hint, signup_url}], services:{nebius:{enabled, reason, base_url, tier1_model, tier2_model}, tavily:{enabled, reason}, paypal:{enabled, reason, base_url}, mattermost:{enabled, reason}, slack:{enabled, reason}}}` |
| `POST /api/config` | `{values:{NAME:"value"}}` (whitelisted names; only `""` unsets; a whitespace-only value → `400`; a non-empty value for a key with `console_writable:false` → `400`) | same as `GET /api/config` |
| `POST /api/config/test` | `{service:"nebius"\|"tavily"\|"paypal"}` | `{service, ok, latency_ms, detail}` (a real call; `ok:false, detail:"not configured"` otherwise). Nebius is `ok` only when the tier-1 and tier-2 model ids are in `GET /models`; `detail` names any that are not |
| `GET /api/workspace` | — | `{live, files, pool:{ready, capacity, hits, misses, stale_discards, last_replenish_ms}, reaper:{queued, reaped, last_reap_ms}, versions:[{number, created_at}], scenarios:[{id, title, description, expected}]}` |
| `POST /api/workspace/run` | `{scenario}` | `{run_id}`; progress arrives on the WebSocket |
| `POST /api/workspace/reset` | `{}` | `GET /api/workspace` shape |
| `GET /api/runs` | — | `{runs:[RunRecord], limit}`: the newest `limit` (10) workspace runs, newest first (see "Run history") |
| `GET /api/approvals` | — | `{pending:[Approval], resolved:[Approval]}` (resolved: newest 50) |
| `POST /api/approvals/{id}/approve` | `{by}` | `Approval` |
| `POST /api/approvals/{id}/reject` | `{by}` | `Approval` |
| `GET /api/paypal/orders` | — | `{rows:[OrderRow], columnDefs:[…], timeline:{resources:[{id,name}], events:[TimelineEvent]}, limit}`: the newest `limit` (20) orders, oldest first, and the timeline events of exactly those orders |
| `POST /api/paypal/order` | `{scenario}` (ids from `GET /api/paypal/scenarios`) | `{order_id}`; result arrives on the WebSocket |
| `GET /api/paypal/scenarios` | — | `{scenarios:[{id, title, description}], stream:{enabled, interval_ms}, sandbox:{enabled, submitted, cap}}` |
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
break, a whitespace-only config value, a refused `NEBIUS_BASE_URL`, or a key that is not `console_writable`; `401` token; `403`
Host/Origin; `404` unknown approval (an id this daemon never issued) or path; `409` approval
already resolved, including ids no longer in the newest-50 `resolved` list (the message then says
`resolved` instead of `approved`/`rejected`); `413` body over 256 KB; `415` a POST whose
`Content-Type` is not `application/json`.

`POST /api/config` saves are serialised: concurrent saves (two tabs, a retry, or
`aegis-daemon config set` while the daemon runs) are applied one after another and none is lost.
A `.env` that is a symbolic link is not replaced (`400`; point `AEGIS_DOTENV` at the target).

Limits: `POST /api/paypal/stream` clamps `interval_ms` to 1000–600000 and answers with the value
in effect; the stream cycles through the scenarios. At most `sandbox.cap` (25) orders per daemon
run are submitted to the PayPal Sandbox, and only Cedar-allowed ones with credentials configured;
`OrderRow.submission` is `"error"` (detail in `submission_detail`) when that call failed.

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
  `NEBIUS_API_KEY` is not set; tier-2 events carry `detail.tavily: {enabled, reason}`.
- Every `ms` is the duration of the real call the stage reports (`Instant` around it); `commit`
  reports the `RENAME_SWAP` time and `rollback` the detach time, with totals in `detail`.
