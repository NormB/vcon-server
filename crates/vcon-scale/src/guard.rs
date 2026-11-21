//! The start guard and the cost projection, for a shared host: no run starts
//! above the load cap, and none whose projected disk use passes its cap.

use std::fmt;

/// The 1-minute load from the text of `/proc/loadavg`.
#[must_use]
pub fn load_one(loadavg: &str) -> Option<f64> {
    loadavg.split_whitespace().next()?.parse().ok()
}

/// Whether a run may start at `load` under `cap`: at or below it. A load
/// that is not a number never starts.
#[must_use]
pub fn may_start(load: f64, cap: f64) -> bool {
    load <= cap
}

/// A run's projected cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Projection {
    /// Disk: the count times the bytes per vCon.
    pub disk_bytes: u64,
    /// Time: the count over the rate, rounded up.
    pub seconds: u64,
}

/// Why a projection refused a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// A rate of 0 vCons a second.
    ZeroRate,
    /// The disk use does not fit in 64 bits.
    Overflow,
    /// The disk use passes the cap.
    DiskOverCap {
        /// The bytes the run needs.
        need: u64,
        /// The cap.
        cap: u64,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::ZeroRate => write!(f, "a rate of 0 vCons a second never ends"),
            PlanError::Overflow => write!(f, "the projected disk use does not fit in 64 bits"),
            PlanError::DiskOverCap { need, cap } => write!(
                f,
                "the run needs {need} bytes of disk, above the cap of {cap}"
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// The cost of storing `count` vCons of `bytes_per_vcon` at `per_second`.
///
/// # Errors
///
/// A [`PlanError`] when the rate is 0, the disk use overflows or passes
/// `disk_cap`.
pub fn project(
    count: u64,
    bytes_per_vcon: u64,
    per_second: u64,
    disk_cap: u64,
) -> Result<Projection, PlanError> {
    if per_second == 0 {
        return Err(PlanError::ZeroRate);
    }
    let disk_bytes = count
        .checked_mul(bytes_per_vcon)
        .ok_or(PlanError::Overflow)?;
    if disk_bytes > disk_cap {
        return Err(PlanError::DiskOverCap {
            need: disk_bytes,
            cap: disk_cap,
        });
    }
    Ok(Projection {
        disk_bytes,
        seconds: count.div_ceil(per_second),
    })
}
