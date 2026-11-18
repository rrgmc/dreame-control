//! UniFFI bindings over the `dreame` facade, for the Android app (Kotlin) and later iOS (Swift).
//!
//! The surface is small on purpose: log in or resume a session, list devices, and for one robot
//! read its status, follow its events, send the basic commands, read and change its settings, and
//! render the saved map. Values cross the boundary as plain records; ids and names are never
//! logged here.
//!
//! Every command method is a write to the robot. The app must ask the user first (CLAUDE.md).
//!
//! The lab app uses the items in this file; the production app uses [`app`].

use dreame::session::FileTokenStore;
use dreame::robot::ConnectionState;
use dreame::{settings, CloudRobot, Event, Robot, Setting};
use dreame_model::codec::HhMm;
use dreame_cloud::{CloudClient, CloudConfig, Region};
use dreame_map::render::{render_png, RenderOptions};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

pub mod app;

/// What went wrong, by kind. The message never contains credentials or ids.
#[derive(Debug, uniffi::Error)]
pub enum DreameError {
    /// Login failed or the session expired; log in again.
    Auth { detail: String },
    /// The cloud or the robot could not be reached.
    Network { detail: String },
    /// The robot does not have this feature.
    Unsupported { detail: String },
    Other { detail: String },
}

impl std::fmt::Display for DreameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, m) = match self {
            Self::Auth { detail } => ("auth", detail),
            Self::Network { detail } => ("network", detail),
            Self::Unsupported { detail } => ("unsupported", detail),
            Self::Other { detail } => ("error", detail),
        };
        write!(f, "{kind}: {m}")
    }
}

impl std::error::Error for DreameError {}

impl From<anyhow::Error> for DreameError {
    fn from(e: anyhow::Error) -> Self {
        let detail = format!("{e:#}");
        match e.downcast_ref::<dreame_core::Error>() {
            Some(dreame_core::Error::Auth(_)) => Self::Auth { detail },
            Some(dreame_core::Error::Http(_)) => Self::Network { detail },
            Some(dreame_core::Error::Unsupported(_)) => Self::Unsupported { detail },
            _ => Self::Other { detail },
        }
    }
}

impl From<dreame_core::Error> for DreameError {
    fn from(e: dreame_core::Error) -> Self {
        anyhow::Error::from(e).into()
    }
}

type Result<T> = std::result::Result<T, DreameError>;

/// One device on the account.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Device {
    pub did: String,
    pub name: String,
    pub model: String,
    pub online: bool,
    pub is_vacuum: bool,
}

/// What the robot is doing, as text labels and numbers. `None` means not known yet.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct RobotStatus {
    pub state: Option<String>,
    pub task: Option<String>,
    pub battery: Option<i64>,
    pub charging: Option<String>,
    pub error: Option<String>,
    pub suction: Option<String>,
    pub last_job_minutes: Option<i64>,
    pub last_job_area_m2: Option<i64>,
}

impl From<&dreame_model::Status> for RobotStatus {
    fn from(s: &dreame_model::Status) -> Self {
        Self {
            state: s.state.map(|x| x.to_string()),
            task: s.task_status.map(|x| x.to_string()),
            battery: s.battery,
            charging: s.charging.map(|x| x.to_string()),
            error: s.error.map(|x| x.to_string()),
            suction: s.suction.map(|x| x.to_string()),
            last_job_minutes: s.cleaning_time_min,
            last_job_area_m2: s.cleaned_area_m2,
        }
    }
}

/// A time of day, 00:00 to 23:59.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl From<HhMm> for TimeOfDay {
    fn from(t: HhMm) -> Self {
        Self { hour: t.hour, minute: t.minute }
    }
}

impl TryFrom<TimeOfDay> for HhMm {
    type Error = DreameError;
    fn try_from(t: TimeOfDay) -> Result<Self> {
        Ok(HhMm::new(t.hour, t.minute)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CarpetLevel {
    Low,
    Medium,
    High,
}

/// One setting and its value. The same type lists the current values and asks for a change.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SettingValue {
    /// Speaker volume, 0 to 100.
    Volume { level: u8 },
    ChildLock { on: bool },
    /// More suction on carpets.
    CarpetBoost { on: bool },
    /// Resume the job after recharging.
    ResumeCleaning { on: bool },
    /// Clean each room with its own settings.
    CustomizedCleaning { on: bool },
    /// Wash the mop during the job.
    SelfClean { on: bool },
    /// Mop drying time, 1 to 12 hours.
    DryingTime { hours: u8 },
    CarpetSensitivity { level: CarpetLevel },
    /// Cleans between automatic bin emptying, 1 to 10.
    AutoEmptyFrequency { cleans: u8 },
    DoNotDisturb { enabled: bool, start: TimeOfDay, end: TimeOfDay },
    OffPeakCharging { enabled: bool, start: TimeOfDay, end: TimeOfDay },
    /// One AI detection switch; `bit` comes from the listing and names it.
    AiSwitch { bit: u32, on: bool },
    /// One entry of the robot's settings store, by key. `-1` usually means off.
    StoreEntry { key: String, value: i64 },
}

impl From<Setting> for SettingValue {
    fn from(s: Setting) -> Self {
        use dreame_model::enums::CarpetSensitivity as C;
        match s {
            Setting::Volume(level) => Self::Volume { level },
            Setting::ChildLock(on) => Self::ChildLock { on },
            Setting::CarpetBoost(on) => Self::CarpetBoost { on },
            Setting::ResumeCleaning(on) => Self::ResumeCleaning { on },
            Setting::CustomizedCleaning(on) => Self::CustomizedCleaning { on },
            Setting::SelfClean(on) => Self::SelfClean { on },
            Setting::DryingTimeHours(hours) => Self::DryingTime { hours },
            Setting::CarpetSensitivity(c) => Self::CarpetSensitivity {
                level: match c {
                    C::Low => CarpetLevel::Low,
                    C::Medium => CarpetLevel::Medium,
                    C::High => CarpetLevel::High,
                },
            },
            Setting::AutoEmptyFrequency(cleans) => Self::AutoEmptyFrequency { cleans },
            Setting::Dnd { enabled, start, end } => Self::DoNotDisturb { enabled, start: start.into(), end: end.into() },
            Setting::OffPeak { enabled, start, end } => Self::OffPeakCharging { enabled, start: start.into(), end: end.into() },
            Setting::AiFlag { bit, on } => Self::AiSwitch { bit, on },
            Setting::AutoSwitch { key, value } => Self::StoreEntry { key, value },
        }
    }
}

impl TryFrom<SettingValue> for Setting {
    type Error = DreameError;
    fn try_from(v: SettingValue) -> Result<Self> {
        use dreame_model::enums::CarpetSensitivity as C;
        use SettingValue as V;
        Ok(match v {
            V::Volume { level } => Self::Volume(level),
            V::ChildLock { on } => Self::ChildLock(on),
            V::CarpetBoost { on } => Self::CarpetBoost(on),
            V::ResumeCleaning { on } => Self::ResumeCleaning(on),
            V::CustomizedCleaning { on } => Self::CustomizedCleaning(on),
            V::SelfClean { on } => Self::SelfClean(on),
            V::DryingTime { hours } => Self::DryingTimeHours(hours),
            V::CarpetSensitivity { level } => Self::CarpetSensitivity(match level {
                CarpetLevel::Low => C::Low,
                CarpetLevel::Medium => C::Medium,
                CarpetLevel::High => C::High,
            }),
            V::AutoEmptyFrequency { cleans } => Self::AutoEmptyFrequency(cleans),
            V::DoNotDisturb { enabled, start, end } => Self::Dnd { enabled, start: start.try_into()?, end: end.try_into()? },
            V::OffPeakCharging { enabled, start, end } => Self::OffPeak { enabled, start: start.try_into()?, end: end.try_into()? },
            V::AiSwitch { bit, on } => Self::AiFlag { bit, on },
            V::StoreEntry { key, value } => Self::AutoSwitch { key, value },
        })
    }
}

/// One setting as the robot has it now.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SettingEntry {
    /// A short English name, such as "child lock" or "pet detection".
    pub name: String,
    pub value: SettingValue,
}

impl From<Setting> for SettingEntry {
    fn from(s: Setting) -> Self {
        Self { name: s.name(), value: s.into() }
    }
}

/// A logged-in Dreamehome account.
#[derive(uniffi::Object)]
pub struct Account {
    client: Arc<CloudClient>,
    store: Arc<FileTokenStore>,
}

fn config(session_file: &str) -> (CloudConfig, Arc<FileTokenStore>) {
    let store = Arc::new(FileTokenStore::new(session_file));
    (CloudConfig::default().with_token_store(store.clone()), store)
}

/// Logs in with the account email and password. The session is saved to `session_file` (a
/// path in the app's private folder) so [`resume`] works later. `region` is a code such as
/// `"us"` or `"eu"`; `None` tries every region.
#[uniffi::export(async_runtime = "tokio")]
pub async fn login(email: String, password: String, region: Option<String>, session_file: String) -> Result<Arc<Account>> {
    let (cfg, store) = config(&session_file);
    let regions: Vec<Region> = region.iter().map(|r| Region::parse(r)).collect();
    let client = CloudClient::login_any_with(cfg, &regions, &email, &password).await?;
    Ok(Arc::new(Account { client: Arc::new(client), store }))
}

/// Resumes the session saved in `session_file`, refreshing its token. Fails with `Auth` when
/// there is no saved session or it has expired.
#[uniffi::export(async_runtime = "tokio")]
pub async fn resume(session_file: String) -> Result<Arc<Account>> {
    let (cfg, store) = config(&session_file);
    let client = CloudClient::from_store(cfg).await?;
    Ok(Arc::new(Account { client: Arc::new(client), store }))
}

#[uniffi::export(async_runtime = "tokio")]
impl Account {
    /// Every device on the account.
    pub async fn devices(&self) -> Result<Vec<Device>> {
        Ok(self
            .client
            .list_devices()
            .await?
            .into_iter()
            .map(|d| Device { is_vacuum: d.is_vacuum(), did: d.did, name: d.name, model: d.model, online: d.online })
            .collect())
    }

    /// The robot with device id `did`.
    pub async fn robot(&self, did: String) -> Result<Arc<RobotHandle>> {
        let device = self
            .client
            .list_devices()
            .await?
            .into_iter()
            .find(|d| d.did == did)
            .ok_or_else(|| DreameError::Other { detail: "that device is not on this account".into() })?;
        Ok(Arc::new(RobotHandle { robot: Robot::cloud(self.client.clone(), device), events: Mutex::new(None) }))
    }

    /// Forgets the saved session. The account object stops working once its token expires.
    pub fn logout(&self) {
        use dreame_cloud::TokenStore;
        self.store.clear();
    }
}

/// Receives live updates from [`RobotHandle::start_events`]. Called on a background thread.
#[uniffi::export(with_foreign)]
pub trait RobotListener: Send + Sync {
    fn on_status(&self, status: RobotStatus);
    /// The live connection's state, as text: connecting, connected, waiting or stopped.
    fn on_connection(&self, state: String);
}

/// One robot.
#[derive(uniffi::Object)]
pub struct RobotHandle {
    robot: Robot<CloudRobot>,
    events: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl RobotHandle {
    pub fn model(&self) -> String {
        self.robot.cloud_link().device.model.clone()
    }

    /// Reads a fresh status from the cloud and the robot. Read-only.
    pub async fn refresh(&self) -> Result<RobotStatus> {
        Ok(RobotStatus::from(&self.robot.refresh().await?))
    }

    /// Starts or resumes a full clean. A write: ask the user first.
    pub async fn start(&self) -> Result<()> {
        self.robot.start().await?;
        Ok(())
    }

    /// A write: ask the user first.
    pub async fn pause(&self) -> Result<()> {
        self.robot.pause().await?;
        Ok(())
    }

    /// A write: ask the user first.
    pub async fn stop(&self) -> Result<()> {
        self.robot.stop().await?;
        Ok(())
    }

    /// Sends the robot to its dock, ending the current job. A write: ask the user first.
    pub async fn dock(&self) -> Result<()> {
        self.robot.dock().await?;
        Ok(())
    }

    /// The robot's settings with their current values, read live. A setting the robot lacks is
    /// left out. Read-only.
    pub async fn settings(&self) -> Result<Vec<SettingEntry>> {
        self.robot.refresh().await?;
        let s = self.robot.read(&settings::all_properties()).await?;
        Ok(settings::known(&s).into_iter().map(SettingEntry::from).collect())
    }

    /// Changes one setting, then reads it back about 3 s later. Returns the value read back,
    /// or `None` when it could not be read. A write: ask the user first.
    pub async fn set_setting(&self, value: SettingValue) -> Result<Option<SettingEntry>> {
        let setting = Setting::try_from(value)?;
        // Settings that share a property (AI switches, off-peak) build on its current value.
        let props = setting.properties();
        self.robot.read(&props).await?;
        self.robot.set(&setting).await?;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let after = self.robot.read(&props).await?;
        let name = setting.name();
        Ok(settings::known(&after).into_iter().find(|s| s.name() == name).map(SettingEntry::from))
    }

    /// The saved map as a PNG image, `scale` image pixels per map cell (cells are 5 cm). Read-only.
    pub async fn map_png(&self, scale: u32) -> Result<Vec<u8>> {
        let map = self.robot.saved_map().await?;
        let opts = RenderOptions { scale: scale.clamp(1, 8), ..RenderOptions::default() };
        Ok(render_png(&map, &opts)?)
    }

    /// Starts following the robot: `listener` gets every status change and connection change
    /// until [`RobotHandle::stop_events`]. Only listens; it never sends to the robot.
    pub async fn start_events(&self, listener: Arc<dyn RobotListener>) -> Result<()> {
        let mut events = self.robot.events()?;
        let task = tokio::spawn(async move {
            while let Some(e) = events.next().await {
                match e {
                    Event::Status { status, .. } => listener.on_status(RobotStatus::from(status.as_ref())),
                    Event::Connection(state) => listener.on_connection(connection_text(&state)),
                    Event::MapFrame(_) | Event::RobotEvent { .. } => {}
                }
            }
            listener.on_connection("stopped".into());
        });
        if let Some(old) = self.events.lock().expect("events lock").replace(task) {
            old.abort();
        }
        Ok(())
    }

    /// Stops following. The connection closes when its task ends.
    pub fn stop_events(&self) {
        if let Some(task) = self.events.lock().expect("events lock").take() {
            task.abort();
        }
    }
}

fn connection_text(s: &ConnectionState) -> String {
    use ConnectionState as C;
    match s {
        C::Connecting { attempt } => format!("connecting (attempt {attempt})"),
        C::Connected => "connected".into(),
        C::Waiting { retry_in, .. } => format!("waiting {} s to reconnect", retry_in.as_secs()),
        C::Stopped => "stopped".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dreame_core::{PropId, RawValue};
    use dreame_model::{DecodeContext, Status};

    #[test]
    fn status_records_carry_labels_and_numbers() {
        let mut s = Status::new(DecodeContext::default());
        s.update(PropId::new(3, 1), &RawValue::Int(77));
        s.update(PropId::new(2, 1), &RawValue::Int(1));
        let r = RobotStatus::from(&s);
        assert_eq!(r.battery, Some(77));
        assert_eq!(r.state.as_deref(), Some("1 (sweeping)"));
        assert_eq!(r.error, None);
    }

    #[test]
    fn settings_cross_the_boundary_both_ways() {
        let t = |h| HhMm::new(h, 30).unwrap();
        for s in [
            Setting::Volume(55),
            Setting::SelfClean(true),
            Setting::CarpetSensitivity(dreame_model::enums::CarpetSensitivity::High),
            Setting::Dnd { enabled: true, start: t(22), end: t(7) },
            Setting::AutoSwitch { key: "MeticulousTwist".into(), value: -1 },
        ] {
            assert_eq!(Setting::try_from(SettingValue::from(s.clone())).unwrap(), s);
        }
        let bad = TimeOfDay { hour: 24, minute: 0 };
        let v = SettingValue::OffPeakCharging { enabled: true, start: bad, end: bad };
        assert!(Setting::try_from(v).is_err());
        assert_eq!(SettingEntry::from(Setting::ChildLock(true)).name, "child lock");
    }

    #[test]
    fn errors_keep_their_kind() {
        let e: DreameError = dreame_core::Error::Unsupported("no dnd".into()).into();
        assert!(matches!(e, DreameError::Unsupported { .. }));
        let e: DreameError = anyhow::anyhow!("something else").into();
        assert!(matches!(e, DreameError::Other { .. }));
        assert_eq!(DreameError::Auth { detail: "expired".into() }.to_string(), "auth: expired");
    }
}
