//! The device profile: a data table of every known property, action and event.
//!
//! The base table ([`table::PROPERTIES`], [`ACTIONS`], [`EVENTS`]) holds the
//! ids, value kinds, access flags and codecs from docs/research/device-model.md
//! section 2. Per-model overrides ([`MODEL_OVERRIDES`]) replace, add or remove
//! rows by model id. Build a profile with [`Profile::for_model`].

pub mod table;

pub use crate::enums::EnumKind;
use dreame_core::{ActionId, EventId, PropId};
use std::collections::BTreeMap;

/// MIoT access flags. Empty means not documented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Access(u8);

impl Access {
    pub const UNKNOWN: Self = Self(0);
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const NOTIFY: Self = Self(4);
    /// Only seen as an event argument or action output.
    pub const EVENT: Self = Self(8);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }
    pub const fn is_unknown(self) -> bool {
        self.0 == 0
    }
    /// True when the profile documents the property as not readable
    /// (write-only, push-only or event-only).
    pub const fn documented_unreadable(self) -> bool {
        !self.is_unknown() && !self.contains(Self::READ)
    }
}

impl std::fmt::Display for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_unknown() {
            return f.write_str("?");
        }
        for (flag, c) in [(Self::READ, 'R'), (Self::WRITE, 'W'), (Self::NOTIFY, 'N'), (Self::EVENT, 'E')] {
            if self.contains(flag) {
                write!(f, "{c}")?;
            }
        }
        Ok(())
    }
}

/// The wire shape of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Int,
    Bool,
    Str,
    /// JSON inside a string.
    Json,
    /// A packed integer (4.23).
    Packed,
    /// A comma-separated list (4.18).
    List,
    /// An integer on new models, JSON on old ones (4.22).
    IntOrJson,
    Unknown,
}

/// How to decode a value. Each variant names a decoder in [`crate::codec`]
/// or [`crate::enums`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// Keep the raw value.
    Raw,
    Bool,
    Enum(EnumKind),
    Percent,
    Minutes,
    Hours,
    SquareMeters,
    UnixSeconds,
    /// `HH:MM` text.
    HhMm,
    /// Generic JSON object or list.
    Json,
    CleaningMode,
    AutoSwitch,
    AiObstacle,
    OffPeakCharging,
    DndTasks,
    Schedule,
    Faults,
    Wetness,
    ConsumablePercent,
    ConsumableTime,
    CleaningProperties,
    /// A cloud object name or a `{obj_name, md5}` pointer.
    CloudObject,
    /// An inline map frame; decoded by dreame-map.
    MapFrame,
}

/// One property row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropDef {
    /// Tasshack's `DreameVacuumProperty` name where one exists.
    pub name: &'static str,
    pub siid: u32,
    pub piid: u32,
    pub kind: ValueKind,
    pub access: Access,
    pub codec: Codec,
    pub desc: &'static str,
}

impl PropDef {
    pub const fn id(&self) -> PropId {
        PropId::new(self.siid, self.piid)
    }
}

/// One action row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub name: &'static str,
    pub siid: u32,
    pub aiid: u32,
    /// The `in` list, described in words.
    pub input: &'static str,
    pub desc: &'static str,
}

impl ActionDef {
    pub const fn id(&self) -> ActionId {
        ActionId::new(self.siid, self.aiid)
    }
}

/// One event row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDef {
    pub name: &'static str,
    pub siid: u32,
    pub eiid: u32,
    /// Argument piids seen or documented.
    pub args: &'static [u32],
    pub desc: &'static str,
}

impl EventDef {
    pub const fn id(&self) -> EventId {
        EventId { siid: self.siid, eiid: self.eiid }
    }
}

const fn a(name: &'static str, siid: u32, aiid: u32, input: &'static str, desc: &'static str) -> ActionDef {
    ActionDef { name, siid, aiid, input, desc }
}

/// Every known action (device-model.md section 2; Tasshack's action mapping).
pub static ACTIONS: &[ActionDef] = &[
    a("START", 2, 1, "none", "Start or resume a full clean"),
    a("PAUSE", 2, 2, "none", "Pause"),
    a("SPEC_START_ROOM_SWEEP", 2, 3, "[2.4]", "Spec room clean; unused by known clients"),
    a("CHARGE", 3, 1, "none", "Return to the dock"),
    a("START_CUSTOM", 4, 1, "[{piid:1, STATUS}, {piid:10, JSON}?]", "Start a room, zone, spot or other custom job"),
    a("STOP", 4, 2, "none", "Stop the job"),
    a("CLEAR_WARNING", 4, 3, "none", "Clear the current warning"),
    a("START_WASHING", 4, 4, "[{piid:10, \"op,flag\"}]", "Station operation"),
    a("GET_PHOTO_INFO", 4, 6, "JSON", "Obstacle photo metadata"),
    a("SHORTCUTS", 4, 8, "[{piid:10, {cmd, params}}]", "Shortcut commands"),
    a("DELETE_DND_TASK", 5, 1, "task id", "Delete a DND task"),
    a("REQUEST_MAP", 6, 1, "[{piid:2, {frame_type:\"I\"}}]", "Ask the robot to upload a map frame"),
    a("UPDATE_MAP_DATA", 6, 2, "[{piid:4, JSON}]", "Map edit"),
    a("BACKUP_MAP", 6, 3, "map id", "Back up a map"),
    a("WIFI_MAP", 6, 4, "", "Wi-Fi map"),
    a("LOCATE", 7, 1, "none", "Play the locate sound"),
    a("PLAY_SOUND", 7, 2, "", "Play a sound"),
    a("DELETE_SCHEDULED_TASK", 8, 1, "[{piid:3, id}]", "Delete a schedule"),
    a("DELETE_CRUISE_SCHEDULE", 8, 2, "", "Delete a cruise schedule"),
    a("DELETE_PET_CRUISE_SCHEDULE", 8, 3, "", "Delete a pet cruise schedule"),
    a("RESET_MAIN_BRUSH", 9, 1, "none", "Reset main brush life"),
    a("RESET_SIDE_BRUSH", 10, 1, "none", "Reset side brush life"),
    a("RESET_FILTER", 11, 1, "none", "Reset filter life"),
    a("START_AUTO_EMPTY", 15, 1, "none", "Empty the dust bin into the station"),
    a("RESET_SENSOR", 16, 1, "none", "Reset sensor cleaning counter"),
    a("RESET_TANK_FILTER", 17, 1, "none", "Reset tank filter life"),
    a("RESET_MOP_PAD", 18, 1, "none", "Reset mop pad life"),
    a("RESET_SILVER_ION", 19, 1, "none", "Reset silver ion life"),
    a("RESET_DETERGENT", 20, 1, "none", "Reset detergent level"),
    a("RESET_SQUEEGEE", 24, 1, "none", "Reset squeegee life"),
    a("RESET_DIRTY_WATER_CHANNEL", 25, 1, "none", "Reset dirty water channel counter"),
    a("RESET_ONBOARD_DIRTY_WATER_TANK", 26, 1, "none", "Reset onboard dirty water tank counter"),
    a("RESET_DEODORIZER", 29, 1, "none", "Reset deodorizer life"),
    a("RESET_WHEEL", 30, 1, "none", "Reset wheel cleaning counter"),
    a("RESET_SCALE_INHIBITOR", 31, 1, "none", "Reset scale inhibitor life"),
    a("RESET_FLUFFING_ROLLER", 32, 1, "none", "Reset fluffing roller counter"),
    a("RESET_ROLLER_MOP_FILTER", 33, 1, "none", "Reset roller mop filter counter"),
    a("RESET_WATER_OUTLET_FILTER", 35, 1, "none", "Reset water outlet filter counter"),
    a("RESET_TRACK_CLEANING", 36, 1, "none", "Reset track cleaning counter"),
    a("RESET_WASHBOARD_CLEANING", 37, 1, "none", "Reset washboard cleaning counter"),
    a("RESET_FILTER_CLEANING", 38, 1, "none", "Reset dock filter cleaning counter"),
    a("STREAM_VIDEO", 10001, 1, "JSON", "Camera video stream"),
    a("STREAM_AUDIO", 10001, 2, "JSON", "Camera audio stream"),
    a("STREAM_PROPERTY", 10001, 3, "JSON", "Camera stream property"),
    a("STREAM_CODE", 10001, 4, "JSON", "Camera stream code"),
    a("VIDEO_VENDOR", 10001, 7, "JSON", "Camera video vendor"),
];

/// Every known event (device-model.md section 2.4, protocols.md section 11.1b).
pub static EVENTS: &[EventDef] = &[
    EventDef { name: "STATUS_CHANGED", siid: 2, eiid: 2, args: &[], desc: "Status changed" },
    EventDef { name: "TASK_STARTED", siid: 2, eiid: 3, args: &[], desc: "Task started (seen on r2579a)" },
    EventDef { name: "LOW_BATTERY", siid: 3, eiid: 1, args: &[], desc: "Low battery (spec)" },
    EventDef {
        name: "CLEANING_RECORD",
        siid: 4,
        eiid: 1,
        args: &[1, 2, 3, 6, 8, 9, 10, 13, 42, 43, 44],
        desc: "Task summary with the history map object",
    },
    EventDef { name: "TASK_ENDED", siid: 4, eiid: 3, args: &[], desc: "Task ended" },
    EventDef { name: "UNKNOWN_4_4", siid: 4, eiid: 4, args: &[], desc: "Seen at task start; meaning unknown" },
];

/// One per-model change to the base table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Override {
    /// Add a row, or replace the row with the same id.
    Set(PropDef),
    /// Remove the row with this id.
    Remove(u32, u32),
}

/// Per-model overrides, keyed by the full model id. No model needs a
/// different siid or piid so far; rows here change meanings or access as
/// robot evidence arrives.
pub static MODEL_OVERRIDES: &[(&str, &[Override])] = &[(
    "dreame.vacuum.r2579a",
    &[
        // 4.38 read -1 in the cache and 0 on a relayed read; meaning unknown.
        Override::Set(PropDef {
            name: "CAPABILITY",
            siid: 4,
            piid: 38,
            kind: ValueKind::Int,
            access: Access::READ,
            codec: Codec::Raw,
            desc: "Meaning unknown; -1 in the cache, 0 on a relayed read (r2579a)",
        }),
        // 2.6 mirrors the 4.23 mode bits on r2579a (device-model.md 4.1).
        Override::Set(PropDef {
            name: "CLEAN_MODE_SETTING",
            siid: 2,
            piid: 6,
            kind: ValueKind::Int,
            access: Access::READ.union(Access::WRITE),
            codec: Codec::Raw,
            desc: "Raw cleaning mode 0..3; equals 4.23 bits 0-1 on r2579a",
        }),
    ],
)];

/// A profile for one model: the base table with that model's overrides.
#[derive(Debug, Clone)]
pub struct Profile {
    pub model: Option<String>,
    props: Vec<PropDef>,
    by_id: BTreeMap<PropId, usize>,
}

impl Default for Profile {
    fn default() -> Self {
        Self::build(None)
    }
}

impl Profile {
    /// The base table without overrides.
    pub fn base() -> Self {
        Self::default()
    }

    /// The base table with the overrides for `model` (for example
    /// `dreame.vacuum.r2579a`). Unknown models get the base table.
    pub fn for_model(model: &str) -> Self {
        Self::build(Some(model))
    }

    fn build(model: Option<&str>) -> Self {
        let mut rows: Vec<PropDef> = table::PROPERTIES.to_vec();
        if let Some(m) = model {
            for (_, ovs) in MODEL_OVERRIDES.iter().filter(|(k, _)| *k == m) {
                for ov in ovs.iter() {
                    match *ov {
                        Override::Set(def) => {
                            let mut replaced = false;
                            for r in rows.iter_mut().filter(|r| r.id() == def.id()) {
                                *r = def;
                                replaced = true;
                            }
                            if !replaced {
                                rows.push(def);
                            }
                        }
                        Override::Remove(s, p) => rows.retain(|r| r.id() != PropId::new(s, p)),
                    }
                }
            }
        }
        rows.sort_by_key(|r| r.id());
        // The first row wins when Tasshack gives two names to one id (7.10).
        let mut by_id = BTreeMap::new();
        for (i, r) in rows.iter().enumerate() {
            by_id.entry(r.id()).or_insert(i);
        }
        Self { model: model.map(str::to_string), props: rows, by_id }
    }

    /// Every row, sorted by id. One id can appear twice with two names.
    pub fn properties(&self) -> &[PropDef] {
        &self.props
    }

    /// Every distinct property id, sorted.
    pub fn prop_ids(&self) -> Vec<PropId> {
        self.by_id.keys().copied().collect()
    }

    pub fn prop(&self, id: PropId) -> Option<&PropDef> {
        self.by_id.get(&id).map(|&i| &self.props[i])
    }

    pub fn prop_by_name(&self, name: &str) -> Option<&PropDef> {
        self.props.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    pub fn actions(&self) -> &'static [ActionDef] {
        ACTIONS
    }

    pub fn action_by_name(&self, name: &str) -> Option<&'static ActionDef> {
        ACTIONS.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    pub fn events(&self) -> &'static [EventDef] {
        EVENTS
    }

    pub fn event(&self, id: EventId) -> Option<&'static EventDef> {
        EVENTS.iter().find(|e| e.id() == id)
    }
}

/// Splits a firmware string such as `4.3.9_1664` into its build number.
pub fn fw_build(firmware: &str) -> Option<u32> {
    firmware.rsplit_once('_').and_then(|(_, b)| b.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_table_has_the_core_rows() {
        let p = Profile::base();
        let st = p.prop(PropId::new(2, 1)).unwrap();
        assert_eq!(st.name, "STATE");
        assert_eq!(st.codec, Codec::Enum(EnumKind::State));
        assert!(st.access.contains(Access::READ));
        assert_eq!(p.prop(PropId::new(4, 23)).unwrap().codec, Codec::CleaningMode);
        assert_eq!(p.prop_by_name("auto_switch_settings").unwrap().id(), PropId::new(4, 50));
        assert!(p.properties().len() > 350);
        assert_eq!(p.action_by_name("REQUEST_MAP").unwrap().id(), ActionId::new(6, 1));
        assert_eq!(p.event(EventId { siid: 4, eiid: 1 }).unwrap().name, "CLEANING_RECORD");
    }

    #[test]
    fn ids_are_sorted_and_distinct() {
        let ids = Profile::base().prop_ids();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        // 7.10 has two names in Tasshack; the id list holds it once.
        assert_eq!(ids.iter().filter(|i| **i == PropId::new(7, 10)).count(), 1);
    }

    #[test]
    fn consumable_rows_follow_the_verified_order() {
        let p = Profile::base();
        assert_eq!(p.prop(PropId::new(9, 1)).unwrap().codec, Codec::ConsumableTime);
        assert_eq!(p.prop(PropId::new(9, 2)).unwrap().codec, Codec::ConsumablePercent);
        assert_eq!(p.prop(PropId::new(11, 1)).unwrap().codec, Codec::ConsumablePercent);
        assert_eq!(p.prop(PropId::new(11, 2)).unwrap().codec, Codec::ConsumableTime);
    }

    #[test]
    fn model_overrides_apply() {
        let base = Profile::base();
        let ae = Profile::for_model("dreame.vacuum.r2579a");
        assert!(base.prop(PropId::new(6, 3)).unwrap().access.documented_unreadable());
        assert_eq!(base.prop(PropId::new(4, 38)).unwrap().access, Access::UNKNOWN);
        assert_eq!(ae.prop(PropId::new(4, 38)).unwrap().access, Access::READ);
        assert!(ae.prop(PropId::new(2, 6)).unwrap().desc.contains("r2579a"));
        assert_eq!(ae.properties().len(), base.properties().len());
        assert_eq!(Profile::for_model("dreame.vacuum.unknown").properties().len(), base.properties().len());
    }

    #[test]
    fn fw_build_parses() {
        assert_eq!(fw_build("4.3.9_1664"), Some(1664));
        assert_eq!(fw_build("4.3.9"), None);
    }

    #[test]
    fn access_display() {
        assert_eq!(Access::READ.union(Access::WRITE).union(Access::NOTIFY).to_string(), "RWN");
        assert_eq!(Access::UNKNOWN.to_string(), "?");
    }
}
