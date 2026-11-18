//! Tests of the archive codec against the user's real recovery archives.
//!
//! They run only when `DREAME_FIXTURES` points at the git-ignored `local/out/` directory, and are
//! skipped otherwise. They assert on structure and counts only.

use dreame_map::archive::Archive;
use dreame_map::segmap::{self, Direction, Segmentation};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The W1 backup, the base of every edited archive (maps.md §12.2).
const BASE: &str = "recovery/1794861475-2.tbz2";
/// The W20 archive: the W1 backup with a room border moved, which the robot loaded (maps.md §16).
const W20: &str = "recovery/w20-edited.tbz2";

fn read(rel: &str) -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("DREAME_FIXTURES")?);
    match std::fs::read(Path::new(&dir).join(rel)) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("missing fixture {rel}; skipping");
            None
        }
    }
}

fn bunzip(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    bzip2::read::BzDecoder::new(b).read_to_end(&mut out).unwrap();
    out
}

#[test]
fn backup_round_trips_byte_for_byte() {
    let Some(b) = read(BASE) else { return };
    let a = Archive::decode(&b).unwrap();
    assert!(a.files().count() > 40);
    assert_eq!(a.encode_tar(), bunzip(&b));
    let again = Archive::decode(&a.encode().unwrap()).unwrap();
    assert_eq!(again.encode_tar(), a.encode_tar());
}

#[test]
fn grow_matches_the_w20_archive() {
    let (Some(base), Some(w20)) = (read(BASE), read(W20)) else { return };
    let mut a = Archive::decode(&base).unwrap();
    let mut s = Segmentation::read(&a).unwrap();
    // W20: the border between room 12 above and room 10 below moved 10 cells up.
    let ch = s.grow(10, 12, Direction::Up, 10).unwrap();
    assert_eq!(ch.cells.len(), 290);
    s.write(&mut a).unwrap();
    let want = Archive::decode(&w20).unwrap();
    let mine = Segmentation::read(&a).unwrap();
    let theirs = Segmentation::read(&want).unwrap();
    for y in 0..mine.height {
        for x in 0..mine.width {
            assert_eq!(mine.grid_byte(x, y), theirs.grid_byte(x, y), "grid cell {x},{y}");
        }
    }
    // Every member other than the four images is unchanged, byte for byte.
    for name in a.files() {
        if name != segmap::GRID && !segmap::PLAIN.contains(&name) {
            assert_eq!(a.get(name), want.get(name), "{name}");
        }
    }
    // The plain images match the Python build too.
    for name in segmap::PLAIN {
        let png = |ar: &Archive| image::load_from_memory(ar.get(name).unwrap()).unwrap().into_luma8().into_raw();
        assert_eq!(png(&a), png(&want), "{name}");
    }
}

/// The editor paints cells of the saved map and the archive edit finds them by world mm. With a
/// saved map and a backup taken at the same moment (W24's fresh backup), every room cell of the
/// saved map must land on a cell of the same room in the archive grid.
#[test]
fn saved_map_cells_land_on_the_same_archive_rooms() {
    let (Some(saved), Some(backup)) = (read("recovery/1794929949-before.txt"), read("recovery/1794929949-backup.tbz2")) else { return };
    let frame = dreame_map::Frame::decode(std::str::from_utf8(&saved).unwrap(), None, None).unwrap();
    let map = dreame_map::Map::from_frame(&frame, 2, true).unwrap();
    let s = Segmentation::read(&Archive::decode(&backup).unwrap()).unwrap();
    let t = map.transform();
    let (mut same, mut other) = (0, 0);
    for row in 0..map.grid.height {
        for col in 0..map.grid.width {
            let Some(room) = map.grid.get(col, row).class.segment() else { continue };
            let (x, y) = t.cell_center(col, row);
            let cells = s.cells_at_mm(&[(x, y)]);
            if cells.first().and_then(|&(cx, cy)| s.room_at(cx, cy)) == Some(room) {
                same += 1;
            } else {
                other += 1;
            }
        }
    }
    assert!(same > 30_000, "{same} cells matched");
    assert_eq!(other, 0, "{other} saved-map cells landed on another room");
}

/// The editor previews a border move on the saved map and the archive edit recomputes it on the
/// archive grid. Both must pick the same cells. The line runs three cells into room 12, above its
/// shared border with room 10, across both rooms.
#[test]
fn border_cells_agree_on_map_and_archive() {
    let (Some(saved), Some(backup)) = (read("recovery/1794929949-before.txt"), read("recovery/1794929949-backup.tbz2")) else { return };
    let frame = dreame_map::Frame::decode(std::str::from_utf8(&saved).unwrap(), None, None).unwrap();
    let map = dreame_map::Map::from_frame(&frame, 2, true).unwrap();
    let t = map.transform();
    // Room 12 cells with room 10 in the next row: the shared border.
    let at = |c: usize, r: usize| (c < map.grid.width && r < map.grid.height).then(|| map.grid.get(c, r).class.segment()).flatten();
    let mut border = Vec::new();
    for r in 0..map.grid.height {
        for c in 0..map.grid.width {
            if at(c, r) == Some(12) && (at(c, r + 1) == Some(10) || r > 0 && at(c, r - 1) == Some(10)) {
                border.push(t.cell_center(c, r));
            }
        }
    }
    assert!(!border.is_empty(), "rooms 10 and 12 share a border");
    let pair: Vec<(f64, f64)> = (0..map.grid.height)
        .flat_map(|r| (0..map.grid.width).map(move |c| (c, r)))
        .filter(|&(c, r)| matches!(at(c, r), Some(10 | 12)))
        .map(|(c, r)| t.cell_center(c, r))
        .collect();
    let x0 = pair.iter().map(|p| p.0).fold(f64::MAX, f64::min) - 100.0;
    let x1 = pair.iter().map(|p| p.0).fold(f64::MIN, f64::max) + 100.0;
    let y = border.iter().map(|p| p.1).sum::<f64>() / border.len() as f64;
    // Room 12 lies on the side of decreasing or increasing y; step three cells into it.
    let (c, r) = t.cell_at(border[0].0, border[0].1 - 50.0).unwrap();
    let above = at(c, r) == Some(12);
    let y = if above { y - 150.0 } else { y + 150.0 };
    let line = [dreame_map::model::Point::new(x0, y), dreame_map::model::Point::new(x1, y)];
    let on_map = dreame_map::edit::border_cells(&map, 10, 12, &line).unwrap();
    assert!(on_map.to_a.len() > 20, "room 10 takes {} cells", on_map.to_a.len());
    let mut s = Segmentation::read(&Archive::decode(&backup).unwrap()).unwrap();
    let ch = s.move_border(10, 12, &[(x0, y), (x1, y)]).unwrap();
    let want: std::collections::BTreeSet<(u32, u32)> = on_map
        .to_a
        .iter()
        .chain(&on_map.to_b)
        .map(|&(c, r)| {
            let (x, y) = t.cell_center(c, r);
            s.cells_at_mm(&[(x, y)])[0]
        })
        .collect();
    let got: std::collections::BTreeSet<(u32, u32)> = ch.cells.iter().copied().collect();
    assert_eq!(got, want);
}

/// The room registries keep their formatting: re-serialising `seg_info` and the neighbour table
/// gives the original text. Then a W24-style split registered with `roomreg::add_room` changes
/// only the four grid images and the three registries, and in `1.dat2` only `seg_info` and
/// `cleanarea_order`.
#[test]
fn new_room_is_registered_and_nothing_else_changes() {
    use dreame_map::roomreg;
    let Some(backup) = read("recovery/1794937671-backup.tbz2") else { return };
    let base = Archive::decode(&backup).unwrap();
    let text = |a: &Archive, n: &str| String::from_utf8(a.get(n).unwrap().to_vec()).unwrap();
    let dat: serde_json::Value = serde_json::from_str(&text(&base, roomreg::DAT2)).unwrap();
    let seg = dat["seg_info"].as_str().unwrap();
    let reparsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(seg).unwrap();
    assert_eq!(serde_json::to_string(&reparsed).unwrap(), seg, "seg_info round-trips");
    let nt = text(&base, roomreg::NEIGHBOURS);
    assert_eq!(roomreg::pretty4(&serde_json::from_str(&nt).unwrap()), nt, "neighbour table round-trips");

    let mut a = base.clone();
    let mut s = Segmentation::read(&a).unwrap();
    s.split(7, 14, segmap::Axis::X, s.cell_at_mm(2600.0, 0.0).0).unwrap();
    s.write(&mut a).unwrap();
    let added = roomreg::add_room(&mut a, &s, 14, 7, &Default::default()).unwrap();
    assert!(added.neighbours.contains(&7), "{:?}", added.neighbours);
    assert_eq!(added.order, 12);
    let a = Archive::decode(&a.encode().unwrap()).unwrap();
    assert!(roomreg::seg_info_rooms(&a).unwrap().contains(&14));
    let mut changed: Vec<&str> = base.files().filter(|n| a.get(n) != base.get(n)).collect();
    changed.sort_unstable();
    let mut want = vec![segmap::GRID, roomreg::DAT2, roomreg::NEIGHBOURS, roomreg::AI_ROOMS, roomreg::AI_FLOORS];
    want.extend(segmap::PLAIN);
    want.sort_unstable();
    assert_eq!(changed, want);
    let mut before: serde_json::Value = serde_json::from_str(&text(&base, roomreg::DAT2)).unwrap();
    let mut after: serde_json::Value = serde_json::from_str(&text(&a, roomreg::DAT2)).unwrap();
    for k in ["seg_info", "cleanarea_order"] {
        before.as_object_mut().unwrap().remove(k);
        after.as_object_mut().unwrap().remove(k);
    }
    assert!(before == after, "1.dat2 changed outside seg_info and cleanarea_order");
    let order = text(&a, roomreg::DAT2);
    assert!(order.contains(r#"{"12":11},{"14":12}]"#), "cleanarea_order ends with the new room");
    let ai = text(&a, roomreg::AI_ROOMS);
    assert!(ai.lines().last().unwrap().starts_with("14 "));
    let t: serde_json::Value = serde_json::from_str(&text(&a, roomreg::NEIGHBOURS)).unwrap();
    assert!(t["RoomIdAndColorLookUpTable"]["RoomIds"].as_array().unwrap().contains(&serde_json::json!(14)));
}

/// Removing a room takes it out of the grid and every room list; removing a room that was just
/// added gives back the original archive, byte for byte.
#[test]
fn removed_room_leaves_no_trace() {
    use dreame_map::roomreg;
    let Some(backup) = read("recovery/1794937671-backup.tbz2") else { return };
    let base = Archive::decode(&backup).unwrap();
    let text = |a: &Archive, n: &str| String::from_utf8(a.get(n).unwrap().to_vec()).unwrap();

    // Add room 14 (W29), then remove it into room 7.
    let mut a = base.clone();
    let mut s = Segmentation::read(&a).unwrap();
    s.split(7, 14, segmap::Axis::X, s.cell_at_mm(2600.0, 0.0).0).unwrap();
    roomreg::add_room(&mut a, &s, 14, 7, &Default::default()).unwrap();
    s.remove(14, 7).unwrap();
    s.write(&mut a).unwrap();
    let r = roomreg::remove_room(&mut a, &s, 14, 7).unwrap();
    assert_eq!(r.order, Some(12));
    // The grid images are re-encoded, so compare their pixels; every other member byte for byte.
    let pixels = |a: &Archive, n: &str| image::load_from_memory(a.get(n).unwrap()).unwrap().to_luma8().into_raw();
    for n in base.files() {
        if n == segmap::GRID || segmap::PLAIN.contains(&n) {
            assert!(pixels(&a, n) == pixels(&base, n), "{n} pixels differ after add and remove");
        } else {
            assert!(a.get(n) == base.get(n), "{n} differs after add and remove");
        }
    }

    // Remove room 12 into room 10: later cleaning-order places move up.
    let mut a = base.clone();
    let mut s = Segmentation::read(&a).unwrap();
    s.remove(12, 10).unwrap();
    s.write(&mut a).unwrap();
    let r = roomreg::remove_room(&mut a, &s, 12, 10).unwrap();
    assert_eq!(r.order, Some(11));
    assert!(!roomreg::seg_info_rooms(&a).unwrap().contains(&12));
    let dat = text(&a, roomreg::DAT2);
    assert!(dat.contains(r#""cleanarea_order":[{"1":2},{"2":1},{"3":4},{"4":3},{"5":6},{"6":5},{"7":7},{"8":8},{"10":9},{"11":10}]"#));
    assert!(!text(&a, roomreg::AI_ROOMS).lines().any(|l| l.starts_with("12 ")));
    let t: serde_json::Value = serde_json::from_str(&text(&a, roomreg::NEIGHBOURS)).unwrap();
    assert!(!t.to_string().contains("\"room_id\":12"));
    assert!(!t["RoomIdAndColorLookUpTable"]["RoomIds"].as_array().unwrap().contains(&serde_json::json!(12)));
    for row in t["RoomNeighborTable"].as_array().unwrap() {
        assert!(!row["room_neighbor_id"].as_array().unwrap().contains(&serde_json::json!(12)));
    }
}

/// A new room with a name and a floor: `seg_info` and the AI lists carry them, and removing the
/// room again gives back the original archive.
#[test]
fn new_room_with_name_and_floor() {
    use dreame_map::roomreg::{self, Floor, RoomMeta};
    let Some(backup) = read("recovery/1794937671-backup.tbz2") else { return };
    let base = Archive::decode(&backup).unwrap();
    let text = |a: &Archive, n: &str| String::from_utf8(a.get(n).unwrap().to_vec()).unwrap();
    let floors = text(&base, roomreg::AI_FLOORS);
    assert_eq!(roomreg::pretty4(&serde_json::from_str(&floors).unwrap()), floors, "the AI floor list round-trips");

    let mut a = base.clone();
    let mut s = Segmentation::read(&a).unwrap();
    s.split(7, 14, segmap::Axis::X, s.cell_at_mm(2600.0, 0.0).0).unwrap();
    let meta = RoomMeta { name: Some("Test".into()), floor: Some(Floor::Wood { direction: 90 }) };
    roomreg::add_room(&mut a, &s, 14, 7, &meta).unwrap();
    let dat: serde_json::Value = serde_json::from_str(&text(&a, roomreg::DAT2)).unwrap();
    let seg: serde_json::Value = serde_json::from_str(dat["seg_info"].as_str().unwrap()).unwrap();
    let e = &seg["14"];
    assert_eq!((e["type"].as_i64(), e["material"].as_i64(), e["direction"].as_i64()), (Some(0), Some(1), Some(90)));
    assert_eq!(e["name"].as_str(), Some("VGVzdA=="));
    assert!(text(&a, roomreg::AI_ROOMS).ends_with("14 47 1 VGVzdA==\n"));
    let f: serde_json::Value = serde_json::from_str(&text(&a, roomreg::AI_FLOORS)).unwrap();
    let last = f["ai_material"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last, serde_json::json!({"id": 14, "material": "wood", "label": 1, "direction": 90}));

    s.remove(14, 7).unwrap();
    s.write(&mut a).unwrap();
    roomreg::remove_room(&mut a, &s, 14, 7).unwrap();
    for n in [roomreg::DAT2, roomreg::NEIGHBOURS, roomreg::AI_ROOMS, roomreg::AI_FLOORS] {
        assert!(a.get(n) == base.get(n), "{n} differs after add and remove");
    }
}
