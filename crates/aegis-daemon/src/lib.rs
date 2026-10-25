//! `aegis-daemon`: serves the Aegis Console (`apps/aegis-console`) and implements its contract,
//! `apps/aegis-console/API.md`, on top of the AegisFlow crates.
//!
//! * Panel 1 — a demo Rust crate under `target/aegis-console/demo/live`, opened with
//!   [`aegis_storage::Workspace`] (background reaper) and a [`aegis_storage::WarmBranchPool`]
//!   (FSEvents watching). Scenarios make real multi-file edits in pooled CoW branches, are scored
//!   by the CoreML context-window model per agent, authorised by Cedar, verified with a fixed
//!   `rustc` command, and committed (`RENAME_SWAP`) or rolled back. Build failures go through the
//!   `RecoveryRouter` tiers; remote tiers run only with Nebius/Tavily keys configured.
//! * Panel 2 — simulated agent purchase intents processed by `adapter_paypal` (redaction, score,
//!   Cedar budget rules, ed25519 receipts); Sandbox submission only with PayPal credentials.
//! * Panel 3 — in-memory team approvals (Mattermost/Slack cards only with webhooks configured),
//!   the benchmark artifact, and live statistics over this daemon's own measurements.
//!
//! Settings come from [`aegis_config::Settings`] (environment first, then `.env`) and are turned
//! into adapter configs explicitly; the daemon never modifies its process environment.

pub mod api;
pub mod approvals;
pub mod demo;
pub mod keepwarm;
pub mod live;
pub mod paypal;
pub mod security;
pub mod services;
pub mod state;
pub mod util;
pub mod workspace;

pub use api::{router, spawn_background};
pub use state::{AppState, DaemonOptions, Events};
