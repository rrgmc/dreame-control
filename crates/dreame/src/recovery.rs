//! Map backup and restore.
//!
//! The robot keeps earlier versions of each saved map in a recovery list (property 6.9). Each
//! entry names an archive of the robot's map store. Action 6.3 adds a backup entry; writing 6.10
//! makes the robot download an archive from a URL and replace a map. Progress shows in 6.14
//! (backup) and 6.11 (restore). See `docs/research/maps.md` §12.

use anyhow::{anyhow, bail, Context, Result};
use dreame_cloud::{CloudClient, DeviceInfo, PropId, RawValue};
use dreame_core::ActionId;
use serde_json::{json, Value};
use std::future::Future;
use std::time::{Duration, Instant};

const RECOVERY_LIST: PropId = PropId::new(6, 9);
const RESTORE: PropId = PropId::new(6, 10);
const RESTORE_STATUS: PropId = PropId::new(6, 11);
const BACKUP_STATUS: PropId = PropId::new(6, 14);
const BACKUP: ActionId = ActionId::new(6, 3);
/// `MAP_EXTEND_DATA`, the input slot of action 6.3.
const BACKUP_PIID: u32 = 4;

/// Why the robot kept a recovery entry (`first`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Edited,
    Original,
    Backup,
    Other(i64),
}

impl EntryKind {
    fn from_code(n: i64) -> Self {
        match n {
            0 => Self::Edited,
            1 => Self::Original,
            2 => Self::Backup,
            n => Self::Other(n),
        }
    }
}

impl std::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edited => write!(f, "edited"),
            Self::Original => write!(f, "original"),
            Self::Backup => write!(f, "backup"),
            Self::Other(n) => write!(f, "kind {n}"),
        }
    }
}

/// One restorable version of a saved map.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub map_id: u32,
    pub kind: EntryKind,
    /// Unix seconds.
    pub time: i64,
    /// Object name of the archive.
    pub objname: String,
    /// Inline preview, a saved-map envelope.
    pub thumbnail: Option<String>,
}

impl Entry {
    /// Archives named `*mb.tbz2` live in permanent storage and resolve through a different
    /// endpoint, as the official app does. On r2579a even backups have plain numeric names.
    pub fn is_permanent(&self) -> bool {
        self.objname.contains("mb.tbz2")
    }
}

/// Parses the recovery list file: `[{"id": M, "info": [{first, time, thb, objname}, ...]}, ...]`.
pub fn parse_list(text: &str) -> Result<Vec<Entry>> {
    let v: Value = serde_json::from_str(text).context("recovery list is not JSON")?;
    let maps = v.as_array().ok_or_else(|| anyhow!("recovery list is not an array"))?;
    let mut out = Vec::new();
    for m in maps {
        let map_id = m["id"].as_u64().or_else(|| m["id"].as_str().and_then(|s| s.parse().ok()));
        let map_id = map_id.ok_or_else(|| anyhow!("recovery list entry without a map id"))? as u32;
        for i in m["info"].as_array().into_iter().flatten() {
            let Some(objname) = i["objname"].as_str().filter(|s| !s.is_empty()) else { continue };
            out.push(Entry {
                map_id,
                kind: EntryKind::from_code(i["first"].as_i64().unwrap_or(-1)),
                time: i["time"].as_i64().unwrap_or(0),
                objname: objname.to_string(),
                thumbnail: i["thb"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            });
        }
    }
    Ok(out)
}

/// The object name in a 6.9 value: either the bare name or `{"object_name": ...}`.
pub fn list_object(value: &str) -> Result<String> {
    let t = value.trim();
    if t.starts_with('{') {
        let v: Value = serde_json::from_str(t).context("6.9 is not JSON")?;
        let o = v["object_name"].as_str().or(v["obj_name"].as_str()).ok_or_else(|| anyhow!("no object in 6.9"))?;
        return Ok(o.to_string());
    }
    if t.is_empty() {
        bail!("6.9 is empty: the robot has no recovery list");
    }
    Ok(t.to_string())
}

/// Reads 6.9 from the cloud cache and downloads the recovery list it names.
pub async fn list(c: &CloudClient, dev: &DeviceInfo) -> Result<Vec<Entry>> {
    let ptr = crate::cloud::cached_text(c, dev, RECOVERY_LIST).await?;
    let text = crate::cloud::download_object(c, dev, &list_object(&ptr)?).await?;
    parse_list(&text)
}

/// A signed URL for an entry's archive, valid for about an hour. Backups resolve through
/// `getOss1dDownloadUrl`, the other entries through `getDownloadUrl`, as the official app does.
pub async fn archive_url(c: &CloudClient, dev: &DeviceInfo, e: &Entry) -> Result<String> {
    Ok(if e.is_permanent() {
        c.permanent_download_url(&dev.did, &dev.model, &e.objname).await?
    } else {
        c.download_url(&dev.did, &dev.model, &e.objname).await?
    })
}

/// State of a backup (6.14) or restore (6.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Idle,
    Running,
    Success,
    Failed(i64),
    Other(i64),
}

impl JobState {
    pub fn from_code(n: i64) -> Self {
        match n {
            0 => Self::Idle,
            2 => Self::Running,
            3 => Self::Success,
            4 | 5 => Self::Failed(n),
            n => Self::Other(n),
        }
    }
    fn is_final(self) -> bool {
        matches!(self, Self::Success | Self::Failed(_))
    }
}

/// How long to wait for a backup or restore, and how often to look.
#[derive(Debug, Clone, Copy)]
pub struct Wait {
    pub timeout: Duration,
    pub poll: Duration,
}

impl Default for Wait {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(180), poll: Duration::from_secs(3) }
    }
}

/// The states seen while waiting, in order, without repeats.
#[derive(Debug, Clone)]
pub struct JobReport {
    pub before: Option<JobState>,
    pub seen: Vec<JobState>,
    pub result: Value,
    /// The caller's own check (see [`restore_until`]) saw the job's effect.
    pub done: bool,
}

impl JobReport {
    pub fn last(&self) -> Option<JobState> {
        self.seen.last().copied()
    }
    /// The status ended in success, or the caller's check saw the effect. After a timeout the
    /// status may still show an earlier job's success.
    pub fn succeeded(&self) -> bool {
        self.done || self.last() == Some(JobState::Success)
    }
}

/// Reads a job status live from the robot.
pub async fn job_state(c: &CloudClient, dev: &DeviceInfo, id: PropId) -> Result<Option<JobState>> {
    let r = c.get_properties(dev, &[id]).await?;
    Ok(r.into_iter()
        .find(|p| p.code == 0)
        .and_then(|p| p.value)
        .and_then(|v| v.to_json().as_i64())
        .map(JobState::from_code))
}

pub async fn backup_state(c: &CloudClient, dev: &DeviceInfo) -> Result<Option<JobState>> {
    job_state(c, dev, BACKUP_STATUS).await
}

pub async fn restore_state(c: &CloudClient, dev: &DeviceInfo) -> Result<Option<JobState>> {
    job_state(c, dev, RESTORE_STATUS).await
}

/// The 6.10 value that starts a restore.
pub fn restore_value(map_id: u32, url: &str) -> String {
    json!({"map_id": map_id, "map_url": url}).to_string()
}

/// Waits until the status reaches success or failure after a write. A final state counts only
/// once the job was seen running or the value moved away from what it was before the write, so a
/// stale success from an earlier job does not end the wait. `done` is polled too, and ends the
/// wait when it returns true; the second value says whether it did.
async fn wait_job<F, Fut>(c: &CloudClient, dev: &DeviceInfo, id: PropId, before: Option<JobState>, wait: Wait, mut done: F) -> Result<(Vec<JobState>, bool)>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    let start = Instant::now();
    let mut seen: Vec<JobState> = Vec::new();
    loop {
        tokio::time::sleep(wait.poll).await;
        if let Some(s) = job_state(c, dev, id).await? {
            if seen.last() != Some(&s) {
                seen.push(s);
            }
            let moved = seen.contains(&JobState::Running) || Some(s) != before;
            if s.is_final() && moved {
                return Ok((seen, false));
            }
        }
        if done().await? {
            return Ok((seen, true));
        }
        if start.elapsed() > wait.timeout {
            return Ok((seen, false));
        }
    }
}

async fn never() -> Result<bool> {
    Ok(false)
}

/// Backs up a saved map (action 6.3) and waits for 6.14. This adds a backup entry to the
/// recovery list and does not change the map.
pub async fn backup(c: &CloudClient, dev: &DeviceInfo, map_id: u32, wait: Wait) -> Result<JobReport> {
    let before = backup_state(c, dev).await?;
    if before == Some(JobState::Running) {
        bail!("a backup is already running");
    }
    let input = vec![json!({"piid": BACKUP_PIID, "value": map_id.to_string()})];
    let mut result = c.action(dev, BACKUP, input).await?;
    strip_did(&mut result);
    if let Some(code) = crate::mapedit::result_code(&result).filter(|c| *c != 0) {
        bail!("the robot refused the backup: result code {code}");
    }
    let (seen, done) = wait_job(c, dev, BACKUP_STATUS, before, wait, never).await?;
    Ok(JobReport { before, seen, result, done })
}

/// Makes the robot replace `map_id` with the archive at `url` (property 6.10) and waits for
/// 6.11. This rewrites the saved map. The caller checks that the robot is idle and docked.
pub async fn restore(c: &CloudClient, dev: &DeviceInfo, map_id: u32, url: &str, wait: Wait) -> Result<JobReport> {
    restore_until(c, dev, map_id, url, wait, never).await
}

/// [`restore`], but the wait also ends once `done` returns true. 6.11 often keeps an earlier
/// success while the robot loads an archive, so a caller passes a check of the effect, such as
/// a new 6.8 pointer.
pub async fn restore_until<F, Fut>(c: &CloudClient, dev: &DeviceInfo, map_id: u32, url: &str, wait: Wait, done: F) -> Result<JobReport>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    let before = restore_state(c, dev).await?;
    if before == Some(JobState::Running) {
        bail!("a restore is already running");
    }
    let r = c.set_properties(dev, &[(RESTORE, RawValue::Str(restore_value(map_id, url)))]).await?;
    let code = r.first().map(|p| p.code);
    if code != Some(0) {
        bail!("the robot refused the restore: code {code:?}");
    }
    let (seen, done) = wait_job(c, dev, RESTORE_STATUS, before, wait, done).await?;
    Ok(JobReport { before, seen, result: json!({"code": 0}), done })
}

fn strip_did(v: &mut Value) {
    if let Some(o) = v.as_object_mut() {
        o.remove("did");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_recovery_list() {
        let t = r#"[{"id":1,"info":[
            {"first":1,"time":1776452401,"thb":"AAA","objname":"ali_dreame/U/D/100"},
            {"first":0,"time":1781452777,"thb":"","objname":"ali_dreame/U/D/102"},
            {"first":2,"time":1781500000,"objname":"ali_dreame/U/D/x.1.mb.tbz2"},
            {"first":0,"time":1,"objname":""}]}]"#;
        let l = parse_list(t).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].kind, EntryKind::Original);
        assert_eq!(l[0].thumbnail.as_deref(), Some("AAA"));
        assert_eq!(l[1].kind, EntryKind::Edited);
        assert_eq!(l[1].thumbnail, None);
        assert_eq!(l[2].kind, EntryKind::Backup);
        assert!(l[2].is_permanent());
        assert!(!l[0].is_permanent());
        assert_eq!(l[2].map_id, 1);
    }

    #[test]
    fn reads_both_6_9_forms() {
        assert_eq!(list_object("ali_dreame/U/D/8").unwrap(), "ali_dreame/U/D/8");
        assert_eq!(list_object(r#"{"object_name":"ali_dreame/U/D/8"}"#).unwrap(), "ali_dreame/U/D/8");
        assert!(list_object("").is_err());
    }

    #[test]
    fn restore_value_is_compact_json() {
        assert_eq!(restore_value(1, "https://h/p?a=1&b=2"), r#"{"map_id":1,"map_url":"https://h/p?a=1&b=2"}"#);
    }

    #[test]
    fn job_states() {
        assert_eq!(JobState::from_code(0), JobState::Idle);
        assert_eq!(JobState::from_code(2), JobState::Running);
        assert!(JobState::from_code(3).is_final());
        assert_eq!(JobState::from_code(5), JobState::Failed(5));
        assert!(!JobState::from_code(7).is_final());
    }
}
