---
tags:
  - aegisflow
last_synced: 2026-10-25T17:28:26Z
git_commit: 9f1f777-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 285.8 | 2026-10-25T17:28:26Z |
| [[Agent-Core-Builder]] | 🟢 verified | 89071.7 | 2026-10-25T17:25:11Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 251772.1 | 2026-10-25T17:28:26Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 350.3 | 2026-10-25T17:28:26Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1050 | 2026-10-25T17:28:23Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1261 | 2026-10-25T17:28:23Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1820 | 2026-10-25T17:28:23Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0042 | 2026-10-25T17:28:23Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1788 | 2026-10-25T17:28:23Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2579 | 2026-10-25T17:28:23Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.0699 | 2026-10-25T17:28:23Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-25T17:28:26Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1050 | 2026-10-25T17:28:23Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1820 | 2026-10-25T17:28:23Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0042 | 2026-10-25T17:28:23Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 2.791 | 2026-10-25T17:28:23Z |

See also: [[Telemetry-Log]]
