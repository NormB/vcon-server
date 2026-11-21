//! SplitMix64: a small seeded generator, so a run's vCons can be made again.

/// SplitMix64's step, the 64-bit golden ratio.
const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

/// The generator's state.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator that starts from `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        finalize(self.state)
    }

    /// A number in `0..n`, without modulo bias worth counting (Lemire's
    /// multiply-high); 0 when `n` is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        let wide = u128::from(self.next_u64()) * u128::from(n);
        // The high half of a 64 x 64 bit product fits in 64 bits.
        u64::try_from(wide >> 64).unwrap_or(0)
    }
}

/// SplitMix64's output function: mixes the bits of `x`.
#[must_use]
pub const fn finalize(x: u64) -> u64 {
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
