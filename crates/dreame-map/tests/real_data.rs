//! Tests against the user's real captures.
//!
//! They run only when `DREAME_FIXTURES` points at the git-ignored `local/out/` directory, and
//! are skipped (they pass without checking anything) otherwise. The captures hold a real floor
//! plan, so these tests assert on structure and counts only and never copy content into the repo.

use dreame_map::grid::{Encoding, PixelClass};
use dreame_map::merge::{MapState, PushOutcome};
use dreame_map::render::{render_png, RenderOptions};
use dreame_map::trailer::json_semantic_eq;
use dreame_map::{Frame, FrameType, Map};
use std::path::PathBuf;

/// r2579a map version (maps.md §2.9).
const VERSION: u8 = 2;

fn fixtures() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("DREAME_FIXTURES")?);
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("DREAME_FIXTURES is not a directory; skipping");
        None
    }
}

fn read(dir: &std::path::Path, rel: &str) -> Option<String> {
    let p = dir.join(rel);
    match std::fs::read_to_string(&p) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!("missing fixture {rel}; skipping");
            None
        }
    }
}

/// The raw trailer JSON of a frame, straight from the inflated bytes.
fn raw_trailer(text: &str) -> serde_json::Value {
    let raw = dreame_map::envelope::decode(text, None, None).unwrap().raw;
    let f = Frame::parse(&raw).unwrap();
    serde_json::from_slice(&raw[27 + f.pixels.len()..]).unwrap()
}

fn saved_envelope(dir: &std::path::Path) -> Option<String> {
    let list: serde_json::Value =
        serde_json::from_str(&read(dir, "map/map-list-6.8.json")?).unwrap();
    Some(list["mapstr"][0]["map"].as_str().unwrap().to_string())
}

fn check_round_trip(text: &str) {
    let f = Frame::decode(text, None, None).unwrap();
    assert!(
        f.trailer.unparsed.is_empty(),
        "keys that failed typed parsing: {:?}",
        f.trailer.unparsed
    );
    let orig = raw_trailer(text);
    assert!(
        json_semantic_eq(&orig, &f.trailer.to_value()),
        "trailer changed on round trip"
    );
    // Re-encode and decode: identical frame.
    let again = Frame::decode(&f.encode().unwrap(), None, None).unwrap();
    assert_eq!(again, f);
}

#[test]
fn saved_map_from_6_8() {
    let Some(dir) = fixtures() else { return };
    let Some(text) = saved_envelope(&dir) else {
        return;
    };
    let f = Frame::decode(&text, None, None).unwrap();
    assert_eq!(f.header.frame_type, FrameType::I);
    assert_eq!(f.header.grid_size, 50);
    assert_eq!(f.encoding(VERSION, true), Encoding::Saved);
    check_round_trip(&text);

    let m = Map::from_frame(&f, VERSION, true).unwrap();
    let seg = f.trailer.seg_inf.as_ref().unwrap();
    assert_eq!(seg.len(), 13);
    assert_eq!(m.rooms.len(), 13);
    assert_eq!(m.hidden_rooms().len(), 2);
    assert_eq!(m.cleaning_order().len(), 11);
    // Every custom-named room decodes to UTF-8.
    for s in seg.values().filter(|s| s.room_type == Some(0)) {
        assert!(s.name_text().is_some());
    }
    // Floor plan: one closed polygon per room, all walls axis-aligned.
    let fp = m.floor_plan.as_ref().unwrap();
    assert_eq!(fp.rooms.len(), 13);
    assert_eq!(fp.doors.len(), 17);
    for r in &fp.rooms {
        assert!(r.is_closed());
        assert!(r
            .walls
            .iter()
            .all(|w| w.line.a.x == w.line.b.x || w.line.a.y == w.line.b.y));
    }
    // Every door joins two rooms that list each other as neighbours.
    for d in &fp.doors {
        assert!(m.are_neighbours(d.rooms.0, d.rooms.1));
    }
    assert_eq!(m.furniture.len(), 9);
    assert_eq!(m.carpets.len(), 2);
    assert!(!m.carpet_areas.is_empty());
    // The carpet command resends the stored entries unchanged.
    let cpt = dreame_map::edit::carpets(&m).unwrap();
    let raw = raw_trailer(&text);
    // Rebuilding the cleaning order from the visible rooms reproduces the stored id-to-position
    // pairs. The robot stores them sorted by id; the command lists them in cleaning order.
    let visible: Vec<u32> = m
        .cleaning_order()
        .into_iter()
        .filter(|id| !m.rooms[id].hidden)
        .collect();
    let order = dreame_map::edit::cleaning_order(&m, &visible).unwrap();
    let pairs = |v: &serde_json::Value| -> std::collections::BTreeMap<String, i64> {
        v.as_array()
            .unwrap()
            .iter()
            .flat_map(|e| e.as_object().unwrap().iter())
            .map(|(k, n)| (k.clone(), n.as_i64().unwrap()))
            .collect()
    };
    assert_eq!(
        pairs(&order.value()["cleanareaorder"]),
        pairs(&raw["cleanareaorder"])
    );
    let vw = &raw["vw"];
    assert_eq!(cpt.value()["cpt"]["addcpt"], vw["addcpt"]);
    assert_eq!(cpt.value()["cpt"]["nocpt"], vw["nocpt"]);
    // With no change, the vw command reproduces the stored vw object.
    let vw_cmd = dreame_map::edit::restrictions(&m).unwrap();
    assert!(json_semantic_eq(&vw_cmd.value()["vw"], vw));
    // The app snaps carpet corners to cell corners. They lie on the transform's lattice, which
    // confirms that the frame origin is a cell centre (map-plugin.md §4.1).
    let tr = m.transform();
    for c in &m.carpet_areas {
        if let dreame_map::model::CarpetShape::Rect(r) = c.shape {
            for (v, o) in [(r.x0, tr.left), (r.y0, tr.top), (r.x1, tr.left), (r.y1, tr.top)] {
                assert_eq!((v - o).rem_euclid(tr.grid_size), 0.0);
            }
        }
    }
    // Encoding E: every wall pixel belongs to a room boundary.
    assert!(m
        .grid
        .pixels
        .iter()
        .all(|p| p.class != PixelClass::Wall(None)));
    let png = render_png(
        &m,
        &RenderOptions {
            floor_plan: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(png.len() > 1000);
    eprintln!(
        "saved map: {} x {}, {} rooms",
        m.grid.width,
        m.grid.height,
        m.rooms.len()
    );
}

#[test]
fn live_frame_6_3_and_rism() {
    let Some(dir) = fixtures() else { return };
    let Some(text) = read(&dir, "map/frame-6.3.txt") else {
        return;
    };
    let f = Frame::decode(&text, None, None).unwrap();
    assert_eq!(f.encoding(VERSION, false), Encoding::FrameMap);
    check_round_trip(&text);
    let m = Map::from_frame(&f, VERSION, false).unwrap();
    assert!(m.grid.segment_histogram().len() >= 10);
    assert!(!m.path.is_empty());
    assert_eq!(
        m.path.len(),
        dreame_map::path::parse_tokens(f.trailer.tr.as_deref().unwrap())
            .unwrap()
            .len()
    );
    assert!(f.trailer.rism.is_some());
    check_round_trip(f.trailer.rism.as_deref().unwrap());

    // Experiment R3: compare the embedded saved map with the 6.8 saved map.
    let rism = f.trailer.decode_rism().unwrap().unwrap();
    let saved = Map::saved_from_live(&f, VERSION).unwrap().unwrap();
    assert!(saved.saved);
    if let Some(list) = saved_envelope(&dir) {
        let s68 = Frame::decode(&list, None, None).unwrap();
        let same_header = rism.header == s68.header;
        let same_pixels = rism.pixels == s68.pixels;
        let same_trailer = rism.trailer == s68.trailer;
        let diff_keys: Vec<String> = match (rism.trailer.to_value(), s68.trailer.to_value()) {
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => a
                .keys()
                .chain(b.keys())
                .filter(|k| a.get(*k) != b.get(*k))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            _ => vec![],
        };
        eprintln!("R3: rism vs 6.8: header equal {same_header}, pixels equal {same_pixels}, trailer equal {same_trailer}, differing keys {diff_keys:?}");
    }
}

/// Inline 6.1 values from the MQTT capture.
fn p_frame_texts(dir: &std::path::Path) -> Option<Vec<String>> {
    let text = read(dir, "mqtt-clean.jsonl")?;
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let p = &v["payload"];
        let params = if p["data"]["params"].is_array() {
            &p["data"]["params"]
        } else {
            &p["params"]
        };
        for item in params.as_array().into_iter().flatten() {
            if item["siid"] == 6 && item["piid"] == 1 {
                if let Some(s) = item["value"].as_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
    Some(out)
}

fn history_frame(dir: &std::path::Path) -> Option<Frame> {
    let obj = dir.join("obj");
    let entry = std::fs::read_dir(&obj)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().ends_with(".bin"))?;
    let text = std::fs::read_to_string(entry.path()).ok()?;
    check_round_trip(&text);
    Some(Frame::decode(&text, None, None).unwrap())
}

#[test]
fn history_map() {
    let Some(dir) = fixtures() else { return };
    let Some(h) = history_frame(&dir) else { return };
    assert_eq!(h.header.frame_type, FrameType::I);
    assert_eq!(h.header.map_id, 5);
    assert_eq!(h.header.frame_id, 123);
    assert_eq!(h.trailer.iscleanlog, Some(true));
    assert!(h.trailer.tr.as_deref().unwrap().starts_with('S'));
    let saved = Map::saved_from_live(&h, VERSION).unwrap().unwrap();
    assert_eq!(saved.rooms.len(), 13);
    let m = Map::from_live(&h, VERSION).unwrap();
    // Live rooms take names and neighbours from the embedded saved map.
    assert!(m
        .rooms
        .values()
        .filter(|r| r.pixel_count > 0)
        .all(|r| r.room_type.is_some()));
    assert!(m.floor_plan.is_some());
    assert!(render_png(&m, &RenderOptions::default()).unwrap().len() > 1000);
}

#[test]
fn mqtt_p_frames_merge() {
    let Some(dir) = fixtures() else { return };
    let Some(texts) = p_frame_texts(&dir) else {
        return;
    };
    assert_eq!(texts.len(), 122);
    let frames: Vec<Frame> = texts
        .iter()
        .map(|t| Frame::decode(t, None, None).unwrap())
        .collect();
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.header.frame_type, FrameType::P);
        assert_eq!(f.header.map_id, 5);
        assert_eq!(
            f.header.frame_id as usize,
            i + 1,
            "frame ids are sequential from 1"
        );
        assert!(
            f.trailer.unparsed.is_empty(),
            "frame {}: {:?}",
            i + 1,
            f.trailer.unparsed
        );
        assert_eq!(f.trailer.fsm, Some(1));
    }
    let with_pixels = frames.iter().filter(|f| f.header.pixel_count() > 0).count();
    let with_origin = frames.iter().filter(|f| f.trailer.origin.is_some()).count();
    let origin_eq_header = frames
        .iter()
        .filter(|f| {
            f.trailer
                .origin
                .is_some_and(|o| o[0] == f.header.left as i64 && o[1] == f.header.top as i64)
        })
        .count();
    let grid_sizes: std::collections::BTreeSet<i16> =
        frames.iter().map(|f| f.header.grid_size).collect();

    // Merge onto an empty base: there is no I-frame for map 5 before the P-frames.
    let mut st = MapState::new(VERSION);
    st.start_empty(5, 50);
    for f in frames.clone() {
        assert!(matches!(st.push(f).unwrap(), PushOutcome::Applied { .. }));
    }
    let merged = st.current().unwrap().clone();
    assert_eq!(merged.header.frame_id, 122);
    let p_tokens: usize = frames
        .iter()
        .filter_map(|f| f.trailer.tr.as_deref())
        .map(|t| dreame_map::path::parse_tokens(t).unwrap().len())
        .sum();
    assert_eq!(st.path().len(), p_tokens);

    eprintln!(
        "P-frames: {} total, {} with pixels, {} with origin ({} equal to header), grid sizes {:?}, path tokens {}",
        frames.len(),
        with_pixels,
        with_origin,
        origin_eq_header,
        grid_sizes,
        p_tokens
    );

    let Some(h) = history_frame(&dir) else { return };
    let h_path = dreame_map::path::Path::parse(h.trailer.tr.as_deref().unwrap()).unwrap();
    // The history path is the merged P-frame path with a few points dropped: an in-order
    // subsequence of it.
    let merged_pts: Vec<(i64, i64)> = st.path().points.iter().map(|p| (p.x, p.y)).collect();
    let mut it = merged_pts.iter();
    let subsequence = h_path.points.iter().all(|p| it.any(|q| *q == (p.x, p.y)));
    eprintln!(
        "path: history {} points, merged {} points, history is a subsequence of merged: {subsequence}; length {:.0} mm vs {:.0} mm",
        h_path.len(),
        st.path().len(),
        h_path.length_mm(),
        st.path().length_mm()
    );
    assert!(subsequence);
    assert!(h_path.len() <= st.path().len() && h_path.len() * 100 >= st.path().len() * 98);
    assert!((h_path.length_mm() - st.path().length_mm()).abs() < st.path().length_mm() * 0.01);

    // Pixels. No I-frame of map 5 precedes the P-frames, so use the earlier live frame (6.3,
    // previous session) as the base, then compare with the history frame (frame 123).
    let Some(text) = read(&dir, "map/frame-6.3.txt") else {
        return;
    };
    let mut base = Frame::decode(&text, None, None).unwrap();
    base.header.map_id = 5;
    base.header.frame_id = 0;
    let count = |cand: &Frame| {
        let g = h.header.grid_size as i32;
        let (hw, hh) = (h.header.width as i32, h.header.height as i32);
        let (cw, ch) = (cand.header.width as i32, cand.header.height as i32);
        let (mut eq, mut ne) = (0usize, 0usize);
        for y in 0..hh {
            for x in 0..hw {
                let hv = h.pixels[(y * hw + x) as usize];
                let (cx, cy) = (
                    (h.left() + x * g - cand.left()) / g,
                    (h.top() + y * g - cand.top()) / g,
                );
                let cv = if cx < 0 || cy < 0 || cx >= cw || cy >= ch {
                    0
                } else {
                    cand.pixels[(cy * cw + cx) as usize]
                };
                if hv == 0 && cv == 0 {
                    continue;
                }
                if hv == cv {
                    eq += 1
                } else {
                    ne += 1
                }
            }
        }
        (eq, ne)
    };
    let run = |version: u8| {
        let mut st = MapState::new(version);
        st.push(base.clone()).unwrap();
        for f in frames.clone() {
            st.push(f).unwrap();
        }
        st.current().unwrap().clone()
    };
    let alone = count(&base);
    let add = count(&run(VERSION));
    let replace = count(&run(3));
    eprintln!("pixels vs history (equal, differ): base alone {alone:?}, add rule {add:?}, replace rule {replace:?}");
    assert!(
        add.1 * 10 < replace.1,
        "the add rule should explain the history frame far better"
    );
    assert!(add.1 * 10 < alone.1);
}
