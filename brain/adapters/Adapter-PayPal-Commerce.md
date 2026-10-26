---
tags:
  - adapter
status: verified
latency_ms: 0.26125
last_verified: 2026-10-26T03:52:53Z
git_commit: 4403ab4-dirty
crate: adapter_paypal
---

# Adapter-PayPal-Commerce

**Status:** 🟢 verified · **p50:** 0.2612 ms · **commit:** `4403ab4-dirty` · **bench run: PRELIMINARY**

> PayPal Sandbox `/v2/checkout/orders` middleware: `[REDACTED]` formatting of sensitive strings, ed25519 receipt (`c2pa:ed25519:...`) in `custom_id`, Cedar budget rules, AG Grid + Bryntum telemetry rows.

**Implemented in:** `crates/adapter-paypal/src/lib.rs`

**Upstream:** [[Agent-Telemetry-UI]] (dashboard rows) · [[Core-ANE-Classifier]] (order risk) · [[Core-Cedar-Rules]] (budget rules)
**Downstream:** [[Bench-EndToEnd-Rollback]] (receipt signing)

Hub: [[00-AegisFlow-Index]] · History: [[Telemetry-Log]]

<!-- aegis:telemetry:start -->
## Live telemetry
_Synced 2026-10-26T03:52:56Z · bench 2026-10-26T03:52:53Z · tests 2026-10-25T08:10:13Z · commit `4403ab4-dirty`_

> **PRELIMINARY: mean attributable CPU contention 2.25 cores over 79 samples at or above the busy threshold 2.00 cores (25% of 8 logical CPUs).** Only a quiet full-scale benchmark run decides the phase result.

| Metric | Measured |
|---|---|
| cargo test --release | 8 passed, 0 failed (2 suites) |
| prepare order (redact+sign+verify+score+Cedar) | p50 261.2 µs · p95 304.5 µs · n=2000 |
| risk score inside prepare | p50 154.7 µs · p95 181.1 µs · n=2000 |
| ed25519 sign | p50 18.458 µs · p95 22.041 µs · n=2000 |
| Sandbox submission | not called · PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set |
<!-- aegis:telemetry:end -->

## Notes
<!-- aegis:user:start -->
_Your notes here are preserved across syncs._
<!-- aegis:user:end -->
