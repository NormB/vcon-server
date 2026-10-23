//! The vcon-server program. The binary in `main.rs` is a thin front end;
//! everything it runs lives here so tests can drive it.

pub mod envelope;
pub mod fs;
pub mod ids;
