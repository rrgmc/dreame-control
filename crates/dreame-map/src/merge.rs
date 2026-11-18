//! Frame sequencing and P-frame merge (maps.md §2.7 and §6.1).
//!
//! [`MapState`] keeps the current frame of one map session. I-frames replace it. P-frames must
//! arrive in order (`frame_id == current + 1`); later ones wait in a queue. A P-frame's pixel
//! rectangle is merged into the current buffer: with map version 3 (encoding A) a non-zero byte
//! replaces the stored byte, otherwise it is added to it modulo 256. Trailer keys of the P-frame
//! replace those of the current frame, except `tr`, which is appended.

use crate::frame::Frame;
use crate::header::FrameType;
use crate::path::Path;
use crate::trailer::Trailer;
use crate::{decode_err, Result};
use std::collections::BTreeMap;

/// Queue length that triggers a request for the missing P-frame.
pub const QUEUE_REQUEST_P: usize = 4;
/// Queue length that triggers a request for a new I-frame.
pub const QUEUE_REQUEST_I: usize = 8;

/// How a P-frame byte combines with the stored byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeRule {
    /// `stored = stored + p` (mod 256). Map versions below 3.
    Add,
    /// `stored = p`. Map version 3.
    Replace,
}

impl MergeRule {
    pub fn for_version(map_version: u8) -> Self {
        if map_version >= 3 {
            Self::Replace
        } else {
            Self::Add
        }
    }
}

/// What [`MapState::push`] did with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    /// The frame (and `drained` queued P-frames after it) were applied.
    Applied { drained: usize },
    /// The frame waits for an earlier frame or an I-frame.
    Queued,
    /// The frame was older than the current frame and was dropped.
    Stale,
    /// A W-frame was stored separately.
    Wifi,
}

/// A request the client should send to the robot (action 6.1, maps.md §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRequest {
    /// `{"frame_type":"I"}`
    IFrame,
    /// `{"map_id":M,"frame_id":F,"frame_type":"P"}`
    PFrame { map_id: i16, frame_id: i16 },
}

impl FrameRequest {
    /// The piid-2 JSON for action 6.1.
    pub fn to_json(&self) -> String {
        match self {
            Self::IFrame => r#"{"frame_type":"I"}"#.to_string(),
            Self::PFrame { map_id, frame_id } => {
                format!(r#"{{"map_id":{map_id},"frame_id":{frame_id},"frame_type":"P"}}"#)
            }
        }
    }
}

/// Live map state of one robot.
#[derive(Debug, Clone)]
pub struct MapState {
    pub map_version: u8,
    current: Option<Frame>,
    path: Path,
    wifi: Option<Frame>,
    /// Waiting P-frames per map id, by frame id.
    queue: BTreeMap<i16, BTreeMap<i16, Frame>>,
}

impl MapState {
    pub fn new(map_version: u8) -> Self {
        Self {
            map_version,
            current: None,
            path: Path::default(),
            wifi: None,
            queue: BTreeMap::new(),
        }
    }

    /// Starts a session from an empty base, for runs where the robot sends only P-frames and no
    /// I-frame is at hand. The base has `frame_id` 0, no pixels and the given grid size.
    pub fn start_empty(&mut self, map_id: i16, grid_size: i16) {
        let mut f = Frame {
            header: Default::default(),
            pixels: Vec::new(),
            trailer: Trailer::default(),
            aes: None,
        };
        f.header.map_id = map_id;
        f.header.grid_size = grid_size;
        self.current = Some(f);
        self.path = Path::default();
        self.drain();
    }

    /// The merged current frame (an I-frame with the P-frames applied).
    pub fn current(&self) -> Option<&Frame> {
        self.current.as_ref()
    }

    /// The path so far, with every P-frame path appended.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The last Wi-Fi map.
    pub fn wifi(&self) -> Option<&Frame> {
        self.wifi.as_ref()
    }

    /// Number of queued frames for all map ids.
    pub fn queued(&self) -> usize {
        self.queue.values().map(BTreeMap::len).sum()
    }

    /// What to ask the robot for, if anything (Tasshack's thresholds, maps.md §2.7).
    pub fn pending_request(&self) -> Option<FrameRequest> {
        let Some(cur) = &self.current else {
            return (self.queued() > 0).then_some(FrameRequest::IFrame);
        };
        let q = self.queue.get(&cur.header.map_id).map_or(0, BTreeMap::len);
        let other = self.queued() - q;
        if q > QUEUE_REQUEST_I || other > 0 {
            Some(FrameRequest::IFrame)
        } else if q > QUEUE_REQUEST_P {
            Some(FrameRequest::PFrame {
                map_id: cur.header.map_id,
                frame_id: cur.header.frame_id.wrapping_add(1),
            })
        } else {
            None
        }
    }

    /// Accepts the next frame.
    pub fn push(&mut self, frame: Frame) -> Result<PushOutcome> {
        match frame.header.frame_type {
            FrameType::W => {
                self.wifi = Some(frame);
                Ok(PushOutcome::Wifi)
            }
            FrameType::I => {
                self.path = match frame.trailer.tr.as_deref() {
                    Some(tr) => Path::parse(tr)?,
                    None => Path::default(),
                };
                let map_id = frame.header.map_id;
                let frame_id = frame.header.frame_id;
                self.current = Some(frame);
                // Drop frames of other sessions and frames the I-frame already covers.
                self.queue.retain(|&m, _| m == map_id);
                if let Some(q) = self.queue.get_mut(&map_id) {
                    q.retain(|&f, _| f > frame_id);
                }
                Ok(PushOutcome::Applied {
                    drained: self.drain(),
                })
            }
            FrameType::P => {
                let (map_id, frame_id) = (frame.header.map_id, frame.header.frame_id);
                match &self.current {
                    Some(cur) if cur.header.map_id == map_id => {
                        let cur_id = cur.header.frame_id;
                        if frame_id <= cur_id {
                            return Ok(PushOutcome::Stale);
                        }
                        if frame_id == cur_id.wrapping_add(1) {
                            self.apply_p(&frame)?;
                            return Ok(PushOutcome::Applied {
                                drained: self.drain(),
                            });
                        }
                    }
                    _ => {}
                }
                self.queue
                    .entry(map_id)
                    .or_default()
                    .insert(frame_id, frame);
                Ok(PushOutcome::Queued)
            }
            FrameType::Other(b) => Err(decode_err(format!("unknown frame type byte {b}"))),
        }
    }

    fn drain(&mut self) -> usize {
        let mut n = 0;
        loop {
            let Some(cur) = &self.current else { return n };
            let (map_id, next) = (cur.header.map_id, cur.header.frame_id.wrapping_add(1));
            let Some(f) = self.queue.get_mut(&map_id).and_then(|q| q.remove(&next)) else {
                return n;
            };
            if self.apply_p(&f).is_err() {
                return n;
            }
            n += 1;
        }
    }

    fn apply_p(&mut self, p: &Frame) -> Result<()> {
        let rule = MergeRule::for_version(self.map_version);
        let cur = self
            .current
            .as_mut()
            .ok_or_else(|| decode_err("no base frame"))?;
        merge_p_frame(cur, p, rule)?;
        if let Some(tr) = p.trailer.tr.as_deref() {
            self.path.append_tr(tr)?;
        }
        Ok(())
    }
}

/// Merges P-frame `p` into `base` in place (maps.md §6.1).
///
/// The pixel buffer grows to the union of both rectangles. Header pose and frame id come from
/// `p`; the frame type stays as it was. Trailer keys of `p` overwrite those of `base`, except `tr`,
/// which is appended, and `origin`, which is set to the merged grid's corner when present.
pub fn merge_p_frame(base: &mut Frame, p: &Frame, rule: MergeRule) -> Result<()> {
    let pw = p.header.width.max(0) as i32;
    let ph = p.header.height.max(0) as i32;
    if pw > 0 && ph > 0 {
        let g = p.header.grid_size.max(1) as i32;
        if base.header.pixel_count() > 0 && base.header.grid_size as i32 != g {
            return Err(decode_err(format!(
                "grid size changed from {} to {g}",
                base.header.grid_size
            )));
        }
        let (bw, bh) = (
            base.header.width.max(0) as i32,
            base.header.height.max(0) as i32,
        );
        let (bl, bt) = (base.left(), base.top());
        let (pl, pt) = (p.left(), p.top());
        let (left, top, right, bottom) = if bw * bh > 0 {
            (
                bl.min(pl),
                bt.min(pt),
                (bl + bw * g).max(pl + pw * g),
                (bt + bh * g).max(pt + ph * g),
            )
        } else {
            (pl, pt, pl + pw * g, pt + ph * g)
        };
        let (w, h) = ((right - left) / g, (bottom - top) / g);
        if w > i16::MAX as i32 || h > i16::MAX as i32 {
            return Err(decode_err("merged grid too large"));
        }
        let mut buf = vec![0u8; (w * h) as usize];
        if bw * bh > 0 {
            let (ox, oy) = ((bl - left) / g, (bt - top) / g);
            for y in 0..bh {
                let src = (y * bw) as usize;
                let dst = ((oy + y) * w + ox) as usize;
                buf[dst..dst + bw as usize].copy_from_slice(&base.pixels[src..src + bw as usize]);
            }
        }
        let (ox, oy) = ((pl - left) / g, (pt - top) / g);
        for y in 0..ph {
            for x in 0..pw {
                let v = p.pixels[(y * pw + x) as usize];
                if v != 0 {
                    let i = ((oy + y) * w + ox + x) as usize;
                    buf[i] = match rule {
                        MergeRule::Add => buf[i].wrapping_add(v),
                        MergeRule::Replace => v,
                    };
                }
            }
        }
        base.pixels = buf;
        base.header.width = w as i16;
        base.header.height = h as i16;
        base.header.left = left as i16;
        base.header.top = top as i16;
        base.header.grid_size = g as i16;
    }
    base.header.frame_id = p.header.frame_id;
    base.header.robot = p.header.robot;
    base.header.charger = p.header.charger;

    // Trailer: overlay keys, append the path.
    let mut cur = match base.trailer.to_value() {
        serde_json::Value::Object(m) => m,
        _ => Default::default(),
    };
    let old_tr = base.trailer.tr.clone();
    if let serde_json::Value::Object(m) = p.trailer.to_value() {
        for (k, v) in m {
            cur.insert(k, v);
        }
    }
    if let Some(ptr) = &p.trailer.tr {
        let joined = format!("{}{}", old_tr.unwrap_or_default(), ptr);
        cur.insert("tr".into(), joined.into());
    }
    if cur.contains_key("origin") {
        cur.insert(
            "origin".into(),
            serde_json::json!([base.header.left, base.header.top]),
        );
    }
    base.trailer = Trailer::from_value(serde_json::Value::Object(cur))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{Header, Pose};

    fn frame(
        ft: FrameType,
        map_id: i16,
        frame_id: i16,
        rect: (i16, i16, i16, i16),
        px: Vec<u8>,
        tr: Option<&str>,
    ) -> Frame {
        let (left, top, width, height) = rect;
        let header = Header {
            map_id,
            frame_id,
            frame_type: ft,
            robot: Pose::new(frame_id, 0, 0),
            charger: Pose::new(0, 0, 0),
            grid_size: 50,
            width,
            height,
            left,
            top,
        };
        let mut t = serde_json::json!({"fsm": 1});
        if let Some(tr) = tr {
            t["tr"] = tr.into();
        }
        Frame {
            header,
            pixels: px,
            trailer: Trailer::from_value(t).unwrap(),
            aes: None,
        }
    }

    #[test]
    fn add_rule_and_union() {
        let mut st = MapState::new(2);
        // 2 x 2 at (0, 0).
        let i = frame(
            FrameType::I,
            5,
            10,
            (0, 0, 2, 2),
            vec![4, 4, 0, 252],
            Some("S0,0L10,0"),
        );
        assert_eq!(st.push(i).unwrap(), PushOutcome::Applied { drained: 0 });
        // 2 x 1 at x = 50 (column 1) .. 150: overlaps column 1 and extends to column 2.
        let p = frame(
            FrameType::P,
            5,
            11,
            (50, 0, 2, 1),
            vec![4, 24],
            Some("l100,0L5,5"),
        );
        assert_eq!(st.push(p).unwrap(), PushOutcome::Applied { drained: 0 });
        let cur = st.current().unwrap();
        assert_eq!(
            (
                cur.header.width,
                cur.header.height,
                cur.header.left,
                cur.header.top
            ),
            (3, 2, 0, 0)
        );
        assert_eq!(cur.header.frame_id, 11);
        assert_eq!(cur.header.frame_type, FrameType::I);
        assert_eq!(cur.header.robot.x, 11);
        // Column 1 row 0: 4 + 4 = 8 (segment 2). Column 2 row 0: 0 + 24.
        assert_eq!(cur.pixels, vec![4, 8, 24, 0, 252, 0]);
        assert_eq!(cur.trailer.tr.as_deref(), Some("S0,0L10,0l100,0L5,5"));
        assert_eq!(st.path().len(), 4);
        assert!(st.path().points[2].connected);
    }

    #[test]
    fn add_wraps_and_replace_rule() {
        let mut st = MapState::new(2);
        st.push(frame(FrameType::I, 1, 1, (0, 0, 1, 1), vec![240], None))
            .unwrap();
        st.push(frame(FrameType::P, 1, 2, (0, 0, 1, 1), vec![20], None))
            .unwrap();
        assert_eq!(st.current().unwrap().pixels, vec![4]);

        let mut st = MapState::new(3);
        st.push(frame(FrameType::I, 1, 1, (0, 0, 1, 1), vec![240], None))
            .unwrap();
        st.push(frame(FrameType::P, 1, 2, (0, 0, 1, 1), vec![20], None))
            .unwrap();
        assert_eq!(st.current().unwrap().pixels, vec![20]);
    }

    #[test]
    fn out_of_order_queue_and_requests() {
        let mut st = MapState::new(2);
        // P-frames before any I-frame are queued.
        assert_eq!(
            st.push(frame(
                FrameType::P,
                5,
                2,
                (0, 0, 0, 0),
                vec![],
                Some("l1,1")
            ))
            .unwrap(),
            PushOutcome::Queued
        );
        assert_eq!(st.pending_request(), Some(FrameRequest::IFrame));
        st.push(frame(
            FrameType::I,
            5,
            1,
            (0, 0, 1, 1),
            vec![4],
            Some("S0,0"),
        ))
        .unwrap();
        assert_eq!(st.current().unwrap().header.frame_id, 2);
        assert_eq!(st.queued(), 0);
        // Gap: 4..9 arrive before 3.
        for id in 4..=9 {
            assert_eq!(
                st.push(frame(FrameType::P, 5, id, (0, 0, 0, 0), vec![], None))
                    .unwrap(),
                PushOutcome::Queued
            );
        }
        assert_eq!(
            st.pending_request(),
            Some(FrameRequest::PFrame {
                map_id: 5,
                frame_id: 3
            })
        );
        assert_eq!(
            FrameRequest::PFrame {
                map_id: 5,
                frame_id: 3
            }
            .to_json(),
            r#"{"map_id":5,"frame_id":3,"frame_type":"P"}"#
        );
        assert_eq!(
            st.push(frame(FrameType::P, 5, 3, (0, 0, 0, 0), vec![], None))
                .unwrap(),
            PushOutcome::Applied { drained: 6 }
        );
        assert_eq!(st.current().unwrap().header.frame_id, 9);
        assert_eq!(st.pending_request(), None);
        // Stale.
        assert_eq!(
            st.push(frame(FrameType::P, 5, 9, (0, 0, 0, 0), vec![], None))
                .unwrap(),
            PushOutcome::Stale
        );
        // A new map id resets on its I-frame.
        st.push(frame(FrameType::P, 6, 2, (0, 0, 0, 0), vec![], None))
            .unwrap();
        assert_eq!(st.pending_request(), Some(FrameRequest::IFrame));
        st.push(frame(FrameType::I, 6, 1, (0, 0, 1, 1), vec![8], None))
            .unwrap();
        assert_eq!(st.current().unwrap().header.map_id, 6);
        assert_eq!(st.current().unwrap().header.frame_id, 2);
    }

    #[test]
    fn start_empty_accepts_p_frames() {
        let mut st = MapState::new(2);
        st.start_empty(5, 50);
        st.push(frame(
            FrameType::P,
            5,
            1,
            (100, 200, 2, 1),
            vec![4, 8],
            Some("l1,1"),
        ))
        .unwrap();
        st.push(frame(
            FrameType::P,
            5,
            2,
            (0, 0, 0, 0),
            vec![],
            Some("L1,1"),
        ))
        .unwrap();
        let cur = st.current().unwrap();
        assert_eq!(
            (
                cur.header.left,
                cur.header.top,
                cur.header.width,
                cur.header.height
            ),
            (100, 200, 2, 1)
        );
        assert_eq!(cur.pixels, vec![4, 8]);
        assert_eq!(cur.header.frame_id, 2);
        assert_eq!(st.path().len(), 2);
    }
}
