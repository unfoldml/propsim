//! The seeded random source — the *only* entropy in a deterministic run.
//!
//! SplitMix64: tiny, fast, and fully deterministic from a `u64` seed. It is the
//! same generator `proptest`/`rand` use to seed other PRNGs, which is more than
//! enough for sampling network delays, loss/dup rolls, and fault subsets.

use propsim_core::Rng;

/// A seeded SplitMix64 generator implementing [`propsim_core::Rng`].
#[derive(Clone, Debug)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// Seed the generator.
    pub fn new(seed: u64) -> Self {
        SimRng { state: seed }
    }

    /// The raw SplitMix64 step.
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A `bool` true with probability `p` (clamped to `[0,1]`).
    pub fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            return false;
        }
        if p >= 1.0 {
            return true;
        }
        // 53-bit uniform in [0,1).
        let u = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        u < p
    }

    /// A `u64` uniformly in `[lo, hi)`; returns `lo` if the range is empty.
    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next() % (hi - lo)
    }
}

impl Rng for SimRng {
    fn next_u64(&mut self) -> u64 {
        self.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_from_seed() {
        let mut a = SimRng::new(0xC0FFEE);
        let mut b = SimRng::new(0xC0FFEE);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SimRng::new(1);
        let mut b = SimRng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
