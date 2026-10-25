//! `aegis-mcp`: AegisFlow MCP server over stdio (one JSON-RPC message per line).
//!
//! Environment: `AEGIS_WORKSPACE` (optional workspace to open at start), `AEGIS_AGENT_ID`,
//! `AEGIS_MATTERMOST_WEBHOOK_URL`, `AEGIS_SLACK_WEBHOOK_URL`, `AEGIS_APPROVAL_CALLBACK_URL`.
//! Diagnostics go to stderr; stdout carries only protocol messages.
//!
//! Rollback deletion runs on a detached background reaper. At end of input the server rolls back
//! the branches still open and waits for the reaper before exiting (bounded by
//! `AEGIS_MCP_SHUTDOWN_WAIT_SECS`, default [`DEFAULT_SHUTDOWN_WAIT`]); whatever an interrupted
//! session leaves in `<store>/trash` is deleted the next time the workspace is opened.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

use adapter_switch_mcp::{ChatConfig, McpServer};
use aegis_ane_scorer::AneScorer;
use aegis_governance::GovernanceEngine;
use aegis_storage::Workspace;

const DEFAULT_SHUTDOWN_WAIT: Duration = Duration::from_secs(60);

fn shutdown_wait() -> Duration {
    std::env::var("AEGIS_MCP_SHUTDOWN_WAIT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(DEFAULT_SHUTDOWN_WAIT, Duration::from_secs)
}

fn main() {
    let scorer = AneScorer::open_default().unwrap_or_else(|e| {
        eprintln!("aegis-mcp: {e}");
        std::process::exit(1);
    });
    let gov = GovernanceEngine::load_default().unwrap_or_else(|e| {
        eprintln!("aegis-mcp: {e}");
        std::process::exit(1);
    });
    let mut server = McpServer::new(Arc::new(scorer), Arc::new(gov), ChatConfig::from_env());
    if let Some(path) = std::env::var_os("AEGIS_WORKSPACE") {
        match Workspace::open(&path) {
            Ok(ws) => {
                match ws.reclaim_trash() {
                    Ok(0) => {}
                    Ok(n) => eprintln!("aegis-mcp: deleted {n} tree(s) a previous session left in {}", ws.store().join("trash").display()),
                    Err(e) => eprintln!("aegis-mcp: reclaiming leftover trash failed: {e}"),
                }
                server = server.with_workspace(ws);
            }
            Err(e) => eprintln!("aegis-mcp: AEGIS_WORKSPACE not opened: {e}"),
        }
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            if writeln!(stdout, "{response}").and_then(|_| stdout.flush()).is_err() {
                break;
            }
        }
    }
    drop(stdout);
    let report = server.shutdown(shutdown_wait());
    for error in &report.rollback_errors {
        eprintln!("aegis-mcp: rollback at shutdown failed: {error}");
    }
    if !report.reaped {
        eprintln!("aegis-mcp: background deletion still running at exit; the next workspace open reclaims the trash");
    }
}
