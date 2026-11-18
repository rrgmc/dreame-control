//! Following a running job from the pushed map frames (6.1): the growing path, the robot's
//! pose and the rooms the job cleans. No I/O: feed it the [`Frame`]s of [`crate::Event::MapFrame`].

use dreame_map::merge::{MapState, PushOutcome};
use dreame_map::path::Path;
use dreame_map::{Frame, FrameType};

/// The robot's position and heading in world mm and degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RobotPose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

/// Feeds map frames into a [`MapState`] and reports the path and pose after each one.
pub struct Tracker {
    state: MapState,
    pub pose: Option<RobotPose>,
    pub frames: usize,
    /// The rooms the job cleans, from the last frame that listed them (`sa`).
    pub active_rooms: Vec<u32>,
}

impl Tracker {
    pub fn new(map_version: u8) -> Self {
        Self { state: MapState::new(map_version), pose: None, frames: 0, active_rooms: Vec::new() }
    }

    /// Accepts one decoded frame. A P-frame that arrives with no base for its map starts an
    /// empty one, as on r2579a, where a run's P-frames come without an I-frame.
    pub fn push(&mut self, f: Frame) {
        let pose = f.header.robot;
        if pose != dreame_map::header::Pose::ABSENT {
            self.pose = Some(RobotPose { x: pose.x as f64, y: pose.y as f64, angle: pose.angle as f64 });
        }
        if let Some(sa) = &f.trailer.sa {
            self.active_rooms = sa.iter().filter_map(|e| u32::try_from(*e.first()?).ok()).collect();
        }
        let p_frame = f.header.frame_type == FrameType::P;
        let (map_id, grid) = (f.header.map_id, f.header.grid_size);
        let same_map = self.state.current().is_some_and(|c| c.header.map_id == map_id);
        if p_frame && !same_map {
            self.state.start_empty(map_id, grid);
        }
        if let Ok(PushOutcome::Applied { .. }) = self.state.push(f) {
            self.frames += 1;
        }
    }

    pub fn path(&self) -> &Path {
        self.state.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_map::header::{Header, Pose};
    use dreame_map::Trailer;
    use serde_json::json;

    fn p_frame(map_id: i16, frame_id: i16, robot: (i16, i16), tr: &str) -> Frame {
        let header = Header {
            map_id,
            frame_id,
            frame_type: FrameType::P,
            robot: Pose::new(robot.0, robot.1, 90),
            charger: Pose::ABSENT,
            grid_size: 50,
            width: 0,
            height: 0,
            left: 0,
            top: 0,
        };
        let trailer = Trailer::from_value(json!({"tr": tr})).unwrap();
        Frame { header, pixels: vec![], trailer, aes: None }
    }

    #[test]
    fn p_frames_without_a_base_build_a_path_and_pose() {
        let mut t = Tracker::new(2);
        t.push(p_frame(5, 1, (100, 200), "S100,200L50,0"));
        t.push(p_frame(5, 2, (150, 250), "L150,250"));
        assert_eq!(t.frames, 2);
        assert_eq!(t.pose, Some(RobotPose { x: 150.0, y: 250.0, angle: 90.0 }));
        let pts: Vec<(i64, i64)> = t.path().points.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(pts.first(), Some(&(100, 200)));
        assert_eq!(pts.last(), Some(&(150, 250)));
        // A frame for another map starts over.
        t.push(p_frame(6, 1, (0, 0), "S0,0L10,0"));
        assert_eq!(t.path().points.first().map(|p| (p.x, p.y)), Some((0, 0)));
    }
}
