//! Capability discovery (docs/research/device-model.md section 5).
//!
//! [`discover`] reads every readable property in the profile with relayed
//! `get_properties` calls and records each result code. It then combines
//! that with the model's static capability set, when one is known, and
//! derives feature flags.
//!
//! How to read result codes on r2579a (device-model.md 2.17):
//!
//! - **0**: the property exists and returned a value.
//! - **-1**: "not readable". This does **not** mean absent. Push-only
//!   properties (6.1, 6.3) and write-only ones (6.10) also return -1.
//!   Use the cloud cache ([`Capabilities::merge_cache`]), the static
//!   capability set or the profile's access flags to tell the cases apart.
//! - **-2**: seen only on the first two piids of siids 20 and 29; meaning
//!   unknown.
//!
//! The cloud cache misses many properties that do exist, so presence must
//! come from relayed reads, not from the cache alone.

use crate::profile::{fw_build, Profile};
use crate::status::DecodeContext;
use dreame_core::{DeviceInfo, Error, PropId, PropResult, RawValue, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::time::Duration;

/// Reads properties from a robot. Implemented for the cloud relay; tests
/// and other transports can supply their own.
pub trait PropertyReader {
    /// Reads the given properties. One result per id is expected, but
    /// missing results are tolerated.
    fn read(&self, ids: &[PropId]) -> impl Future<Output = Result<Vec<PropResult>>> + Send;
}

/// The cloud relay: `sendCommand` `get_properties`. Read-only.
///
/// This parses replies leniently (see [`parse_relay_result`]), unlike
/// `CloudClient::get_properties`, which rejects a whole batch when one item
/// lacks `siid`/`piid`. r2579a sends such items for some ids.
impl PropertyReader for (&dreame_cloud::CloudClient, &DeviceInfo) {
    fn read(&self, ids: &[PropId]) -> impl Future<Output = Result<Vec<PropResult>>> + Send {
        let (client, dev) = *self;
        async move { relay_read(client, dev, ids).await.map(|(r, _)| r) }
    }
}

/// A [`PropertyReader`] over the cloud relay that also keeps the reply
/// items it could not match by `siid`/`piid`.
pub struct CloudReader<'a> {
    pub client: &'a dreame_cloud::CloudClient,
    pub dev: &'a DeviceInfo,
    issues: std::sync::Mutex<Vec<RelayItemIssue>>,
}

impl<'a> CloudReader<'a> {
    pub fn new(client: &'a dreame_cloud::CloudClient, dev: &'a DeviceInfo) -> Self {
        Self { client, dev, issues: Default::default() }
    }

    /// Reply items without `siid`/`piid` seen so far.
    pub fn issues(&self) -> Vec<RelayItemIssue> {
        self.issues.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl PropertyReader for CloudReader<'_> {
    async fn read(&self, ids: &[PropId]) -> Result<Vec<PropResult>> {
        let (r, issues) = relay_read(self.client, self.dev, ids).await?;
        self.issues.lock().unwrap_or_else(|p| p.into_inner()).extend(issues);
        Ok(r)
    }
}

async fn relay_read(
    client: &dreame_cloud::CloudClient,
    dev: &DeviceInfo,
    ids: &[PropId],
) -> Result<(Vec<PropResult>, Vec<RelayItemIssue>)> {
    let params: Vec<serde_json::Value> =
        ids.iter().map(|p| serde_json::json!({"did": dev.did, "siid": p.siid, "piid": p.piid})).collect();
    // Retry once when the cloud reports success without a result, as the
    // cloud client does.
    for _ in 0..2 {
        let v = client.send_command(dev, "get_properties", serde_json::Value::Array(params.clone()), true).await?;
        if let Some(r) = v.pointer("/data/result").or_else(|| v.get("result")).filter(|r| !r.is_null()) {
            return parse_relay_result(ids, r);
        }
    }
    Err(Error::Decode("get_properties returned no result".into()))
}

/// A reply item that had no `siid`/`piid`.
#[derive(Debug, Clone, PartialEq)]
pub struct RelayItemIssue {
    /// Position in the reply list.
    pub index: usize,
    /// The id requested at that position, if the list lengths match.
    pub assumed: Option<PropId>,
    /// The item's keys (values are not kept; they may hold identifiers).
    pub keys: Vec<String>,
    pub code: Option<i64>,
}

/// Parses a relayed `get_properties` result list. Items with `siid` and
/// `piid` map directly. An item without them is matched to the requested
/// id at the same position when the reply has one item per request, and
/// keeps its `code`; otherwise it is dropped. Every unmatched item is
/// reported as a [`RelayItemIssue`].
///
/// On r2579a (fw 1664), every id in a service the robot lacks (13, 17,
/// 24-26, 31-38, 40) came back as `{siid, did, code: -1}` without `piid`,
/// in request order. That shape suggests a missing service, but this is
/// not confirmed, so the item is kept as code -1.
pub fn parse_relay_result(ids: &[PropId], result: &serde_json::Value) -> Result<(Vec<PropResult>, Vec<RelayItemIssue>)> {
    use serde_json::Value;
    let items = result.as_array().ok_or_else(|| Error::Decode("get_properties result is not a list".into()))?;
    let num = |v: Option<&Value>| v.and_then(|x| x.as_u64().or_else(|| x.as_str()?.parse().ok())).map(|n| n as u32);
    let same_len = items.len() == ids.len();
    let mut out = Vec::with_capacity(items.len());
    let mut issues = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let code = it.get("code").and_then(Value::as_i64);
        let id = match (num(it.get("siid")), num(it.get("piid"))) {
            (Some(s), Some(p)) => Some(PropId::new(s, p)),
            _ => {
                let assumed = if same_len { Some(ids[i]) } else { None };
                let keys = it.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
                issues.push(RelayItemIssue { index: i, assumed, keys, code });
                assumed
            }
        };
        let Some(id) = id else { continue };
        let code = code.unwrap_or(0);
        let value = match it.get("value") {
            Some(v) if code == 0 && !v.is_null() => Some(RawValue::from_json(v)),
            _ => None,
        };
        // An item without ids and without a code says nothing reliable.
        if issues.last().is_some_and(|x| x.index == i && x.code.is_none()) {
            continue;
        }
        out.push(PropResult { id, code, value });
    }
    Ok((out, issues))
}

/// What one relayed read said about one property.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    /// Code 0 with a value.
    Value(RawValue),
    /// Code 0 without a value (null or empty).
    Empty,
    /// Code -1: not readable. It may still exist.
    NotReadable,
    /// Any other code, such as -2.
    Code(i64),
    /// The batch failed or the reply had no item for this id.
    NoAnswer(String),
}

/// Whether a property exists, as far as the evidence goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Yes,
    /// Not readable, but the profile says it is write-only or push-only.
    Likely,
    Unknown,
    No,
}

/// Options for [`discover`].
#[derive(Debug, Clone)]
pub struct DiscoverOptions {
    /// Ids per relayed call. The cloud relays at most 15.
    pub batch: usize,
    /// Pause between calls, to keep request volume modest.
    pub delay: Duration,
    /// Skip properties the profile documents as not readable.
    pub skip_unreadable: bool,
}

impl Default for DiscoverOptions {
    fn default() -> Self {
        Self { batch: 15, delay: Duration::from_millis(300), skip_unreadable: true }
    }
}

/// The result of discovery.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub model: String,
    pub fw_build: Option<u32>,
    /// Every probe result, by id.
    pub probes: BTreeMap<PropId, Probe>,
    /// Ids seen in the cloud cache. Presence there proves existence.
    pub cached: BTreeSet<PropId>,
    /// The model's static capability flags that are on for this fw build,
    /// or `None` when the model is not in [`MODEL_CAPABILITIES`].
    pub model_flags: Option<BTreeSet<&'static str>>,
    /// Derived feature flags.
    pub features: BTreeMap<Feature, bool>,
}

impl Capabilities {
    /// Ids that returned code 0.
    pub fn readable(&self) -> impl Iterator<Item = PropId> + '_ {
        self.probes.iter().filter(|(_, p)| matches!(p, Probe::Value(_) | Probe::Empty)).map(|(id, _)| *id)
    }

    /// Ids by result code: 0, -1, other codes, and no answer.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut c = (0, 0, 0, 0);
        for p in self.probes.values() {
            match p {
                Probe::Value(_) | Probe::Empty => c.0 += 1,
                Probe::NotReadable => c.1 += 1,
                Probe::Code(_) => c.2 += 1,
                Probe::NoAnswer(_) => c.3 += 1,
            }
        }
        c
    }

    /// The values read with code 0, for [`crate::status::Status::from_values`].
    pub fn values(&self) -> BTreeMap<PropId, RawValue> {
        self.probes
            .iter()
            .filter_map(|(id, p)| match p {
                Probe::Value(v) => Some((*id, v.clone())),
                _ => None,
            })
            .collect()
    }

    /// Whether a property exists. `profile` supplies access flags.
    pub fn presence(&self, id: PropId, profile: &Profile) -> Presence {
        if self.cached.contains(&id) {
            return Presence::Yes;
        }
        match self.probes.get(&id) {
            Some(Probe::Value(_) | Probe::Empty) => Presence::Yes,
            Some(Probe::NotReadable) => {
                if profile.prop(id).is_some_and(|d| d.access.documented_unreadable()) {
                    Presence::Likely
                } else {
                    Presence::Unknown
                }
            }
            // A read that the robot answered with "absent" for this id would
            // be the only evidence for No; MIoT has no such code on r2579a.
            Some(Probe::Code(_)) | Some(Probe::NoAnswer(_)) | None => Presence::Unknown,
        }
    }

    /// Presence as Tasshack uses it: code 0, or listed in the cache.
    pub fn present(&self, id: PropId) -> bool {
        self.cached.contains(&id) || matches!(self.probes.get(&id), Some(Probe::Value(_) | Probe::Empty))
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.model_flags.as_ref().is_some_and(|f| f.contains(flag))
    }

    pub fn feature(&self, f: Feature) -> bool {
        self.features.get(&f).copied().unwrap_or(false)
    }

    /// Whether every property got an answer. Only a complete result is worth saving with
    /// [`Capabilities::known`].
    pub fn complete(&self) -> bool {
        !self.probes.values().any(|p| matches!(p, Probe::NoAnswer(_)))
    }

    /// What this result says, in a form to save. `firmware` is the robot's firmware text; a
    /// saved result is only good for the same model and firmware.
    pub fn known(&self, firmware: &str) -> KnownCapabilities {
        let present = self.probes.keys().chain(self.cached.iter()).copied().filter(|id| self.present(*id)).collect::<BTreeSet<_>>();
        KnownCapabilities {
            model: self.model.clone(),
            firmware: firmware.to_string(),
            present: present.into_iter().collect(),
            values: self.values().into_iter().collect(),
        }
    }

    /// Rebuilds a result from a saved one. Properties that existed read as present, with the
    /// values they had then; "not readable" answers are not kept, so [`Capabilities::presence`]
    /// says `Unknown` for those.
    pub fn from_known(k: &KnownCapabilities) -> Self {
        let values: BTreeMap<PropId, RawValue> = k.values.iter().cloned().collect();
        let build = fw_build(&k.firmware);
        let probes = k
            .present
            .iter()
            .map(|id| (*id, values.get(id).cloned().map_or(Probe::Empty, Probe::Value)))
            .collect();
        let mut caps = Capabilities {
            model: k.model.clone(),
            fw_build: build,
            probes,
            model_flags: model_flags(&k.model, build),
            ..Default::default()
        };
        caps.derive();
        caps
    }

    /// Marks ids that the cloud cache holds, then recomputes features.
    pub fn merge_cache(&mut self, ids: impl IntoIterator<Item = PropId>) {
        self.cached.extend(ids);
        self.derive();
    }

    /// Decode settings for [`crate::status::Status`].
    pub fn decode_context(&self) -> DecodeContext {
        DecodeContext {
            mop_pad_lifting: self.feature(Feature::MopPadLifting),
            self_wash_base: self.feature(Feature::SelfWashBase),
            legacy_warnings: self.has_flag("LEGACY_WARNINGS"),
            ..DecodeContext::default()
        }
    }

    /// Recomputes [`Self::features`] from presence and the model flags
    /// (device-model.md 5.1 layers 1 and 3).
    pub fn derive(&mut self) {
        let p = |s, i| self.present(PropId::new(s, i));
        let flag = |f: &str| self.has_flag(f);
        let value = |s, i| match self.probes.get(&PropId::new(s, i)) {
            Some(Probe::Value(v)) => v.as_i64(),
            _ => None,
        };
        let lidar = !p(13, 1);
        let self_wash = p(4, 25);
        let auto_empty = p(15, 3) && value(15, 3) != Some(3) && (self.model_flags.is_none() || flag("AUTO_EMPTYING"));
        let mop_lifting = flag("MOP_PAD_LIFTING") || flag("MOP_PAD_LIFTING_PLUS") || (self_wash && auto_empty);
        let extr_freq = match self.probes.get(&PropId::new(4, 50)) {
            Some(Probe::Value(v)) => crate::codec::auto_switch::AutoSwitchSettings::decode(v).ok().and_then(|s| s.get("ExtrFreq")),
            _ => None,
        };
        let drainage = flag("DRAINAGE") && p(4, 60);
        let pairs = [
            (Feature::LidarNavigation, lidar),
            (Feature::MultiFloorMap, p(6, 7) && lidar),
            (Feature::AiDetection, p(4, 22)),
            (Feature::SelfWashBase, self_wash),
            (Feature::AutoEmptyBase, auto_empty),
            (Feature::CustomizedCleaning, p(4, 26)),
            (Feature::TightMopping, p(4, 29)),
            (Feature::AutoSwitchSettings, p(4, 50)),
            (Feature::CarpetRecognition, p(4, 33) || p(4, 36)),
            (Feature::WifiMap, p(6, 15)),
            (Feature::BackupMap, p(6, 14)),
            (Feature::DndTask, p(5, 4)),
            (Feature::Dnd, p(5, 4) || p(5, 1)),
            (Feature::Shortcuts, p(4, 48)),
            (Feature::OffPeakCharging, p(3, 3)),
            (Feature::VoiceAssistant, p(7, 5)),
            (Feature::MopPadLifting, mop_lifting),
            (Feature::Wetness, flag("WETNESS_LEVEL") || (flag("MOPPING_SETTINGS") && value(28, 1).is_some_and(|w| w != 0))),
            (Feature::Drainage, drainage),
            (Feature::UltraCleanMode, flag("ULTRA_CLEAN_MODE") && drainage),
            (Feature::Detergent, self_wash && (flag("DETERGENT") || p(20, 1))),
            (Feature::MopExtend, (flag("MOP_PAD_SWING") || flag("MOP_PAD_SWING_PLUS")) && extr_freq.is_some_and(|v| v >= 0)),
            (Feature::CleaningProgress, p(4, 63)),
            (Feature::StationCleaning, self_wash && flag("GEN5")),
            (Feature::IntelligentAutoEmpty, flag("INTELLIGENT_AUTO_EMPTY")),
        ];
        self.features = pairs.into_iter().collect();
    }
}

/// A discovery result in a form to save and load, so that an app can skip discovery, which
/// takes about 25 seconds, on later starts. See [`Capabilities::known`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KnownCapabilities {
    pub model: String,
    pub firmware: String,
    /// Ids that exist: answered with code 0, or listed in the cloud cache.
    pub present: Vec<PropId>,
    /// The values read with code 0.
    pub values: Vec<(PropId, RawValue)>,
}

impl KnownCapabilities {
    /// Whether this result is for a robot of `model` on `firmware`.
    pub fn fits(&self, model: &str, firmware: &str) -> bool {
        self.model == model && self.firmware == firmware
    }
}

/// Feature flags derived by [`Capabilities::derive`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    LidarNavigation,
    MultiFloorMap,
    AiDetection,
    SelfWashBase,
    AutoEmptyBase,
    CustomizedCleaning,
    TightMopping,
    AutoSwitchSettings,
    CarpetRecognition,
    WifiMap,
    BackupMap,
    DndTask,
    Dnd,
    Shortcuts,
    OffPeakCharging,
    VoiceAssistant,
    MopPadLifting,
    Wetness,
    Drainage,
    UltraCleanMode,
    Detergent,
    MopExtend,
    /// 4.63 is push only, so a read cannot show it; expect false until a
    /// push arrives.
    CleaningProgress,
    StationCleaning,
    IntelligentAutoEmpty,
}

/// One static capability: a flag, optionally gated by a minimum fw build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapFlag {
    pub name: &'static str,
    pub min_build: Option<u32>,
}

const fn on(name: &'static str) -> CapFlag {
    CapFlag { name, min_build: None }
}

/// Tasshack's capability set 152 (decoded from `DEVICE_INFO`,
/// device-model.md 5.3).
pub static SET_152: &[CapFlag] = &[
    on("DRAINAGE"),
    on("MOPPING_AFTER_SWEEPING"),
    on("MAX_SUCTION_POWER"),
    on("OBSTACLE_IMAGE_CROP"),
    on("MOP_PAD_SWING"),
    on("HOT_WASHING"),
    on("AUTO_EMPTY_MODE"),
    on("FLOOR_DIRECTION_CLEANING"),
    on("SEGMENT_VISIBILITY"),
    on("MOP_PAD_SWING_PLUS"),
    on("AUTO_REWASHING"),
    on("MOP_PAD_LIFTING_PLUS"),
    on("PET_FURNITURE"),
    on("CLEANING_ROUTE"),
    on("MOPPING_SETTINGS"),
    on("TASK_TYPE"),
    on("ULTRA_CLEAN_MODE"),
    on("EXTENDED_FURNITURES"),
    on("SELF_CLEAN_FREQUENCY"),
    on("CLEANGENIUS"),
    on("FLUID_DETECTION"),
    on("INTENSIVE_CARPET_CLEANING"),
    on("CLEAN_CARPETS_FIRST"),
    on("WETNESS_LEVEL"),
    on("AUTO_RENAME_SEGMENT"),
    on("FLOOR_MATERIAL"),
    on("GEN5"),
    on("FURNITURES_V2"),
    on("SAVED_FURNITURES"),
    on("OBSTACLES"),
    on("WATER_CHECK"),
    on("AUTO_CARPET_CLEANING"),
    on("SEGMENT_MOPPING_SETTINGS"),
    on("MAX_SUCTION_POWER_EXTENDED"),
    on("AUTO_RECLEANING"),
    on("NEW_STATE"),
    on("CAMERA_STREAMING"),
    on("CLEANGENIUS_MODE"),
    on("WASHING_MODE"),
    on("SMART_MOP_WASHING"),
    on("DND_FUNCTIONS"),
    on("RAMPS"),
    on("VIRTUAL_TRACKS"),
    on("WHEEL"),
    on("AREA_ROTATION"),
    on("MOP_PAD_LIFTING"),
    on("DYNAMIC_OBSTACLE_CLEANING"),
    on("CARPET_CLEANSET_V2"),
    on("CARPET_SHAPE"),
    on("CARPET_ID"),
    on("AUTO_EMPTYING"),
    on("WATER_TANK_DRAINING"),
    on("AUTO_ADD_DETERGENT"),
    on("FILL_LIGHT"),
    on("DEODORIZER_MODULE"),
    on("CLEANING_SEQUENCE_V2"),
    on("WALLS_V3"),
    on("QUICK_MAP_RECOVERY_V2"),
    on("CURTAINS"),
    on("STREAM_TX"),
    on("STREAM_ALI"),
    on("VIDEO_DYNAMIC_VENDOR"),
    on("VIDEO_DURING_CLEANING"),
    on("PET_SNAPSHOT"),
    on("MULTI_SEGMENT_FLOOR_MATERIAL"),
    on("FLOOR_MATERIAL_EDIT"),
    on("DOCK_STATUS"),
    on("SKIP_AREA"),
    CapFlag { name: "LOCAL_STORAGE", min_build: Some(1681) },
];

/// Model suffix (text after the last `.`) to static capability set. Only
/// the sets verified for this project are ported so far; other models fall
/// back to presence detection.
pub static MODEL_CAPABILITIES: &[(&str, &[CapFlag])] =
    &[("r2579a", SET_152), ("r25799", SET_152), ("r2579c", SET_152), ("r2579h", SET_152), ("r2579k", SET_152), ("r500za", SET_152)];

/// The static flags on for `model` at `build`, or `None` for an unknown model.
pub fn model_flags(model: &str, build: Option<u32>) -> Option<BTreeSet<&'static str>> {
    let suffix = model.rsplit('.').next().unwrap_or(model);
    let set = MODEL_CAPABILITIES.iter().find(|(s, _)| *s == suffix)?.1;
    Some(
        set.iter()
            .filter(|f| match (f.min_build, build) {
                (None, _) => true,
                (Some(min), Some(b)) => b >= min,
                (Some(_), None) => false,
            })
            .map(|f| f.name)
            .collect(),
    )
}

/// Reads every property in `profile` through `reader` and builds
/// [`Capabilities`]. Read-only: it only calls `get_properties`.
///
/// A failed batch is recorded as [`Probe::NoAnswer`] and discovery goes
/// on. If every batch fails, the first error is returned.
pub async fn discover<R: PropertyReader>(
    reader: &R,
    profile: &Profile,
    model: &str,
    firmware: &str,
    opts: &DiscoverOptions,
) -> Result<Capabilities> {
    let ids: Vec<PropId> = profile
        .prop_ids()
        .into_iter()
        .filter(|id| match profile.prop(*id) {
            Some(d) if opts.skip_unreadable => !d.access.documented_unreadable(),
            _ => true,
        })
        .collect();
    let build = fw_build(firmware);
    let mut caps = Capabilities { model: model.to_string(), fw_build: build, ..Default::default() };
    let mut first_err: Option<Error> = None;
    let mut ok_batches = 0;
    let batch = opts.batch.clamp(1, 15);
    for (i, chunk) in ids.chunks(batch).enumerate() {
        if i > 0 && !opts.delay.is_zero() {
            tokio::time::sleep(opts.delay).await;
        }
        match reader.read(chunk).await {
            Ok(results) => {
                ok_batches += 1;
                for r in results {
                    let probe = match (r.code, r.value) {
                        (0, Some(RawValue::Str(s))) if s.is_empty() => Probe::Empty,
                        (0, Some(v)) => Probe::Value(v),
                        (0, None) => Probe::Empty,
                        (-1, _) => Probe::NotReadable,
                        (c, _) => Probe::Code(c),
                    };
                    caps.probes.insert(r.id, probe);
                }
                for id in chunk {
                    caps.probes.entry(*id).or_insert_with(|| Probe::NoAnswer("no item in reply".into()));
                }
            }
            Err(e) => {
                let msg = e.to_string();
                for id in chunk {
                    caps.probes.insert(*id, Probe::NoAnswer(msg.clone()));
                }
                if matches!(e, Error::Auth(_)) {
                    return Err(e);
                }
                first_err.get_or_insert(e);
            }
        }
    }
    if ok_batches == 0 {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    caps.model_flags = model_flags(model, build);
    caps.derive();
    Ok(caps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A fake robot answering from a table: ids in `values` return code 0,
    /// ids in `codes` return that code, others -1.
    struct Fake {
        values: BTreeMap<PropId, RawValue>,
        codes: BTreeMap<PropId, i64>,
        calls: Mutex<Vec<usize>>,
        fail_call: Option<usize>,
    }

    impl PropertyReader for Fake {
        fn read(&self, ids: &[PropId]) -> impl Future<Output = Result<Vec<PropResult>>> + Send {
            let n = {
                let mut c = self.calls.lock().unwrap();
                c.push(ids.len());
                c.len()
            };
            let out = if self.fail_call == Some(n) {
                Err(Error::NoAck)
            } else {
                Ok(ids
                    .iter()
                    .map(|id| match (self.values.get(id), self.codes.get(id)) {
                        (Some(v), _) => PropResult { id: *id, code: 0, value: Some(v.clone()) },
                        (None, Some(c)) => PropResult { id: *id, code: *c, value: None },
                        _ => PropResult { id: *id, code: -1, value: None },
                    })
                    .collect())
            };
            async move { out }
        }
    }

    fn fake(fail_call: Option<usize>) -> Fake {
        let values = crate::status::tests::r2579a_cache();
        let codes = [(PropId::new(20, 1), -2), (PropId::new(29, 1), -2)].into_iter().collect();
        Fake { values, codes, calls: Mutex::new(Vec::new()), fail_call }
    }

    fn opts() -> DiscoverOptions {
        DiscoverOptions { delay: Duration::ZERO, ..Default::default() }
    }

    #[tokio::test]
    async fn discovers_r2579a_like_robot() {
        let f = fake(None);
        let profile = Profile::for_model("dreame.vacuum.r2579a");
        let caps = discover(&f, &profile, "dreame.vacuum.r2579a", "4.3.9_1664", &opts()).await.unwrap();
        assert!(f.calls.lock().unwrap().iter().all(|n| *n <= 15));
        assert_eq!(caps.fw_build, Some(1664));
        assert!(caps.has_flag("GEN5"));
        assert!(!caps.has_flag("LOCAL_STORAGE"), "LOCAL_STORAGE needs build 1681");
        assert_eq!(caps.probes.get(&PropId::new(2, 1)), Some(&Probe::Value(RawValue::Int(8))));
        assert_eq!(caps.probes.get(&PropId::new(20, 1)), Some(&Probe::Code(-2)));
        // 6.3 is push-only in the profile, so it is not read.
        assert!(!caps.probes.contains_key(&PropId::new(6, 3)));
        for f in [Feature::SelfWashBase, Feature::AutoEmptyBase, Feature::MopPadLifting, Feature::Wetness, Feature::Dnd, Feature::MopExtend, Feature::Drainage, Feature::LidarNavigation, Feature::StationCleaning] {
            assert!(caps.feature(f), "{f:?}");
        }
        for f in [Feature::DndTask, Feature::Detergent, Feature::IntelligentAutoEmpty] {
            assert!(!caps.feature(f), "{f:?}");
        }
        let ctx = caps.decode_context();
        assert!(ctx.mop_pad_lifting && ctx.self_wash_base && !ctx.legacy_warnings);
        let (ok, minus1, other, none) = caps.counts();
        assert_eq!(ok, caps.values().len());
        assert_eq!((other, none), (2, 0));
        assert!(minus1 > 0);
    }

    #[tokio::test]
    async fn not_readable_is_not_absent() {
        let f = fake(None);
        let profile = Profile::base();
        let all = DiscoverOptions { skip_unreadable: false, ..opts() };
        let mut caps = discover(&f, &profile, "dreame.vacuum.r2579a", "4.3.9_1664", &all).await.unwrap();
        assert_eq!(caps.presence(PropId::new(6, 3), &profile), Presence::Likely);
        assert_eq!(caps.presence(PropId::new(6, 14), &profile), Presence::Unknown);
        assert_eq!(caps.presence(PropId::new(2, 1), &profile), Presence::Yes);
        assert!(!caps.feature(Feature::BackupMap));
        caps.merge_cache([PropId::new(6, 14)]);
        assert_eq!(caps.presence(PropId::new(6, 14), &profile), Presence::Yes);
        assert!(caps.feature(Feature::BackupMap));
    }

    #[tokio::test]
    async fn failed_batches_are_recorded() {
        let f = fake(Some(1));
        let caps = discover(&f, &Profile::base(), "dreame.vacuum.x", "", &opts()).await.unwrap();
        assert!(caps.model_flags.is_none());
        assert_eq!(caps.counts().3, 15);
        // Without a model table, the presence layer still works.
        assert!(caps.feature(Feature::SelfWashBase));
        assert!(!caps.complete());
    }

    #[tokio::test]
    async fn a_saved_result_gives_the_same_features() {
        let f = fake(None);
        let model = "dreame.vacuum.r2579a";
        let mut caps = discover(&f, &Profile::for_model(model), model, "4.3.9_1664", &opts()).await.unwrap();
        caps.merge_cache([PropId::new(6, 3)]);
        assert!(caps.complete());
        let known = caps.known("4.3.9_1664");
        let json = serde_json::to_string(&known).unwrap();
        let back: KnownCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, known);
        assert!(back.fits(model, "4.3.9_1664"));
        assert!(!back.fits(model, "4.3.9_1700"));
        let again = Capabilities::from_known(&back);
        assert_eq!(again.features, caps.features);
        assert_eq!(again.model_flags, caps.model_flags);
        assert_eq!(again.values(), caps.values());
        assert!(again.present(PropId::new(6, 3)));
    }

    #[tokio::test]
    async fn all_batches_failing_is_an_error() {
        struct Down;
        impl PropertyReader for Down {
            async fn read(&self, _: &[PropId]) -> Result<Vec<PropResult>> {
                Err(Error::NoAck)
            }
        }
        let r = discover(&Down, &Profile::base(), "m", "", &opts()).await;
        assert!(matches!(r, Err(Error::NoAck)));
    }

    #[test]
    fn lenient_relay_parse() {
        use serde_json::json;
        let ids = [PropId::new(2, 1), PropId::new(9, 1), PropId::new(6, 3)];
        let reply = json!([
            {"did": "x", "siid": 2, "piid": 1, "value": 8, "code": 0},
            {"code": -704002000},
            {"did": "x", "siid": 6, "piid": 3, "code": -1}
        ]);
        let (r, issues) = parse_relay_result(&ids, &reply).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].value, Some(RawValue::Int(8)));
        assert_eq!(r[1], PropResult { id: PropId::new(9, 1), code: -704002000, value: None });
        assert_eq!(r[2].code, -1);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].assumed, Some(PropId::new(9, 1)));
        assert_eq!(issues[0].keys, vec!["code".to_string()]);

        // Length mismatch: the odd item cannot be placed and is dropped.
        let (r, issues) = parse_relay_result(&ids[..2], &json!([{"code": 1}, {"x": 1}, {"siid": 2, "piid": 1, "value": 1}])).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.assumed.is_none()));
        // An item without ids or code is reported but not used.
        let (r, issues) = parse_relay_result(&ids[..1], &json!([{"value": 3}])).unwrap();
        assert!(r.is_empty());
        assert_eq!(issues[0].code, None);
        assert!(parse_relay_result(&ids, &json!({})).is_err());
    }

    #[test]
    fn model_flag_lookup() {
        assert!(model_flags("dreame.vacuum.r2579a", Some(1700)).unwrap().contains("LOCAL_STORAGE"));
        assert!(!model_flags("dreame.vacuum.r2579a", None).unwrap().contains("LOCAL_STORAGE"));
        assert_eq!(model_flags("dreame.vacuum.r2579a", Some(1664)).unwrap().len(), 68);
        assert!(model_flags("dreame.vacuum.r9999", None).is_none());
    }
}
