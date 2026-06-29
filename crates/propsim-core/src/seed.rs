//! The [`Seed`] newtype.
//!
//! The seed reproduces the *deterministic executor only* (architecture.md §14).
//! `Display`/`FromStr` round-trip the `0x…` hex form so that the reproduction
//! block and the `PROPSIM_SEED=0x…` env idiom (mirroring `PROPTEST_*`) speak the
//! same notation.

use std::fmt;
use std::str::FromStr;

/// An 8-byte seed for the deterministic executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Seed(pub u64);

impl fmt::Display for Seed {
    /// Renders as `0x` followed by 16 lowercase hex digits (zero-padded), so the
    /// width is stable across seeds.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:016x}", self.0)
    }
}

/// An error parsing a [`Seed`] from a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedParseError(String);

impl fmt::Display for SeedParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid seed `{}`: expected a u64, optionally 0x-prefixed",
            self.0
        )
    }
}

impl std::error::Error for SeedParseError {}

impl FromStr for Seed {
    type Err = SeedParseError;

    /// Accepts both the canonical `0x…` hex form and a plain decimal `u64`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let parsed = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16)
        } else {
            s.parse::<u64>()
        };
        parsed.map(Seed).map_err(|_| SeedParseError(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_zero_padded_hex() {
        assert_eq!(Seed(0).to_string(), "0x0000000000000000");
        assert_eq!(Seed(0xC0FFEE).to_string(), "0x0000000000c0ffee");
        assert_eq!(Seed(0x9f3a17c4e2b01a55).to_string(), "0x9f3a17c4e2b01a55");
    }

    #[test]
    fn round_trips_through_string() {
        for raw in [0u64, 1, 0xC0FFEE, 0x9f3a17c4e2b01a55, u64::MAX] {
            let s = Seed(raw);
            let back: Seed = s.to_string().parse().unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn parses_decimal_and_hex() {
        assert_eq!("255".parse::<Seed>().unwrap(), Seed(255));
        assert_eq!("0xff".parse::<Seed>().unwrap(), Seed(255));
        assert_eq!("0XFF".parse::<Seed>().unwrap(), Seed(255));
    }

    #[test]
    fn rejects_garbage() {
        assert!("not-a-seed".parse::<Seed>().is_err());
        assert!("0xZZ".parse::<Seed>().is_err());
    }
}
