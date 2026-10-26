---
tags:
  - aegisflow
last_synced: 2026-10-26T07:03:04Z
git_commit: c3ad550
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 249.3 | 2026-10-26T07:03:04Z |
| [[Agent-Core-Builder]] | 🟢 verified | 104358.1 | 2026-10-26T06:58:59Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 336813.6 | 2026-10-26T07:03:04Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 444.1 | 2026-10-26T07:03:04Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1060 | 2026-10-26T07:03:00Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1261 | 2026-10-26T07:03:00Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1944 | 2026-10-26T07:03:00Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0042 | 2026-10-26T07:03:00Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1953 | 2026-10-26T07:03:00Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2750 | 2026-10-26T07:03:00Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.1047 | 2026-10-26T07:03:00Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-26T07:03:04Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1060 | 2026-10-26T07:03:00Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1944 | 2026-10-26T07:03:00Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0042 | 2026-10-26T07:03:00Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 3.164 | 2026-10-26T07:03:00Z |

See also: [[Telemetry-Log]]
