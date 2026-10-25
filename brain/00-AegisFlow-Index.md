---
tags:
  - aegisflow
last_synced: 2026-10-25T08:11:35Z
git_commit: a29112b
---

# AegisFlow brain

Live map of the AegisFlow harness. Open [[AegisFlow_Architecture.canvas|the architecture canvas]] or the graph view (colour groups: purple `#agent`, cyan `#core`, orange `#adapter`, green `#benchmark`).

Every value below is copied from `artifacts/*.json` produced by real test and benchmark runs; empty means not measured yet.

## Sub-Agents

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Agent-Systems-Architect]] | 🟢 verified | 219.9 | 2026-10-25T08:08:53Z |
| [[Agent-Core-Builder]] | 🟢 verified | 88739.7 | 2026-10-25T08:06:11Z |
| [[Agent-QA-Reviewer]] | 🟢 verified | 27245.5 | 2026-10-25T08:06:38Z |
| [[Agent-Telemetry-UI]] | 🟢 verified | 168.7 | 2026-10-25T08:06:38Z |

## Core Engine Modules

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Core-APFS-Clonefile]] | 🟢 verified | 1.116 | 2026-10-25T08:06:37Z |
| [[Core-Memory-Snapshot]] | 🟢 verified | 0.1253 | 2026-10-25T08:06:37Z |
| [[Core-ANE-Classifier]] | 🟢 verified | 0.0395 | 2026-10-25T08:06:37Z |
| [[Core-Cedar-Rules]] | 🟢 verified | 0.0043 | 2026-10-25T08:06:37Z |

## Competition Adapters

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Adapter-Nebius-Nemotron]] | 🟢 verified | 0.0424 | 2026-10-25T08:06:37Z |
| [[Adapter-PayPal-Commerce]] | 🟢 verified | 0.1165 | 2026-10-25T08:06:37Z |
| [[Adapter-Switch-MCP]] | 🟢 verified | 0.0315 | 2026-10-25T08:06:37Z |
| [[Adapter-Kiro-Specs]] | 🟢 verified | — | 2026-10-25T08:11:35Z |

## Hardware Telemetry Proofs

| Node | Status | latency_ms | last_verified |
|---|---|---|---|
| [[Bench-APFS-Latency]] | 🟡 target missed | 1.116 | 2026-10-25T08:06:37Z |
| [[Bench-ANE-FP16]] | 🟡 target missed | 0.0395 | 2026-10-25T08:06:37Z |
| [[Bench-Cedar-Eval]] | 🟢 verified | 0.0043 | 2026-10-25T08:06:37Z |
| [[Bench-EndToEnd-Rollback]] | 🟢 verified | 9.035 | 2026-10-25T08:06:37Z |

See also: [[Telemetry-Log]]
