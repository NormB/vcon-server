//! Latency summaries: exact nearest-rank percentiles over every sample.

/// The summary of one phase's latencies, in microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// How many samples.
    pub count: u64,
    /// The smallest.
    pub min: u64,
    /// The 50th percentile.
    pub p50: u64,
    /// The 95th percentile.
    pub p95: u64,
    /// The 99th percentile.
    pub p99: u64,
    /// The 99.9th percentile.
    pub p999: u64,
    /// The largest.
    pub max: u64,
}

/// The nearest-rank percentile `per_mille` (thousandths: 990 is p99) of
/// `sorted`, which is in ascending order; `None` when it is empty or
/// `per_mille` passes 1000.
#[must_use]
pub fn nearest_rank(sorted: &[u64], per_mille: u64) -> Option<u64> {
    if sorted.is_empty() || per_mille > 1000 {
        return None;
    }
    let n = sorted.len() as u128;
    // The rank is ceil(per_mille / 1000 * n), at least 1.
    let rank = (u128::from(per_mille) * n).div_ceil(1000).max(1);
    let index = usize::try_from(rank - 1).ok()?;
    sorted.get(index).copied()
}

/// Sorts `samples` and summarizes them; `None` when there are none.
pub fn summarize(samples: &mut [u64]) -> Option<Summary> {
    samples.sort_unstable();
    Some(Summary {
        count: samples.len() as u64,
        min: *samples.first()?,
        p50: nearest_rank(samples, 500)?,
        p95: nearest_rank(samples, 950)?,
        p99: nearest_rank(samples, 990)?,
        p999: nearest_rank(samples, 999)?,
        max: *samples.last()?,
    })
}

/// `count` events in `micros` microseconds, per second; `None` for no time.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a rate is reported, not counted: 53 bits are enough"
)]
pub fn per_second(count: u64, micros: u64) -> Option<f64> {
    if micros == 0 {
        return None;
    }
    Some(count as f64 * 1_000_000.0 / micros as f64)
}
