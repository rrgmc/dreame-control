//! Network work off the UI thread. The app owns a tokio runtime; each action runs as a task and
//! reports with one [`Msg`], then asks the UI to repaint. The live connection is a long task that
//! sends status, path and pose as they arrive.

use crate::config::Paths;
use anyhow::{anyhow, Result};
use dreame::live::{RobotPose, Tracker};
use dreame::{CloudRobot, Event, Robot};
use dreame_cloud::region::Region;
use dreame_cloud::{CloudClient, CloudConfig};
use dreame_core::DeviceInfo;
use dreame_i18n::{tr, trf};
use dreame_map::path::Path;
use dreame_map::{Frame, Map};
use dreame_model::{Capabilities, Status};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

pub type SharedRobot = Arc<Robot<CloudRobot>>;

pub enum Msg {
    /// No saved session, or it could not be used (the reason): show the sign-in screen.
    NeedLogin(Option<String>),
    LoginFailed(String),
    /// Signed in; the account's vacuums.
    Devices { client: Arc<CloudClient>, devices: Vec<DeviceInfo> },
    /// Connected to one robot, with its first status; `discover` when what it supports is not
    /// known yet.
    Connected { robot: SharedRobot, status: Box<Status>, discover: bool },
    /// Discovery finished: the same robot, now knowing what it supports.
    Discovered(SharedRobot),
    ConnectFailed(String),
    Map(Box<LoadedMap>),
    MapFailed(String),
    Status(Box<Status>),
    Live { path: Path, pose: Option<RobotPose>, active_rooms: Vec<u32> },
    /// The live connection's state, as a short word.
    Link(String),
    /// A command finished; `Err` holds why it failed.
    Done { what: String, result: Result<(), String> },
    /// A map edit finished: the map before it, and the map read back or why it failed.
    Edited { before: Box<dreame_map::Map>, result: Result<Box<LoadedMap>, String> },
    /// A step of a running advanced edit.
    Progress(String),
    /// The robot has barely moved for a while in a state where it should move.
    Stall(String),
    /// The schedules, read live from the robot.
    Schedules(Result<Vec<dreame_model::codec::schedule::ScheduleSlot>, String>),
    /// Past jobs, newest first.
    History(Result<Vec<dreame::history::Job>, String>),
    /// The map of the job that started at `started`.
    JobMap { started: Option<i64>, result: Result<Box<dreame_map::Map>, String> },
    /// A remote-control step was sent; `Err` holds why it failed.
    Drove(Result<(), String>),
    /// The obstacles and pets in the live map.
    Obstacles(Result<Vec<dreame_map::obstacle::Obstacle>, String>),
    /// The photo of obstacle `index`, as JPEG bytes.
    Photo { index: usize, result: Result<Vec<u8>, String> },
}

/// The saved map and the path of the current or last job.
pub struct LoadedMap {
    pub map: Map,
    pub path: Path,
}

/// A queued drive step: the step, whether it is the first, and when it was queued.
type RemoteStep = (dreame_model::commands::RemoteMove, bool, std::time::Instant);

pub struct Backend {
    rt: tokio::runtime::Runtime,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: eframe::egui::Context,
    paths: Paths,
    live_stop: Option<Arc<AtomicBool>>,
    /// The drive worker's queue; see [`Backend::drive`].
    drive_tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<(SharedRobot, RemoteStep)>>>,
}

impl Backend {
    pub fn new(ctx: eframe::egui::Context, paths: Paths) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let (tx, rx) = channel();
        Ok(Self { rt, tx, rx, ctx, paths, live_stop: None, drive_tx: Default::default() })
    }

    pub fn ctx(&self) -> &eframe::egui::Context {
        &self.ctx
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn poll(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    /// Runs `fut` and sends its message.
    fn run(&self, fut: impl Future<Output = Msg> + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let _ = tx.send(fut.await);
            ctx.request_repaint();
        });
    }

    fn config(&self) -> CloudConfig {
        CloudConfig::default().with_token_store(Arc::new(dreame::session::FileTokenStore::new(&self.paths.session)))
    }

    /// Resumes the saved session, if there is one.
    pub fn resume(&self) {
        if !self.paths.session.is_file() {
            let _ = self.tx.send(Msg::NeedLogin(None));
            return;
        }
        let cfg = self.config();
        let log = self.paths.log.clone();
        self.run(async move {
            match CloudClient::from_store(cfg).await {
                Ok(c) => devices(Arc::new(c)).await.unwrap_or_else(|e| Msg::LoginFailed(format!("{e:#}"))),
                Err(e) => {
                    crate::config::log_line(&log, &format!("the saved sign-in could not be used: {e}"));
                    Msg::NeedLogin(Some(trf("Your saved sign-in could not be used, so please sign in again. ({error})", &[("error", &e)])))
                }
            }
        });
    }

    /// Signs in with the password. The password is not kept; the session token is.
    pub fn login(&self, email: String, password: String, region: Option<String>) {
        let cfg = self.config();
        self.run(async move {
            let regions: Vec<Region> = region.iter().map(|r| Region::parse(r)).collect();
            match CloudClient::login_any_with(cfg, &regions, email.trim(), &password).await {
                Ok(c) => devices(Arc::new(c)).await.unwrap_or_else(|e| Msg::LoginFailed(format!("{e:#}"))),
                Err(e) => Msg::LoginFailed(friendly_login_error(&e.to_string())),
            }
        });
    }

    /// Forgets the session and stops following the robot.
    pub fn sign_out(&mut self) {
        self.stop_live();
        let _ = std::fs::remove_file(&self.paths.session);
    }

    /// Connects to one robot. What it supports comes from the saved discovery result when that
    /// fits the robot's firmware; the status comes from the cloud cache. Both are quick, so the
    /// home screen shows at once. [`Msg::Connected`] says whether discovery still has to run.
    pub fn connect(&self, client: Arc<CloudClient>, device: DeviceInfo) {
        let file = self.paths.capabilities.clone();
        self.run(async move {
            let mut robot = Robot::cloud(client, device);
            let known = dreame::known::load(&file, &robot.cloud_link().device);
            let discover = known.is_none();
            if let Some(k) = &known {
                robot.set_capabilities(Capabilities::from_known(k));
            }
            match robot.refresh_cached().await {
                Ok(status) => Msg::Connected { robot: Arc::new(robot), status: Box::new(status), discover },
                Err(e) => Msg::ConnectFailed(format!("{e:#}")),
            }
        });
    }

    /// Learns what the robot supports by reading every property (about 25 seconds), saves the
    /// result, and sends a robot that knows it as [`Msg::Discovered`]. Read-only.
    pub fn discover(&self, robot: &SharedRobot) {
        let link = robot.cloud_link();
        let (client, device) = (link.client.clone(), link.device.clone());
        let (file, log) = (self.paths.capabilities.clone(), self.paths.log.clone());
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let mut robot = Robot::cloud(client, device.clone());
            match robot.discover(&Default::default()).await {
                // Only a complete result is kept; a partial one would hide features for good.
                Ok(caps) => {
                    dreame::known::save(&file, &device, caps);
                }
                Err(e) => {
                    crate::config::log_line(&log, &format!("discovery failed: {e:#}"));
                    return;
                }
            }
            let _ = tx.send(Msg::Discovered(Arc::new(robot)));
            ctx.request_repaint();
        });
    }

    pub fn load_map(&self, robot: &SharedRobot) {
        let robot = robot.clone();
        self.run(async move {
            match dreame::mapedit::read_saved(robot.cloud_link()).await {
                Ok(saved) => Msg::Map(Box::new(LoadedMap { path: live_path(&saved.live_envelope), map: saved.map })),
                Err(e) => Msg::MapFailed(format!("{e:#}")),
            }
        });
    }

    pub fn refresh(&self, robot: &SharedRobot) {
        let robot = robot.clone();
        self.run(async move {
            match robot.refresh().await {
                Ok(s) => Msg::Status(Box::new(s)),
                Err(e) => Msg::Link(format!("offline: {e:#}")),
            }
        });
    }

    /// Reads the schedules live from the robot.
    pub fn load_schedules(&self, robot: &SharedRobot) {
        let robot = robot.clone();
        self.run(async move { Msg::Schedules(robot.schedules().await.map_err(|e| format!("{e:#}"))) });
    }

    /// Reads the newest `limit` past jobs from the cloud.
    pub fn load_history(&self, robot: &SharedRobot, limit: u32) {
        let robot = robot.clone();
        self.run(async move {
            Msg::History(dreame::history::jobs(robot.cloud_link(), limit).await.map_err(|e| format!("{e:#}")))
        });
    }

    /// Downloads the map a job left, with its path.
    pub fn load_job_map(&self, robot: &SharedRobot, job: dreame::history::Job) {
        let robot = robot.clone();
        self.run(async move {
            let result = match dreame::history::job_map(robot.cloud_link(), &job).await {
                Ok(Some(m)) => Ok(Box::new(m)),
                Ok(None) => Err(tr("the robot kept no map of this job").to_string()),
                Err(e) => Err(format!("{e:#}")),
            };
            Msg::JobMap { started: job.started(), result }
        });
    }

    /// Queues one remote-control step. One worker sends the steps in order, so a stop never
    /// overtakes the step before it. Each step and its round trip go to the log.
    pub fn drive(&self, robot: &SharedRobot, step: dreame_model::commands::RemoteMove, first: bool) {
        let queued = std::time::Instant::now();
        let mut drive_tx = self.drive_tx.lock().expect("drive lock");
        if drive_tx.as_ref().is_none_or(|t| t.is_closed()) {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(SharedRobot, RemoteStep)>();
            let (msgs, ctx, log) = (self.tx.clone(), self.ctx.clone(), self.paths.log.clone());
            self.rt.spawn(async move {
                while let Some((robot, (step, first, queued))) = rx.recv().await {
                    let started = queued.elapsed();
                    let result = robot.remote_move(step, first).await.map(|_| ()).map_err(|e| format!("{e:#}"));
                    crate::config::log_line(
                        &log,
                        &format!(
                            "drive at ms {} forward {} turn {} first {first}: waited {} ms, took {} ms, {}",
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() % 100_000),
                            step.forward,
                            step.turn,
                            started.as_millis(),
                            (queued.elapsed() - started).as_millis(),
                            result.as_ref().map_or_else(|e| e.as_str(), |_| "ok")
                        ),
                    );
                    let _ = msgs.send(Msg::Drove(result));
                    ctx.request_repaint();
                }
            });
            *drive_tx = Some(tx);
        }
        if let Some(tx) = drive_tx.as_ref() {
            let _ = tx.send((robot.clone(), (step, first, queued)));
        }
    }

    /// Lists the obstacles in the live map, then fetches each photo. Read-only.
    pub fn load_obstacles(&self, robot: &SharedRobot) {
        let (tx, ctx, robot) = (self.tx.clone(), self.ctx.clone(), robot.clone());
        self.rt.spawn(async move {
            let send = |m: Msg| {
                let _ = tx.send(m);
                ctx.request_repaint();
            };
            let list = match robot.obstacles().await {
                Ok(l) => l,
                Err(e) => return send(Msg::Obstacles(Err(format!("{e:#}")))),
            };
            send(Msg::Obstacles(Ok(list.clone())));
            for (index, o) in list.iter().enumerate() {
                if let Some(p) = o.photo.as_ref().filter(|p| !p.upload_failed()) {
                    let result = robot.obstacle_photo(p).await.map_err(|e| format!("{e:#}"));
                    send(Msg::Photo { index, result });
                }
            }
        });
    }

    /// Reads the settings live from the robot, as the cloud cache can lag.
    pub fn read_settings(&self, robot: &SharedRobot) {
        let robot = robot.clone();
        self.run(async move {
            match robot.read(&dreame::settings::all_properties()).await {
                Ok(s) => Msg::Status(Box::new(s)),
                Err(e) => Msg::Link(format!("offline: {e:#}")),
            }
        });
    }

    /// Runs one robot command; `what` names it in the result message.
    pub fn command<F, Fut, T>(&self, robot: &SharedRobot, what: &str, f: F)
    where
        F: FnOnce(SharedRobot) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let robot = robot.clone();
        let what = what.to_string();
        self.run(async move {
            let result = f(robot.clone()).await.map(|_| ()).map_err(|e| format!("{e:#}"));
            // Show the effect at once rather than at the next push.
            if result.is_ok() {
                tokio::time::sleep(Duration::from_millis(800)).await;
                let _ = robot.refresh().await;
            }
            Msg::Done { what, result }
        });
    }

    /// Sends one map edit, checks that the robot took it, and reads the map back. Copies of
    /// the map before and after go to the app folder's `edits` folder.
    pub fn apply_edit(&self, robot: &SharedRobot, before: dreame_map::Map, edit: dreame::mapedit::Edit) {
        let robot = robot.clone();
        let dir = self.paths.dir.join("edits");
        self.run(async move {
            let link = robot.cloud_link();
            let stamp = dreame::session::stamp();
            let result = match dreame::mapedit::apply(link, &edit, &dir, &stamp, Default::default()).await {
                Ok(_) => reload(link).await,
                Err(e) => Err(format!("{e:#}")),
            };
            Msg::Edited { before: Box::new(before), result }
        });
    }

    /// Runs an advanced edit of the room grid through a backup and restore of the map
    /// ([`dreame::archiveedit::apply`]); steps arrive as [`Msg::Progress`]. The backup stays in
    /// the app folder's `backups` folder, so the map can be put back.
    pub fn apply_grid(&self, robot: &SharedRobot, before: dreame_map::Map, edit: dreame::archiveedit::GridEdit) {
        let (tx, ctx, robot) = (self.tx.clone(), self.ctx.clone(), robot.clone());
        let dir = self.paths.dir.join("backups");
        // The progress callback is not Send, so this runs on a thread of its own.
        std::thread::spawn(move || {
            let send = |m: Msg| {
                let _ = tx.send(m);
                ctx.request_repaint();
            };
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return send(Msg::Edited { before: Box::new(before), result: Err(e.to_string()) }),
            };
            let opts = dreame::archiveedit::Options {
                dir,
                stamp: dreame::session::stamp(),
                port: GRID_PORT,
                prime: true,
                archive: None,
            };
            let mut progress = |line: &str| send(Msg::Progress(line.to_string()));
            let result = rt.block_on(async {
                let link = robot.cloud_link();
                let report = dreame::archiveedit::apply(link, &edit, &opts, &mut progress).await.map_err(|e| format!("{e:#}"))?;
                if let Err(e) = &report.settings {
                    progress(&trf("The room settings could not be put back: {error}", &[("error", e)]));
                }
                reload(link).await
            });
            send(Msg::Edited { before: Box::new(before), result });
        });
    }

    /// Follows the robot over MQTT until [`Backend::stop_live`]. Reconnects after a drop.
    pub fn start_live(&mut self, robot: &SharedRobot, map_version: u8) {
        self.stop_live();
        let stop = Arc::new(AtomicBool::new(false));
        self.live_stop = Some(stop.clone());
        let (tx, ctx, robot) = (self.tx.clone(), self.ctx.clone(), robot.clone());
        self.rt.spawn(async move {
            let send = |m: Msg| {
                let ok = tx.send(m).is_ok();
                ctx.request_repaint();
                ok
            };
            let mut tracker = Tracker::new(map_version);
            // Warns when the robot barely moves while it should, before it reports being stuck.
            let mut watch = dreame::watch::RunWatch::default();
            let start = std::time::Instant::now();
            while !stop.load(Ordering::Relaxed) {
                let mut events = match robot.events() {
                    Ok(e) => e,
                    Err(e) => {
                        send(Msg::Link(format!("offline: {e:#}")));
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        continue;
                    }
                };
                loop {
                    let event = tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {
                            if stop.load(Ordering::Relaxed) { break } else { continue }
                        }
                        e = events.next() => e,
                    };
                    let Some(event) = event else { break };
                    let ok = match event {
                        Event::Connection(state) => send(Msg::Link(format!("{state:?}").to_lowercase())),
                        Event::Status { status, .. } => {
                            watch.update(&status);
                            send(Msg::Status(status))
                        }
                        Event::MapFrame(f) => {
                            let pose = Some(f.header.robot).filter(|p| *p != dreame_map::header::Pose::ABSENT);
                            tracker.push(*f);
                            let t = start.elapsed().as_secs_f64();
                            if let Some(line) = pose.and_then(|p| watch.update_pose(t, f64::from(p.x), f64::from(p.y))) {
                                if line.starts_with("warning") {
                                    send(Msg::Stall(line));
                                }
                            }
                            send(Msg::Live {
                                path: tracker.path().clone(),
                                pose: tracker.pose,
                                active_rooms: tracker.active_rooms.clone(),
                            })
                        }
                        Event::RobotEvent { .. } => true,
                    };
                    if !ok {
                        return;
                    }
                }
                events.stop().await;
                if !stop.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        });
    }

    pub fn stop_live(&mut self) {
        if let Some(s) = self.live_stop.take() {
            s.store(true, Ordering::Relaxed);
        }
    }
}

/// Port the edited map backup is served on to the robot, during an advanced edit.
const GRID_PORT: u16 = 8770;

async fn reload(link: &CloudRobot) -> Result<Box<LoadedMap>, String> {
    match dreame::mapedit::read_saved(link).await {
        Ok(saved) => Ok(Box::new(LoadedMap { path: live_path(&saved.live_envelope), map: saved.map })),
        Err(e) => Err(trf("the edit was sent, but the map could not be read back: {error}", &[("error", &format!("{e:#}"))])),
    }
}

async fn devices(client: Arc<CloudClient>) -> Result<Msg> {
    let devices: Vec<DeviceInfo> = client.list_devices().await?.into_iter().filter(|d| d.is_vacuum()).collect();
    if devices.is_empty() {
        return Err(anyhow!(tr("There is no robot vacuum on this account.").to_string()));
    }
    Ok(Msg::Devices { client, devices })
}

/// The path in a live frame; empty when the frame has none or does not decode.
fn live_path(envelope: &str) -> Path {
    Frame::decode(envelope, None, None)
        .ok()
        .and_then(|f| Map::from_live(&f, dreame::mapedit::MAP_VERSION).ok())
        .map(|m| m.path)
        .unwrap_or_default()
}

/// A sign-in error in words a user can act on; the detail follows for support.
fn friendly_login_error(detail: &str) -> String {
    let d = detail.to_lowercase();
    if d.contains("password") || d.contains("credential") || d.contains("401") || d.contains("auth") {
        trf("The email or password is not right. ({detail})", &[("detail", &detail)])
    } else if d.contains("dns") || d.contains("connect") || d.contains("timed out") {
        trf("Cannot reach the Dreame cloud. Check the internet connection. ({detail})", &[("detail", &detail)])
    } else {
        detail.to_string()
    }
}
