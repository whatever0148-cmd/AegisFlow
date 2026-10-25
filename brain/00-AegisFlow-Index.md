---
tags:
  - aegisflow
last_synced: 2026-10-25T15:38:39Z
git_commit: b3f6810
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 289.3 | 2026-10-25T15:28:05Z |
| [[Agent-Core-Builder]] | 🟢 verified | 103831.1 | 2026-10-25T15:23:33Z |
| [[Agent-QA-Reviewer]] | 🔴 failing | 322360.3 | 2026-10-25T15:38:38Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 417.9 | 2026-10-25T15:38:39Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1074 | 2026-10-25T15:38:36Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1296 | 2026-10-25T15:38:36Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1751 | 2026-10-25T15:38:36Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0045 | 2026-10-25T15:38:36Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1737 | 2026-10-25T15:38:36Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2448 | 2026-10-25T15:38:36Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.1447 | 2026-10-25T15:38:36Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-25T15:38:39Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1074 | 2026-10-25T15:38:36Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1751 | 2026-10-25T15:38:36Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0045 | 2026-10-25T15:38:36Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 3.366 | 2026-10-25T15:38:36Z |

See also: [[Telemetry-Log]]
