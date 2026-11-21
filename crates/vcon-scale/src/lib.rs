//! The scale harness's logic, kept apart from the HTTP calls in `main.rs` so
//! each rule can be tested without a server: the seeded generator of
//! synthetic vCons, the size mix, the latency summary, the start guard and
//! cost projection, the host facts, the command line and the result file.
#![allow(
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    reason = "an internal crate (publish = false): no semver promise"
)]

pub mod args;
pub mod guard;
pub mod host;
pub mod mix;
pub mod report;
pub mod rng;
pub mod stats;
pub mod synth;
