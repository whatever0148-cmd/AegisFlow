---
tags:
  - adapter
status: verified
latency_ms: 0.279416
last_verified: 2026-10-25T15:28:03Z
git_commit: 60fc826-dirty
crate: adapter_paypal
---

# Adapter-PayPal-Commerce

**Status:** 🟢 verified · **p50:** 0.2794 ms · **commit:** `60fc826-dirty` · **bench run: PRELIMINARY**

> PayPal Sandbox `/v2/checkout/orders` middleware: `[REDACTED]` formatting of sensitive strings, ed25519 receipt (`c2pa:ed25519:...`) in `custom_id`, Cedar budget rules, AG Grid + Bryntum telemetry rows.

**Implemented in:** `crates/adapter-paypal/src/lib.rs`

**Upstream:** [[Agent-Telemetry-UI]] (dashboard rows) · [[Core-ANE-Classifier]] (order risk) · [[Core-Cedar-Rules]] (budget rules)
**Downstream:** [[Bench-EndToEnd-Rollback]] (receipt signing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-25T15:28:06Z · bench 2026-10-25T15:28:03Z · tests 2026-10-25T08:10:13Z · commit `60fc826-dirty`_

> **PRELIMINARY: 1-min load average 11.47 at end above the busy threshold 4.** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (2 suites) |
| prepare order (redact+sign+verify+score+Cedar) | p50 279.4 µs · p95 409.3 µs · n=2000 |
| risk score inside prepare | p50 165.2 µs · p95 234.0 µs · n=2000 |
| ed25519 sign | p50 19.250 µs · p95 26.292 µs · n=2000 |
| Sandbox submission | not called · PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
