//! One robot, typed: a status snapshot, capability discovery, a live event stream, and command
//! methods that check capabilities before they send.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! use dreame::{Robot, Session};
//! use dreame::session::pick_device;
//! use std::sync::Arc;
//!
//! let client = Arc::new(Session::new().client().await?);
//! let device = pick_device(&client, None).await?;
//! let mut robot = Robot::cloud(client, device);
//! robot.discover(&Default::default()).await?; // optional; enables capability checks
//! let status = robot.refresh().await?;
//! println!("battery {:?}", status.battery);
//! let mut events = robot.events()?;
//! while let Some(e) = events.next().await {
//!     println!("{e:?}");
//! }
//! # Ok(()) }
//! ```
//!
//! Every command method is a write to the robot. The library sends what the caller asks for;
//! asking the user first is the app's job (CLAUDE.md).

use crate::cloud::CloudRobot;
use crate::control::{self, CleanOptions};
use anyhow::{anyhow, bail, Result};
use dreame_core::{Error, PropId, PropResult, RawValue};
use dreame_map::model::Rect;
use dreame_map::{Frame, Map};
use dreame_model::codec::consumables::Consumable;
use dreame_model::codec::station::StationOp;
use dreame_model::codec::HhMm;
use dreame_model::enums::{CleaningMode, SuctionLevel, WaterVolume};
use dreame_model::{commands, Capabilities, Command, DecodeContext, DiscoverOptions, Feature, Profile, Status};
pub use dreame_mqtt::ConnectionState;
use dreame_mqtt::{MqttClient, MqttConfig, MqttHandle, Push, PushStream};
use serde_json::Value;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Property 6.1: map frames pushed while the robot works. Large, so kept out of [`Status`].
pub const MAP_FRAMES: PropId = PropId { siid: 6, piid: 1 };

/// What a [`Robot`] needs from its connection. [`CloudRobot`] is the real one; tests use fakes.
/// The futures are `Send`, so a robot can be used from any tokio task.
pub trait Link: Send + Sync {
    fn model(&self) -> &str;
    fn firmware(&self) -> &str;
    /// Values from the cloud cache. Cheap; may be minutes old.
    fn cached(&self, ids: &[PropId]) -> impl Future<Output = Result<Vec<(PropId, RawValue)>>> + Send;
    /// Values read from the robot now.
    fn read(&self, ids: &[PropId]) -> impl Future<Output = Result<Vec<PropResult>>> + Send;
    /// Sends one command and returns the raw result.
    fn send(&self, cmd: &Command) -> impl Future<Output = Result<Value>> + Send;
}

impl Link for CloudRobot {
    fn model(&self) -> &str {
        &self.device.model
    }
    fn firmware(&self) -> &str {
        &self.device.firmware
    }
    async fn cached(&self, ids: &[PropId]) -> Result<Vec<(PropId, RawValue)>> {
        Ok(self.client.cached_properties(&self.device.did, ids).await?.into_iter().map(|p| (p.id, p.value)).collect())
    }
    async fn read(&self, ids: &[PropId]) -> Result<Vec<PropResult>> {
        Ok(self.client.get_properties(&self.device, ids).await?)
    }
    async fn send(&self, cmd: &Command) -> Result<Value> {
        control::send(&self.client, &self.device, cmd).await
    }
}

/// A robot: the connection, its profile, what discovery found, and the latest status.
pub struct Robot<L = CloudRobot> {
    link: L,
    profile: Profile,
    caps: Option<Capabilities>,
    status: Arc<Mutex<Status>>,
}

impl Robot<CloudRobot> {
    /// A robot reached through the Dreamehome cloud. No I/O happens until a method is called.
    pub fn cloud(client: Arc<dreame_cloud::CloudClient>, device: dreame_core::DeviceInfo) -> Self {
        Self::new(CloudRobot::new(client, device))
    }

    /// The cloud connection, for the map edit flow ([`crate::mapedit`]) and other direct use.
    pub fn cloud_link(&self) -> &CloudRobot {
        &self.link
    }

    /// The current saved map, with the live frame's per-room settings. Read-only.
    pub async fn saved_map(&self) -> Result<Map> {
        Ok(crate::mapedit::read_saved(&self.link).await?.map)
    }

    /// Saved map `map_id` (one floor). Only the current map has per-room settings. Read-only.
    pub async fn saved_map_by_id(&self, map_id: i64) -> Result<Map> {
        Ok(crate::mapedit::read_saved_map(&self.link, Some(map_id)).await?.map)
    }

    /// Every saved map (one per floor) with the list's current map id. Read-only.
    pub async fn saved_maps(&self) -> Result<crate::maplist::SavedMaps> {
        crate::mapedit::read_saved_maps(&self.link).await
    }

    /// The obstacles and pets the camera recognised, from the live map. Read-only.
    pub async fn obstacles(&self) -> Result<Vec<dreame_map::obstacle::Obstacle>> {
        crate::camera::obstacles(&self.link).await
    }

    /// One obstacle photo as JPEG bytes. Read-only.
    pub async fn obstacle_photo(&self, p: &dreame_map::obstacle::PhotoRef) -> Result<Vec<u8>> {
        crate::camera::photo(&self.link, p).await
    }

    /// Opens the live event stream over the cloud's MQTT broker. Pushed property values also
    /// update [`Robot::status`]. The stream only listens; it never sends to the robot.
    pub fn events(&self) -> Result<Events> {
        let cfg = MqttConfig::from_cloud(&self.link.device, &self.link.client)?;
        let (handle, pushes) = MqttClient::connect(cfg, self.link.client.clone());
        let conn = handle.state();
        Ok(Events { handle, pushes, conn, status: self.status.clone(), queue: VecDeque::new() })
    }
}

impl<L: Link> Robot<L> {
    pub fn new(link: L) -> Self {
        let profile = Profile::for_model(link.model());
        Self { link, profile, caps: None, status: Arc::new(Mutex::new(Status::new(DecodeContext::default()))) }
    }

    pub fn link(&self) -> &L {
        &self.link
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// What [`Robot::discover`] found, if it ran.
    pub fn capabilities(&self) -> Option<&Capabilities> {
        self.caps.as_ref()
    }

    /// Whether the robot has a feature: `Some` after discovery, `None` before.
    pub fn has(&self, f: Feature) -> Option<bool> {
        self.caps.as_ref().map(|c| c.feature(f))
    }

    /// Reads every profile property from the robot to learn which exist (read-only; about one
    /// relayed call per 15 properties). Afterwards values decode with the robot's own context,
    /// and command methods refuse features the robot lacks.
    pub async fn discover(&mut self, opts: &DiscoverOptions) -> Result<&Capabilities> {
        let reader = LinkReader(&self.link);
        let mut caps = dreame_model::discover(&reader, &self.profile, self.link.model(), self.link.firmware(), opts).await?;
        caps.derive();
        Ok(self.set_capabilities(caps))
    }

    /// Uses capabilities found earlier, for example rebuilt with
    /// [`Capabilities::from_known`] from a saved result, in place of [`Robot::discover`].
    pub fn set_capabilities(&mut self, caps: Capabilities) -> &Capabilities {
        let ctx = caps.decode_context();
        {
            let mut s = self.status.lock().expect("status lock");
            let values = caps.values();
            *s = Status::from_values(s.raw.iter().chain(values.iter()), ctx);
        }
        self.caps.insert(caps)
    }

    /// Reads the status from the cloud cache alone: one quick call, where [`Robot::refresh`]
    /// also asks the robot itself, which takes seconds. The cache can lag the robot a little.
    /// Read-only.
    pub async fn refresh_cached(&self) -> Result<Status> {
        let values = self.link.cached(&self.profile.prop_ids()).await?;
        Ok(self.replace_status(&values))
    }

    fn replace_status(&self, values: &[(PropId, RawValue)]) -> Status {
        let mut s = self.status.lock().expect("status lock");
        let mut fresh = Status::new(s.ctx);
        for (id, v) in values.iter().filter(|(id, _)| *id != MAP_FRAMES) {
            fresh.update(*id, v);
        }
        *s = fresh;
        s.clone()
    }

    /// The latest status: from [`Robot::refresh`], then updated by pushed values while an
    /// [`Events`] stream runs.
    pub fn status(&self) -> Status {
        self.status.lock().expect("status lock").clone()
    }

    /// Reads a fresh status: every profile property from the cloud cache, then the core services
    /// (siid 2 to 4) live from the robot, which win over the cache. Read-only.
    pub async fn refresh(&self) -> Result<Status> {
        let ids = self.profile.prop_ids();
        let mut values: Vec<(PropId, RawValue)> = self.link.cached(&ids).await?;
        let live: Vec<PropId> = ids
            .iter()
            .copied()
            .filter(|id| matches!(id.siid, 2..=4))
            .filter(|id| self.profile.prop(*id).is_none_or(|d| !d.access.documented_unreadable()))
            .filter(|id| self.caps.as_ref().is_none_or(|c| c.present(*id)))
            .collect();
        for r in self.link.read(&live).await? {
            if let (0, Some(v)) = (r.code, r.value) {
                values.push((r.id, v));
            }
        }
        Ok(self.replace_status(&values))
    }

    /// Reads the given properties live from the robot into the kept status and returns it.
    /// Read-only. Use it for settings outside the core services that [`Robot::refresh`] reads
    /// live, or to read a value back right after changing it.
    pub async fn read(&self, ids: &[PropId]) -> Result<Status> {
        let results = self.link.read(ids).await?;
        let mut s = self.status.lock().expect("status lock");
        for r in results {
            if let (0, Some(v)) = (r.code, r.value) {
                if r.id != MAP_FRAMES {
                    s.update(r.id, &v);
                }
            }
        }
        Ok(s.clone())
    }

    // ---- commands ---------------------------------------------------------------------------

    /// Refuses a feature that discovery showed the robot does not have. Before discovery
    /// nothing is refused.
    fn require(&self, f: Feature) -> Result<()> {
        match self.has(f) {
            Some(false) => Err(Error::Unsupported(format!("this robot has no {f:?}")).into()),
            _ => Ok(()),
        }
    }

    /// Sends one command and checks the robot accepted it: an action's `code`, or every
    /// property write's code. Returns the raw result.
    pub async fn send(&self, cmd: &Command) -> Result<Value> {
        let r = self.link.send(cmd).await?;
        let refused = match cmd {
            Command::Action { .. } => control::result_code(&r).is_some_and(|c| c != 0),
            Command::SetProperty { .. } => {
                r.as_array().is_some_and(|a| a.iter().any(|p| p.get("code").and_then(Value::as_i64) != Some(0)))
            }
        };
        if refused {
            bail!("the robot did not accept {}: {r}", control::describe(cmd));
        }
        Ok(r)
    }

    /// Changes one setting: refuses a property discovery found missing, builds the writes from
    /// the kept status (see [`crate::Setting::commands`]), sends them, and returns one result per
    /// write. A write to the robot: ask the user first.
    pub async fn set(&self, setting: &crate::Setting) -> Result<Vec<Value>> {
        if let Some(caps) = &self.caps {
            if let Some(id) = setting.properties().into_iter().find(|id| !caps.present(*id)) {
                return Err(Error::Unsupported(format!("this robot has no property {id} ({})", setting.name())).into());
            }
        }
        let cmds = setting.commands(&self.status())?;
        let mut out = Vec::new();
        for c in &cmds {
            out.push(self.send(c).await?);
        }
        Ok(out)
    }

    /// Starts or resumes a full clean.
    pub async fn start(&self) -> Result<Value> {
        self.send(&commands::start()).await
    }
    pub async fn pause(&self) -> Result<Value> {
        self.send(&commands::pause()).await
    }
    pub async fn stop(&self) -> Result<Value> {
        self.send(&commands::stop()).await
    }
    /// Sends the robot to its dock; this ends the current job.
    pub async fn dock(&self) -> Result<Value> {
        self.send(&commands::dock()).await
    }
    /// Makes the robot announce where it is.
    pub async fn locate(&self) -> Result<Value> {
        self.send(&commands::locate()).await
    }
    pub async fn clear_warning(&self) -> Result<Value> {
        self.send(&commands::clear_warning()).await
    }

    /// One remote-control step (4.15). The robot moves for about a second, so repeat a held
    /// direction every [`commands::REMOTE_REPEAT_MS`] and send [`commands::RemoteMove::STOP`]
    /// on release. `prompt` plays the robot's voice prompt; set it on the first step only.
    pub async fn remote_move(&self, m: commands::RemoteMove, prompt: bool) -> Result<Value> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| (d.subsec_millis() % 1000) as u16);
        self.send(&commands::remote_move(m, prompt, nonce)).await
    }

    /// Asks the robot to upload one obstacle photo to the cloud (4.a6), for an obstacle whose
    /// photo is not there yet.
    pub async fn upload_obstacle_photo(&self, p: &dreame_map::obstacle::PhotoRef) -> Result<Value> {
        self.send(&commands::upload_obstacle_photo(&p.file, &p.pwd, &p.object())).await
    }

    /// Cleans rooms of `map` in the given order, with each room's stored settings unless
    /// `opts` overrides them.
    pub async fn clean_rooms(&self, map: &Map, rooms: &[u32], opts: CleanOptions) -> Result<Value> {
        self.send(&control::clean_rooms(map, rooms, opts)?).await
    }

    /// Cleans one rectangle of `map` (world mm).
    pub async fn clean_zone(&self, map: &Map, zone: Rect, opts: CleanOptions) -> Result<Value> {
        self.send(&control::clean_zone(map, zone, opts)?).await
    }

    pub async fn set_suction(&self, level: SuctionLevel) -> Result<Value> {
        self.send(&commands::set_suction(level)).await
    }

    /// The three-step water volume, for robots without the 1 to 32 wetness scale.
    pub async fn set_water_volume(&self, level: WaterVolume) -> Result<Value> {
        self.send(&commands::set_water_volume(level)).await
    }

    /// Mop wetness 1 to 32, on robots that have it.
    pub async fn set_wetness(&self, level: u8) -> Result<Value> {
        self.require(Feature::Wetness)?;
        self.send(&commands::set_wetness(level)?).await
    }

    /// Sweep, mop, or both. The mode shares property 4.23 with other settings, so the current
    /// value must be known: call [`Robot::refresh`] first.
    pub async fn set_mop_mode(&self, mode: CleaningMode) -> Result<Value> {
        let (current, polarity) = {
            let s = self.status.lock().expect("status lock");
            (s.cleaning_mode, s.ctx.polarity)
        };
        let current = current.ok_or_else(|| anyhow!("the cleaning mode (4.23) is not known; refresh first"))?;
        self.send(&commands::set_mop_mode(&current, mode, polarity)?).await
    }

    /// The do-not-disturb window: on or off, and its start and end.
    pub async fn set_dnd(&self, enabled: bool, start: HhMm, end: HhMm) -> Result<Vec<Value>> {
        self.require(Feature::Dnd)?;
        let mut out = Vec::new();
        for c in commands::set_dnd(enabled, start, end) {
            out.push(self.send(&c).await?);
        }
        Ok(out)
    }

    pub async fn set_dnd_enabled(&self, enabled: bool) -> Result<Value> {
        self.require(Feature::Dnd)?;
        self.send(&commands::set_dnd_enabled(enabled)).await
    }

    /// Resets a consumable's life counter, after it was replaced or cleaned.
    pub async fn reset_consumable(&self, c: Consumable) -> Result<Value> {
        self.send(&commands::reset_consumable(c)).await
    }

    /// A self-wash station operation, such as washing the mop.
    pub async fn station(&self, op: StationOp) -> Result<Value> {
        self.require(Feature::SelfWashBase)?;
        let build = self.caps.as_ref().and_then(|c| c.fw_build);
        self.send(&commands::station(op, build)).await
    }

    /// Empties the robot's bin into the station.
    pub async fn start_auto_empty(&self) -> Result<Value> {
        self.require(Feature::AutoEmptyBase)?;
        self.send(&commands::start_auto_empty()).await
    }

    /// The schedules, read live from the robot (8.2). Read-only.
    pub async fn schedules(&self) -> Result<Vec<dreame_model::codec::schedule::ScheduleSlot>> {
        let id = PropId::new(8, 2);
        let s = self.read(&[id]).await?;
        match s.raw.get(&id) {
            Some(v) => Ok(dreame_model::codec::schedule::decode(v)?),
            None => Ok(Vec::new()),
        }
    }

    /// Replaces the whole schedule list (8.2). A write: ask first.
    pub async fn set_schedules(&self, slots: &[dreame_model::codec::schedule::ScheduleSlot]) -> Result<Value> {
        self.send(&commands::set_schedules(slots)).await
    }
}

/// Adapts a [`Link`] to discovery's reader.
struct LinkReader<'a, L>(&'a L);

impl<L: Link> dreame_model::PropertyReader for LinkReader<'_, L> {
    async fn read(&self, ids: &[PropId]) -> dreame_core::Result<Vec<PropResult>> {
        self.0.read(ids).await.map_err(|e| Error::Http(format!("{e:#}")))
    }
}

/// One thing the robot reported.
#[derive(Debug, Clone)]
pub enum Event {
    /// Properties changed. `status` is the snapshot after the change.
    Status { changed: Vec<PropId>, status: Box<Status> },
    /// A map frame pushed on 6.1 while the robot works, decoded. Feed it to
    /// [`dreame_map::merge::MapState`] to follow the map and the robot's path.
    MapFrame(Box<Frame>),
    /// A robot event (`event_occured`), such as a finished job; `arguments` are `(piid, value)`.
    RobotEvent { siid: u32, eiid: u32, arguments: Vec<(u32, RawValue)> },
    /// The MQTT connection changed state.
    Connection(ConnectionState),
}

/// The events of one push, applied to the status. Pure, so it is tested without a broker.
pub fn apply_push(status: &mut Status, push: Push) -> Vec<Event> {
    match push {
        Push::PropertiesChanged(changes) => {
            let mut out = Vec::new();
            let mut changed = Vec::new();
            for (id, v) in changes {
                if id == MAP_FRAMES {
                    let text = match v.to_json() {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    if let Ok(f) = Frame::decode(&text, None, None) {
                        out.push(Event::MapFrame(Box::new(f)));
                    }
                } else {
                    status.update(id, &v);
                    changed.push(id);
                }
            }
            if !changed.is_empty() {
                out.insert(0, Event::Status { changed, status: Box::new(status.clone()) });
            }
            out
        }
        Push::Event { siid, eiid, arguments } => vec![Event::RobotEvent { siid, eiid, arguments }],
        // Network reports, OTA progress and unknown messages are not part of the typed stream.
        Push::Props(_) | Push::OtcInfo { .. } | Push::Other(_) => vec![],
    }
}

/// The live event stream of a cloud robot. Stop it with [`Events::stop`]; dropping it also ends
/// the connection.
pub struct Events {
    handle: MqttHandle,
    pushes: PushStream,
    conn: watch::Receiver<ConnectionState>,
    status: Arc<Mutex<Status>>,
    queue: VecDeque<Event>,
}

impl Events {
    /// The next event. `None` once the connection has stopped for good.
    ///
    /// Cancel-safe: dropping the future (for example in `tokio::select!`) loses no event.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(e) = self.queue.pop_front() {
                return Some(e);
            }
            tokio::select! {
                changed = self.conn.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                    let s = self.conn.borrow().clone();
                    return Some(Event::Connection(s));
                }
                push = self.pushes.recv() => {
                    let push = push?;
                    let mut s = self.status.lock().expect("status lock");
                    self.queue.extend(apply_push(&mut s, push));
                }
            }
        }
    }

    /// Closes the connection.
    pub async fn stop(self) {
        self.handle.stop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_model::enums::{Open, State};
    use serde_json::json;

    /// A robot that answers reads from fixed tables and records what it is sent.
    struct Fake {
        cached: Vec<(PropId, RawValue)>,
        live: Vec<(PropId, RawValue)>,
        reply: Value,
        sent: Mutex<Vec<Command>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                // Cache: battery 50, state 1 (sweeping); live: battery 80.
                cached: vec![(PropId::new(3, 1), RawValue::Int(50)), (PropId::new(2, 1), RawValue::Int(1))],
                live: vec![(PropId::new(3, 1), RawValue::Int(80))],
                reply: json!({"code": 0, "out": []}),
                sent: Mutex::new(vec![]),
            }
        }
    }

    impl Link for Fake {
        fn model(&self) -> &str {
            "dreame.vacuum.r2579a"
        }
        fn firmware(&self) -> &str {
            "4.3.9_1664"
        }
        async fn cached(&self, ids: &[PropId]) -> Result<Vec<(PropId, RawValue)>> {
            Ok(self.cached.iter().filter(|(id, _)| ids.contains(id)).cloned().collect())
        }
        async fn read(&self, ids: &[PropId]) -> Result<Vec<PropResult>> {
            Ok(ids
                .iter()
                .map(|id| match self.live.iter().find(|(i, _)| i == id) {
                    Some((_, v)) => PropResult { id: *id, code: 0, value: Some(v.clone()) },
                    None => PropResult { id: *id, code: -1, value: None },
                })
                .collect())
        }
        async fn send(&self, cmd: &Command) -> Result<Value> {
            self.sent.lock().unwrap().push(cmd.clone());
            Ok(match cmd {
                Command::Action { .. } => self.reply.clone(),
                Command::SetProperty { id, .. } => json!([{"id": id.to_string(), "code": self.reply["code"]}]),
            })
        }
    }

    #[tokio::test]
    async fn refresh_prefers_live_values_over_the_cache() {
        let robot = Robot::new(Fake::new());
        let s = robot.refresh().await.unwrap();
        assert_eq!(s.battery, Some(80), "live 3.1 wins over the cached 50");
        assert_eq!(s.state, Some(Open::Known(State::Sweeping)), "2.1 from the cache");
        assert_eq!(robot.status().battery, Some(80), "the snapshot is kept");
    }

    #[tokio::test]
    async fn read_merges_live_values_into_the_status() {
        let robot = Robot::new(Fake { live: vec![(PropId::new(15, 2), RawValue::Int(3))], ..Fake::new() });
        robot.refresh().await.unwrap();
        let s = robot.read(&[PropId::new(15, 2), PropId::new(15, 9)]).await.unwrap();
        assert_eq!(s.raw.get(&PropId::new(15, 2)), Some(&RawValue::Int(3)));
        assert!(!s.raw.contains_key(&PropId::new(15, 9)), "unreadable properties are skipped");
        assert_eq!(s.state, Some(Open::Known(State::Sweeping)), "the refreshed values stay");
    }

    #[tokio::test]
    async fn commands_check_the_result_and_capabilities() {
        let robot = Robot::new(Fake::new());
        robot.dock().await.unwrap();
        assert_eq!(robot.link().sent.lock().unwrap()[0], commands::dock());
        robot.set_suction(SuctionLevel::Strong).await.unwrap();
        // Before discovery nothing is refused.
        robot.set_dnd_enabled(true).await.unwrap();

        let refusing = Robot::new(Fake { reply: json!({"code": 5}), ..Fake::new() });
        assert!(refusing.pause().await.is_err());
        assert!(refusing.set_suction(SuctionLevel::Quiet).await.is_err(), "a property write with code 5");

        // After discovery a missing feature is refused before anything is sent.
        let mut robot = Robot::new(Fake::new());
        let mut caps = Capabilities::default();
        caps.features.insert(Feature::Dnd, false);
        caps.features.insert(Feature::SelfWashBase, false);
        robot.caps = Some(caps);
        let e = robot.set_dnd_enabled(true).await.unwrap_err();
        assert!(matches!(e.downcast_ref::<Error>(), Some(Error::Unsupported(_))), "{e}");
        assert!(robot.station(StationOp::ReturnAndWash).await.is_err());
        assert!(robot.link().sent.lock().unwrap().is_empty());
        // The mop mode needs 4.23 from a refresh.
        assert!(robot.set_mop_mode(CleaningMode::Sweeping).await.is_err());

        // A setting whose property discovery did not find is refused before sending.
        let e = robot.set(&crate::Setting::ChildLock(true)).await.unwrap_err();
        assert!(matches!(e.downcast_ref::<Error>(), Some(Error::Unsupported(_))), "{e}");
        assert!(robot.link().sent.lock().unwrap().is_empty());
        // Before discovery, a setting is built from the kept status and sent.
        let robot = Robot::new(Fake::new());
        robot.set(&crate::Setting::Volume(30)).await.unwrap();
        assert_eq!(robot.link().sent.lock().unwrap()[0], Command::write(PropId::new(7, 1), RawValue::Int(30)));
    }

    #[test]
    fn pushes_update_the_status_and_split_out_map_frames() {
        let mut s = Status::new(DecodeContext::default());
        let events = apply_push(
            &mut s,
            Push::PropertiesChanged(vec![(PropId::new(3, 1), RawValue::Int(42)), (MAP_FRAMES, RawValue::Str("not a frame".into()))]),
        );
        assert_eq!(s.battery, Some(42));
        assert!(!s.raw.contains_key(&MAP_FRAMES), "map frames stay out of the status");
        let [Event::Status { changed, status }] = &events[..] else { panic!("{events:?}") };
        assert_eq!(changed, &[PropId::new(3, 1)]);
        assert_eq!(status.battery, Some(42));
        let e = apply_push(&mut s, Push::Event { siid: 4, eiid: 1, arguments: vec![(1, RawValue::Int(3))] });
        assert!(matches!(e[..], [Event::RobotEvent { siid: 4, eiid: 1, .. }]));
        assert!(apply_push(&mut s, Push::Other(json!({}))).is_empty());
    }
}
