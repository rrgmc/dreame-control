//! FAULTS (4.18): a comma list of active codes (device-model.md 4.5).

use super::{decode_err, text};
use crate::enums::{ErrorCode, Open};
use dreame_core::{RawValue, Result};

/// The raw list as sent, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Faults {
    pub codes: Vec<i64>,
}

/// Context for [`Faults::active`], following Tasshack's filter
/// (T/device:1482-1524).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FaultFilter {
    /// Codes from the `IGNORED_WARNINGS` capability parameter.
    pub ignored: Vec<i64>,
    /// Capability `LEGACY_WARNINGS`: remap 84 to 85 and 85 to 86.
    pub legacy_warnings: bool,
    /// Drop 20 (battery low) while charging.
    pub charging: bool,
    /// Drop 68 on self-wash models; 68 follows a normal finished task there.
    pub self_wash_base: bool,
}

impl Faults {
    /// Decodes `"68"`, `"68,101"`, `"0"`, an empty string or a number.
    pub fn decode(v: &RawValue) -> Result<Self> {
        let s = text(v);
        let mut codes = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            codes.push(part.parse::<i64>().map_err(|_| decode_err("4.18", v))?);
        }
        Ok(Self { codes })
    }

    /// The codes as open enums, unfiltered.
    pub fn all(&self) -> Vec<Open<ErrorCode>> {
        self.codes.iter().map(|c| Open::from_code(*c)).collect()
    }

    /// The codes an app should show as active faults.
    pub fn active(&self, f: &FaultFilter) -> Vec<Open<ErrorCode>> {
        self.codes
            .iter()
            .filter(|c| !f.ignored.contains(c))
            .map(|&c| match (f.legacy_warnings, c) {
                (true, 84) => 85,
                (true, 85) => 86,
                _ => c,
            })
            .filter(|&c| !matches!(c, 0 | 84 | 122))
            .filter(|&c| !(c == 20 && f.charging))
            .filter(|&c| !(c == 68 && f.self_wash_base))
            .map(Open::from_code)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_68_is_not_a_fault_on_self_wash_models() {
        let f = Faults::decode(&RawValue::from_cache("68")).unwrap();
        assert_eq!(f.codes, vec![68]);
        assert_eq!(f.all(), vec![Open::Known(ErrorCode::RemoveMop)]);
        let ctx = FaultFilter { self_wash_base: true, ..Default::default() };
        assert!(f.active(&ctx).is_empty());
        assert_eq!(f.active(&FaultFilter::default()).len(), 1);
    }

    #[test]
    fn filters() {
        let f = Faults::decode(&RawValue::Str("0,20,84,101,122,999".into())).unwrap();
        let ctx = FaultFilter { charging: true, ..Default::default() };
        assert_eq!(f.active(&ctx), vec![Open::Known(ErrorCode::BinFull), Open::Unknown(999)]);
        let legacy = FaultFilter { legacy_warnings: true, ignored: vec![101], ..Default::default() };
        let codes: Vec<i64> = f.active(&legacy).iter().map(|c| c.code()).collect();
        assert_eq!(codes, vec![20, 85, 999]);
    }

    #[test]
    fn empty_and_zero() {
        assert!(Faults::decode(&RawValue::Str(String::new())).unwrap().codes.is_empty());
        assert!(Faults::decode(&RawValue::Int(0)).unwrap().active(&FaultFilter::default()).is_empty());
        assert!(Faults::decode(&RawValue::Str("a,b".into())).is_err());
    }
}
