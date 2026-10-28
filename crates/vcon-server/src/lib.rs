//! The vcon-server program. The binary in `main.rs` is a thin front end;
//! everything it runs lives here so tests can drive it.

pub mod access;
pub mod api;
pub mod audit;
pub(crate) mod backend;
pub mod catalog;
pub mod clock;
pub mod commands;
pub mod config;
pub mod console;
pub mod datadir;
pub mod envelope;
pub mod fs;
pub mod ids;
pub mod mcp;
pub mod query;
pub mod retention;
pub mod serve;
pub mod store;
pub mod tenants;
