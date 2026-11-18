//! The saved-map list (property 6.8): every saved map the robot holds, one per floor.
//!
//! With multi-floor maps on (6.7), the list holds several maps; with it off, one. Each entry
//! carries the map's envelope inline (`map`), or names a cloud object to fetch (`rismobj`, when
//! the list has `server == 1`). The list's `curr_id` is the `map_id` of the map the robot uses
//! now. The entry's own `id` field is an index, not a `map_id` (maps.md §2.5), so a map is
//! identified by the `map_id` in its decoded header.

use anyhow::{anyhow, bail, Context, Result};
use dreame_map::Frame;
use serde_json::Value;

/// Where an entry's envelope is.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// The envelope text, inline in the list.
    Inline(String),
    /// A cloud object to download.
    Object(String),
}

/// One entry of the list, before its envelope is fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The custom map name; empty when the user has not named the map.
    pub name: String,
    /// The saved display rotation in degrees.
    pub angle: i64,
    pub source: Source,
}

/// The list as it comes from the cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct MapList {
    /// `curr_id`: the `map_id` of the robot's current map.
    pub current: Option<i64>,
    pub entries: Vec<Entry>,
}

/// Parses the list file. An entry without map data is skipped, as the app does.
pub fn parse(text: &str) -> Result<MapList> {
    let list: Value = serde_json::from_str(text).context("the saved-map list is not JSON")?;
    let server = list["server"].as_i64() == Some(1);
    let mut entries = Vec::new();
    for e in list["mapstr"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let source = match (e["map"].as_str().filter(|s| !s.is_empty()), e["rismobj"].as_str()) {
            (Some(t), _) => Source::Inline(t.to_string()),
            (None, Some(o)) if server || !o.is_empty() => Source::Object(o.to_string()),
            _ => continue,
        };
        entries.push(Entry { name: e["name"].as_str().unwrap_or_default().to_string(), angle: int(&e["angle"]).unwrap_or(0), source });
    }
    Ok(MapList { current: int(&list["curr_id"]), entries })
}

/// The object a 6.8 value (`{"obj_name"|"object_name": ..., "md5": ...}`) points at.
pub fn pointer_object(pointer: &str) -> Result<String> {
    let ptr: Value = serde_json::from_str(pointer).context("6.8 is not JSON")?;
    ptr["obj_name"].as_str().or(ptr["object_name"].as_str()).map(str::to_string).ok_or_else(|| anyhow!("no object in 6.8"))
}

/// A number, or a number in a string (the robot sends `angle` as `"0"`).
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// One saved map with its envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedMap {
    /// The `map_id` from the envelope's header.
    pub map_id: i64,
    pub name: String,
    pub angle: i64,
    pub envelope: String,
}

/// Every saved map, with the envelopes fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedMaps {
    /// `curr_id` of the list.
    pub current: Option<i64>,
    pub maps: Vec<SavedMap>,
}

impl SavedMaps {
    /// Decodes each envelope's header to learn its `map_id`. `envelopes` holds one envelope per
    /// entry of `list`, in order.
    pub fn new(list: MapList, envelopes: Vec<String>) -> Result<Self> {
        if list.entries.len() != envelopes.len() {
            bail!("{} list entries but {} envelopes", list.entries.len(), envelopes.len());
        }
        let mut maps = Vec::with_capacity(envelopes.len());
        for (i, (e, envelope)) in list.entries.into_iter().zip(envelopes).enumerate() {
            let frame = Frame::decode(&envelope, None, None).with_context(|| format!("decode saved map {i} of the list"))?;
            maps.push(SavedMap { map_id: i64::from(frame.header.map_id), name: e.name, angle: e.angle, envelope });
        }
        Ok(Self { current: list.current, maps })
    }

    /// The map the robot uses now: the one named by `curr_id`, else the first.
    pub fn current_map(&self) -> Option<&SavedMap> {
        self.current.and_then(|id| self.by_id(id)).or_else(|| self.maps.first())
    }

    pub fn by_id(&self, map_id: i64) -> Option<&SavedMap> {
        self.maps.iter().find(|m| m.map_id == map_id)
    }

    /// `map_id` when given, else the current map. Fails when the list is empty or has no such map.
    pub fn pick(&self, map_id: Option<i64>) -> Result<&SavedMap> {
        match map_id {
            Some(id) => self.by_id(id).ok_or_else(|| anyhow!("no saved map {id}; the robot has maps {:?}", self.ids())),
            None => self.current_map().ok_or_else(|| anyhow!("the robot has no saved map")),
        }
    }

    pub fn ids(&self) -> Vec<i64> {
        self.maps.iter().map(|m| m.map_id).collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use dreame_map::header::{FrameType, Header, Pose};
    use dreame_map::trailer::Trailer;
    use serde_json::json;

    /// A 2 x 1 saved map with header `map_id`.
    pub(crate) fn envelope(map_id: i16) -> String {
        let header = Header {
            map_id,
            frame_id: 0,
            frame_type: FrameType::I,
            robot: Pose::new(0, 0, 0),
            charger: Pose::new(0, 0, 32767),
            grid_size: 50,
            width: 2,
            height: 1,
            left: 25,
            top: 25,
        };
        let trailer = Trailer::from_value(json!({"seg_inf": {"1": {"type": 1, "index": 0}}})).unwrap();
        Frame { header, pixels: vec![1, 1], trailer, aes: None }.encode().unwrap()
    }

    #[test]
    fn one_map_as_on_r2579a() {
        // The shape r2579a sends with multi-floor off: `id` 0 is an index, the map is map 1.
        let text = json!({"mapstr": [{"id": 0, "name": "", "angle": "0", "map": envelope(1)}], "curr_id": 1}).to_string();
        let list = parse(&text).unwrap();
        assert_eq!(list.current, Some(1));
        assert_eq!(list.entries.len(), 1);
        let maps = SavedMaps::new(list, vec![envelope(1)]).unwrap();
        assert_eq!(maps.ids(), [1]);
        assert_eq!(maps.pick(None).unwrap().map_id, 1);
        assert!(maps.pick(Some(2)).unwrap_err().to_string().contains("[1]"));
    }

    #[test]
    fn several_maps_are_found_by_header_map_id() {
        // Two floors. `id` counts entries, so `curr_id` 2 must not pick entry `id` 2 (there is
        // none) or entry index 2; it picks the map whose header says map 2.
        let text = json!({
            "mapstr": [
                {"id": 0, "name": "Ground", "angle": "90", "map": envelope(2)},
                {"id": 1, "name": "", "angle": 0, "map": envelope(1)},
            ],
            "curr_id": "2"
        })
        .to_string();
        let list = parse(&text).unwrap();
        assert_eq!(list.current, Some(2));
        assert_eq!((list.entries[0].name.as_str(), list.entries[0].angle), ("Ground", 90));
        let maps = SavedMaps::new(list, vec![envelope(2), envelope(1)]).unwrap();
        assert_eq!(maps.ids(), [2, 1]);
        assert_eq!(maps.current_map().unwrap().name, "Ground");
        assert_eq!(maps.pick(Some(1)).unwrap().envelope, envelope(1));
    }

    #[test]
    fn current_falls_back_to_the_first_map() {
        let list = MapList { current: Some(9), entries: vec![Entry { name: String::new(), angle: 0, source: Source::Inline(envelope(3)) }] };
        let maps = SavedMaps::new(list, vec![envelope(3)]).unwrap();
        assert_eq!(maps.pick(None).unwrap().map_id, 3);
        let empty = SavedMaps { current: None, maps: vec![] };
        assert!(empty.pick(None).is_err());
    }

    #[test]
    fn object_entries_and_empty_entries() {
        let text = json!({
            "server": 1,
            "mapstr": [
                {"id": 0, "map": "", "rismobj": "obj/a"},
                {"id": 1, "map": ""},
            ],
            "curr_id": 1
        })
        .to_string();
        let list = parse(&text).unwrap();
        assert_eq!(list.entries.len(), 1, "an entry without data is skipped");
        assert_eq!(list.entries[0].source, Source::Object("obj/a".into()));
        assert!(parse("{}").unwrap().entries.is_empty());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn pointer_forms() {
        assert_eq!(pointer_object(r#"{"obj_name":"a/9","md5":"x"}"#).unwrap(), "a/9");
        assert_eq!(pointer_object(r#"{"object_name":"a/9"}"#).unwrap(), "a/9");
        assert!(pointer_object("{}").is_err());
    }
}
