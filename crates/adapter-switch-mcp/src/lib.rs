//! Switch chat bridge and MCP server for AegisFlow.
//!
//! * [`cards`] renders workspace-diff + team-approval cards for Mattermost (message attachments
//!   with interactive actions) and Slack (Block Kit) and posts them to incoming webhooks that are
//!   configured in the environment. Diff excerpts are redacted before rendering.
//! * [`mcp`] is a Model Context Protocol server (JSON-RPC 2.0, newline-delimited on stdio) exposing
//!   branch / diff / score / policy / commit / rollback / approval tools; `aegis-mcp` runs it.

pub mod cards;
pub mod mcp;

pub use cards::{mattermost_card, post_webhook, slack_card, ApprovalRequest, CardError, ChatConfig};
pub use mcp::{McpServer, ShutdownReport};
