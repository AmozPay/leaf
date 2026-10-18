//! The MCP server of Leaf, as a library so tests (and other transports)
//! can serve it. The `leaf-mcp` binary serves it over stdio.

mod server;

pub use server::Leaf;
