//! CLEANING_MODE (4.23): a packed integer (device-model.md 4.1).
//!
//! Layout when `hi < 4`: bits 0-1 raw cleaning mode, bits 8-15 self-clean
//! frequency (m² or minutes), bits 16-23 water level or mop pad humidity.
//! The polarity of raw modes 0 and 2 is disputed between Tasshack and
//! node-dreame, so [`CleaningModeGroup::logical`] takes a [`ModePolarity`].

use super::{decode_err, int};
use crate::enums::{CleaningMode, Open};
use dreame_core::{Error, RawValue, Result};

/// Which reading of raw modes 0 and 2 to use. Raw 1 and 3 agree in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModePolarity {
    /// Tasshack: 0 sweeping and mopping, 2 sweeping.
    #[default]
    Tasshack,
    /// node-dreame: 0 sweeping, 2 sweeping and mopping.
    NodeDreame,
}

/// The split fields of 4.23.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleaningModeGroup {
    /// The value as read, kept for round trips.
    pub raw: i64,
    /// Raw cleaning mode, bits 0-1 (bit 0 only without mop pad lifting).
    pub mode: u8,
    /// Self-clean frequency value: m² or minutes, per `BackWashType`.
    pub mid: u32,
    /// Water level or mop pad humidity 1..3; 0 on wetness-level models.
    pub hi: u32,
}

impl CleaningModeGroup {
    /// Splits a value as Tasshack's `split_group_value` does.
    /// `mop_pad_lifting` selects a 2-bit mode field instead of 1 bit.
    pub fn split(value: i64, mop_pad_lifting: bool) -> Self {
        let mode = if mop_pad_lifting { value & 0x03 } else { value & 0x01 } as u8;
        // This clears only bits 8-9 of (value >> 8), which hold `hi` when
        // `hi < 4`. Larger `hi` values leak into `mid`, as in Tasshack.
        let mid = ((value >> 8) & !0x300) as u32;
        let hi = (value >> 16) as u32;
        Self { raw: value, mode, mid, hi }
    }

    /// Decodes a property value.
    pub fn decode(v: &RawValue, mop_pad_lifting: bool) -> Result<Self> {
        Ok(Self::split(int(v)?, mop_pad_lifting))
    }

    /// Packs the fields as Tasshack's `combine_group_value` does.
    pub fn combine(mode: u8, mid: u32, hi: u32) -> i64 {
        ((((hi as i64) << 8) ^ mid as i64) << 8) ^ mode as i64
    }

    /// The packed value of these fields.
    pub fn value(&self) -> i64 {
        Self::combine(self.mode, self.mid, self.hi)
    }

    /// The logical mode under the chosen polarity.
    pub fn logical(&self, polarity: ModePolarity) -> Open<CleaningMode> {
        let code = match (self.mode, polarity) {
            (0, ModePolarity::Tasshack) | (2, ModePolarity::NodeDreame) => 2,
            (2, ModePolarity::Tasshack) | (0, ModePolarity::NodeDreame) => 0,
            (1, _) => 1,
            (3, _) => 3,
            (m, _) => return Open::Unknown(m as i64),
        };
        Open::from_code(code)
    }

    /// A copy with a new logical mode, keeping `mid` and `hi`.
    ///
    /// Refuses when `mid` is 0: node-dreame reports that such a write breaks
    /// the next clean. The 0/2 polarity is not verified on r2579a, so check
    /// it before sending the result.
    pub fn with_logical(&self, mode: CleaningMode, polarity: ModePolarity) -> Result<Self> {
        if self.mid == 0 {
            return Err(Error::Unsupported("4.23 with self-clean value 0 breaks the next clean".into()));
        }
        let raw_mode = match (mode, polarity) {
            (CleaningMode::SweepingAndMopping, ModePolarity::Tasshack) | (CleaningMode::Sweeping, ModePolarity::NodeDreame) => 0,
            (CleaningMode::Sweeping, ModePolarity::Tasshack) | (CleaningMode::SweepingAndMopping, ModePolarity::NodeDreame) => 2,
            (CleaningMode::Mopping, _) => 1,
            (CleaningMode::MoppingAfterSweeping, _) => 3,
        };
        let mut out = *self;
        out.mode = raw_mode;
        out.raw = out.value();
        Ok(out)
    }
}

/// Checks that a value fits the normal layout (`hi < 4`).
pub fn validate(v: i64) -> Result<()> {
    if v < 0 || (v >> 16) >= 4 {
        return Err(decode_err("4.23 layout", v));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_5123() {
        // 4.23 = 5123 = 0x00_14_03 on r2579a (verified).
        let g = CleaningModeGroup::decode(&RawValue::Int(5123), true).unwrap();
        assert_eq!((g.mode, g.mid, g.hi), (3, 20, 0));
        assert_eq!(g.value(), 5123);
        let m = CleaningMode::MoppingAfterSweeping;
        assert_eq!(g.logical(ModePolarity::Tasshack), Open::Known(m));
        assert_eq!(g.logical(ModePolarity::NodeDreame), Open::Known(m));
        // The cache sends the same value as a string.
        assert_eq!(CleaningModeGroup::decode(&RawValue::from_cache("5123"), true).unwrap(), g);
    }

    #[test]
    fn r2579a_5122() {
        // 5122 was also seen live: raw mode 2, self-clean every 20 m².
        let g = CleaningModeGroup::split(5122, true);
        assert_eq!((g.mode, g.mid, g.hi), (2, 20, 0));
        assert_eq!(g.logical(ModePolarity::Tasshack), Open::Known(CleaningMode::Sweeping));
        assert_eq!(g.logical(ModePolarity::NodeDreame), Open::Known(CleaningMode::SweepingAndMopping));
        assert_eq!(g.value(), 5122);
    }

    #[test]
    fn without_mop_lifting_only_bit_0_counts() {
        let g = CleaningModeGroup::split(5123, false);
        assert_eq!(g.mode, 1);
    }

    #[test]
    fn combine_matches_the_docs() {
        assert_eq!(CleaningModeGroup::combine(0, 20, 0), 5120);
        assert_eq!(CleaningModeGroup::combine(2, 20, 0), 5122);
        assert_eq!(CleaningModeGroup::combine(1, 20, 2), 0x02_14_01);
        let g = CleaningModeGroup::split(0x02_14_01, true);
        assert_eq!((g.mode, g.mid, g.hi), (1, 20, 2));
    }

    #[test]
    fn with_logical_keeps_mid_and_refuses_zero() {
        let g = CleaningModeGroup::split(5123, true);
        let t = g.with_logical(CleaningMode::SweepingAndMopping, ModePolarity::Tasshack).unwrap();
        assert_eq!(t.raw, 5120);
        let n = g.with_logical(CleaningMode::SweepingAndMopping, ModePolarity::NodeDreame).unwrap();
        assert_eq!(n.raw, 5122);
        assert!(CleaningModeGroup::split(3, true).with_logical(CleaningMode::Mopping, ModePolarity::Tasshack).is_err());
    }

    #[test]
    fn layout_check() {
        assert!(validate(5123).is_ok());
        assert!(validate(0x04_00_00).is_err());
        assert!(validate(-1).is_err());
    }
}
