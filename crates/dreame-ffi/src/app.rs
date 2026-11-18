//! The API of the Android app (`apps/dreame-android`): one robot's home screen, its
//! map, its room settings and its robot page.
//!
//! [`HomeRobot`] keeps the robot's status and saved map. Every change reaches the app through one
//! [`HomeListener`]: a new status, the live path, the connection state, a notice for the user,
//! or a new map. Commands return once the robot has accepted them; the status they cause
//! arrives through the listener.
//!
//! The map crosses the boundary in cell space: the map image's pixel space at one pixel per
//! cell, x to the right and y down. The app draws the image and everything on it in that space
//! and never needs world coordinates.
//!
//! Every command, setting and room change is a write to the robot. The app asks the user
//! where the official app does (ending a job, resetting a part) and otherwise acts on a tap, as
//! the official app does.

use crate::{Account, DreameError, Result, SettingValue, TimeOfDay};
use dreame::control::{CleanOptions, ZONE_DEFAULTS};
use dreame::history::Job;
use dreame::live::Tracker;
use dreame::mapedit::{self, Edit};
use dreame::phase::{self, Phase as P, Primary};
use dreame::watch::RunWatch;
use dreame::{CloudRobot, Event, Robot};
use dreame_map::model::{Point, Rect};
use dreame_map::path::Path;
use dreame_map::render::{render_png, RenderOptions};
use dreame_map::Map;
use dreame_model::codec::ai_obstacle::{AiFlags, AiObstacle};
use dreame_model::codec::consumables::Consumable;
use dreame_model::codec::station::StationOp;
use dreame_model::enums::{CarpetSensitivity, CleaningMode, CodeEnum, Open, SuctionLevel, WaterVolume};
use dreame_model::codec::schedule::{RoomTask, ScheduleSlot};
use dreame_model::commands::{RemoteMove, REMOTE_REPEAT_MS};
use dreame_model::{Capabilities, Feature, Status};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

// ---- records ----------------------------------------------------------------------------------

/// The robot's situation as the user sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Phase {
    /// No status read yet.
    Unknown,
    Cleaning,
    Paused,
    /// Driving back to the station.
    Returning,
    /// At the station, busy with the mop, bin or water.
    AtStation,
    /// On the dock, charging or charged.
    Docked,
    /// Off the dock and not working.
    Idle,
    /// Stopped by a fault.
    Error,
}

impl From<P> for Phase {
    fn from(p: P) -> Self {
        match p {
            P::Unknown => Self::Unknown,
            P::Cleaning => Self::Cleaning,
            P::Paused => Self::Paused,
            P::Returning => Self::Returning,
            P::AtStation => Self::AtStation,
            P::Docked => Self::Docked,
            P::Idle => Self::Idle,
            P::Error => Self::Error,
        }
    }
}

/// What the main button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MainAction {
    /// Start a new job of the chosen kind.
    Start,
    Pause,
    /// Continue the paused or interrupted job.
    Resume,
}

/// Vacuum and mop mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CleanMode {
    Vacuum,
    Mop,
    VacuumAndMop,
    VacuumThenMop,
}

impl CleanMode {
    fn to_model(self) -> CleaningMode {
        match self {
            Self::Vacuum => CleaningMode::Sweeping,
            Self::Mop => CleaningMode::Mopping,
            Self::VacuumAndMop => CleaningMode::SweepingAndMopping,
            Self::VacuumThenMop => CleaningMode::MoppingAfterSweeping,
        }
    }

    fn from_model(m: CleaningMode) -> Self {
        match m {
            CleaningMode::Sweeping => Self::Vacuum,
            CleaningMode::Mopping => Self::Mop,
            CleaningMode::SweepingAndMopping => Self::VacuumAndMop,
            CleaningMode::MoppingAfterSweeping => Self::VacuumThenMop,
        }
    }
}

/// Suction power, in the official app's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Suction {
    Quiet,
    Standard,
    Strong,
    Max,
}

impl Suction {
    fn to_model(self) -> SuctionLevel {
        match self {
            Self::Quiet => SuctionLevel::Quiet,
            Self::Standard => SuctionLevel::Standard,
            Self::Strong => SuctionLevel::Strong,
            Self::Max => SuctionLevel::Turbo,
        }
    }

    fn from_model(l: SuctionLevel) -> Self {
        match l {
            SuctionLevel::Quiet => Self::Quiet,
            SuctionLevel::Standard => Self::Standard,
            SuctionLevel::Strong => Self::Strong,
            SuctionLevel::Turbo => Self::Max,
        }
    }
}

/// Water for the mop, on robots without the 1 to 32 wetness scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Water {
    Low,
    Medium,
    High,
}

impl Water {
    fn to_model(self) -> WaterVolume {
        match self {
            Self::Low => WaterVolume::Low,
            Self::Medium => WaterVolume::Medium,
            Self::High => WaterVolume::High,
        }
    }

    fn from_model(w: WaterVolume) -> Self {
        match w {
            WaterVolume::Low => Self::Low,
            WaterVolume::Medium => Self::Medium,
            WaterVolume::High => Self::High,
        }
    }
}

/// One part that wears: a brush, a filter, a pad.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Part {
    /// Pass it to [`HomeRobot::reset_part`].
    pub id: u32,
    pub name: String,
    pub percent_left: Option<i64>,
    pub hours_left: Option<i64>,
}

/// The do-not-disturb hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct QuietHours {
    pub enabled: bool,
    pub start: TimeOfDay,
    pub end: TimeOfDay,
}

/// One obstacle-avoidance switch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiToggle {
    /// Pass it back in [`SettingValue::AiSwitch`].
    pub bit: u32,
    pub name: String,
    pub on: bool,
}

/// The settings on the robot page. `None` means the robot lacks it or it was not read yet.
/// A change goes through [`HomeRobot::set_setting`].
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RobotSettings {
    pub carpet_boost: Option<bool>,
    pub resume_cleaning: Option<bool>,
    /// Wash the mop during the job.
    pub self_clean: Option<bool>,
    pub carpet_sensitivity: Option<crate::CarpetLevel>,
    /// Mop drying time, 1 to 12 hours.
    pub drying_hours: Option<u8>,
    /// Voice volume, 0 to 100.
    pub volume: Option<u8>,
    pub child_lock: Option<bool>,
    pub quiet_hours: Option<QuietHours>,
    pub ai: Vec<AiToggle>,
    /// Cleans between automatic bin emptying, 1 to 10.
    pub auto_empty_every: Option<u8>,
}

/// What the robot can do, as far as the app's controls go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct Features {
    /// Mop water is the 1 to 32 wetness scale, not three steps.
    pub wetness: bool,
    pub auto_empty: bool,
    pub self_wash: bool,
}

/// Everything the home screen and the robot page show.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct HomeStatus {
    pub phase: Phase,
    /// One line, such as "Cleaning rooms" or "Charging".
    pub headline: String,
    /// The fault the robot reports, as a sentence.
    pub error: Option<String>,
    /// A job is under way, paused or not.
    pub job_active: bool,
    pub main_action: MainAction,
    /// Dock makes sense: the robot is not on or going to its dock.
    pub can_dock: bool,
    pub battery: Option<i64>,
    pub charging: bool,
    pub area_m2: Option<i64>,
    pub minutes: Option<i64>,
    /// 1 to 100 while a job runs, where the robot reports it.
    pub progress: Option<i64>,
    pub mode: Option<CleanMode>,
    pub suction: Option<Suction>,
    pub water: Option<Water>,
    /// 1 to 32.
    pub wetness: Option<u8>,
    /// Each room cleans with its own settings (4.26).
    pub per_room_settings: Option<bool>,
    pub settings: RobotSettings,
    pub parts: Vec<Part>,
}

/// A point in cell space.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CellPoint {
    pub x: f64,
    pub y: f64,
}

/// A position and heading in cell space. `angle` is in degrees, counter-clockwise from the
/// right, as on the map: draw the heading towards `(cos a, -sin a)` on screen.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CellPose {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
}

/// A rectangle in cell space.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CellRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// One point of a path in cell space. `connected` is false where a new stroke starts.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PathStep {
    pub x: f32,
    pub y: f32,
    pub connected: bool,
}

/// A room's stored cleaning settings, as the official app's room settings show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RoomSettings {
    /// 0 quiet to 3 max.
    pub suction: i64,
    /// 1 to 32 on wetness robots, else 1 low to 3 high.
    pub water: i64,
    /// Passes, 1 to 3.
    pub passes: i64,
    /// 0 vacuum, 1 mop, 2 vacuum and mop; `None` when the robot does not store it.
    pub mode: Option<i64>,
}

/// One room.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RoomInfo {
    pub id: u32,
    pub name: String,
    /// A hidden room is not cleaned and not shown.
    pub hidden: bool,
    /// Where to put its name: a point well inside the room.
    pub label: Option<CellPoint>,
    pub settings: Option<RoomSettings>,
}

/// The live job: its path so far, the robot, and the rooms it cleans.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct LiveFrame {
    pub path: Vec<PathStep>,
    pub robot: Option<CellPose>,
    pub active_rooms: Vec<u32>,
}

/// The live connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Link {
    Connecting,
    /// Pushes arrive as they happen.
    Live,
    /// No live connection; the status is read every 20 seconds.
    Offline,
}

/// Something to tell the user. `good` is true for good news, such as a finished job.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub good: bool,
}

/// A room settings change; `None` keeps a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RoomChange {
    pub id: u32,
    pub suction: Option<i64>,
    pub water: Option<i64>,
    pub passes: Option<i64>,
    pub mode: Option<i64>,
}

/// A station action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StationAction {
    EmptyBin,
    WashMop,
    DryMop,
    StopDrying,
}

/// Receives everything that changes. Called on a background thread.
#[uniffi::export(with_foreign)]
pub trait HomeListener: Send + Sync {
    fn on_status(&self, status: HomeStatus);
    fn on_live(&self, frame: LiveFrame);
    fn on_link(&self, link: Link);
    fn on_notice(&self, notice: Notice);
    /// A new saved map: after a job ends, after a room change, or after [`HomeRobot::load_map`].
    fn on_map(&self, map: Arc<HomeMap>);
    /// What the robot supports changed: background discovery finished.
    fn on_features(&self, features: Features);
}

// ---- the map ----------------------------------------------------------------------------------

/// The saved map.
#[derive(uniffi::Object)]
pub struct HomeMap {
    pub(crate) map: Map,
    labels: std::collections::BTreeMap<u32, Point>,
}

impl HomeMap {
    pub(crate) fn new(map: Map) -> Self {
        Self { labels: map.label_points(), map }
    }

    pub(crate) fn to_cell(&self, p: Point) -> CellPoint {
        let (x, y) = self.map.transform().world_to_image(p.x, p.y, 1.0);
        CellPoint { x, y }
    }

    pub(crate) fn to_world(&self, x: f64, y: f64) -> Point {
        let (x, y) = self.map.transform().image_to_world(x, y, 1.0);
        Point::new(x, y)
    }

    fn world_rect(&self, r: &CellRect) -> Rect {
        let a = self.to_world(r.left, r.top);
        let b = self.to_world(r.right, r.bottom);
        Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    fn cells(&self, path: &Path) -> Vec<PathStep> {
        path.points
            .iter()
            .map(|p| {
                let c = self.to_cell(Point::new(p.x as f64, p.y as f64));
                PathStep { x: c.x as f32, y: c.y as f32, connected: p.connected }
            })
            .collect()
    }

    fn pose(&self, x: f64, y: f64, angle: f64) -> CellPose {
        let c = self.to_cell(Point::new(x, y));
        CellPose { x: c.x, y: c.y, angle }
    }
}

/// Smallest zone side the robot takes, in mm.
const MIN_ZONE_MM: f64 = 300.0;

#[uniffi::export]
impl HomeMap {
    /// Width in cells.
    pub fn width(&self) -> u32 {
        self.map.grid.width as u32
    }

    /// Height in cells.
    pub fn height(&self) -> u32 {
        self.map.grid.height as u32
    }

    /// Millimetres per cell.
    pub fn mm_per_cell(&self) -> f64 {
        f64::from(self.map.grid.grid_size.max(1))
    }

    /// The rooms, walls and floor as a PNG of `scale` pixels per cell. Rooms in `coloured` are
    /// drawn in colour and the others grey; an empty list colours every room. Path, dock and
    /// robot are left for the app to draw.
    pub fn png(&self, coloured: Vec<u32>, scale: u32) -> Result<Vec<u8>> {
        let opts = RenderOptions {
            scale: scale.clamp(1, 8),
            path: false,
            markers: false,
            highlight_active: true,
            ..RenderOptions::default()
        };
        let mut m = self.map.clone();
        m.active_rooms = coloured;
        Ok(render_png(&m, &opts)?)
    }

    /// The shown rooms in the saved cleaning order, then the hidden rooms.
    pub fn rooms(&self) -> Vec<RoomInfo> {
        dreame::shapes::initial_order(&self.map)
            .into_iter()
            .chain(self.map.hidden_rooms())
            .filter_map(|id| self.map.rooms.get(&id))
            .map(|r| RoomInfo {
                id: r.id,
                name: r.display_name(),
                hidden: r.hidden,
                label: self.labels.get(&r.id).map(|p| self.to_cell(*p)),
                settings: r.settings.as_ref().filter(|s| s.len() >= 3).map(|s| RoomSettings {
                    suction: s[0],
                    water: s[1],
                    passes: s[2],
                    mode: s.get(4).copied().filter(|m| (0..3).contains(m)),
                }),
            })
            .collect()
    }

    /// The room under a cell-space point, hidden rooms included.
    pub fn room_at(&self, x: f64, y: f64) -> Option<u32> {
        self.map.room_at(self.to_world(x, y))
    }

    pub fn dock(&self) -> Option<CellPose> {
        self.map.dock.map(|d| self.pose(d.x, d.y, d.angle))
    }

    /// Where the robot was when the map was saved.
    pub fn robot(&self) -> Option<CellPose> {
        self.map.robot.map(|r| self.pose(r.x, r.y, r.angle))
    }

    /// The rooms of the job the map comes from; empty for the saved map.
    pub fn active_rooms(&self) -> Vec<u32> {
        self.map.active_rooms.clone()
    }

    /// The last job's path.
    pub fn path(&self) -> Vec<PathStep> {
        self.cells(&self.map.path)
    }

    /// A drawn zone with its corners on cell corners, or `None` when a side is under 30 cm.
    pub fn snap_zone(&self, zone: CellRect) -> Option<CellRect> {
        let z = self.map.snap_to_cells(&self.world_rect(&zone));
        if z.width() < MIN_ZONE_MM || z.height() < MIN_ZONE_MM {
            return None;
        }
        let a = self.to_cell(Point::new(z.x0, z.y1));
        let b = self.to_cell(Point::new(z.x1, z.y0));
        Some(CellRect { left: a.x, top: a.y, right: b.x, bottom: b.y })
    }

    /// A zone's width and height in metres.
    pub fn zone_size_m(&self, zone: CellRect) -> Vec<f64> {
        let z = self.world_rect(&zone);
        vec![z.width() / 1000.0, z.height() / 1000.0]
    }
}

// ---- the robot --------------------------------------------------------------------------------

struct State {
    status: Status,
    map: Option<Arc<HomeMap>>,
}

pub(crate) struct Inner {
    /// Swapped once background discovery finishes; see [`Inner::discover`].
    robot: RwLock<Arc<Robot<CloudRobot>>>,
    name: String,
    /// The app's private folder; map edit snapshots go to its `edits` folder.
    files: PathBuf,
    state: Mutex<State>,
    listener: Mutex<Option<Arc<dyn HomeListener>>>,
    /// Set to stop the running [`follow`] task.
    live_stop: Mutex<Option<Arc<AtomicBool>>>,
    /// The jobs last listed, so a job's map can be read by its index.
    jobs: Mutex<Vec<Job>>,
    /// The schedules last read, so a change builds on the whole list.
    schedules: Mutex<Vec<ScheduleSlot>>,
    /// The obstacles last listed, so a photo can be fetched by its index.
    obstacles: Mutex<Vec<dreame_map::obstacle::Obstacle>>,
}

impl Inner {
    fn robot(&self) -> Arc<Robot<CloudRobot>> {
        self.robot.read().expect("robot lock").clone()
    }

    fn features(&self) -> Features {
        let wetness = self.state.lock().expect("state lock").status.wetness.is_some();
        Features {
            wetness: self.has(Feature::Wetness) || wetness,
            auto_empty: self.has(Feature::AutoEmptyBase),
            self_wash: self.has(Feature::SelfWashBase),
        }
    }

    /// Starts following with the current robot, ending any earlier follow.
    fn follow(self: &Arc<Self>) {
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.live_stop.lock().expect("live lock").replace(stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        tokio::spawn(follow(self.clone(), stop));
    }

    /// Learns what the robot supports by reading every property (about 25 seconds), saves the
    /// result in `file` for later starts, and swaps in a robot that knows it. Following, if on,
    /// restarts with it. Read-only.
    async fn discover(self: &Arc<Self>, file: PathBuf) {
        let old = self.robot();
        let link = old.cloud_link();
        let mut robot = Robot::cloud(link.client.clone(), link.device.clone());
        match robot.discover(&Default::default()).await {
            Ok(caps) => {
                dreame::known::save(&file, &link.device, caps);
            }
            Err(_) => return,
        }
        *self.robot.write().expect("robot lock") = Arc::new(robot);
        if self.live_stop.lock().expect("live lock").is_some() {
            self.follow();
        }
        if let Ok(s) = self.robot().refresh().await {
            self.publish(s);
        }
        if let Some(l) = self.listener() {
            l.on_features(self.features());
        }
    }
    fn listener(&self) -> Option<Arc<dyn HomeListener>> {
        self.listener.lock().expect("listener lock").clone()
    }

    pub(crate) fn map(&self) -> Option<Arc<HomeMap>> {
        self.state.lock().expect("state lock").map.clone()
    }

    fn has(&self, f: Feature) -> bool {
        self.robot().has(f) == Some(true)
    }

    fn home_status(&self, s: &Status) -> HomeStatus {
        home_status(s, self.has(Feature::Dnd))
    }

    /// Takes a new status: keeps it, tells the listener, says what changed, and reads the map
    /// again when a job ended (the saved map then has its path).
    fn publish(self: &Arc<Self>, s: Status) {
        let (notices, ended) = {
            let mut st = self.state.lock().expect("state lock");
            let notices = phase::notices(&st.status, &s);
            let ended = phase::job_active(&st.status) && !phase::job_active(&s);
            st.status = s.clone();
            (notices, ended)
        };
        let Some(l) = self.listener() else { return };
        l.on_status(self.home_status(&s));
        for n in notices {
            let good = n.good;
            l.on_notice(Notice { title: n.title, body: n.body, good });
        }
        if ended {
            let me = self.clone();
            tokio::spawn(async move {
                let _ = me.reload().await;
            });
        }
    }

    async fn reload(self: &Arc<Self>) -> Result<Arc<HomeMap>> {
        let saved = mapedit::read_saved(self.robot().cloud_link()).await?;
        let map = Arc::new(HomeMap::new(saved.map));
        self.state.lock().expect("state lock").map = Some(map.clone());
        if let Some(l) = self.listener() {
            l.on_map(map.clone());
        }
        Ok(map)
    }

    /// After a command: reads the status soon, so its effect shows before the next push.
    async fn after_command(self: &Arc<Self>) {
        tokio::time::sleep(Duration::from_millis(800)).await;
        if let Ok(s) = self.robot().refresh().await {
            self.publish(s);
        }
    }

    pub(crate) async fn edit(self: &Arc<Self>, e: Edit) -> Result<Arc<HomeMap>> {
        let dir = self.files.join("edits");
        mapedit::apply(self.robot().cloud_link(), &e, &dir, &dreame::session::stamp(), Default::default()).await?;
        self.reload().await
    }
}

/// One robot, for the home screen.
#[derive(uniffi::Object)]
pub struct HomeRobot {
    pub(crate) inner: Arc<Inner>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Account {
    /// The robot vacuums on the account.
    pub async fn vacuums(&self) -> Result<Vec<crate::Device>> {
        Ok(self.devices().await?.into_iter().filter(|d| d.is_vacuum).collect())
    }

    /// Connects to the robot with device id `did`. It returns in a second or two: what the
    /// robot supports comes from the result saved at an earlier start, when it fits the
    /// firmware, and the first status from the cloud cache. The live status read follows in the
    /// background; on the first start, or after a firmware update, discovery too, and the
    /// listener then gets [`HomeListener::on_features`]. `files_dir` is the app's private
    /// folder. Read-only.
    pub async fn connect(&self, did: String, files_dir: String) -> Result<Arc<HomeRobot>> {
        let device = self
            .client
            .list_devices()
            .await?
            .into_iter()
            .find(|d| d.did == did)
            .ok_or_else(|| DreameError::Other { detail: "that robot is not on this account".into() })?;
        let name = if device.name.trim().is_empty() { device.model.clone() } else { device.name.clone() };
        let files = PathBuf::from(files_dir);
        let file = files.join("capabilities.json");
        let known = dreame::known::load(&file, &device);
        let mut robot = Robot::cloud(self.client.clone(), device);
        if let Some(k) = &known {
            robot.set_capabilities(Capabilities::from_known(k));
        }
        let status = robot.refresh_cached().await?;
        let inner = Arc::new(Inner {
            robot: RwLock::new(Arc::new(robot)),
            name,
            files,
            state: Mutex::new(State { status, map: None }),
            listener: Mutex::new(None),
            live_stop: Mutex::new(None),
            jobs: Mutex::new(Vec::new()),
            schedules: Mutex::new(Vec::new()),
            obstacles: Mutex::new(Vec::new()),
        });
        let me = inner.clone();
        tokio::spawn(async move {
            // The first status came from the cloud cache; ask the robot itself too.
            if let Ok(s) = me.robot().refresh().await {
                me.publish(s);
            }
            if known.is_none() {
                me.discover(file).await;
            }
        });
        Ok(Arc::new(HomeRobot { inner }))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl HomeRobot {
    /// The name the user gave the robot, or its model.
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    pub fn did(&self) -> String {
        self.inner.robot().cloud_link().device.did.clone()
    }

    /// What the robot supports. It can grow once background discovery ends; the listener then
    /// gets [`HomeListener::on_features`].
    pub fn features(&self) -> Features {
        self.inner.features()
    }

    /// The status as last read or pushed.
    pub fn status(&self) -> HomeStatus {
        let s = self.inner.state.lock().expect("state lock").status.clone();
        self.inner.home_status(&s)
    }

    /// The saved map as last read.
    pub fn map(&self) -> Option<Arc<HomeMap>> {
        self.inner.map()
    }

    /// Reads the saved map. It also arrives through [`HomeListener::on_map`]. Read-only.
    pub async fn load_map(&self) -> Result<Arc<HomeMap>> {
        self.inner.reload().await
    }

    /// Reads the status again, settings included. It arrives through the listener. Read-only.
    pub async fn refresh(&self) -> Result<()> {
        let i = &self.inner;
        let r = i.robot();
        r.refresh().await?;
        let s = r.read(&dreame::settings::all_properties()).await?;
        i.publish(s);
        Ok(())
    }

    /// Starts following the robot: status, live path and connection go to `listener`. Pushes
    /// come over the live connection; without it the status is read every 20 seconds. Only
    /// listens; it never sends to the robot.
    pub async fn start(&self, listener: Arc<dyn HomeListener>) {
        *self.inner.listener.lock().expect("listener lock") = Some(listener);
        self.inner.follow();
    }

    /// Stops following. The listener gets nothing more.
    pub fn stop(&self) {
        if let Some(s) = self.inner.live_stop.lock().expect("live lock").take() {
            s.store(true, Ordering::Relaxed);
        }
    }

    // ---- commands: writes to the robot ----

    /// Cleans the whole home with the stored room settings.
    pub async fn clean_home(&self) -> Result<()> {
        self.inner.robot().start().await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Continues a paused job.
    pub async fn resume(&self) -> Result<()> {
        self.clean_home().await
    }

    pub async fn pause(&self) -> Result<()> {
        self.inner.robot().pause().await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Ends the job; the robot stays where it is.
    pub async fn end(&self) -> Result<()> {
        self.inner.robot().stop().await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Sends the robot to its dock, ending a job.
    pub async fn dock(&self) -> Result<()> {
        self.inner.robot().dock().await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// The robot says where it is.
    pub async fn locate(&self) -> Result<()> {
        self.inner.robot().locate().await?;
        Ok(())
    }

    /// Cleans `rooms` in that order, `passes` times each.
    pub async fn clean_rooms(&self, rooms: Vec<u32>, passes: u32) -> Result<()> {
        let map = self.inner.map().ok_or_else(no_map)?;
        let opts = CleanOptions { passes: Some(passes.clamp(1, 3)), ..Default::default() };
        self.inner.robot().clean_rooms(&map.map, &rooms, opts).await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Sends the robot to a point of the map (cell space). It drives there without cleaning
    /// and stops. The point must lie on the floor of a room that is not hidden.
    pub async fn go_to(&self, point: CellPoint) -> Result<()> {
        let map = self.inner.map().ok_or_else(no_map)?;
        let p = map.to_world(point.x, point.y);
        self.inner.robot().go_to(&map.map, p).await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Cleans a zone drawn on the map, `passes` times, at the current suction.
    pub async fn clean_zone(&self, zone: CellRect, passes: u32) -> Result<()> {
        let map = self.inner.map().ok_or_else(no_map)?;
        let rect = map.map.snap_to_cells(&map.world_rect(&zone));
        let suction = self.inner.state.lock().expect("state lock").status.suction.map(|s| s.code()).or(Some(ZONE_DEFAULTS.1));
        let opts = CleanOptions { passes: Some(passes.clamp(1, 3)), suction, water: None };
        self.inner.robot().clean_zone(&map.map, rect, opts).await?;
        self.inner.after_command().await;
        Ok(())
    }

    pub async fn set_mode(&self, mode: CleanMode) -> Result<()> {
        self.inner.robot().set_mop_mode(mode.to_model()).await?;
        self.inner.after_command().await;
        Ok(())
    }

    pub async fn set_suction(&self, suction: Suction) -> Result<()> {
        self.inner.robot().set_suction(suction.to_model()).await?;
        self.inner.after_command().await;
        Ok(())
    }

    pub async fn set_water(&self, water: Water) -> Result<()> {
        self.inner.robot().set_water_volume(water.to_model()).await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Mop wetness, 1 to 32.
    pub async fn set_wetness(&self, level: u8) -> Result<()> {
        self.inner.robot().set_wetness(level.clamp(1, 32)).await?;
        self.inner.after_command().await;
        Ok(())
    }

    /// Changes one setting of the robot page, then reads the settings back.
    pub async fn set_setting(&self, value: SettingValue) -> Result<()> {
        let setting = dreame::Setting::try_from(value)?;
        let i = &self.inner;
        // Settings that share a property (AI switches, off-peak) build on its current value.
        let props = setting.properties();
        let r = i.robot();
        r.read(&props).await?;
        r.set(&setting).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let s = r.read(&props).await?;
        i.publish(s);
        Ok(())
    }

    /// Resets a part's remaining life to 100%, after it was replaced or cleaned.
    pub async fn reset_part(&self, id: u32) -> Result<()> {
        let c = Consumable::from_siid(id).ok_or_else(|| DreameError::Other { detail: "unknown part".into() })?;
        self.inner.robot().reset_consumable(c).await?;
        self.inner.after_command().await;
        Ok(())
    }

    pub async fn station(&self, action: StationAction) -> Result<()> {
        let r = self.inner.robot();
        match action {
            StationAction::EmptyBin => r.start_auto_empty().await?,
            StationAction::WashMop => r.station(StationOp::ReturnAndWash).await?,
            StationAction::DryMop => r.station(StationOp::StartDrying).await?,
            StationAction::StopDrying => r.station(StationOp::StopDrying).await?,
        };
        self.inner.after_command().await;
        Ok(())
    }

    // ---- room changes: map edits, saved on the robot ----

    /// Changes one room's stored settings. Only while the robot is idle on its dock.
    pub async fn set_room(&self, change: RoomChange) -> Result<Arc<HomeMap>> {
        self.inner
            .edit(Edit::RoomSettings {
                id: change.id,
                suction: change.suction,
                water: change.water,
                times: change.passes,
                clean_type: change.mode,
                mop_effect: None,
            })
            .await
    }

    /// Saves a new cleaning order, every room listed once.
    pub async fn set_order(&self, rooms: Vec<u32>) -> Result<Arc<HomeMap>> {
        self.inner.edit(Edit::CleaningOrder(rooms)).await
    }

    /// Shows or hides one room. A hidden room is not cleaned.
    pub async fn set_room_shown(&self, id: u32, shown: bool) -> Result<Arc<HomeMap>> {
        let map = self.inner.map().ok_or_else(no_map)?;
        let ids = dreame::shapes::toggled_hidden(&map.map, id, !shown);
        self.inner.edit(Edit::HideRooms(ids)).await
    }

    /// Turns per-room settings on or off.
    pub async fn set_per_room_settings(&self, on: bool) -> Result<()> {
        self.set_setting(SettingValue::CustomizedCleaning { on }).await
    }
}

// ---- history and schedules --------------------------------------------------------------------

/// One past job.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PastJob {
    /// Pass it to [`HomeRobot::job_map`].
    pub index: u32,
    /// When it started, Unix seconds.
    pub started: Option<i64>,
    pub minutes: Option<i64>,
    pub area_m2: Option<i64>,
    /// How it ended, such as "Completed" or "Ended by hand".
    pub outcome: String,
    /// The robot kept a map of the job, with its path.
    pub has_map: bool,
}

/// One room of a schedule, with its own settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ScheduleRoom {
    pub room: u32,
    /// 1 to 3.
    pub passes: u8,
    /// 0 quiet to 3 max.
    pub suction: u8,
    /// 0 vacuum, 1 mop, 2 vacuum and mop.
    pub mode: u8,
    /// Mop wetness, 1 to 32.
    pub wetness: u8,
}

/// One schedule.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Schedule {
    /// Pass it to the schedule methods.
    pub index: u32,
    pub time: TimeOfDay,
    /// Seven days, Monday first.
    pub days: Vec<bool>,
    /// Every week; false runs once.
    pub weekly: bool,
    pub enabled: bool,
    /// The rooms, in cleaning order, for a schedule of chosen rooms. `None` for other kinds
    /// (such as whole-home or CleanGenius ones), which are edited in the official app.
    pub rooms: Option<Vec<ScheduleRoom>>,
}

/// A schedule of chosen rooms, new or changed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScheduleDraft {
    pub time: TimeOfDay,
    /// Seven days, Monday first.
    pub days: Vec<bool>,
    pub weekly: bool,
    pub rooms: Vec<ScheduleRoom>,
}

fn schedule_record(index: usize, s: &ScheduleSlot) -> Schedule {
    Schedule {
        index: index as u32,
        time: s.time.into(),
        days: s.days().to_vec(),
        weekly: !s.once(),
        enabled: s.enabled(),
        rooms: s.room_tasks().map(|tasks| {
            tasks
                .iter()
                .map(|t| ScheduleRoom { room: u32::from(t.room), passes: t.passes, suction: t.suction, mode: t.mode, wetness: t.wetness })
                .collect()
        }),
    }
}

/// The room tasks of a draft. A room already in `before` keeps the bits the codec does not
/// decode; a new room gets them as every sample had them.
fn draft_tasks(rooms: &[ScheduleRoom], before: &[RoomTask]) -> Result<Vec<RoomTask>> {
    rooms
        .iter()
        .map(|r| {
            let id = u8::try_from(r.room).map_err(|_| DreameError::Other { detail: format!("room {} cannot be scheduled", r.room) })?;
            let mut t = before.iter().find(|t| t.room == id).copied().unwrap_or_else(|| RoomTask::new(id, 1, 1, 2, 16));
            t.passes = r.passes.clamp(1, 3);
            t.suction = r.suction.min(3);
            t.mode = r.mode.min(2);
            t.wetness = r.wetness.clamp(1, 32);
            Ok(t)
        })
        .collect()
}

fn days_array(days: &[bool]) -> Result<[bool; 7]> {
    <[bool; 7]>::try_from(days).map_err(|_| DreameError::Other { detail: "a schedule needs seven days".into() })
}

impl Inner {
    /// Writes the whole schedule list, then reads it back.
    async fn write_schedules(&self, slots: Vec<ScheduleSlot>) -> Result<Vec<Schedule>> {
        let r = self.robot();
        r.set_schedules(&slots).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        self.read_schedules().await
    }

    async fn read_schedules(&self) -> Result<Vec<Schedule>> {
        let slots = self.robot().schedules().await?;
        let out = slots.iter().enumerate().filter(|(_, s)| !s.invalid()).map(|(i, s)| schedule_record(i, s)).collect();
        *self.schedules.lock().expect("schedules lock") = slots;
        Ok(out)
    }

    fn slots(&self) -> Vec<ScheduleSlot> {
        self.schedules.lock().expect("schedules lock").clone()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl HomeRobot {
    /// The newest `limit` past jobs, newest first, from the cloud. Read-only.
    pub async fn jobs(&self, limit: u32) -> Result<Vec<PastJob>> {
        let robot = self.inner.robot();
        let jobs = dreame::history::jobs(robot.cloud_link(), limit).await?;
        let out = jobs
            .iter()
            .enumerate()
            .map(|(i, j)| PastJob {
                index: i as u32,
                started: j.started(),
                minutes: j.record.cleaning_time_min,
                area_m2: j.record.cleaned_area_m2,
                outcome: phase::capitalise(dreame_i18n::tr(dreame::history::outcome(&j.record))),
                has_map: j.record.log_object.is_some(),
            })
            .collect();
        *self.inner.jobs.lock().expect("jobs lock") = jobs;
        Ok(out)
    }

    /// The map the robot kept for a job listed by [`HomeRobot::jobs`], with the job's path and
    /// its rooms in [`HomeMap::active_rooms`]. Read-only.
    pub async fn job_map(&self, index: u32) -> Result<Option<Arc<HomeMap>>> {
        let job = self.inner.jobs.lock().expect("jobs lock").get(index as usize).cloned();
        let Some(job) = job else { return Ok(None) };
        let robot = self.inner.robot();
        Ok(dreame::history::job_map(robot.cloud_link(), &job).await?.map(|m| Arc::new(HomeMap::new(m))))
    }

    /// The schedules, read live from the robot. Read-only.
    pub async fn schedules(&self) -> Result<Vec<Schedule>> {
        self.inner.read_schedules().await
    }

    /// Turns a schedule on or off. A write: the whole list is sent.
    pub async fn set_schedule_enabled(&self, index: u32, on: bool) -> Result<Vec<Schedule>> {
        let mut slots = self.inner.slots();
        let s = slots.get_mut(index as usize).ok_or_else(no_schedule)?;
        s.set_enabled(on);
        self.inner.write_schedules(slots).await
    }

    /// Deletes a schedule. A write: the whole list is sent.
    pub async fn delete_schedule(&self, index: u32) -> Result<Vec<Schedule>> {
        let mut slots = self.inner.slots();
        if index as usize >= slots.len() {
            return Err(no_schedule());
        }
        slots.remove(index as usize);
        self.inner.write_schedules(slots).await
    }

    /// Adds a schedule of chosen rooms (`index` `None`) or changes one. A write: the whole
    /// list is sent.
    pub async fn save_schedule(&self, index: Option<u32>, draft: ScheduleDraft) -> Result<Vec<Schedule>> {
        let days = days_array(&draft.days)?;
        let time = dreame_model::codec::HhMm::try_from(draft.time)?;
        let repeat = if draft.weekly { "1" } else { "0" }.to_string();
        let mut slots = self.inner.slots();
        match index {
            Some(i) => {
                let s = slots.get_mut(i as usize).ok_or_else(no_schedule)?;
                let tasks = draft_tasks(&draft.rooms, &s.room_tasks().unwrap_or_default())?;
                s.time = time;
                s.set_days(&days);
                s.repeat = repeat;
                s.set_room_tasks(&tasks);
            }
            None => {
                let tasks = draft_tasks(&draft.rooms, &[])?;
                let id = slots.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                let mut s = ScheduleSlot::new_rooms(id, time, &dreame_model::codec::schedule::mask_from_days(&days), &tasks);
                s.repeat = repeat;
                slots.push(s);
            }
        }
        self.inner.write_schedules(slots).await
    }
}

fn no_schedule() -> DreameError {
    DreameError::Other { detail: "that schedule is gone; read the schedules again".into() }
}

fn no_map() -> DreameError {
    DreameError::Other { detail: "the map is not loaded yet".into() }
}

/// How often the status is read without the live connection, and with it.
const POLL_OFFLINE: Duration = Duration::from_secs(20);
const POLL_LIVE: Duration = Duration::from_secs(300);

/// Follows the robot until `stop`: pushes over the live connection, which reconnects after a
/// drop, and a status read now and then.
async fn follow(inner: Arc<Inner>, stop: Arc<AtomicBool>) {
    let tell = |link: Link| {
        if let Some(l) = inner.listener() {
            l.on_link(link);
        }
    };
    let stopped = || stop.load(Ordering::Relaxed);
    let mut tracker = Tracker::new(mapedit::MAP_VERSION);
    let mut watch = RunWatch::default();
    let start = Instant::now();
    let mut last_read = Instant::now();
    let mut live = false;
    // The settings are not part of the status reads; read them once at the start.
    if let Ok(s) = inner.robot().read(&dreame::settings::all_properties()).await {
        inner.publish(s);
    }
    while !stopped() {
        tell(Link::Connecting);
        let mut events = match inner.robot().events() {
            Ok(e) => e,
            Err(_) => {
                tell(Link::Offline);
                if let Ok(s) = inner.robot().refresh().await {
                    inner.publish(s);
                }
                sleep_unless(&stop, POLL_OFFLINE).await;
                continue;
            }
        };
        loop {
            let event = tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if stopped() { break }
                    let every = if live { POLL_LIVE } else { POLL_OFFLINE };
                    if last_read.elapsed() > every {
                        last_read = Instant::now();
                        if let Ok(s) = inner.robot().refresh().await {
                            inner.publish(s);
                        }
                    }
                    continue;
                }
                e = events.next() => e,
            };
            if stopped() {
                break;
            }
            let Some(event) = event else { break };
            match event {
                Event::Connection(state) => {
                    use dreame::robot::ConnectionState as C;
                    live = matches!(state, C::Connected);
                    tell(match state {
                        C::Connected => Link::Live,
                        C::Connecting { .. } => Link::Connecting,
                        _ => Link::Offline,
                    });
                }
                Event::Status { status, .. } => {
                    watch.update(&status);
                    inner.publish(*status);
                }
                Event::MapFrame(f) => {
                    let pose = Some(f.header.robot).filter(|p| *p != dreame_map::header::Pose::ABSENT);
                    tracker.push(*f);
                    let t = start.elapsed().as_secs_f64();
                    if let Some(line) = pose.and_then(|p| watch.update_pose(t, f64::from(p.x), f64::from(p.y))) {
                        if line.starts_with("warning") {
                            if let Some(l) = inner.listener() {
                                let body = dreame_i18n::tr("It has hardly moved for a while. Check that nothing holds it.").to_string();
                                l.on_notice(Notice { title: dreame_i18n::tr("The robot may be stuck").into(), body, good: false });
                            }
                        }
                    }
                    if let (Some(l), Some(map)) = (inner.listener(), inner.map()) {
                        l.on_live(LiveFrame {
                            path: map.cells(tracker.path()),
                            robot: tracker.pose.map(|p| map.pose(p.x, p.y, p.angle)),
                            active_rooms: tracker.active_rooms.clone(),
                        });
                    }
                }
                Event::RobotEvent { .. } => {}
            }
        }
        events.stop().await;
        live = false;
        if !stopped() {
            tell(Link::Offline);
            sleep_unless(&stop, Duration::from_secs(5)).await;
        }
    }
}

/// Sleeps `d`, waking early when `stop` is set.
async fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ---- status for the screen --------------------------------------------------------------------

/// The obstacle-avoidance switches the robot page shows, with their names there.
const AI_TOGGLES: [(u32, &str); 6] = [
    (AiFlags::OBSTACLE, "Avoid obstacles seen by the camera"),
    (AiFlags::FURNITURE, "Recognise furniture"),
    (AiFlags::FLUID, "Detect spilled liquids"),
    (AiFlags::PET, "Recognise pets"),
    (AiFlags::PET_AVOIDANCE, "Keep away from pet waste"),
    (AiFlags::OBSTACLE_PICTURE, "Keep pictures of obstacles"),
];

/// The value when it is one the app knows.
fn known<T>(o: Option<Open<T>>) -> Option<T> {
    match o {
        Some(Open::Known(v)) => Some(v),
        _ => None,
    }
}

fn home_status(s: &Status, has_dnd: bool) -> HomeStatus {
    let raw = |siid: u32, piid: u32| s.raw.get(&dreame_core::PropId::new(siid, piid)).and_then(|v| v.as_i64());
    let small = |v: i64| u8::try_from(v).ok();
    let p = phase::phase(s);
    let job = phase::job_active(s);
    let quiet_hours = match (s.dnd.enabled, s.dnd.start, s.dnd.end) {
        (Some(enabled), Some(start), Some(end)) if has_dnd || s.dnd.enabled.is_some() => {
            Some(QuietHours { enabled, start: start.into(), end: end.into() })
        }
        _ => None,
    };
    let ai = match &s.ai_obstacle {
        Some(AiObstacle::Bits(f)) => AI_TOGGLES.iter().map(|(bit, name)| AiToggle { bit: *bit, name: dreame_i18n::tr(name).into(), on: f.has(*bit) }).collect(),
        _ => Vec::new(),
    };
    HomeStatus {
        phase: p.into(),
        headline: phase::headline(s),
        error: phase::error_text(s),
        job_active: job,
        main_action: match phase::primary(s) {
            Primary::Start => MainAction::Start,
            Primary::Pause => MainAction::Pause,
            Primary::Resume => MainAction::Resume,
        },
        can_dock: !matches!(p, P::Docked | P::Returning | P::AtStation),
        battery: s.battery,
        charging: p == P::Docked,
        area_m2: s.cleaned_area_m2,
        minutes: s.cleaning_time_min,
        progress: s.cleaning_progress.filter(|p| (1..=100).contains(p)),
        mode: known(s.logical_cleaning_mode()).map(CleanMode::from_model),
        suction: known(s.suction).map(Suction::from_model),
        water: known(s.water_volume).map(Water::from_model),
        wetness: s.wetness.map(|w| w.clamp(1, 32) as u8),
        per_room_settings: raw(4, 26).map(|v| v != 0),
        settings: RobotSettings {
            carpet_boost: s.carpet_boost,
            resume_cleaning: s.resume_cleaning,
            self_clean: raw(4, 34).map(|v| v != 0),
            carpet_sensitivity: raw(4, 28).and_then(CarpetSensitivity::from_code).map(|c| match c {
                CarpetSensitivity::Low => crate::CarpetLevel::Low,
                CarpetSensitivity::Medium => crate::CarpetLevel::Medium,
                CarpetSensitivity::High => crate::CarpetLevel::High,
            }),
            drying_hours: raw(4, 40).and_then(small),
            volume: s.volume.and_then(small),
            child_lock: s.child_lock,
            quiet_hours,
            ai,
            auto_empty_every: raw(15, 2).and_then(small),
        },
        parts: s
            .consumables
            .iter()
            .map(|(c, life)| Part {
                id: c.siid(),
                name: phase::capitalise(dreame_i18n::tr(c.label())),
                percent_left: life.percent.map(|p| p.clamp(0, 100)),
                hours_left: life.time_left,
            })
            .collect(),
    }
}

/// One remote-control step, as the official joystick offers them. There is no reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Drive {
    Forward,
    Left,
    Right,
    /// Sent once, not repeated.
    TurnAround,
    Stop,
}

impl Drive {
    fn step(self) -> RemoteMove {
        match self {
            Self::Forward => RemoteMove::FORWARD,
            Self::Left => RemoteMove::LEFT,
            Self::Right => RemoteMove::RIGHT,
            Self::TurnAround => RemoteMove::TURN_AROUND,
            Self::Stop => RemoteMove::STOP,
        }
    }
}

/// Sets the language of the text this library returns: headlines, notices, part names, room
/// type names and job outcomes. Takes a locale tag such as `pt-BR`; any Portuguese gives
/// Brazilian Portuguese and anything else English. Returns the tag of the language chosen.
#[uniffi::export]
pub fn set_language(tag: String) -> String {
    let lang = dreame_i18n::Language::from_tag(&tag);
    dreame_i18n::set_language(lang);
    lang.tag().to_string()
}

/// How often to repeat a held [`Drive`] step, in milliseconds.
#[uniffi::export]
pub fn drive_repeat_ms() -> u64 {
    REMOTE_REPEAT_MS
}

/// An obstacle or pet the robot's camera recognised, from the live map.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ObstacleInfo {
    /// Pass to [`HomeRobot::obstacle_photo`].
    pub index: u32,
    /// The robot's obstacle class code.
    pub kind: u32,
    pub is_pet: bool,
    /// 0 to 1.
    pub confidence: f64,
    /// Position on the current map, when one is loaded.
    pub at: Option<CellPoint>,
    pub has_photo: bool,
}

#[uniffi::export(async_runtime = "tokio")]
impl HomeRobot {
    /// One remote-control step. Repeat a held direction every [`drive_repeat_ms`] and send
    /// [`Drive::Stop`] on release; the robot stops by itself about a second after the last
    /// step and leaves remote control within 10 s. `first` plays the robot's voice prompt.
    /// A docked robot leaves the dock on the first step instead of carrying it out.
    pub async fn drive(&self, step: Drive, first: bool) -> Result<()> {
        self.inner.robot().remote_move(step.step(), first).await?;
        Ok(())
    }

    /// The obstacles and pets in the live map. Read-only.
    pub async fn obstacles(&self) -> Result<Vec<ObstacleInfo>> {
        let list = self.inner.robot().obstacles().await?;
        let map = self.inner.map();
        let out = list
            .iter()
            .enumerate()
            .map(|(i, o)| ObstacleInfo {
                index: i as u32,
                kind: o.kind,
                is_pet: o.is_pet(),
                confidence: o.confidence,
                at: map.as_ref().map(|m| m.to_cell(Point::new(o.x, o.y))),
                has_photo: o.photo.as_ref().is_some_and(|p| !p.upload_failed()),
            })
            .collect();
        *self.inner.obstacles.lock().expect("obstacles lock") = list;
        Ok(out)
    }

    /// The photo of obstacle `index` from the last [`HomeRobot::obstacles`], as JPEG bytes.
    /// Read-only.
    pub async fn obstacle_photo(&self, index: u32) -> Result<Vec<u8>> {
        let photo = self
            .inner
            .obstacles
            .lock()
            .expect("obstacles lock")
            .get(index as usize)
            .and_then(|o| o.photo.clone())
            .ok_or_else(|| DreameError::Other { detail: format!("obstacle {index} has no photo") })?;
        Ok(self.inner.robot().obstacle_photo(&photo).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_core::{PropId, RawValue};

    fn status(values: &[((u32, u32), i64)]) -> Status {
        let mut s = Status::default();
        for ((siid, piid), v) in values {
            s.update(PropId::new(*siid, *piid), &RawValue::Int(*v));
        }
        s
    }

    #[test]
    fn a_room_job_shows_pause_and_its_numbers() {
        let s = status(&[((2, 1), 1), ((4, 7), 3), ((3, 1), 80), ((4, 3), 12), ((4, 2), 20), ((4, 4), 2)]);
        let h = home_status(&s, false);
        assert_eq!(h.phase, Phase::Cleaning);
        assert!(h.job_active);
        assert_eq!(h.main_action, MainAction::Pause);
        assert_eq!(h.headline, "Cleaning rooms");
        assert_eq!((h.area_m2, h.minutes, h.battery), (Some(12), Some(20), Some(80)));
        assert_eq!(h.suction, Some(Suction::Strong));
        assert!(h.can_dock);
    }

    #[test]
    fn docked_robot_cannot_dock_and_lists_its_settings() {
        let s = status(&[((2, 1), 6), ((4, 7), 0), ((3, 1), 64), ((4, 26), 1), ((4, 40), 3)]);
        let h = home_status(&s, false);
        assert_eq!(h.phase, Phase::Docked);
        assert!(h.charging);
        assert!(!h.can_dock);
        assert_eq!(h.main_action, MainAction::Start);
        assert_eq!(h.per_room_settings, Some(true));
        assert_eq!(h.settings.drying_hours, Some(3));
        assert_eq!(h.settings.quiet_hours, None);
    }

    #[test]
    fn a_schedule_edit_keeps_the_undecoded_bits() {
        let t = |h| TimeOfDay { hour: h, minute: 0 };
        let mut before = RoomTask::new(10, 1, 1, 2, 16);
        before.raw ^= 1 << 13; // an undecoded bit unlike the samples
        let rooms = [
            ScheduleRoom { room: 10, passes: 3, suction: 9, mode: 0, wetness: 40 },
            ScheduleRoom { room: 12, passes: 1, suction: 2, mode: 1, wetness: 25 },
        ];
        let tasks = draft_tasks(&rooms, &[before]).unwrap();
        assert_eq!((tasks[0].passes, tasks[0].suction, tasks[0].mode, tasks[0].wetness), (3, 3, 0, 32));
        assert_eq!(tasks[0].encode() & (1 << 13), before.raw & (1 << 13));
        assert_eq!(tasks[1].room, 12);
        assert!(draft_tasks(&[ScheduleRoom { room: 300, ..rooms[0] }], &[]).is_err());

        let fri = [false, false, false, false, true, false, false];
        let slot = ScheduleSlot::new_rooms(1, t(9).try_into().unwrap(), &dreame_model::codec::schedule::mask_from_days(&fri), &tasks);
        let s = schedule_record(0, &slot);
        assert_eq!(s.days, fri.to_vec());
        assert_eq!(s.time, t(9));
        assert_eq!(s.rooms.unwrap().len(), 2);
        assert!(days_array(&[true; 6]).is_err());
    }

    #[test]
    fn modes_and_levels_map_both_ways() {
        for m in [CleanMode::Vacuum, CleanMode::Mop, CleanMode::VacuumAndMop, CleanMode::VacuumThenMop] {
            assert_eq!(CleanMode::from_model(m.to_model()), m);
        }
        for l in [Suction::Quiet, Suction::Standard, Suction::Strong, Suction::Max] {
            assert_eq!(Suction::from_model(l.to_model()), l);
        }
        for w in [Water::Low, Water::Medium, Water::High] {
            assert_eq!(Water::from_model(w.to_model()), w);
        }
    }
}
