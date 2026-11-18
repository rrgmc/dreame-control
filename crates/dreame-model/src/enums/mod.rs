//! Open enums for coded property values.
//!
//! Every enum is an open set: [`Open::Known`] for a listed code and
//! [`Open::Unknown`] for any other. The raw code is always available through
//! [`Open::code`], so nothing is lost when firmware adds values. Labels are
//! hints taken from Tasshack's tables; meanings can differ by firmware
//! (docs/research/device-model.md section 3).

use serde::{Serialize, Serializer};
use std::fmt;

/// A closed list of known codes. Implemented by every enum in this module.
pub trait CodeEnum: Sized + Copy + Eq + fmt::Debug + 'static {
    /// The Rust type name, for messages.
    const NAME: &'static str;
    /// Every known variant, in table order.
    const ALL: &'static [Self];
    fn from_code(code: i64) -> Option<Self>;
    fn code(self) -> i64;
    /// A lower-case English hint, for example `"drying"`.
    fn label(self) -> &'static str;
}

/// A coded value that may or may not be in the known table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Open<T> {
    Known(T),
    Unknown(i64),
}

impl<T: CodeEnum> Open<T> {
    pub fn from_code(code: i64) -> Self {
        T::from_code(code).map(Self::Known).unwrap_or(Self::Unknown(code))
    }

    /// The raw code, known or not.
    pub fn code(&self) -> i64 {
        match self {
            Self::Known(t) => t.code(),
            Self::Unknown(c) => *c,
        }
    }

    /// The label hint, or `None` for an unknown code.
    pub fn label(&self) -> Option<&'static str> {
        match self {
            Self::Known(t) => Some(t.label()),
            Self::Unknown(_) => None,
        }
    }

    pub fn known(&self) -> Option<T> {
        match self {
            Self::Known(t) => Some(*t),
            Self::Unknown(_) => None,
        }
    }

    pub fn is(&self, t: T) -> bool {
        self.known() == Some(t)
    }
}

impl<T: CodeEnum> From<T> for Open<T> {
    fn from(t: T) -> Self {
        Self::Known(t)
    }
}

impl<T: CodeEnum> fmt::Display for Open<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(t) => write!(f, "{} ({})", t.code(), t.label()),
            Self::Unknown(c) => write!(f, "{c} (unknown)"),
        }
    }
}

/// Serialises as `{"code": n, "label": "..."}` so the raw code is kept.
impl<T: CodeEnum> Serialize for Open<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Open", 2)?;
        st.serialize_field("code", &self.code())?;
        st.serialize_field("label", &self.label())?;
        st.end()
    }
}

/// Declares a closed enum with codes and labels, and implements [`CodeEnum`].
macro_rules! open_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $code:literal => $label:literal,)* }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($var,)*
        }

        impl CodeEnum for $name {
            const NAME: &'static str = stringify!($name);
            const ALL: &'static [Self] = &[$(Self::$var,)*];
            fn from_code(code: i64) -> Option<Self> {
                match code {
                    $($code => Some(Self::$var),)*
                    _ => None,
                }
            }
            fn code(self) -> i64 {
                match self {
                    $(Self::$var => $code,)*
                }
            }
            fn label(self) -> &'static str {
                match self {
                    $(Self::$var => $label,)*
                }
            }
        }
    };
}

include!("table.rs");

/// Names an enum type at run time, so profile rows can point at one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnumKind {
    State,
    VacuumStatus,
    TaskStatus,
    TaskType,
    ErrorCode,
    ChargingStatus,
    SuctionLevel,
    WaterTank,
    WaterVolume,
    CarpetSensitivity,
    CarpetCleaning,
    RelocationStatus,
    DustCollection,
    AutoEmptyStatus,
    SelfWashBaseStatus,
    MopWashLevel,
    CleanGeniusMode,
    LowWaterWarning,
    DrainageStatus,
    MapRecoveryStatus,
    MapBackupStatus,
    CleanWaterTankStatus,
    DirtyWaterTankStatus,
    DustBagStatus,
    DetergentStatus,
    HotWaterStatus,
    StationDrainageStatus,
    DustBagDryingStatus,
    WaterTemperature,
    CleanLogStatus,
}

impl EnumKind {
    /// The label hint for a code, or `None` when the code is not known.
    pub fn label(self, code: i64) -> Option<&'static str> {
        fn l<T: CodeEnum>(code: i64) -> Option<&'static str> {
            T::from_code(code).map(T::label)
        }
        match self {
            Self::State => l::<State>(code),
            Self::VacuumStatus => l::<VacuumStatus>(code),
            Self::TaskStatus => l::<TaskStatus>(code),
            Self::TaskType => l::<TaskType>(code),
            Self::ErrorCode => l::<ErrorCode>(code),
            Self::ChargingStatus => l::<ChargingStatus>(code),
            Self::SuctionLevel => l::<SuctionLevel>(code),
            Self::WaterTank => l::<WaterTank>(code),
            Self::WaterVolume => l::<WaterVolume>(code),
            Self::CarpetSensitivity => l::<CarpetSensitivity>(code),
            Self::CarpetCleaning => l::<CarpetCleaning>(code),
            Self::RelocationStatus => l::<RelocationStatus>(code),
            Self::DustCollection => l::<DustCollection>(code),
            Self::AutoEmptyStatus => l::<AutoEmptyStatus>(code),
            Self::SelfWashBaseStatus => l::<SelfWashBaseStatus>(code),
            Self::MopWashLevel => l::<MopWashLevel>(code),
            Self::CleanGeniusMode => l::<CleanGeniusMode>(code),
            Self::LowWaterWarning => l::<LowWaterWarning>(code),
            Self::DrainageStatus => l::<DrainageStatus>(code),
            Self::MapRecoveryStatus => l::<MapRecoveryStatus>(code),
            Self::MapBackupStatus => l::<MapBackupStatus>(code),
            Self::CleanWaterTankStatus => l::<CleanWaterTankStatus>(code),
            Self::DirtyWaterTankStatus => l::<DirtyWaterTankStatus>(code),
            Self::DustBagStatus => l::<DustBagStatus>(code),
            Self::DetergentStatus => l::<DetergentStatus>(code),
            Self::HotWaterStatus => l::<HotWaterStatus>(code),
            Self::StationDrainageStatus => l::<StationDrainageStatus>(code),
            Self::DustBagDryingStatus => l::<DustBagDryingStatus>(code),
            Self::WaterTemperature => l::<WaterTemperature>(code),
            Self::CleanLogStatus => l::<CleanLogStatus>(code),
        }
    }
}

/// Auto-empty mode 15.1. The code table depends on the model capability
/// `INTELLIGENT_AUTO_EMPTY`, so the caller picks the reading.
pub fn auto_empty_mode_label(code: i64, intelligent: bool) -> Option<&'static str> {
    if intelligent {
        AutoEmptyModeV2::from_code(code).map(AutoEmptyModeV2::label)
    } else {
        AutoEmptyMode::from_code(code).map(AutoEmptyMode::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes_unique<T: CodeEnum>() {
        let mut seen = std::collections::BTreeSet::new();
        for v in T::ALL {
            assert!(seen.insert(v.code()), "{}: duplicate code {}", T::NAME, v.code());
            assert_eq!(T::from_code(v.code()), Some(*v), "{}: round trip", T::NAME);
            assert!(!v.label().is_empty());
        }
    }

    #[test]
    fn every_table_round_trips() {
        codes_unique::<State>();
        codes_unique::<StateOld>();
        codes_unique::<VacuumStatus>();
        codes_unique::<TaskStatus>();
        codes_unique::<TaskType>();
        codes_unique::<ErrorCode>();
        codes_unique::<ChargingStatus>();
        codes_unique::<SuctionLevel>();
        codes_unique::<CleaningMode>();
        codes_unique::<WaterTank>();
        codes_unique::<WaterVolume>();
        codes_unique::<MopPadHumidity>();
        codes_unique::<CarpetSensitivity>();
        codes_unique::<CarpetCleaning>();
        codes_unique::<RelocationStatus>();
        codes_unique::<DustCollection>();
        codes_unique::<AutoEmptyStatus>();
        codes_unique::<SelfWashBaseStatus>();
        codes_unique::<MopWashLevel>();
        codes_unique::<MoppingType>();
        codes_unique::<CleaningRoute>();
        codes_unique::<SelfCleanFrequency>();
        codes_unique::<MopExtendFrequency>();
        codes_unique::<AutoEmptyMode>();
        codes_unique::<AutoEmptyModeV2>();
        codes_unique::<CleanGenius>();
        codes_unique::<CleanGeniusMode>();
        codes_unique::<SecondCleaning>();
        codes_unique::<FloorMaterial>();
        codes_unique::<LowWaterWarning>();
        codes_unique::<DrainageStatus>();
        codes_unique::<MapRecoveryStatus>();
        codes_unique::<MapBackupStatus>();
        codes_unique::<CleanWaterTankStatus>();
        codes_unique::<DirtyWaterTankStatus>();
        codes_unique::<DustBagStatus>();
        codes_unique::<DetergentStatus>();
        codes_unique::<HotWaterStatus>();
        codes_unique::<StationDrainageStatus>();
        codes_unique::<DustBagDryingStatus>();
        codes_unique::<WaterTemperature>();
        codes_unique::<CleanLogStatus>();
    }

    #[test]
    fn r2579a_values_decode() {
        // Cached values from docs/research/l40-ultra-ae.md and device-model.md.
        assert_eq!(Open::<State>::from_code(8), Open::Known(State::Drying));
        assert_eq!(Open::<VacuumStatus>::from_code(14), Open::Known(VacuumStatus::Sleeping));
        assert_eq!(Open::<TaskStatus>::from_code(0), Open::Known(TaskStatus::Completed));
        assert_eq!(Open::<TaskType>::from_code(0), Open::Known(TaskType::Idle));
        assert_eq!(Open::<TaskType>::from_code(17), Open::Known(TaskType::BoostedEdgeCleaning));
        assert_eq!(Open::<ErrorCode>::from_code(68), Open::Known(ErrorCode::RemoveMop));
        assert_eq!(Open::<ChargingStatus>::from_code(1), Open::Known(ChargingStatus::Charging));
        assert_eq!(Open::<SuctionLevel>::from_code(2), Open::Known(SuctionLevel::Strong));
        assert_eq!(Open::<SelfWashBaseStatus>::from_code(2), Open::Known(SelfWashBaseStatus::Drying));
        assert_eq!(Open::<MopWashLevel>::from_code(1), Open::Known(MopWashLevel::Daily));
        assert_eq!(Open::<CleanGeniusMode>::from_code(2).code(), 2);
        assert_eq!(Open::<DustCollection>::from_code(1), Open::Known(DustCollection::Available));
        assert_eq!(Open::<CleanLogStatus>::from_code(1), Open::Known(CleanLogStatus::Completed));
    }

    #[test]
    fn unknown_codes_are_kept() {
        let s = Open::<State>::from_code(4242);
        assert_eq!(s, Open::Unknown(4242));
        assert_eq!(s.code(), 4242);
        assert_eq!(s.label(), None);
        assert_eq!(s.to_string(), "4242 (unknown)");
        assert_eq!(Open::<State>::from_code(8).to_string(), "8 (drying)");
        let j = serde_json::to_value(Open::<State>::from_code(8)).unwrap();
        assert_eq!(j, serde_json::json!({"code": 8, "label": "drying"}));
    }

    #[test]
    fn enum_kind_dispatch() {
        assert_eq!(EnumKind::State.label(8), Some("drying"));
        assert_eq!(EnumKind::ErrorCode.label(100000), None);
        assert_eq!(auto_empty_mode_label(2, false), Some("high frequency"));
        assert_eq!(auto_empty_mode_label(2, true), Some("intelligent"));
    }
}
