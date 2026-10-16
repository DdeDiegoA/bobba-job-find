//! The core side of the agent sandbox (AD-6, S2.3a): the per-run workspace, the
//! per-run MCP token, the MCP config the CLI is started with, and the core's
//! own MCP endpoint that serves the run's tool subset over an owner-only local
//! socket.
//!
//! Flow (architecture §5.4): `config::prepare` creates the workspace and mints
//! the token, `server::McpEndpoint::start` binds the endpoint for that token,
//! and the runtime spawns the CLI with `config::run_env`. When the run ends,
//! dropping the `RunSandbox` deletes the workspace and dropping the endpoint
//! revokes the token.

pub mod config;
pub mod server;

pub use config::{RunSandbox, Workspace, mcp_config_json, new_token, prepare, run_env};
pub use server::{McpEndpoint, ToolHandler};
