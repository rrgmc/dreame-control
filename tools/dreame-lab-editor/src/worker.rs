//! A background thread that talks to the robot, so the UI never blocks on the network.
//!
//! The thread owns a single-threaded tokio runtime and the cloud connection. It takes one
//! [`Request`] at a time and answers with one [`Reply`], then asks the UI to repaint.

use anyhow::{anyhow, Result};
use dreame::archiveedit::{self, GridEdit};
use dreame::control;
use dreame::mapedit::{self, Edit, Report};
use dreame_model::Command;
use dreame::session::{default_out_dir, pick_device, stamp};
use dreame::{CloudRobot, Session};
use dreame_map::path::Path;
use dreame_map::{Frame, Map};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub enum Request {
    /// Download the saved map (and the live frame's room settings) from the robot.
    LoadRobot,
    /// Read a snapshot file written by `dreame map snapshot` or an edit.
    LoadFile(PathBuf),
    /// Send one edit with the full apply flow, then reload.
    Apply(Edit),
    /// Run one room grid edit through backup and restore ([`archiveedit::apply`]), then reload.
    /// Progress arrives as [`Reply::Progress`].
    ApplyGrid(GridEdit),
    /// Send one control command (start, pause, dock, clean rooms or a zone).
    Control { cmd: Command, what: String },
    /// Start following the robot over MQTT (listen only).
    StartLive,
    /// Stop following.
    StopLive,
}

/// A loaded map and where it came from.
pub struct Loaded {
    pub map: Map,
    /// Trailer keys the parser could not read; editing is refused while any exist.
    pub unparsed: Vec<String>,
    /// True when the map came from the robot, so edits can be applied.
    pub from_robot: bool,
    pub source: String,
    /// The robot's path from the live frame (the current or last job); empty for files.
    pub path: Path,
}

pub enum Reply {
    Loaded(Box<Loaded>),
    Applied { report: Report, loaded: Box<Loaded> },
    /// A grid edit finished: what changed and how to revert, one line each.
    GridApplied { lines: Vec<String>, loaded: Box<Loaded> },
    /// A step of a running grid edit.
    Progress(String),
    Controlled { what: String, result: serde_json::Value },
    /// The live path and pose so far, after `frames` merged map frames.
    Live { path: Path, pose: Option<crate::live::RobotPose>, frames: usize, active_rooms: Vec<u32> },
    /// A line about the live connection.
    LiveStatus(String),
    Failed(String),
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    /// True from a request until its reply arrives.
    pub busy: bool,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context) -> Self {
        let (tx, req_rx) = channel::<Request>();
        let (reply_tx, rx) = channel::<Reply>();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = reply_tx.send(Reply::Failed(format!("cannot start the runtime: {e}")));
                    return;
                }
            };
            let mut robot: Option<CloudRobot> = None;
            let mut live_stop: Option<Arc<AtomicBool>> = None;
            for req in req_rx {
                let req = match req {
                    Request::StartLive => {
                        if let Some(s) = live_stop.take() {
                            s.store(true, Ordering::Relaxed);
                        }
                        match rt.block_on(connect(&mut robot)) {
                            Ok(r) => {
                                let stop = Arc::new(AtomicBool::new(false));
                                crate::live::spawn(r.clone(), reply_tx.clone(), ctx.clone(), stop.clone());
                                live_stop = Some(stop);
                                continue;
                            }
                            Err(e) => {
                                let _ = reply_tx.send(Reply::Failed(format!("live view: {e:#}")));
                                ctx.request_repaint();
                                continue;
                            }
                        }
                    }
                    Request::ApplyGrid(edit) => {
                        let progress_tx = reply_tx.clone();
                        let progress_ctx = ctx.clone();
                        let mut progress = move |line: &str| {
                            let _ = progress_tx.send(Reply::Progress(line.to_string()));
                            progress_ctx.request_repaint();
                        };
                        let reply = rt
                            .block_on(apply_grid(&mut robot, &edit, &mut progress))
                            .unwrap_or_else(|e| Reply::Failed(format!("{e:#}")));
                        if reply_tx.send(reply).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                        continue;
                    }
                    Request::StopLive => {
                        if let Some(s) = live_stop.take() {
                            s.store(true, Ordering::Relaxed);
                        }
                        continue;
                    }
                    other => other,
                };
                let reply = rt.block_on(handle(&mut robot, req)).unwrap_or_else(|e| Reply::Failed(format!("{e:#}")));
                if reply_tx.send(reply).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        Self { tx, rx, busy: false }
    }

    pub fn send(&mut self, r: Request) {
        if self.tx.send(r).is_ok() {
            self.busy = true;
        }
    }

    /// Sends a request that gets no direct reply (start or stop following), so the worker is
    /// not marked busy.
    pub fn send_quiet(&mut self, r: Request) {
        let _ = self.tx.send(r);
    }

    /// The next reply, if one has arrived. Live updates arrive at any time and do not end a
    /// pending request.
    pub fn poll(&mut self) -> Option<Reply> {
        let r = self.rx.try_recv().ok();
        if matches!(
            r,
            Some(Reply::Loaded(_) | Reply::Applied { .. } | Reply::GridApplied { .. } | Reply::Controlled { .. } | Reply::Failed(_))
        ) {
            self.busy = false;
        }
        r
    }
}

async fn connect(robot: &mut Option<CloudRobot>) -> Result<&CloudRobot> {
    if robot.is_none() {
        let client = Session::new().client().await?;
        let device = pick_device(&client, None).await?;
        *robot = Some(CloudRobot::new(Arc::new(client), device));
    }
    robot.as_ref().ok_or_else(|| anyhow!("no robot"))
}

async fn load_robot(robot: &CloudRobot) -> Result<Loaded> {
    let saved = mapedit::read_saved(robot).await?;
    Ok(Loaded {
        path: live_path(&saved.live_envelope),
        map: saved.map,
        unparsed: saved.unparsed,
        from_robot: true,
        source: format!("robot ({})", robot.device.model),
    })
}

/// The path in a live frame; empty when the frame has none or does not decode.
fn live_path(envelope: &str) -> Path {
    Frame::decode(envelope, None, None)
        .ok()
        .and_then(|f| Map::from_live(&f, mapedit::MAP_VERSION).ok())
        .map(|m| m.path)
        .unwrap_or_default()
}

/// Port the edited archive is served on; the same as the CLI's, so one firewall rule covers both.
const GRID_PORT: u16 = 8770;

async fn apply_grid(robot: &mut Option<CloudRobot>, edit: &GridEdit, progress: &mut dyn FnMut(&str)) -> Result<Reply> {
    let r = connect(robot).await?;
    let opts = archiveedit::Options { dir: default_out_dir("recovery"), stamp: stamp(), port: GRID_PORT, prime: true, archive: None };
    let report = archiveedit::apply(r, edit, &opts, progress).await?;
    let mut lines = Vec::new();
    lines.push(match &report.settings {
        Ok(code) => format!("per-room settings put back: result code {code:?}"),
        Err(e) => format!("per-room settings NOT put back: {e}"),
    });
    if report.diff.is_empty() {
        lines.push("no change seen in the saved map".into());
    }
    lines.extend(report.diff.iter().map(|d| format!("  {d}")));
    lines.push(report.revert_hint());
    Ok(Reply::GridApplied { lines, loaded: Box::new(load_robot(r).await?) })
}

async fn handle(robot: &mut Option<CloudRobot>, req: Request) -> Result<Reply> {
    Ok(match req {
        Request::LoadRobot => {
            let r = connect(robot).await?;
            Reply::Loaded(Box::new(load_robot(r).await?))
        }
        Request::LoadFile(path) => {
            let text = std::fs::read_to_string(&path)?;
            let frame = Frame::decode(&text, None, None)?;
            let map = Map::from_frame(&frame, mapedit::MAP_VERSION, true)?;
            Reply::Loaded(Box::new(Loaded {
                map,
                unparsed: frame.trailer.unparsed.clone(),
                path: Path::default(),
                from_robot: false,
                source: format!("file {}", path.display()),
            }))
        }
        Request::Apply(edit) => {
            let r = connect(robot).await?;
            let report =
                mapedit::apply(r, &edit, &default_out_dir("edits"), &stamp(), mapedit::Wait::default()).await?;
            Reply::Applied { report, loaded: Box::new(load_robot(r).await?) }
        }
        Request::StartLive | Request::StopLive | Request::ApplyGrid(_) => unreachable!("handled by the worker loop"),
        Request::Control { cmd, what } => {
            let r = connect(robot).await?;
            let result = control::send(&r.client, &r.device, &cmd).await?;
            Reply::Controlled { what, result }
        }
    })
}
