---
tags:
  - aegisflow
last_synced: 2026-10-26T03:52:56Z
git_commit: 4403ab4-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 307.0 | 2026-10-26T03:52:55Z |
| [[Agent-Core-Builder]] | 🟢 verified | 17936.6 | 2026-10-26T03:51:33Z |
| [[Agent-QA-Reviewer]] | 🔴 failing | 88783.7 | 2026-10-26T03:52:55Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 374.2 | 2026-10-26T03:52:55Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1140 | 2026-10-26T03:52:53Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1303 | 2026-10-26T03:52:53Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1815 | 2026-10-26T03:52:53Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0043 | 2026-10-26T03:52:53Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1791 | 2026-10-26T03:52:53Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2612 | 2026-10-26T03:52:53Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.1123 | 2026-10-26T03:52:53Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-26T03:52:56Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1140 | 2026-10-26T03:52:53Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1815 | 2026-10-26T03:52:53Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0043 | 2026-10-26T03:52:53Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 3.353 | 2026-10-26T03:52:53Z |

See also: [[Telemetry-Log]]
