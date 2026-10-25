---
tags:
  - aegisflow
last_synced: 2026-10-25T15:28:06Z
git_commit: 60fc826-dirty
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 289.3 | 2026-10-25T15:28:05Z |
| [[Agent-Core-Builder]] | 🟢 verified | 103831.1 | 2026-10-25T15:23:33Z |
| [[Agent-QA-Reviewer]] | 🔴 failing | 337048.2 | 2026-10-25T15:28:05Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 376.7 | 2026-10-25T15:28:05Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 0.1053 | 2026-10-25T15:28:03Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1288 | 2026-10-25T15:28:03Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.1785 | 2026-10-25T15:28:03Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0045 | 2026-10-25T15:28:03Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.1719 | 2026-10-25T15:28:03Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.2794 | 2026-10-25T15:28:03Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.1428 | 2026-10-25T15:28:03Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-25T15:28:06Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟢 verified | 0.1053 | 2026-10-25T15:28:03Z |
| [[Bench-ANE-FP16]] | 🟢 verified | 0.1785 | 2026-10-25T15:28:03Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0045 | 2026-10-25T15:28:03Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 4.178 | 2026-10-25T15:28:03Z |

See also: [[Telemetry-Log]]
