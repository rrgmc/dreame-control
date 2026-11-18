//! Payloads for START_CUSTOM (4.a1) in property 4.10 (device-model.md 4.6).

use super::compact;
use serde_json::json;

/// One room in a segment clean: `[seg, repeats, suction, water, order]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomClean {
    pub segment: u32,
    pub repeats: u32,
    pub suction: i64,
    /// Water volume, or mop pad humidity 1..3 on self-wash models.
    pub water: i64,
}

/// One zone in map millimetres: `[x0, y0, x1, y1, repeats, suction, water]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneClean {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
    pub repeats: u32,
    pub suction: i64,
    pub water: i64,
}

/// `{"selects":[...]}`. With `customized_cleaning` (4.26 present) every
/// order field is 1; Tasshack notes that other values break gen5 robots.
/// Otherwise the order is the 1-based position.
pub fn rooms_payload(rooms: &[RoomClean], customized_cleaning: bool) -> String {
    let selects: Vec<_> = rooms
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let order = if customized_cleaning { 1 } else { i + 1 };
            json!([r.segment, r.repeats.max(1), r.suction, r.water, order])
        })
        .collect();
    compact(&json!({ "selects": selects }))
}

/// `{"areas":[...]}`.
pub fn zones_payload(zones: &[ZoneClean]) -> String {
    let areas: Vec<_> =
        zones.iter().map(|z| json!([z.x0, z.y0, z.x1, z.y1, z.repeats.max(1), z.suction, z.water])).collect();
    compact(&json!({ "areas": areas }))
}

/// `{"points":[[x, y, repeats, suction, water], ...]}`.
pub fn spots_payload(points: &[(i64, i64)], repeats: u32, suction: i64, water: i64) -> String {
    let pts: Vec<_> = points.iter().map(|(x, y)| json!([x, y, repeats.max(1), suction, water])).collect();
    compact(&json!({ "points": pts }))
}

/// `{"tpoint":[[x, y, 0, 0]]}`: one target for a go to point task (status 23).
pub fn goto_payload(x: i64, y: i64) -> String {
    compact(&json!({ "tpoint": [[x, y, 0, 0]] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms() {
        let r = [RoomClean { segment: 3, repeats: 0, suction: 1, water: 2 }, RoomClean { segment: 7, repeats: 2, suction: 2, water: 3 }];
        assert_eq!(rooms_payload(&r, true), r#"{"selects":[[3,1,1,2,1],[7,2,2,3,1]]}"#);
        assert_eq!(rooms_payload(&r, false), r#"{"selects":[[3,1,1,2,1],[7,2,2,3,2]]}"#);
    }

    #[test]
    fn zones_and_spots() {
        let z = [ZoneClean { x0: -100, y0: 200, x1: 1500, y1: 2600, repeats: 1, suction: 2, water: 2 }];
        assert_eq!(zones_payload(&z), r#"{"areas":[[-100,200,1500,2600,1,2,2]]}"#);
        assert_eq!(spots_payload(&[(10, 20)], 0, 1, 1), r#"{"points":[[10,20,1,1,1]]}"#);
        assert_eq!(goto_payload(-150, 2400), r#"{"tpoint":[[-150,2400,0,0]]}"#);
    }
}
