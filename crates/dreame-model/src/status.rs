//! A typed status snapshot built from property values.
//!
//! Feed it values from any source: a discovery read, the cloud cache or
//! MQTT pushes. Every field is optional, unknown codes stay as
//! [`Open::Unknown`], and every value is also kept raw in [`Status::raw`].
//! Values that fail to decode are listed in [`Status::decode_errors`].

use crate::codec::ai_obstacle::AiObstacle;
use crate::codec::auto_switch::AutoSwitchSettings;
use crate::codec::cleaning_mode::{CleaningModeGroup, ModePolarity};
use crate::codec::consumables::{Consumable, ConsumableLife};
use crate::codec::dnd::{self, DndTask, DndWindow};
use crate::codec::faults::{FaultFilter, Faults};
use crate::codec::off_peak::OffPeakCharging;
use crate::codec::{boolean, int};
use crate::enums::*;
use dreame_core::{PropId, RawValue, Result};
use std::collections::BTreeMap;

/// Model facts that change how values decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeContext {
    /// Capability `MOP_PAD_LIFTING`: 4.23 has a 2-bit mode field.
    pub mop_pad_lifting: bool,
    /// Self-wash base present: fault 68 is not an active fault.
    pub self_wash_base: bool,
    /// Capability `LEGACY_WARNINGS`.
    pub legacy_warnings: bool,
    pub polarity: ModePolarity,
}

impl Default for DecodeContext {
    /// Defaults for current self-wash models such as r2579a.
    fn default() -> Self {
        Self { mop_pad_lifting: true, self_wash_base: true, legacy_warnings: false, polarity: ModePolarity::Tasshack }
    }
}

/// Station state (siids 4, 15 and 27).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StationStatus {
    pub self_wash_base: Option<Open<SelfWashBaseStatus>>,
    pub clean_water_tank: Option<Open<CleanWaterTankStatus>>,
    pub dirty_water_tank: Option<Open<DirtyWaterTankStatus>>,
    pub dust_bag: Option<Open<DustBagStatus>>,
    pub detergent: Option<Open<DetergentStatus>>,
    pub dust_collection: Option<Open<DustCollection>>,
    pub auto_empty: Option<Open<AutoEmptyStatus>>,
    /// Raw 15.1; the code table depends on `INTELLIGENT_AUTO_EMPTY`.
    pub auto_empty_mode: Option<i64>,
    pub drainage: Option<Open<DrainageStatus>>,
    pub low_water_warning: Option<Open<LowWaterWarning>>,
    pub water_tank: Option<Open<WaterTank>>,
    pub drying_time_h: Option<i64>,
}

/// Lifetime totals (siid 12).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub first_cleaning: Option<i64>,
    pub cleaning_time_min: Option<i64>,
    pub cleaning_count: Option<i64>,
    pub cleaned_area_m2: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub ctx: DecodeContext,
    pub state: Option<Open<State>>,
    pub status: Option<Open<VacuumStatus>>,
    pub task_status: Option<Open<TaskStatus>>,
    pub task_type: Option<Open<TaskType>>,
    pub battery: Option<i64>,
    pub charging: Option<Open<ChargingStatus>>,
    pub error: Option<Open<ErrorCode>>,
    pub faults: Option<Faults>,
    pub cleaning_time_min: Option<i64>,
    pub cleaned_area_m2: Option<i64>,
    /// 4.63, push only.
    pub cleaning_progress: Option<i64>,
    /// 4.64, push only.
    pub drying_progress: Option<i64>,
    pub suction: Option<Open<SuctionLevel>>,
    pub water_volume: Option<Open<WaterVolume>>,
    /// 28.1, 1..32.
    pub wetness: Option<i64>,
    pub cleaning_mode: Option<CleaningModeGroup>,
    /// 2.6 raw mode.
    pub clean_mode_setting: Option<i64>,
    pub mop_wash_level: Option<Open<MopWashLevel>>,
    pub dnd: DndWindow,
    pub dnd_tasks: Option<Vec<DndTask>>,
    pub off_peak: Option<OffPeakCharging>,
    pub ai_obstacle: Option<AiObstacle>,
    pub settings: Option<AutoSwitchSettings>,
    pub consumables: BTreeMap<Consumable, ConsumableLife>,
    pub station: StationStatus,
    pub totals: Totals,
    pub volume: Option<i64>,
    pub child_lock: Option<bool>,
    pub carpet_boost: Option<bool>,
    pub resume_cleaning: Option<bool>,
    pub relocation: Option<Open<RelocationStatus>>,
    /// Every value applied, by id.
    pub raw: BTreeMap<PropId, RawValue>,
    pub decode_errors: Vec<(PropId, String)>,
}

impl Status {
    pub fn new(ctx: DecodeContext) -> Self {
        Self { ctx, ..Default::default() }
    }

    /// Builds a snapshot from a map of values.
    pub fn from_values<'a>(values: impl IntoIterator<Item = (&'a PropId, &'a RawValue)>, ctx: DecodeContext) -> Self {
        let mut s = Self::new(ctx);
        for (id, v) in values {
            s.update(*id, v);
        }
        s
    }

    /// Applies one value, for example an MQTT `properties_changed` item.
    /// Returns true when a typed field was set.
    pub fn update(&mut self, id: PropId, v: &RawValue) -> bool {
        self.raw.insert(id, v.clone());
        match self.apply(id, v) {
            Ok(hit) => hit,
            Err(e) => {
                self.decode_errors.retain(|(i, _)| *i != id);
                self.decode_errors.push((id, e.to_string()));
                false
            }
        }
    }

    fn apply(&mut self, id: PropId, v: &RawValue) -> Result<bool> {
        fn open<T: CodeEnum>(v: &RawValue) -> Result<Option<Open<T>>> {
            Ok(Some(Open::from_code(int(v)?)))
        }
        if let Some((c, field)) = Consumable::field_of(id) {
            self.consumables.entry(c).or_default().set(field, int(v)?);
            return Ok(true);
        }
        if self.dnd.apply(id, v)? {
            return Ok(true);
        }
        let st = &mut self.station;
        match (id.siid, id.piid) {
            (2, 1) => self.state = open(v)?,
            (2, 2) => self.error = open(v)?,
            (2, 6) => self.clean_mode_setting = Some(int(v)?),
            (3, 1) => self.battery = Some(int(v)?),
            (3, 2) => self.charging = open(v)?,
            (3, 3) => self.off_peak = Some(OffPeakCharging::decode(v)?),
            (4, 1) => self.status = open(v)?,
            (4, 2) => self.cleaning_time_min = Some(int(v)?),
            (4, 3) => self.cleaned_area_m2 = Some(int(v)?),
            (4, 4) => self.suction = open(v)?,
            (4, 5) => self.water_volume = open(v)?,
            (4, 6) => st.water_tank = open(v)?,
            (4, 7) => self.task_status = open(v)?,
            (4, 11) => self.resume_cleaning = Some(boolean(v)?),
            (4, 12) => self.carpet_boost = Some(boolean(v)?),
            (4, 18) => self.faults = Some(Faults::decode(v)?),
            (4, 20) => self.relocation = open(v)?,
            (4, 22) => self.ai_obstacle = Some(AiObstacle::decode(v)?),
            (4, 23) => self.cleaning_mode = Some(CleaningModeGroup::decode(v, self.ctx.mop_pad_lifting)?),
            (4, 25) => st.self_wash_base = open(v)?,
            (4, 27) => self.child_lock = Some(boolean(v)?),
            (4, 40) => st.drying_time_h = Some(int(v)?),
            (4, 41) => st.low_water_warning = open(v)?,
            (4, 46) => self.mop_wash_level = open(v)?,
            (4, 50) => self.settings = Some(AutoSwitchSettings::decode(v)?),
            (4, 58) => self.task_type = open(v)?,
            (4, 60) => st.drainage = open(v)?,
            (4, 63) => self.cleaning_progress = Some(int(v)?),
            (4, 64) => self.drying_progress = Some(int(v)?),
            (5, 4) => self.dnd_tasks = Some(dnd::decode_tasks(v)?),
            (7, 1) => self.volume = Some(int(v)?),
            (12, 1) => self.totals.first_cleaning = Some(int(v)?),
            (12, 2) => self.totals.cleaning_time_min = Some(int(v)?),
            (12, 3) => self.totals.cleaning_count = Some(int(v)?),
            (12, 4) => self.totals.cleaned_area_m2 = Some(int(v)?),
            (15, 1) => st.auto_empty_mode = Some(int(v)?),
            (15, 3) => st.dust_collection = open(v)?,
            (15, 5) => st.auto_empty = open(v)?,
            (27, 1) => st.clean_water_tank = open(v)?,
            (27, 2) => st.dirty_water_tank = open(v)?,
            (27, 3) => st.dust_bag = open(v)?,
            (27, 4) => st.detergent = open(v)?,
            (28, 1) => self.wetness = Some(int(v)?),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Faults an app should show, filtered for this model.
    pub fn active_faults(&self) -> Vec<Open<ErrorCode>> {
        let f = FaultFilter {
            ignored: Vec::new(),
            legacy_warnings: self.ctx.legacy_warnings,
            charging: self.charging.is_some_and(|c| c.is(ChargingStatus::Charging)),
            self_wash_base: self.ctx.self_wash_base,
        };
        self.faults.as_ref().map(|x| x.active(&f)).unwrap_or_default()
    }

    /// The logical cleaning mode from 4.23, under the context's polarity.
    pub fn logical_cleaning_mode(&self) -> Option<Open<CleaningMode>> {
        self.cleaning_mode.map(|g| g.logical(self.ctx.polarity))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Cached values from docs/research/l40-ultra-ae.md and device-model.md
    /// (r2579a, 2026-11-15), as the cache returns them: strings.
    pub(crate) fn r2579a_cache() -> BTreeMap<PropId, RawValue> {
        let pairs: &[(u32, u32, &str)] = &[
            (2, 1, "8"),
            (2, 2, "68"),
            (2, 6, "3"),
            (3, 1, "99"),
            (3, 2, "1"),
            (3, 3, r#"{"enable":false,"startTime":"22:00","endTime":"08:00"}"#),
            (4, 1, "14"),
            (4, 2, "15"),
            (4, 3, "11"),
            (4, 4, "2"),
            (4, 6, "1"),
            (4, 7, "0"),
            (4, 11, "1"),
            (4, 12, "1"),
            (4, 18, "68"),
            (4, 20, "0"),
            (4, 22, "271"),
            (4, 23, "5123"),
            (4, 25, "2"),
            (4, 27, "0"),
            (4, 40, "3"),
            (4, 41, "0"),
            (4, 46, "1"),
            (4, 50, r#"[{"k":"StainIdentify","v":1},{"k":"SmartAutoMop","v":-1},{"k":"ExtrFreq","v":2},{"k":"CleanType","v":0}]"#),
            (4, 58, "0"),
            (4, 60, "0"),
            (5, 1, "false"),
            (5, 2, "22:00"),
            (5, 3, "08:00"),
            (7, 1, "60"),
            (9, 1, "192"),
            (9, 2, "64"),
            (10, 1, "92"),
            (10, 2, "46"),
            (11, 1, "28"),
            (11, 2, "42"),
            (12, 1, "1776452925"),
            (12, 2, "6474"),
            (12, 3, "250"),
            (12, 4, "5082"),
            (15, 1, "2"),
            (15, 3, "1"),
            (15, 5, "0"),
            (16, 1, "45"),
            (16, 2, "13"),
            (27, 1, "0"),
            (27, 2, "0"),
            (27, 3, "0"),
            (27, 4, "0"),
            (28, 1, "15"),
            (28, 5, "2"),
            (30, 1, "20"),
            (30, 2, "68"),
        ];
        pairs.iter().map(|(s, p, v)| (PropId::new(*s, *p), RawValue::from_cache(v))).collect()
    }

    #[test]
    fn r2579a_cached_snapshot() {
        let values = r2579a_cache();
        let s = Status::from_values(&values, DecodeContext::default());
        assert!(s.decode_errors.is_empty(), "{:?}", s.decode_errors);
        assert_eq!(s.state, Some(Open::Known(State::Drying)));
        assert_eq!(s.status, Some(Open::Known(VacuumStatus::Sleeping)));
        assert_eq!(s.task_status, Some(Open::Known(TaskStatus::Completed)));
        assert_eq!(s.task_type, Some(Open::Known(TaskType::Idle)));
        assert_eq!(s.battery, Some(99));
        assert_eq!(s.charging, Some(Open::Known(ChargingStatus::Charging)));
        assert_eq!(s.error, Some(Open::Known(ErrorCode::RemoveMop)));
        assert_eq!(s.faults.as_ref().unwrap().codes, vec![68]);
        assert!(s.active_faults().is_empty(), "68 is not a fault on self-wash models");
        assert_eq!((s.cleaning_time_min, s.cleaned_area_m2), (Some(15), Some(11)));
        assert_eq!(s.suction, Some(Open::Known(SuctionLevel::Strong)));
        assert_eq!(s.wetness, Some(15));
        let g = s.cleaning_mode.unwrap();
        assert_eq!((g.mode, g.mid), (3, 20));
        assert_eq!(s.clean_mode_setting, Some(g.mode as i64));
        assert_eq!(s.logical_cleaning_mode(), Some(Open::Known(CleaningMode::MoppingAfterSweeping)));
        assert_eq!(s.mop_wash_level, Some(Open::Known(MopWashLevel::Daily)));
        assert_eq!(s.dnd.enabled, Some(false));
        assert_eq!(s.dnd.start.unwrap().to_string(), "22:00");
        assert!(!s.off_peak.as_ref().unwrap().enable);
        assert!(matches!(s.ai_obstacle, Some(AiObstacle::Bits(f)) if f.0 == 271));
        assert_eq!(s.settings.as_ref().unwrap().get("ExtrFreq"), Some(2));
        assert_eq!(s.consumables[&Consumable::MainBrush], ConsumableLife { percent: Some(64), time_left: Some(192) });
        assert_eq!(s.consumables[&Consumable::Filter], ConsumableLife { percent: Some(28), time_left: Some(42) });
        assert_eq!(s.consumables[&Consumable::Wheel].percent, Some(68));
        assert_eq!(s.consumables.len(), 5);
        assert_eq!(s.station.self_wash_base, Some(Open::Known(SelfWashBaseStatus::Drying)));
        assert_eq!(s.station.water_tank, Some(Open::Known(WaterTank::Installed)));
        assert_eq!(s.station.dust_collection, Some(Open::Known(DustCollection::Available)));
        assert_eq!(s.station.auto_empty_mode, Some(2));
        assert_eq!(s.station.clean_water_tank.unwrap().code(), 0);
        assert_eq!(s.totals.cleaning_count, Some(250));
        assert_eq!(s.volume, Some(60));
        assert_eq!(s.raw.len(), values.len());
    }

    #[test]
    fn pushes_update_and_errors_are_kept() {
        let mut s = Status::new(DecodeContext::default());
        // A clean in progress, as pushed over MQTT on r2579a.
        assert!(s.update(PropId::new(4, 63), &RawValue::Int(42)));
        assert!(s.update(PropId::new(2, 1), &RawValue::Int(1)));
        assert_eq!(s.state, Some(Open::Known(State::Sweeping)));
        assert!(s.update(PropId::new(2, 1), &RawValue::Int(4321)));
        assert_eq!(s.state, Some(Open::Unknown(4321)));
        assert!(!s.update(PropId::new(99, 98), &RawValue::Str("eN...".into())));
        assert!(!s.update(PropId::new(4, 50), &RawValue::Str("not json".into())));
        assert_eq!(s.decode_errors.len(), 1);
        assert_eq!(s.cleaning_progress, Some(42));
        assert_eq!(s.raw.len(), 4);
    }
}
