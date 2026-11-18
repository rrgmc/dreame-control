//! Robot control: start, pause, stop, return to the dock, and cleaning chosen rooms or a zone.
//!
//! Every function that sends is a write that makes the robot move; callers must have the user's
//! approval for each one. The builders only build.
//!
//! Payloads follow the official app (map-plugin.md, robot control): a room clean is action 4.1
//! with piid 1 = 18 and piid 10 = `{"selects":[[room, passes, suction, water, 1], ...]}`; a zone
//! clean is piid 1 = 19 and piid 10 = `{"areas":[[x0, y0, x1, y1, passes, suction, water]]}`.
//! Go to point is piid 1 = 23 and piid 10 = `{"tpoint":[[x, y, 0, 0]]}`.

use anyhow::{anyhow, bail, Result};
use dreame_cloud::{CloudClient, DeviceInfo};
use dreame_map::model::{Map, Point, Rect};
use dreame_model::codec::custom_clean::{RoomClean, ZoneClean};
use dreame_model::commands::{self, Command};
use serde_json::Value;

/// Overrides for a clean; `None` takes the default (the room's stored setting, or the values
/// in [`ZONE_DEFAULTS`] for a zone).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanOptions {
    pub passes: Option<u32>,
    pub suction: Option<i64>,
    pub water: Option<i64>,
}

/// Passes, suction and water for a zone clean when not given: one pass, standard suction,
/// water level 2.
pub const ZONE_DEFAULTS: (u32, i64, i64) = (1, 1, 2);

/// Cleans the given rooms, in the order given. Each room's passes, suction and water come from
/// its stored settings (`cleanset`) unless overridden. Hidden or unknown rooms are refused.
pub fn clean_rooms(map: &Map, rooms: &[u32], o: CleanOptions) -> Result<Command> {
    if rooms.is_empty() {
        bail!("give at least one room");
    }
    let mut list = Vec::with_capacity(rooms.len());
    for &id in rooms {
        let room = map.rooms.get(&id).ok_or_else(|| anyhow!("room {id} does not exist"))?;
        if room.hidden {
            bail!("room {id} is hidden");
        }
        let s = room.settings.as_deref().unwrap_or(&[]);
        let stored = |i: usize| s.get(i).copied();
        list.push(RoomClean {
            segment: id,
            repeats: passes_or_default(o.passes, stored(2)),
            suction: o.suction.or(stored(0)).unwrap_or(1),
            water: o.water.or(stored(1)).unwrap_or(2),
        });
    }
    // The app sends 1 in every order field (customized cleaning).
    Ok(commands::clean_rooms(&list, true)?)
}

fn passes_or_default(given: Option<u32>, stored: Option<i64>) -> u32 {
    given.or(stored.and_then(|p| u32::try_from(p).ok())).unwrap_or(1).max(1)
}

/// Cleans one rectangle (world mm).
pub fn clean_zone(map: &Map, zone: Rect, o: CleanOptions) -> Result<Command> {
    if zone.width() <= 0.0 || zone.height() <= 0.0 {
        bail!("the zone has no area");
    }
    if !map.bounds().contains_rect(&zone) {
        bail!("the zone {:?} leaves the map", zone.to_quad());
    }
    let q = zone.to_quad();
    let (p, s, w) = ZONE_DEFAULTS;
    let z = ZoneClean {
        x0: q[0],
        y0: q[1],
        x1: q[2],
        y1: q[3],
        repeats: o.passes.unwrap_or(p).max(1),
        suction: o.suction.unwrap_or(s),
        water: o.water.unwrap_or(w),
    };
    Ok(commands::clean_zones(&[z])?)
}

/// Sends the robot to one point (world mm) without cleaning. The point must lie on the floor
/// of a room that is not hidden, so the robot has somewhere to go.
pub fn go_to(map: &Map, p: Point) -> Result<Command> {
    let room = map.room_at(p).ok_or_else(|| anyhow!("the point ({:.0}, {:.0}) is not on the floor of a room", p.x, p.y))?;
    if map.rooms.get(&room).is_some_and(|r| r.hidden) {
        bail!("the point is in hidden room {room}");
    }
    Ok(commands::go_to(p.x.round() as i64, p.y.round() as i64))
}

/// The command as JSON, for showing before it is sent.
pub fn describe(cmd: &Command) -> Value {
    match cmd {
        Command::Action { id, input } => {
            serde_json::json!({"action": format!("{}.{}", id.siid, id.aiid), "in": input})
        }
        Command::SetProperty { id, value } => {
            serde_json::json!({"set": format!("{}.{}", id.siid, id.piid), "value": value.to_json()})
        }
    }
}

/// Sends one command. This is a write. Returns the action result (without the echoed device
/// id) or the property results.
pub async fn send(c: &CloudClient, dev: &DeviceInfo, cmd: &Command) -> Result<Value> {
    match cmd {
        Command::Action { id, input } => {
            let mut r = c.action(dev, *id, input.clone()).await?;
            if let Some(o) = r.as_object_mut() {
                o.remove("did");
            }
            Ok(r)
        }
        Command::SetProperty { id, value } => {
            let r = c.set_properties(dev, &[(*id, value.clone())]).await?;
            Ok(Value::Array(r.iter().map(|p| serde_json::json!({"id": p.id.to_string(), "code": p.code})).collect()))
        }
    }
}

/// The result code of an action result (`code`), if any. 0 means accepted.
pub fn result_code(r: &Value) -> Option<i64> {
    r.get("code").and_then(Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_map::header::{FrameType, Header, Pose};
    use dreame_map::{Frame, Trailer};
    use serde_json::json;

    /// A 4 x 2 map: room 1 left (settings suction 2, water 3, 1 pass), room 2 right (hidden).
    fn map() -> Map {
        let header = Header {
            map_id: 1,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width: 4,
            height: 2,
            left: 25,
            top: 25,
        };
        let trailer = Trailer::from_value(json!({"delsr": [2]})).unwrap();
        let frame = Frame { header, pixels: vec![1, 1, 2, 2, 1, 1, 2, 2], trailer, aes: None };
        let mut m = Map::from_frame(&frame, 2, true).unwrap();
        m.rooms.get_mut(&1).unwrap().settings = Some(vec![2, 3, 1, 1, 2, 546]);
        m
    }

    fn input(c: &Command) -> Value {
        match c {
            Command::Action { input, .. } => Value::Array(input.clone()),
            other => panic!("not an action: {other:?}"),
        }
    }

    #[test]
    fn room_clean_uses_stored_settings_like_the_app() {
        let m = map();
        let c = clean_rooms(&m, &[1], CleanOptions::default()).unwrap();
        assert_eq!(describe(&c)["action"], "4.1");
        assert_eq!(input(&c), json!([{"piid": 1, "value": 18}, {"piid": 10, "value": "{\"selects\":[[1,1,2,3,1]]}"}]));
        let c = clean_rooms(&m, &[1], CleanOptions { passes: Some(2), suction: Some(0), water: None }).unwrap();
        assert_eq!(input(&c)[1]["value"], "{\"selects\":[[1,2,0,3,1]]}");
        assert!(clean_rooms(&m, &[2], CleanOptions::default()).is_err(), "hidden");
        assert!(clean_rooms(&m, &[7], CleanOptions::default()).is_err(), "unknown");
        assert!(clean_rooms(&m, &[], CleanOptions::default()).is_err());
    }

    #[test]
    fn zone_clean() {
        let m = map();
        let c = clean_zone(&m, Rect::new(0.0, 0.0, 100.0, 50.0), CleanOptions::default()).unwrap();
        assert_eq!(input(&c), json!([{"piid": 1, "value": 19}, {"piid": 10, "value": "{\"areas\":[[0,0,100,50,1,1,2]]}"}]));
        assert!(clean_zone(&m, Rect::new(0.0, 0.0, 0.0, 50.0), CleanOptions::default()).is_err());
        assert!(clean_zone(&m, Rect::new(0.0, 0.0, 5000.0, 50.0), CleanOptions::default()).is_err());
    }

    #[test]
    fn go_to_point() {
        let m = map();
        let c = go_to(&m, Point::new(60.4, 40.0)).unwrap();
        assert_eq!(input(&c), json!([{"piid": 1, "value": 23}, {"piid": 10, "value": "{\"tpoint\":[[60,40,0,0]]}"}]));
        assert!(go_to(&m, Point::new(175.0, 40.0)).is_err(), "hidden room");
        assert!(go_to(&m, Point::new(5000.0, 40.0)).is_err(), "off the map");
    }

    #[test]
    fn simple_commands() {
        assert_eq!(describe(&commands::dock()), json!({"action": "3.1", "in": []}));
        assert_eq!(describe(&commands::pause())["action"], "2.2");
        assert_eq!(result_code(&json!({"code": 0})), Some(0));
    }
}
