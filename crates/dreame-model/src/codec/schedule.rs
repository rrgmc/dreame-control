//! SCHEDULE (8.2): `;`-separated slots of 9 `-`-separated fields
//! (device-model.md 4.4). Fields 5-8 are disputed between Tasshack and
//! node-dreame, and r2579a had no schedule to check, so they stay raw text.

use super::{decode_err, text, HhMm};
use dreame_core::{RawValue, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSlot {
    pub id: i64,
    /// Raw field 1: 1 or 2 enabled, 3 invalid.
    pub state: u8,
    pub time: HhMm,
    /// Field 3, zero-padded to 7 characters of `0`/`1`, Sunday first (verified on r2579a).
    pub weekdays: String,
    /// Field 4: `0` means once (Tasshack); node-dreame reads 1 as repeating.
    pub repeat: String,
    /// Fields 5 to 8 as sent: map id or room scope, suction or wetness,
    /// water or packed config, options or rooms.
    pub rest: [String; 4],
}

impl ScheduleSlot {
    pub fn enabled(&self) -> bool {
        matches!(self.state, 1 | 2)
    }
    pub fn invalid(&self) -> bool {
        self.state == 3
    }
    pub fn once(&self) -> bool {
        self.repeat == "0"
    }
    /// Field 8 split on `,`, or empty when it is `0`.
    pub fn options(&self) -> Vec<&str> {
        if self.rest[3] == "0" {
            Vec::new()
        } else {
            self.rest[3].split(',').collect()
        }
    }

    /// The days the slot runs, Monday first. The weekday field starts on Sunday (verified on
    /// r2579a: the official app showed `0000100` as Thursday).
    pub fn days(&self) -> [bool; 7] {
        days_from_mask(&self.weekdays)
    }

    /// Sets the days, given Monday first.
    pub fn set_days(&mut self, days: &[bool; 7]) {
        self.weekdays = mask_from_days(days);
    }

    fn parse(slot: &str) -> Result<Self> {
        let f: Vec<&str> = slot.split('-').collect();
        if f.len() < 9 {
            return Err(decode_err("schedule slot", slot));
        }
        let num = |s: &str| s.trim().parse::<i64>().map_err(|_| decode_err("schedule slot", slot));
        Ok(Self {
            id: num(f[0])?,
            state: num(f[1])? as u8,
            time: HhMm::parse(f[2])?,
            weekdays: format!("{:0>7}", f[3]),
            repeat: f[4].to_string(),
            // Field 8 may itself contain `-` in a negative value; keep the tail joined.
            rest: [f[5].to_string(), f[6].to_string(), f[7].to_string(), f[8..].join("-")],
        })
    }

    fn render(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}-{}-{}-{}-{}",
            self.id, self.state, self.time, self.weekdays, self.repeat, self.rest[0], self.rest[1], self.rest[2], self.rest[3]
        )
    }
}

/// One room of a schedule that cleans chosen rooms with their own settings (field 5 = `1`;
/// field 8 lists one packed number per room). Decoded on r2579a, 2026-11-17, by changing one
/// setting at a time in the official app:
///
/// | Bits | Field |
/// |---|---|
/// | 0-7 | room id |
/// | 8-11 | passes, 1 to 3 |
/// | 12-15 | 1 in every sample; kept as read |
/// | 16-19 | suction, 0 quiet to 3 max |
/// | 20-23 | mode: 0 vacuum, 1 mop, 2 vacuum and mop |
/// | 24-25 | 3 in every sample; kept as read |
/// | 26-31 | mop wetness, 1 to 32 (16 by default) |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomTask {
    pub room: u8,
    pub passes: u8,
    pub suction: u8,
    pub mode: u8,
    pub wetness: u8,
    /// The value as read, so the bits not decoded above survive an edit.
    pub raw: u32,
}

impl RoomTask {
    /// The bits no field above covers, as every sample had them.
    const DEFAULT_REST: u32 = (1 << 12) | (3 << 24);

    pub fn decode(v: u32) -> Self {
        Self {
            room: (v & 0xFF) as u8,
            passes: ((v >> 8) & 0xF) as u8,
            suction: ((v >> 16) & 0xF) as u8,
            mode: ((v >> 20) & 0xF) as u8,
            wetness: ((v >> 26) & 0x3F) as u8,
            raw: v,
        }
    }

    /// A new task: the undecoded bits as every sample had them.
    pub fn new(room: u8, passes: u8, suction: u8, mode: u8, wetness: u8) -> Self {
        let mut t = Self::decode(Self::DEFAULT_REST);
        (t.room, t.passes, t.suction, t.mode, t.wetness) = (room, passes, suction, mode, wetness);
        t
    }

    pub fn encode(&self) -> u32 {
        let keep = self.raw & ((0xF << 12) | (0x3 << 24));
        keep | u32::from(self.room)
            | (u32::from(self.passes.clamp(1, 3)) << 8)
            | (u32::from(self.suction.min(3)) << 16)
            | (u32::from(self.mode.min(2)) << 20)
            | (u32::from(self.wetness.clamp(1, 32)) << 26)
    }
}

impl ScheduleSlot {
    /// The rooms and their settings, for a slot that cleans chosen rooms (field 5 = `1`).
    /// `None` for other slots, such as whole-home ones.
    pub fn room_tasks(&self) -> Option<Vec<RoomTask>> {
        if self.rest[0] != "1" {
            return None;
        }
        self.options().iter().map(|s| s.trim().parse::<u32>().ok().map(RoomTask::decode)).collect()
    }

    /// Sets the rooms of a room slot (field 5 = `1`, fields 6 and 7 = `0`, as the app writes).
    pub fn set_room_tasks(&mut self, tasks: &[RoomTask]) {
        self.rest[0] = "1".into();
        self.rest[1] = "0".into();
        self.rest[2] = "0".into();
        self.rest[3] = if tasks.is_empty() {
            "0".into()
        } else {
            tasks.iter().map(|t| t.encode().to_string()).collect::<Vec<_>>().join(",")
        };
    }

    /// A new enabled, weekly room slot.
    pub fn new_rooms(id: i64, time: HhMm, weekdays: &str, tasks: &[RoomTask]) -> Self {
        let mut s = Self {
            id,
            state: 1,
            time,
            weekdays: format!("{weekdays:0>7}"),
            repeat: "1".into(),
            rest: ["1".into(), "0".into(), "0".into(), "0".into()],
        };
        s.set_room_tasks(tasks);
        s
    }

    /// Turns the slot on or off. Invalid slots stay invalid.
    pub fn set_enabled(&mut self, on: bool) {
        if !self.invalid() {
            self.state = if on { 1 } else { 0 };
        }
    }
}

/// The days of a weekday field, Monday first. The field starts on Sunday.
pub fn days_from_mask(mask: &str) -> [bool; 7] {
    let c: Vec<char> = format!("{mask:0>7}").chars().collect();
    std::array::from_fn(|k| c.get((k + 1) % 7) == Some(&'1'))
}

/// The weekday field (Sunday first) for days given Monday first.
pub fn mask_from_days(days: &[bool; 7]) -> String {
    (0..7).map(|f| if days[(f + 6) % 7] { '1' } else { '0' }).collect()
}

/// Decodes 8.2. An empty value means no schedules. Slots without `-` are
/// skipped, as Tasshack does.
pub fn decode(v: &RawValue) -> Result<Vec<ScheduleSlot>> {
    let s = text(v);
    s.split(';').filter(|t| t.contains('-')).map(ScheduleSlot::parse).collect()
}

/// Encodes slots back to the 8.2 text.
pub fn encode(slots: &[ScheduleSlot]) -> RawValue {
    RawValue::Str(slots.iter().map(ScheduleSlot::render).collect::<Vec<_>>().join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic: r2579a had no schedule. Shape per Tasshack device.py.
    const SAMPLE: &str = "1-1-09:30-1111100-1-0-2-2-0;2-3-18:05-0000011-0-1-1-3-1,4";

    #[test]
    fn decodes_and_round_trips() {
        let slots = decode(&RawValue::Str(SAMPLE.into())).unwrap();
        assert_eq!(slots.len(), 2);
        assert!(slots[0].enabled());
        assert_eq!(slots[0].time.to_string(), "09:30");
        assert_eq!(slots[0].weekdays, "1111100");
        assert!(slots[0].options().is_empty());
        assert!(slots[1].invalid());
        assert!(slots[1].once());
        assert_eq!(slots[1].options(), vec!["1", "4"]);
        assert_eq!(encode(&slots), RawValue::Str(SAMPLE.into()));
    }

    /// The r2579a schedule after each step of the decoding (2026-11-17): as created, then
    /// room 10 to 3 passes and room 12 to max suction, then room 10 to vacuum only and room 12
    /// to wetness 25.
    const R2579A: [&str; 3] = [
        "1-1-22:00-1111111-1-1-0-0-1126240522,1126240524",
        "1-1-22:00-1111111-1-1-0-0-1126241034,1126371596",
        "1-1-22:00-1111111-1-1-0-0-1124143882,1730351372",
    ];

    #[test]
    fn room_tasks_of_a_real_schedule() {
        let tasks = |s: &str| decode(&RawValue::Str(s.into())).unwrap()[0].room_tasks().unwrap();
        let t: Vec<_> = tasks(R2579A[0]).iter().map(|t| (t.room, t.passes, t.suction, t.mode, t.wetness)).collect();
        assert_eq!(t, [(10, 1, 1, 2, 16), (12, 1, 1, 2, 16)]);
        let t: Vec<_> = tasks(R2579A[1]).iter().map(|t| (t.room, t.passes, t.suction)).collect();
        assert_eq!(t, [(10, 3, 1), (12, 1, 3)]);
        let t: Vec<_> = tasks(R2579A[2]).iter().map(|t| (t.room, t.mode, t.wetness)).collect();
        assert_eq!(t, [(10, 0, 16), (12, 2, 25)]);
        // Each value re-encodes to itself, and a new task matches the app's.
        for s in R2579A {
            for t in tasks(s) {
                assert_eq!(t.encode(), t.raw);
            }
        }
        assert_eq!(RoomTask::new(10, 1, 1, 2, 16).encode(), 1_126_240_522);
    }

    #[test]
    fn editing_a_room_slot() {
        let mut s = decode(&RawValue::Str(R2579A[0].into())).unwrap().remove(0);
        let mut tasks = s.room_tasks().unwrap();
        tasks[0].passes = 3;
        tasks[1].suction = 3;
        s.set_room_tasks(&tasks);
        assert_eq!(encode(&[s.clone()]), RawValue::Str(R2579A[1].into()));
        let n = ScheduleSlot::new_rooms(2, HhMm { hour: 9, minute: 5 }, "1111100", &[RoomTask::new(3, 2, 0, 1, 20)]);
        let RawValue::Str(text) = encode(std::slice::from_ref(&n)) else { unreachable!() };
        assert!(text.starts_with("2-1-09:05-1111100-1-1-0-0-"), "{text}");
        assert_eq!(n.room_tasks().unwrap()[0].wetness, 20);
        // Whole-home slots have no room tasks.
        let home = decode(&RawValue::Str(SAMPLE.into())).unwrap();
        assert!(home[0].room_tasks().is_none());
    }

    #[test]
    fn short_weekday_field_is_padded() {
        let slots = decode(&RawValue::Str("3-1-07:00-11-1-0-1-1-0".into())).unwrap();
        assert_eq!(slots[0].weekdays, "0000011");
    }

    #[test]
    fn empty_and_bad() {
        assert!(decode(&RawValue::Str(String::new())).unwrap().is_empty());
        assert!(decode(&RawValue::Str("1-1-09:30".into())).is_err());
    }
}
