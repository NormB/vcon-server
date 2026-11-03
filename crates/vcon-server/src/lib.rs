//! The vcon-server program. The binary in `main.rs` is a thin front end;
//! everything it runs lives here so tests can drive it.
//!
//! This library is not an API for other crates. It exists for the
//! vcon-server binary, its tests and its fuzz targets, and any release may
//! change it without notice: semver covers the program's command line, HTTP
//! API and data directory, not these modules. To read or check vCons from
//! Rust, use the [vcon](https://docs.rs/vcon) crate.
// The API Guidelines' future-proofing rules are for a public library, which
// this is not (above): its enums and structs are matched and built whole
// by the binary, tests and fuzz targets.
#![allow(
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    reason = "an internal library: no semver promise, see the crate docs"
)]

pub mod access;
#[expect(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    reason = "the Rust style rules, met module by module"
)]
pub mod api;
#[expect(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
pub mod audit;
pub(crate) mod backend;
#[expect(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::multiple_unsafe_ops_per_block,
    reason = "the Rust style rules, met module by module"
)]
pub mod catalog;
pub mod clock;
#[expect(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
pub mod commands;
pub mod config;
pub mod console;
#[expect(
    missing_docs,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
pub mod datadir;
pub mod envelope;
pub mod fs;
pub mod ids;
pub mod mcp;
pub mod query;
pub mod retention;
#[expect(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
pub mod serve;
#[expect(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
// Its one undocumented panic is in code built only for tests.
#[cfg_attr(
    test,
    expect(
        clippy::missing_panics_doc,
        reason = "the Rust style rules, met module by module"
    )
)]
pub mod store;
#[expect(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "the Rust style rules, met module by module"
)]
// Its one undocumented panic is in code built only for tests.
#[cfg_attr(
    test,
    expect(
        clippy::missing_panics_doc,
        reason = "the Rust style rules, met module by module"
    )
)]
pub mod tenants;
