---
tags:
  - aegisflow
last_synced: 2026-10-26T04:52:49Z
git_commit: b5edd4f
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 264.8 | 2026-10-26T04:52:49Z |
| [[Agent-Core-Builder]] | 🟢 verified | 90435.5 | 2026-10-26T04:49:09Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 284532.3 | 2026-10-26T04:52:49Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 326.7 | 2026-10-26T04:52:49Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1055 | 2026-10-26T04:52:47Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1234 | 2026-10-26T04:52:47Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1765 | 2026-10-26T04:52:47Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0042 | 2026-10-26T04:52:47Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.2154 | 2026-10-26T04:52:47Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2672 | 2026-10-26T04:52:47Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.0499 | 2026-10-26T04:52:47Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-26T04:52:49Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1055 | 2026-10-26T04:52:47Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1765 | 2026-10-26T04:52:47Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0042 | 2026-10-26T04:52:47Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 2.876 | 2026-10-26T04:52:47Z |

See also: [[Telemetry-Log]]
