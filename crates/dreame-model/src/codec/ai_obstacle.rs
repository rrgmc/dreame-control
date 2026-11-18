//! AI_OBSTACLE_DETECTION (4.22): a bitmask on new models, JSON on old ones
//! (device-model.md 4.3). r2579a uses the integer form.

use super::{compact, decode_err, json};
use dreame_core::{RawValue, Result};
use serde_json::{Map, Value};

/// Bits of the integer form (`DreameVacuumAIProperty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AiFlags(pub u32);

impl AiFlags {
    pub const FURNITURE: u32 = 1;
    pub const OBSTACLE: u32 = 2;
    pub const OBSTACLE_PICTURE: u32 = 4;
    pub const FLUID: u32 = 8;
    pub const PET: u32 = 16;
    pub const OBSTACLE_IMAGE_UPLOAD: u32 = 32;
    pub const AI_IMAGE: u32 = 64;
    pub const PET_AVOIDANCE: u32 = 128;
    pub const FUZZY_OBSTACLE: u32 = 256;
    pub const PET_PICTURE: u32 = 512;
    pub const PET_FOCUSED: u32 = 1024;
    pub const LARGE_PARTICLES_BOOST: u32 = 2048;

    /// Bit names in bit order.
    pub const NAMES: [(u32, &'static str); 12] = [
        (Self::FURNITURE, "furniture detection"),
        (Self::OBSTACLE, "obstacle detection"),
        (Self::OBSTACLE_PICTURE, "obstacle picture"),
        (Self::FLUID, "fluid detection"),
        (Self::PET, "pet detection"),
        (Self::OBSTACLE_IMAGE_UPLOAD, "obstacle image upload"),
        (Self::AI_IMAGE, "AI image"),
        (Self::PET_AVOIDANCE, "pet avoidance"),
        (Self::FUZZY_OBSTACLE, "fuzzy obstacle detection"),
        (Self::PET_PICTURE, "pet picture"),
        (Self::PET_FOCUSED, "pet-focused detection"),
        (Self::LARGE_PARTICLES_BOOST, "large particles boost"),
    ];

    pub fn has(self, bit: u32) -> bool {
        self.0 & bit == bit
    }

    /// The whole integer with one bit changed, as Tasshack writes it.
    pub fn with(self, bit: u32, on: bool) -> Self {
        Self(if on { self.0 | bit } else { self.0 & !bit })
    }

    /// Names of the bits that are on.
    pub fn enabled(self) -> Vec<&'static str> {
        Self::NAMES.iter().filter(|(b, _)| self.has(*b)).map(|(_, n)| *n).collect()
    }

    /// Bits that are on but have no name.
    pub fn unnamed(self) -> u32 {
        self.0 & !Self::NAMES.iter().fold(0, |a, (b, _)| a | b)
    }
}

/// The decoded value in either form.
#[derive(Debug, Clone, PartialEq)]
pub enum AiObstacle {
    Bits(AiFlags),
    /// Old JSON form, for example `{"obstacle_detect_switch":true, ...}`.
    Switches(Map<String, Value>),
}

impl AiObstacle {
    pub fn decode(v: &RawValue) -> Result<Self> {
        if let Some(i) = v.as_i64() {
            if !(0..=u32::MAX as i64).contains(&i) {
                return Err(decode_err("4.22", v));
            }
            return Ok(Self::Bits(AiFlags(i as u32)));
        }
        match json(v)? {
            Value::Object(m) => Ok(Self::Switches(m)),
            _ => Err(decode_err("4.22", v)),
        }
    }

    /// The value to write after changing one setting. For the JSON form,
    /// Tasshack writes a partial object with only the changed switch.
    pub fn encode_bits(flags: AiFlags) -> RawValue {
        RawValue::Int(flags.0 as i64)
    }

    pub fn encode_switch(name: &str, on: bool) -> RawValue {
        let mut m = Map::new();
        m.insert(name.to_string(), Value::Bool(on));
        RawValue::Str(compact(&m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r2579a_271() {
        // 4.22 = 271 = 256 + 8 + 4 + 2 + 1 on r2579a (verified).
        let AiObstacle::Bits(f) = AiObstacle::decode(&RawValue::Int(271)).unwrap() else { panic!("int form expected") };
        assert_eq!(
            f.enabled(),
            vec!["furniture detection", "obstacle detection", "obstacle picture", "fluid detection", "fuzzy obstacle detection"]
        );
        assert!(!f.has(AiFlags::PET));
        assert_eq!(f.unnamed(), 0);
        assert_eq!(f.with(AiFlags::PET, true).0, 287);
        assert_eq!(f.with(AiFlags::FLUID, false).0, 263);
        assert_eq!(AiObstacle::decode(&RawValue::from_cache("271")).unwrap(), AiObstacle::Bits(f));
        assert_eq!(AiObstacle::encode_bits(f), RawValue::Int(271));
    }

    #[test]
    fn json_form() {
        let v = RawValue::Str(r#"{"obstacle_detect_switch":true,"whether_have_pet":false}"#.into());
        let AiObstacle::Switches(m) = AiObstacle::decode(&v).unwrap() else { panic!("json form expected") };
        assert_eq!(m["obstacle_detect_switch"], Value::Bool(true));
        assert_eq!(AiObstacle::encode_switch("whether_have_pet", true), RawValue::Str(r#"{"whether_have_pet":true}"#.into()));
        assert!(AiObstacle::decode(&RawValue::Int(-5)).is_err());
    }
}
