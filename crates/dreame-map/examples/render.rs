//! Decodes a map envelope file and writes a PNG.
//!
//! ```text
//! cargo run -p dreame-map --example render -- <input> <output.png> [--version N] [--saved] [--rism] [--scale N] [--key K]
//! ```
//!
//! `<input>` holds an envelope (`"<b64>"` or `"<b64>,<key>"`) or a 6.8 saved-map list (JSON with
//! `mapstr`; its first map is used and treated as saved). `--rism` renders the saved map embedded
//! in a live frame. `--version` is the map version (default 2, as on r2579a).

use dreame_map::render::{save_png, RenderOptions};
use dreame_map::{Frame, Map};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut pos = Vec::new();
    let (mut version, mut saved, mut rism, mut scale, mut key) =
        (2u8, false, false, 4u32, None::<String>);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => version = it.next().ok_or("--version needs a value")?.parse()?,
            "--scale" => scale = it.next().ok_or("--scale needs a value")?.parse()?,
            "--key" => key = Some(it.next().ok_or("--key needs a value")?),
            "--saved" => saved = true,
            "--rism" => rism = true,
            _ => pos.push(a),
        }
    }
    let [input, output] = pos.as_slice() else {
        return Err("usage: render <input> <output.png> [--version N] [--saved] [--rism] [--scale N] [--key K]".into());
    };
    let text = std::fs::read_to_string(input)?;
    let text = text.trim();
    let envelope = if text.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(text)?;
        saved = true;
        v["mapstr"][0]["map"]
            .as_str()
            .ok_or("no mapstr[0].map in the list file")?
            .to_string()
    } else {
        text.to_string()
    };
    let mut frame = Frame::decode(&envelope, key.as_deref(), None)?;
    if rism {
        frame = frame.trailer.decode_rism().ok_or("frame has no rism")??;
        saved = true;
    }
    let map = if saved {
        Map::from_frame(&frame, version, true)?
    } else {
        Map::from_live(&frame, version)?
    };
    let h = &frame.header;
    println!(
        "map_id {} frame_id {} type {:?}: {} x {} cells of {} mm, encoding {:?}, {} rooms, {} path points",
        h.map_id,
        h.frame_id,
        h.frame_type,
        h.width,
        h.height,
        h.grid_size,
        map.grid.encoding,
        map.rooms.len(),
        map.path.len()
    );
    let opts = RenderOptions {
        scale,
        floor_plan: map.floor_plan.is_some(),
        ..Default::default()
    };
    save_png(&map, &opts, std::path::Path::new(output))?;
    println!("wrote {output}");
    Ok(())
}
