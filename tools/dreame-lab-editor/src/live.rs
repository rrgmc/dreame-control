//! Follows the robot over MQTT while it cleans: merges the pushed map frames (6.1) and sends the
//! growing path and the robot's pose to the UI.
//!
//! It runs on its own thread with its own runtime, sharing the cloud client (and so the token
//! refresh) with the worker. It reads [`Robot::events`], so it only listens; it never sends
//! anything to the robot.

use crate::worker::Reply;
pub use dreame::live::{RobotPose, Tracker};
use dreame::{CloudRobot, Event, Robot};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

/// Starts following. Messages go to `tx` as [`Reply::Live`] and [`Reply::LiveStatus`]; the
/// thread ends when `stop` is set or the connection ends.
pub fn spawn(robot: CloudRobot, tx: Sender<Reply>, ctx: eframe::egui::Context, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let send = |r: Reply| {
            let ok = tx.send(r).is_ok();
            ctx.request_repaint();
            ok
        };
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                send(Reply::LiveStatus(format!("live view failed to start: {e}")));
                return;
            }
        };
        rt.block_on(async {
            let robot = Robot::new(robot);
            let mut events = match robot.events() {
                Ok(e) => e,
                Err(e) => {
                    send(Reply::LiveStatus(format!("live view: {e:#}")));
                    return;
                }
            };
            send(Reply::LiveStatus("live view: connecting".into()));
            let mut tracker = Tracker::new(dreame::mapedit::MAP_VERSION);
            // Warns when the robot barely moves while it should, before it reports being stuck.
            let mut watch = dreame::watch::RunWatch::default();
            let start = std::time::Instant::now();
            let mut last_line = String::new();
            while !stop.load(Ordering::Relaxed) {
                let event = tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(500)) => continue,
                    e = events.next() => e,
                };
                let Some(event) = event else { break };
                let ok = match event {
                    Event::Connection(state) => send(Reply::LiveStatus(format!("live view: {state:?}"))),
                    Event::MapFrame(f) => {
                        // Only this frame's own position: a frame without one must not count as
                        // standing still.
                        let pose = Some(f.header.robot).filter(|p| *p != dreame_map::header::Pose::ABSENT);
                        tracker.push(*f);
                        let t = start.elapsed().as_secs_f64();
                        if let Some(w) = pose.and_then(|p| watch.update_pose(t, f64::from(p.x), f64::from(p.y))) {
                            send(Reply::LiveStatus(w));
                        }
                        send(Reply::Live {
                            path: tracker.path().clone(),
                            pose: tracker.pose,
                            frames: tracker.frames,
                            active_rooms: tracker.active_rooms.clone(),
                        })
                    }
                    Event::Status { status, .. } => {
                        watch.update(&status);
                        // One log line whenever state, task or battery changes.
                        let line = status_line(&status);
                        if line != last_line {
                            last_line = line.clone();
                            send(Reply::LiveStatus(line))
                        } else {
                            true
                        }
                    }
                    Event::RobotEvent { .. } => true,
                };
                if !ok {
                    break;
                }
            }
            events.stop().await;
            send(Reply::LiveStatus("live view stopped".into()));
        });
    });
}

/// A short line about what the robot is doing.
pub fn status_line(s: &dreame_model::Status) -> String {
    let show = |v: Option<String>| v.unwrap_or_else(|| "?".into());
    let error = match s.error.filter(|e| e.code() != 0) {
        // The error code says why a run stopped, such as "robot stuck on threshold".
        Some(e) => format!(", error {} ({e})", e.code()),
        None => String::new(),
    };
    format!(
        "robot: {}, task {}, battery {}%{error}",
        show(s.state.map(|x| x.to_string())),
        show(s.task_status.map(|x| x.to_string())),
        show(s.battery.map(|b| b.to_string()))
    )
}
