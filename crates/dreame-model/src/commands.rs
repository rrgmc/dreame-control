//! Command builders. Each function returns the action id and `in` list, or
//! the property id and value, that a transport would send.
//!
//! These are builders only: nothing here talks to a robot. Sending a
//! [`Command`] is a write and needs the user's approval (see CLAUDE.md).
//! Payloads follow docs/research/device-model.md; most are not yet verified
//! on r2579a.

use crate::codec::cleaning_mode::{CleaningModeGroup, ModePolarity};
use crate::codec::consumables::Consumable;
use crate::codec::custom_clean::{rooms_payload, zones_payload, RoomClean, ZoneClean};
use crate::codec::dnd::DndWindow;
use crate::codec::station::StationOp;
use crate::codec::{compact, HhMm};
use crate::enums::{CleaningMode, MopWashLevel, SuctionLevel, VacuumStatus, WaterVolume};
use crate::enums::CodeEnum;
use dreame_core::{ActionId, Error, PropId, RawValue, Result};
use serde_json::{json, Value};

/// One thing to send.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// An MIoT action with its `in` list of `{piid, value}` objects.
    Action { id: ActionId, input: Vec<Value> },
    /// A property write.
    SetProperty { id: PropId, value: RawValue },
}

impl Command {
    fn action(siid: u32, aiid: u32, input: Vec<Value>) -> Self {
        Self::Action { id: ActionId::new(siid, aiid), input }
    }
    fn set(siid: u32, piid: u32, value: RawValue) -> Self {
        Self::SetProperty { id: PropId::new(siid, piid), value }
    }

    /// A write of `value` to property `id`.
    pub fn write(id: PropId, value: RawValue) -> Self {
        Self::SetProperty { id, value }
    }
}

fn arg(piid: u32, value: impl Into<Value>) -> Value {
    json!({"piid": piid, "value": value.into()})
}

/// 2.a1: start or resume a full clean.
pub fn start() -> Command {
    Command::action(2, 1, vec![])
}

/// 2.a2: pause.
pub fn pause() -> Command {
    Command::action(2, 2, vec![])
}

/// 4.a2: stop the job.
pub fn stop() -> Command {
    Command::action(4, 2, vec![])
}

/// 3.a1: return to the dock.
pub fn dock() -> Command {
    Command::action(3, 1, vec![])
}

/// 7.a1: play the locate sound.
pub fn locate() -> Command {
    Command::action(7, 1, vec![])
}

/// 4.a3: clear the current warning.
pub fn clear_warning() -> Command {
    Command::action(4, 3, vec![])
}

/// 4.a1 with an explicit status and optional 4.10 text.
pub fn start_custom(status: i64, properties: Option<String>) -> Command {
    let mut input = vec![arg(1, status)];
    if let Some(p) = properties {
        input.push(arg(10, p));
    }
    Command::action(4, 1, input)
}

/// 4.a1 status 18: clean rooms. `customized_cleaning` is true when the
/// robot has 4.26; then every order field is 1.
pub fn clean_rooms(rooms: &[RoomClean], customized_cleaning: bool) -> Result<Command> {
    if rooms.is_empty() {
        return Err(Error::Unsupported("no rooms given".into()));
    }
    Ok(start_custom(VacuumStatus::SegmentCleaning.code(), Some(rooms_payload(rooms, customized_cleaning))))
}

/// 4.a1 status 19: clean zones (map millimetres).
pub fn clean_zones(zones: &[ZoneClean]) -> Result<Command> {
    if zones.is_empty() {
        return Err(Error::Unsupported("no zones given".into()));
    }
    for z in zones {
        if z.x0 >= z.x1 || z.y0 >= z.y1 {
            return Err(Error::Unsupported(format!("empty zone {z:?}")));
        }
    }
    Ok(start_custom(VacuumStatus::ZoneCleaning.code(), Some(zones_payload(zones))))
}

/// 4.4: suction level.
pub fn set_suction(level: SuctionLevel) -> Command {
    Command::set(4, 4, RawValue::Int(level.code()))
}

/// 4.5: water volume (models without a self-wash base).
pub fn set_water_volume(level: WaterVolume) -> Command {
    Command::set(4, 5, RawValue::Int(level.code()))
}

/// 28.1: mop wetness 1..32 (wetness-level models such as r2579a).
pub fn set_wetness(level: u8) -> Result<Command> {
    if !(1..=32).contains(&level) {
        return Err(Error::Unsupported(format!("wetness {level} outside 1..32")));
    }
    Ok(Command::set(28, 1, RawValue::Int(level as i64)))
}

/// 4.23: cleaning mode, rebuilt from the current 4.23 value so the
/// self-clean and water fields are kept. The 0/2 polarity is not verified
/// on r2579a; pass the one confirmed for the model.
pub fn set_mop_mode(current: &CleaningModeGroup, mode: CleaningMode, polarity: ModePolarity) -> Result<Command> {
    let next = current.with_logical(mode, polarity)?;
    Ok(Command::set(4, 23, RawValue::Int(next.raw)))
}

/// 2.6: raw cleaning mode 0..3, node-dreame's alternative to 4.23.
pub fn set_clean_mode_setting(raw_mode: u8) -> Result<Command> {
    if raw_mode > 3 {
        return Err(Error::Unsupported(format!("raw mode {raw_mode} outside 0..3")));
    }
    Ok(Command::set(2, 6, RawValue::Int(raw_mode as i64)))
}

/// 4.46: mop wash level.
pub fn set_mop_wash_level(level: MopWashLevel) -> Command {
    Command::set(4, 46, RawValue::Int(level.code()))
}

/// 5.1-5.3: the single DND window.
pub fn set_dnd(enabled: bool, start: HhMm, end: HhMm) -> Vec<Command> {
    DndWindow::encode(enabled, start, end).into_iter().map(|(id, value)| Command::SetProperty { id, value }).collect()
}

/// 5.1 alone: switch DND on or off.
pub fn set_dnd_enabled(enabled: bool) -> Command {
    Command::set(5, 1, RawValue::Bool(enabled))
}

/// `<siid>.a1`: reset a consumable's life counter.
pub fn reset_consumable(c: Consumable) -> Command {
    Command::Action { id: c.reset_action(), input: vec![] }
}

/// 4.a4: a station operation. Firmware builds up to 1037 take no argument
/// (Tasshack), so pass the fw build when known.
pub fn station(op: StationOp, fw_build: Option<u32>) -> Command {
    let input = match fw_build {
        Some(b) if b <= 1037 => vec![],
        _ => vec![arg(10, op.encode())],
    };
    Command::action(4, 4, input)
}

/// 8.2: the whole schedule list. Adding, changing and removing a schedule all write the full
/// list, with the other slots as read.
pub fn set_schedules(slots: &[crate::codec::schedule::ScheduleSlot]) -> Command {
    Command::write(PropId::new(8, 2), crate::codec::schedule::encode(slots))
}

/// 15.a1: empty the robot's bin into the station.
pub fn start_auto_empty() -> Command {
    Command::action(15, 1, vec![])
}

/// 6.a1: ask the robot to upload a full map frame (`frame_type` `I`).
pub fn request_map() -> Command {
    request_map_with(json!({"frame_type": "I"}))
}

/// 6.a1 with custom `FRAME_INFO` keys (`req_type`, `force_type`, `map_id`, ...).
pub fn request_map_with(frame_info: Value) -> Command {
    Command::action(6, 1, vec![arg(2, compact(&frame_info))])
}

/// One remote-control step: forward speed and turn speed, as the official joystick sends them.
/// See docs/research/remote-and-camera.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemoteMove {
    /// Forward speed, likely mm/s. The app sends 200 or 300. There is no reverse.
    pub forward: i32,
    /// Turn speed, likely degrees per second; positive turns left (tested). The app sends 45 or 120,
    /// and 180 once to turn around.
    pub turn: i32,
}

impl RemoteMove {
    pub const STOP: Self = Self { forward: 0, turn: 0 };
    /// The official joystick's steps: forward 300 mm/s, turns 120 deg/s.
    pub const FORWARD: Self = Self { forward: 300, turn: 0 };
    pub const LEFT: Self = Self { forward: 0, turn: 120 };
    pub const RIGHT: Self = Self { forward: 0, turn: -120 };
    /// Sent once, not repeated.
    pub const TURN_AROUND: Self = Self { forward: 0, turn: 180 };

    pub fn is_stop(&self) -> bool {
        *self == Self::STOP
    }
}

/// How often to repeat a held [`RemoteMove`]. The robot seems to stop after about 1.2 s
/// without a new step; the official app repeats every 1.0 to 1.3 s.
pub const REMOTE_REPEAT_MS: u64 = 1000;

/// 4.15: one remote-control step. The first write puts the robot in remote control (status 23,
/// work mode 13); there is no command to leave it. `prompt` asks the robot to play its voice
/// prompt, which the app does only on the first step. `nonce` makes repeated steps differ.
pub fn remote_move(m: RemoteMove, prompt: bool, nonce: u16) -> Command {
    let audio = if prompt { "true" } else { "false" };
    let v = format!(r#"{{"spdv":{},"spdw":{},"audio":"{audio}","random":{nonce}}}"#, m.forward, m.turn);
    Command::write(PropId::new(4, 15), RawValue::Str(v))
}

/// 4.a6 (`getPhotoInfo`): asks the robot to upload one obstacle photo to the cloud. `name` and
/// `pwd` come from the map's `ai_obstacle` entry; `object` is `name + "-" + key`.
pub fn upload_obstacle_photo(name: &str, pwd: &str, object: &str) -> Command {
    let v = compact(&json!({"name": name, "pwd": pwd, "object": object}));
    Command::action(4, 6, vec![arg(10, v)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_move_payload() {
        let c = remote_move(RemoteMove::LEFT, true, 7);
        assert_eq!(
            c,
            Command::set(4, 15, RawValue::Str(r#"{"spdv":0,"spdw":120,"audio":"true","random":7}"#.into()))
        );
        let c = remote_move(RemoteMove::STOP, false, 0);
        assert_eq!(c, Command::set(4, 15, RawValue::Str(r#"{"spdv":0,"spdw":0,"audio":"false","random":0}"#.into())));
    }

    #[test]
    fn obstacle_upload_payload() {
        let c = upload_obstacle_photo("/a/b.jpg", "pw", "/a/b.jpg-k");
        let (id, input) = action(&c);
        assert_eq!(id, ActionId::new(4, 6));
        assert_eq!(input[0]["piid"], 10);
        let v: Value = serde_json::from_str(input[0]["value"].as_str().unwrap()).unwrap();
        assert_eq!(v, json!({"name": "/a/b.jpg", "pwd": "pw", "object": "/a/b.jpg-k"}));
    }

    fn action(c: &Command) -> (ActionId, &Vec<Value>) {
        match c {
            Command::Action { id, input } => (*id, input),
            other => panic!("not an action: {other:?}"),
        }
    }

    #[test]
    fn simple_actions() {
        assert_eq!(action(&start()).0, ActionId::new(2, 1));
        assert_eq!(action(&pause()).0, ActionId::new(2, 2));
        assert_eq!(action(&stop()).0, ActionId::new(4, 2));
        assert_eq!(action(&dock()).0, ActionId::new(3, 1));
        assert_eq!(action(&locate()).0, ActionId::new(7, 1));
        assert_eq!(action(&clear_warning()).0, ActionId::new(4, 3));
        assert_eq!(action(&start_auto_empty()).0, ActionId::new(15, 1));
        assert!(action(&start()).1.is_empty());
    }

    #[test]
    fn rooms_and_zones() {
        let c = clean_rooms(&[RoomClean { segment: 4, repeats: 1, suction: 1, water: 2 }], true).unwrap();
        let (id, input) = action(&c);
        assert_eq!(id, ActionId::new(4, 1));
        assert_eq!(input[0], json!({"piid": 1, "value": 18}));
        assert_eq!(input[1], json!({"piid": 10, "value": "{\"selects\":[[4,1,1,2,1]]}"}));
        assert!(clean_rooms(&[], true).is_err());
        let z = ZoneClean { x0: 0, y0: 0, x1: 1000, y1: 1000, repeats: 1, suction: 1, water: 1 };
        let zc = clean_zones(&[z]).unwrap();
        let (_, input) = action(&zc);
        assert_eq!(input[0]["value"], 19);
        assert!(clean_zones(&[ZoneClean { x1: 0, ..z }]).is_err());
    }

    #[test]
    fn property_writes() {
        assert_eq!(set_suction(SuctionLevel::Turbo), Command::set(4, 4, RawValue::Int(3)));
        assert_eq!(set_water_volume(WaterVolume::High), Command::set(4, 5, RawValue::Int(3)));
        assert_eq!(set_wetness(16).unwrap(), Command::set(28, 1, RawValue::Int(16)));
        assert!(set_wetness(0).is_err());
        assert_eq!(set_mop_wash_level(MopWashLevel::Deep), Command::set(4, 46, RawValue::Int(2)));
        assert!(set_clean_mode_setting(4).is_err());
        let g = CleaningModeGroup::split(5123, true);
        assert_eq!(
            set_mop_mode(&g, CleaningMode::SweepingAndMopping, ModePolarity::NodeDreame).unwrap(),
            Command::set(4, 23, RawValue::Int(5122))
        );
    }

    #[test]
    fn dnd_consumables_station_map() {
        let d = set_dnd(true, HhMm::new(22, 0).unwrap(), HhMm::new(8, 0).unwrap());
        assert_eq!(d.len(), 3);
        assert_eq!(d[2], Command::set(5, 3, RawValue::Str("08:00".into())));
        assert_eq!(set_dnd_enabled(false), Command::set(5, 1, RawValue::Bool(false)));
        assert_eq!(action(&reset_consumable(Consumable::SideBrush)).0, ActionId::new(10, 1));
        let s = station(StationOp::StartDrying, Some(1664));
        assert_eq!(action(&s).1[0], json!({"piid": 10, "value": "3,1"}));
        assert!(action(&station(StationOp::StartDrying, Some(1000))).1.is_empty());
        let rm = request_map();
        let (id, input) = action(&rm);
        assert_eq!(id, ActionId::new(6, 1));
        assert_eq!(input[0], json!({"piid": 2, "value": "{\"frame_type\":\"I\"}"}));
    }
}
