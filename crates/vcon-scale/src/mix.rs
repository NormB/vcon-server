//! The size mix: which share of the synthetic vCons has which size.

use std::fmt;

/// The default mix: 70% of 2 KiB, 25% of 16 KiB and 5% of 256 KiB.
pub const DEFAULT_MIX: &str = "70:2048,25:16384,5:262144";

/// Weighted sizes, in bytes, read from `weight:bytes,weight:bytes,...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeMix {
    classes: Vec<(u64, usize)>,
}

/// Why a mix was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MixError {
    /// No class at all.
    Empty,
    /// A part that is not `weight:bytes` with two whole numbers.
    Malformed(String),
    /// A part whose weight is 0.
    ZeroWeight(String),
    /// A part whose size is 0.
    ZeroSize(String),
    /// A size above the largest body the server accepts.
    TooLarge {
        /// The size asked for.
        bytes: usize,
        /// The largest allowed.
        max: usize,
    },
}

impl fmt::Display for MixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MixError::Empty => write!(f, "the size mix names no size"),
            MixError::Malformed(part) => {
                write!(f, "the size mix part {part:?} is not weight:bytes")
            }
            MixError::ZeroWeight(part) => write!(f, "the size mix part {part:?} has weight 0"),
            MixError::ZeroSize(part) => write!(f, "the size mix part {part:?} has size 0"),
            MixError::TooLarge { bytes, max } => write!(
                f,
                "the size mix names {bytes} bytes, above the body limit of {max}"
            ),
        }
    }
}

impl std::error::Error for MixError {}

impl SizeMix {
    /// Reads `text`; no size may pass `max_bytes`.
    ///
    /// # Errors
    ///
    /// A [`MixError`] naming the first part that is wrong.
    pub fn parse(text: &str, max_bytes: usize) -> Result<SizeMix, MixError> {
        if text.trim().is_empty() {
            return Err(MixError::Empty);
        }
        let mut classes = Vec::new();
        for raw in text.split(',') {
            let part = raw.trim();
            let malformed = || MixError::Malformed(part.to_owned());
            let (weight, bytes) = part.split_once(':').ok_or_else(malformed)?;
            let weight: u64 = weight.trim().parse().map_err(|_| malformed())?;
            let bytes: usize = bytes.trim().parse().map_err(|_| malformed())?;
            if weight == 0 {
                return Err(MixError::ZeroWeight(part.to_owned()));
            }
            if bytes == 0 {
                return Err(MixError::ZeroSize(part.to_owned()));
            }
            if bytes > max_bytes {
                return Err(MixError::TooLarge {
                    bytes,
                    max: max_bytes,
                });
            }
            classes.push((weight, bytes));
        }
        Ok(SizeMix { classes })
    }

    /// The sum of the weights.
    #[must_use]
    pub fn total_weight(&self) -> u64 {
        self.classes.iter().map(|(weight, _)| weight).sum()
    }

    /// The size a draw in `0..total_weight()` falls on; a larger draw
    /// falls on the last class.
    #[must_use]
    pub fn pick(&self, draw: u64) -> usize {
        let mut left = draw;
        for &(weight, bytes) in &self.classes {
            if left < weight {
                return bytes;
            }
            left -= weight;
        }
        self.classes.last().map_or(0, |&(_, bytes)| bytes)
    }

    /// The mean size in bytes, rounded down.
    #[must_use]
    pub fn mean_bytes(&self) -> u64 {
        let total: u128 = self
            .classes
            .iter()
            .map(|&(weight, bytes)| u128::from(weight) * bytes as u128)
            .sum();
        let weights = u128::from(self.total_weight()).max(1);
        u64::try_from(total / weights).unwrap_or(u64::MAX)
    }
}
