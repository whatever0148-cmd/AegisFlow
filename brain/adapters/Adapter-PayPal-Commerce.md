---
tags:
  - adapter
status: verified
latency_ms: 0.275
last_verified: 2026-10-26T07:03:00Z
git_commit: c3ad550
crate: adapter_paypal
---

# Adapter-PayPal-Commerce

**Status:** 🟢 verified · **p50:** 0.2750 ms · **commit:** `c3ad550`

> PayPal Sandbox `/v2/checkout/orders` middleware: `[REDACTED]` formatting of sensitive strings, ed25519 receipt (`c2pa:ed25519:...`) in `custom_id`, Cedar budget rules, AG Grid + Bryntum telemetry rows.

**Implemented in:** `crates/adapter-paypal/src/lib.rs`

**Upstream:** [[Agent-Telemetry-UI]] (dashboard rows) · [[Core-ANE-Classifier]] (order risk) · [[Core-Cedar-Rules]] (budget rules)
**Downstream:** [[Bench-EndToEnd-Rollback]] (receipt signing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T07:03:04Z · bench 2026-10-26T07:03:00Z · tests 2026-10-25T08:10:13Z · commit `c3ad550`_

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (2 suites) |
| prepare order (redact+sign+verify+score+Cedar) | p50 275.0 µs · p95 314.2 µs · n=2000 |
| risk score inside prepare | p50 173.0 µs · p95 197.7 µs · n=2000 |
| ed25519 sign | p50 18.083 µs · p95 23.125 µs · n=2000 |
| Sandbox submission | not called · PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
