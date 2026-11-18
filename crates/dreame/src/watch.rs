//! Waits for a cleaning run to end and reports how it ended, with every error and fault code
//! seen on the way.
//!
//! A stuck robot shows error state 4 and an error code (2.2) such as 80 "robot stuck" or 93
//! "robot stuck on threshold" for a short time, then pauses with code 0 (W33). A poll every
//! half minute misses that, so [`wait_run_end`] follows the MQTT pushes, which carry each change,
//! and polls as a fallback. [`RunWatch`] holds the logic and is fed statuses, so it is tested
//! without a robot.
//!
//! A robot can also be stuck long before it says so: it keeps trying to free itself while its
//! state stays "sweeping" (it was lifted free before it gave up, 2026-11-17). So [`RunWatch`]
//! also takes the robot's position from the pushed map frames and warns when it has barely moved
//! for a while ([`StallCheck`]) in a state where it should be moving.

use crate::robot::{Event, Events, Robot};
use crate::CloudRobot;
use anyhow::Result;
use dreame_model::enums::{ErrorCode, Open, State};
use dreame_model::Status;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// When [`RunWatch::update_pose`] calls the robot stalled: in a moving state, all its positions
/// over the last `window` lie within `min_move_mm` of each other (bounding-box diagonal).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StallCheck {
    pub window: Duration,
    pub min_move_mm: f64,
}

impl Default for StallCheck {
    fn default() -> Self {
        Self { window: Duration::from_secs(60), min_move_mm: 300.0 }
    }
}

/// States in which the robot should keep moving. Washing, drying, emptying and spot cleaning
/// keep it in one place on purpose.
fn moving(state: Open<State>) -> bool {
    matches!(
        state,
        Open::Known(
            State::Sweeping
                | State::Mopping
                | State::SweepingAndMopping
                | State::Returning
                | State::ReturningToWash
                | State::ReturningAutoEmpty
                | State::ReturningToDrain
                | State::HeadingToExtraCleaning
        )
    )
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// Back on the dock (charging, or charged).
    Docked,
    /// Paused off the dock: by the user, or after an error such as a stuck robot.
    Paused,
    /// In the error state.
    Error,
    /// Idle off the dock.
    Stopped,
    /// Still running when the wait ended.
    TimedOut,
}

impl std::fmt::Display for RunEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Docked => "docked",
            Self::Paused => "paused",
            Self::Error => "error",
            Self::Stopped => "stopped off the dock",
            Self::TimedOut => "timed out, still running",
        })
    }
}

/// Follows a run from statuses. Feed every status to [`RunWatch::update`].
#[derive(Debug, Clone, Default)]
pub struct RunWatch {
    /// True once a working state (not docked, paused, error or idle) was seen.
    pub ran: bool,
    /// Nonzero error codes (2.2) and active fault codes (4.18) seen off the dock, in first-seen
    /// order. On r2579a, 68 follows a normal finished run, so it is ignored on the dock.
    pub codes: Vec<Open<ErrorCode>>,
    /// One line per change of state, task or error, and per stall warning.
    pub log: Vec<String>,
    /// Where the robot stalled ([`StallCheck`]), in world mm, once per stall.
    pub stalls: Vec<(f64, f64)>,
    pub stall_check: StallCheck,
    last: String,
    /// The robot's positions in the current moving state: (seconds, x, y).
    poses: VecDeque<(f64, f64, f64)>,
    in_motion: bool,
    stalled: bool,
}

impl RunWatch {
    /// Takes one status. Returns how the run ended once it has: docked after it ran, paused,
    /// in error, or idle off the dock after it ran.
    pub fn update(&mut self, s: &Status) -> Option<RunEnd> {
        let show = |v: Option<String>| v.unwrap_or_else(|| "?".into());
        let line = format!(
            "state {}, task {}, error {}",
            show(s.state.map(|x| x.to_string())),
            show(s.task_status.map(|x| x.to_string())),
            show(s.error.map(|x| x.to_string()))
        );
        if line != self.last {
            self.last = line.clone();
            self.log.push(line);
        }
        let state = s.state?;
        if !moving(state) {
            self.poses.clear();
            self.stalled = false;
        }
        self.in_motion = moving(state);
        let docked = s.charging.is_some_and(|c| c.is(dreame_model::enums::ChargingStatus::Charging))
            || state.is(State::Charging)
            || state.is(State::ChargingCompleted);
        // Codes count off the dock only: on the dock, 68 lingers from the run before.
        if !docked {
            let mut found: Vec<Open<ErrorCode>> = s.active_faults();
            if let Some(e) = s.error.filter(|e| e.code() != 0) {
                found.insert(0, e);
            }
            for c in found {
                if !self.codes.contains(&c) {
                    self.codes.push(c);
                }
            }
        }
        match state {
            Open::Known(State::Error) => Some(RunEnd::Error),
            Open::Known(State::Paused) => Some(RunEnd::Paused),
            _ if docked => self.ran.then_some(RunEnd::Docked),
            Open::Known(State::Idle) => self.ran.then_some(RunEnd::Stopped),
            _ => {
                self.ran = true;
                None
            }
        }
    }

    /// Takes one robot position, `t` seconds into the watch. Returns a warning line (also added to
    /// [`RunWatch::log`]) when the robot starts to stall, and a line when it moves again.
    pub fn update_pose(&mut self, t: f64, x: f64, y: f64) -> Option<String> {
        if !self.in_motion {
            return None;
        }
        self.poses.push_back((t, x, y));
        let w = self.stall_check.window.as_secs_f64();
        // Keep one position at or before the window's start, so the window is known to be full.
        while self.poses.len() > 2 && self.poses[1].0 <= t - w {
            self.poses.pop_front();
        }
        let full = self.poses.front().is_some_and(|p| p.0 <= t - w);
        let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
        for &(_, px, py) in &self.poses {
            (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
        }
        let spread = (x1 - x0).hypot(y1 - y0);
        let line = if full && spread < self.stall_check.min_move_mm && !self.stalled {
            self.stalled = true;
            self.stalls.push((x, y));
            format!(
                "warning: the robot moved {spread:.0} mm in {w:.0} s at ({x:.0}, {y:.0}); it may be stuck without saying so"
            )
        } else if self.stalled && spread >= self.stall_check.min_move_mm {
            self.stalled = false;
            "the robot moves again".to_string()
        } else {
            return None;
        };
        self.log.push(line.clone());
        Some(line)
    }
}

/// Settings for [`wait_run_end`].
#[derive(Debug, Clone, Copy)]
pub struct WaitRun {
    /// Give up after this long.
    pub timeout: Duration,
    /// Poll this often when no push arrives.
    pub poll: Duration,
    pub stall: StallCheck,
}

impl Default for WaitRun {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(90 * 60), poll: Duration::from_secs(20), stall: StallCheck::default() }
    }
}

/// What [`wait_run_end`] saw.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub end: RunEnd,
    pub watch: RunWatch,
    /// The last status read or pushed.
    pub status: Status,
}

/// What the stream gave [`wait_run_end`].
enum Pushed {
    Status(Box<Status>),
    Pose(f64, f64),
    /// No push within the poll interval.
    Quiet,
    /// The stream closed.
    Ended,
}

/// The next pushed status or robot position.
async fn next_push(events: &mut Events) -> Pushed {
    loop {
        match events.next().await {
            None => return Pushed::Ended,
            Some(Event::Status { status, .. }) => return Pushed::Status(status),
            Some(Event::MapFrame(f)) if f.header.robot != dreame_map::header::Pose::ABSENT => {
                return Pushed::Pose(f64::from(f.header.robot.x), f64::from(f.header.robot.y));
            }
            Some(_) => continue,
        }
    }
}

/// Waits until the current run ends; read-only. `progress` gets each new line of
/// [`RunWatch::log`], stall warnings included; a stall does not end the wait, since the robot
/// may still free itself. Without MQTT it only polls, can miss a short error state, and sees no
/// positions.
pub async fn wait_run_end(robot: &Robot<CloudRobot>, opts: WaitRun, progress: &mut dyn FnMut(&str)) -> Result<RunReport> {
    let mut w = RunWatch { stall_check: opts.stall, ..RunWatch::default() };
    let start = Instant::now();
    let deadline = start + opts.timeout;
    let mut events = match robot.events() {
        Ok(e) => Some(e),
        Err(e) => {
            progress(&format!("no live updates ({e:#}); polling every {} s", opts.poll.as_secs()));
            None
        }
    };
    let mut status = robot.refresh().await?;
    let mut end = None;
    loop {
        let before = w.log.len();
        end = end.or(w.update(&status));
        for l in &w.log[before..] {
            progress(l);
        }
        if end.is_some() || Instant::now() >= deadline {
            break;
        }
        let pushed = match events.as_mut() {
            Some(ev) => tokio::select! {
                p = next_push(ev) => p,
                _ = tokio::time::sleep(opts.poll) => Pushed::Quiet,
            },
            None => {
                tokio::time::sleep(opts.poll).await;
                Pushed::Quiet
            }
        };
        status = match pushed {
            Pushed::Status(s) => *s,
            Pushed::Pose(x, y) => {
                if let Some(l) = w.update_pose(start.elapsed().as_secs_f64(), x, y) {
                    progress(&l);
                }
                continue;
            }
            Pushed::Quiet => robot.refresh().await.unwrap_or(status),
            Pushed::Ended => {
                events = None;
                robot.refresh().await.unwrap_or(status)
            }
        };
    }
    if let Some(ev) = events {
        ev.stop().await;
    }
    Ok(RunReport { end: end.unwrap_or(RunEnd::TimedOut), watch: w, status })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_model::enums::ChargingStatus;

    fn st(state: State, error: i64) -> Status {
        Status { state: Some(Open::Known(state)), error: Some(Open::from_code(error)), ..Status::default() }
    }

    #[test]
    fn a_stuck_run_keeps_its_error_code() {
        let mut w = RunWatch::default();
        // Still docked at the start: not the end of the run.
        let mut docked = st(State::Charging, 68);
        docked.charging = Some(Open::Known(ChargingStatus::Charging));
        assert_eq!(w.update(&docked), None);
        assert_eq!(w.update(&st(State::Sweeping, 0)), None);
        assert!(w.ran);
        // Stuck: the error state carries code 93, which is kept after it clears.
        assert_eq!(w.update(&st(State::Error, 93)), Some(RunEnd::Error));
        assert_eq!(w.update(&st(State::Paused, 0)), Some(RunEnd::Paused));
        assert_eq!(w.codes, [Open::Known(ErrorCode::RobotStuckOnThreshold)]);
        assert_eq!(w.log.len(), 4, "{:?}", w.log);
    }

    #[test]
    fn a_robot_that_stops_moving_is_flagged() {
        let mut w = RunWatch::default();
        // Before a moving state, positions are ignored.
        assert_eq!(w.update_pose(0.0, 0.0, 0.0), None);
        w.update(&st(State::Sweeping, 0));
        // Moving: 1 m every 10 s.
        for i in 0..10 {
            assert_eq!(w.update_pose(f64::from(i) * 10.0, f64::from(i) * 1000.0, 0.0), None);
        }
        // Then 70 s within 100 mm: flagged once, after a full minute.
        let mut warned = Vec::new();
        for i in 0..8 {
            let t = 100.0 + f64::from(i) * 10.0;
            warned.extend(w.update_pose(t, 9000.0 + f64::from(i % 2) * 100.0, 0.0));
        }
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert!(warned[0].starts_with("warning: the robot moved 100 mm in 60 s"), "{warned:?}");
        assert_eq!(w.stalls.len(), 1);
        // Moving again clears it.
        assert_eq!(w.update_pose(180.0, 12000.0, 0.0).as_deref(), Some("the robot moves again"));
        // Washing at the station is no stall.
        w.update(&st(State::Washing, 0));
        for i in 0..20 {
            assert_eq!(w.update_pose(200.0 + f64::from(i) * 10.0, 0.0, 0.0), None);
        }
    }

    #[test]
    fn a_normal_run_ends_docked() {
        let mut w = RunWatch::default();
        assert_eq!(w.update(&st(State::Sweeping, 0)), None);
        assert_eq!(w.update(&st(State::Returning, 0)), None);
        assert_eq!(w.update(&st(State::Charging, 0)), Some(RunEnd::Docked));
        assert!(w.codes.is_empty());
        // Idle before it ran is not an end either.
        let mut w = RunWatch::default();
        assert_eq!(w.update(&st(State::Idle, 0)), None);
        assert_eq!(w.update(&st(State::Sweeping, 0)), None);
        assert_eq!(w.update(&st(State::Idle, 0)), Some(RunEnd::Stopped));
    }
}
