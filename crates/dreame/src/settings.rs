//! Robot settings as one typed list: each [`Setting`] knows its property, how to read its
//! current value from a [`Status`], and the property writes that change it.
//!
//! Write forms follow the official app's r2579a plugin where it shows them: the AI detection
//! switches (4.22) are the whole integer with one bit changed, and the settings store (4.50) is
//! one `{"k","v"}` object per change. Changing a setting is a write to the robot: ask first.

use anyhow::{anyhow, bail, Result};
use dreame_core::{PropId, RawValue};
use dreame_model::codec::ai_obstacle::{AiFlags, AiObstacle};
use dreame_model::codec::auto_switch::AutoSwitchSettings;
use dreame_model::codec::off_peak::OffPeakCharging;
use dreame_model::codec::HhMm;
use dreame_model::enums::{CarpetSensitivity, CodeEnum};
use dreame_model::{Command, Status};


/// One setting and the value to give it.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    /// Speaker volume, 0 to 100 (7.1).
    Volume(u8),
    /// Child lock (4.27).
    ChildLock(bool),
    /// More suction on carpets (4.12).
    CarpetBoost(bool),
    /// Resume the job after recharging (4.11).
    ResumeCleaning(bool),
    /// Clean each room with its own settings (4.26).
    CustomizedCleaning(bool),
    /// Wash the mop during the job (4.34).
    SelfClean(bool),
    /// Mop drying time in hours, 1 to 12 (4.40).
    DryingTimeHours(u8),
    /// Carpet detection sensitivity (4.28).
    CarpetSensitivity(CarpetSensitivity),
    /// Cleans between automatic bin emptying, 1 to 10 (15.2).
    AutoEmptyFrequency(u8),
    /// The do-not-disturb window (5.1 to 5.3).
    Dnd { enabled: bool, start: HhMm, end: HhMm },
    /// Off-peak charging (3.3). Fields the robot stores beyond these three are kept.
    OffPeak { enabled: bool, start: HhMm, end: HhMm },
    /// One AI detection switch (4.22): `bit` is one of the [`AiFlags`] constants.
    AiFlag { bit: u32, on: bool },
    /// One entry of the settings store (4.50), by key, such as `MeticulousTwist`.
    AutoSwitch { key: String, value: i64 },
}

fn p(siid: u32, piid: u32) -> PropId {
    PropId::new(siid, piid)
}

impl Setting {
    /// The properties this setting writes.
    pub fn properties(&self) -> Vec<PropId> {
        match self {
            Self::Volume(_) => vec![p(7, 1)],
            Self::ChildLock(_) => vec![p(4, 27)],
            Self::CarpetBoost(_) => vec![p(4, 12)],
            Self::ResumeCleaning(_) => vec![p(4, 11)],
            Self::CustomizedCleaning(_) => vec![p(4, 26)],
            Self::SelfClean(_) => vec![p(4, 34)],
            Self::DryingTimeHours(_) => vec![p(4, 40)],
            Self::CarpetSensitivity(_) => vec![p(4, 28)],
            Self::AutoEmptyFrequency(_) => vec![p(15, 2)],
            Self::Dnd { .. } => vec![p(5, 1), p(5, 2), p(5, 3)],
            Self::OffPeak { .. } => vec![p(3, 3)],
            Self::AiFlag { .. } => vec![p(4, 22)],
            Self::AutoSwitch { .. } => vec![p(4, 50)],
        }
    }

    /// A short name, for listings and confirmations.
    pub fn name(&self) -> String {
        match self {
            Self::Volume(_) => "volume".into(),
            Self::ChildLock(_) => "child lock".into(),
            Self::CarpetBoost(_) => "carpet boost".into(),
            Self::ResumeCleaning(_) => "resume after recharge".into(),
            Self::CustomizedCleaning(_) => "per-room settings".into(),
            Self::SelfClean(_) => "wash mop during the job".into(),
            Self::DryingTimeHours(_) => "mop drying time".into(),
            Self::CarpetSensitivity(_) => "carpet sensitivity".into(),
            Self::AutoEmptyFrequency(_) => "cleans between auto-empties".into(),
            Self::Dnd { .. } => "do not disturb".into(),
            Self::OffPeak { .. } => "off-peak charging".into(),
            Self::AiFlag { bit, .. } => ai_name(*bit).map_or_else(|| format!("AI bit {bit}"), str::to_string),
            Self::AutoSwitch { key, .. } => format!("setting {key}"),
        }
    }

    /// The setting's current value in `status`, as text; `None` when it was not read.
    pub fn current(&self, s: &Status) -> Option<String> {
        let raw = |id: PropId| s.raw.get(&id).map(|v| v.to_json().to_string());
        Some(match self {
            Self::Volume(_) => s.volume?.to_string(),
            Self::ChildLock(_) => s.child_lock?.to_string(),
            Self::CarpetBoost(_) => s.carpet_boost?.to_string(),
            Self::ResumeCleaning(_) => s.resume_cleaning?.to_string(),
            Self::Dnd { .. } => format!(
                "{} {}-{}",
                s.dnd.enabled?,
                s.dnd.start.map(|t| t.to_string()).unwrap_or_default(),
                s.dnd.end.map(|t| t.to_string()).unwrap_or_default()
            ),
            Self::OffPeak { .. } => {
                let o = s.off_peak.as_ref()?;
                format!("{} {}-{}", o.enable, o.start, o.end)
            }
            Self::AiFlag { bit, .. } => match s.ai_obstacle.as_ref()? {
                AiObstacle::Bits(f) => f.has(*bit).to_string(),
                AiObstacle::Switches(m) => format!("{m:?}"),
            },
            Self::AutoSwitch { key, .. } => s.settings.as_ref()?.get(key)?.to_string(),
            other => raw(other.properties()[0])?,
        })
    }

    /// The property writes that give the setting its value. Settings that share a property with
    /// others (4.22) need the current value from `status`; refresh it first.
    pub fn commands(&self, status: &Status) -> Result<Vec<Command>> {
        let int = |id: PropId, v: i64| Command::write(id, RawValue::Int(v));
        // An on/off setting is written in the type the robot stores: r2579a keeps 4.26 and 4.34
        // as 0 and 1 although they read as switches, and 4.27 and 4.12 as booleans.
        let flag = |id: PropId, on: bool| match status.raw.get(&id) {
            Some(RawValue::Int(_)) => Command::write(id, RawValue::Int(i64::from(on))),
            _ => Command::write(id, RawValue::Bool(on)),
        };
        let id = self.properties()[0];
        Ok(match self {
            Self::Volume(v) if *v <= 100 => vec![int(id, i64::from(*v))],
            Self::ChildLock(on) | Self::CarpetBoost(on) | Self::ResumeCleaning(on) | Self::CustomizedCleaning(on) | Self::SelfClean(on) => {
                vec![flag(id, *on)]
            }
            Self::DryingTimeHours(h) if (1..=12).contains(h) => vec![int(id, i64::from(*h))],
            Self::CarpetSensitivity(level) => vec![int(id, level.code())],
            Self::AutoEmptyFrequency(n) if (1..=10).contains(n) => vec![int(id, i64::from(*n))],
            Self::Dnd { enabled, start, end } => {
                dreame_model::codec::dnd::DndWindow::encode(*enabled, *start, *end).into_iter().map(|(id, v)| Command::write(id, v)).collect()
            }
            Self::OffPeak { enabled, start, end } => {
                let extra = status.off_peak.as_ref().map(|o| o.extra.clone()).unwrap_or_default();
                let o = OffPeakCharging { enable: *enabled, start: *start, end: *end, extra };
                vec![Command::write(id, o.encode())]
            }
            Self::AiFlag { bit, on } => {
                if ai_name(*bit).is_none() {
                    bail!("AI bit {bit} is not a known single switch");
                }
                match &status.ai_obstacle {
                    Some(AiObstacle::Bits(f)) => vec![Command::write(id, AiObstacle::encode_bits(f.with(*bit, *on)))],
                    Some(AiObstacle::Switches(_)) => bail!("this robot stores 4.22 as JSON switches; not supported yet"),
                    None => bail!("the AI switches (4.22) are not known; refresh first"),
                }
            }
            Self::AutoSwitch { key, value } => {
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
                    bail!("setting key {key:?} is not a plain name");
                }
                vec![Command::write(id, AutoSwitchSettings::encode_single(key, *value))]
            }
            other => return Err(anyhow!("{} value out of range: {other:?}", other.name())),
        })
    }
}

/// Every property a setting reads. Read these live (see `Robot::read`) before [`known`], since
/// the cloud cache can lag behind the robot.
pub fn all_properties() -> Vec<PropId> {
    let t = HhMm { hour: 0, minute: 0 };
    [
        Setting::Volume(0),
        Setting::ChildLock(false),
        Setting::CarpetBoost(false),
        Setting::ResumeCleaning(false),
        Setting::CustomizedCleaning(false),
        Setting::SelfClean(false),
        Setting::DryingTimeHours(1),
        Setting::CarpetSensitivity(CarpetSensitivity::Low),
        Setting::AutoEmptyFrequency(1),
        Setting::Dnd { enabled: false, start: t, end: t },
        Setting::OffPeak { enabled: false, start: t, end: t },
        Setting::AiFlag { bit: 0, on: false },
        Setting::AutoSwitch { key: String::new(), value: 0 },
    ]
    .iter()
    .flat_map(Setting::properties)
    .collect()
}

/// Every setting whose value is known in `s`, each holding that value. The order is fixed: the
/// single settings, then one entry per named AI switch, then each integer key of the settings
/// store. A setting the robot lacks or that was not read is left out.
pub fn known(s: &Status) -> Vec<Setting> {
    let int = |siid, piid| s.raw.get(&p(siid, piid)).and_then(RawValue::as_i64);
    let small = |v: i64| u8::try_from(v).ok();
    let mut out = Vec::new();
    out.extend(s.volume.and_then(small).map(Setting::Volume));
    out.extend(s.child_lock.map(Setting::ChildLock));
    out.extend(s.carpet_boost.map(Setting::CarpetBoost));
    out.extend(s.resume_cleaning.map(Setting::ResumeCleaning));
    out.extend(int(4, 26).map(|v| Setting::CustomizedCleaning(v != 0)));
    out.extend(int(4, 34).map(|v| Setting::SelfClean(v != 0)));
    out.extend(int(4, 40).and_then(small).map(Setting::DryingTimeHours));
    out.extend(int(4, 28).and_then(CarpetSensitivity::from_code).map(Setting::CarpetSensitivity));
    out.extend(int(15, 2).and_then(small).map(Setting::AutoEmptyFrequency));
    if let (Some(enabled), Some(start), Some(end)) = (s.dnd.enabled, s.dnd.start, s.dnd.end) {
        out.push(Setting::Dnd { enabled, start, end });
    }
    if let Some(o) = &s.off_peak {
        out.push(Setting::OffPeak { enabled: o.enable, start: o.start, end: o.end });
    }
    if let Some(AiObstacle::Bits(f)) = &s.ai_obstacle {
        out.extend(AiFlags::NAMES.iter().map(|(bit, _)| Setting::AiFlag { bit: *bit, on: f.has(*bit) }));
    }
    if let Some(store) = &s.settings {
        out.extend(store.values.iter().map(|(k, v)| Setting::AutoSwitch { key: k.clone(), value: *v }));
    }
    out
}

/// The name of an AI detection bit, if it is a known single bit.
pub fn ai_name(bit: u32) -> Option<&'static str> {
    AiFlags::NAMES.iter().find(|(b, _)| *b == bit).map(|(_, n)| *n)
}

/// The AI detection bit with this name (see [`AiFlags::NAMES`]), ignoring case.
pub fn ai_bit(name: &str) -> Option<u32> {
    AiFlags::NAMES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name.trim())).map(|(b, _)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_model::DecodeContext;

    fn status() -> Status {
        let mut s = Status::new(DecodeContext::default());
        s.update(p(4, 22), &RawValue::Int(AiFlags::OBSTACLE as i64 | AiFlags::FURNITURE as i64));
        s.update(p(3, 3), &RawValue::Str(r#"{"enable":false,"startTime":"22:00","endTime":"08:00","x":1}"#.into()));
        s.update(p(7, 1), &RawValue::Int(40));
        s
    }

    /// Each write as (property, value); a string holding JSON shows as that JSON.
    fn writes(cmds: &[Command]) -> Vec<(String, String)> {
        cmds.iter()
            .map(|c| match c {
                Command::SetProperty { id, value } => (id.to_string(), value.to_json().to_string()),
                Command::Action { .. } => panic!("not a write"),
            })
            .collect()
    }

    #[test]
    fn simple_settings_write_one_property() {
        let s = status();
        assert_eq!(writes(&Setting::Volume(60).commands(&s).unwrap()), [("7.1".into(), "60".into())]);
        assert!(Setting::Volume(101).commands(&s).is_err());
        assert_eq!(writes(&Setting::ChildLock(true).commands(&s).unwrap()), [("4.27".into(), "true".into())]);
        // A switch the robot stores as a number is written as a number.
        let mut n = status();
        n.update(p(4, 26), &RawValue::Int(0));
        assert_eq!(writes(&Setting::CustomizedCleaning(true).commands(&n).unwrap()), [("4.26".into(), "1".into())]);
        assert_eq!(writes(&Setting::CarpetSensitivity(CarpetSensitivity::High).commands(&s).unwrap()), [("4.28".into(), "3".into())]);
        assert!(Setting::DryingTimeHours(0).commands(&s).is_err());
        assert_eq!(Setting::Volume(0).current(&s).as_deref(), Some("40"));
    }

    #[test]
    fn shared_values_keep_what_they_do_not_change() {
        let s = status();
        // One AI bit changes; the others stay as read (obstacle and furniture on).
        let on = Setting::AiFlag { bit: AiFlags::PET, on: true }.commands(&s).unwrap();
        assert_eq!(writes(&on), [("4.22".into(), "19".into())]);
        let off = Setting::AiFlag { bit: AiFlags::OBSTACLE, on: false }.commands(&s).unwrap();
        assert_eq!(writes(&off), [("4.22".into(), "1".into())]);
        assert!(Setting::AiFlag { bit: 3, on: true }.commands(&s).is_err(), "two bits at once");
        assert!(Setting::AiFlag { bit: AiFlags::PET, on: true }.commands(&Status::new(DecodeContext::default())).is_err());
        // Off-peak keeps the unknown field.
        let t = |h| HhMm::new(h, 0).unwrap();
        let w = writes(&Setting::OffPeak { enabled: true, start: t(23), end: t(6) }.commands(&s).unwrap());
        assert_eq!(w, [("3.3".into(), r#"{"enable":true,"startTime":"23:00","endTime":"06:00","x":1}"#.into())]);
        // The settings store gets one {"k","v"} object, as the app writes it.
        let w = writes(&Setting::AutoSwitch { key: "MeticulousTwist".into(), value: -1 }.commands(&s).unwrap());
        assert_eq!(w, [("4.50".into(), r#"{"k":"MeticulousTwist","v":-1}"#.into())]);
        assert!(Setting::AutoSwitch { key: "a\"b".into(), value: 1 }.commands(&s).is_err());
        assert_eq!(ai_bit("Pet detection"), Some(AiFlags::PET));
    }

    #[test]
    fn known_lists_read_settings_with_their_values() {
        let mut s = status();
        s.update(p(4, 34), &RawValue::Int(1));
        s.update(p(4, 28), &RawValue::Int(2));
        let k = known(&s);
        assert_eq!(k[0], Setting::Volume(40));
        assert!(k.contains(&Setting::SelfClean(true)));
        assert!(k.contains(&Setting::CarpetSensitivity(CarpetSensitivity::Medium)));
        let t = |h| HhMm::new(h, 0).unwrap();
        assert!(k.contains(&Setting::OffPeak { enabled: false, start: t(22), end: t(8) }));
        assert!(k.contains(&Setting::AiFlag { bit: AiFlags::PET, on: false }));
        assert!(k.contains(&Setting::AiFlag { bit: AiFlags::OBSTACLE, on: true }));
        // Not read, so not listed.
        assert!(!k.iter().any(|x| matches!(x, Setting::ChildLock(_) | Setting::Dnd { .. })));
        // Each listed setting writes properties that all_properties reads.
        let all = all_properties();
        assert!(k.iter().flat_map(Setting::properties).all(|id| all.contains(&id)));
    }
}
