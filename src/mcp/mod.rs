//! MCP (Model Context Protocol) server module.
//!
//! Exposes Lekton documentation to IDE agents (Claude Code, Cursor, etc.)
//! via the Streamable HTTP transport. Authenticated with PATs (Personal
//! Access Tokens) stored in the service-tokens collection.

pub mod auth;
pub mod server;
