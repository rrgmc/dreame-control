//! Room grid edits through a backup and restore.
//!
//! The robot has no command that moves a room border cell by cell, but it loads the room grid
//! from a restored archive (`docs/research/maps.md` §12.2). So an edit runs in these steps:
//!
//! 1. Back up the map ([`fresh_backup`]), so the archive holds the robot's current state. An
//!    older archive brings back old map data with it.
//! 2. Edit the archive's segmentation images ([`edit_archive`]).
//! 3. Restore a different map first ([`prime_entry`]): the robot skips an archive of the map it
//!    already holds.
//! 4. Restore the edited archive from this machine ([`serve_and_restore`]).
//! 5. Put back the per-room settings, which a restore drops
//!    ([`crate::mapedit::Edit::RestoreSettings`]).
//!
//! The functions here do one step each, so a caller can report and stop between them. [`apply`]
//! runs them all for one [`GridEdit`], as `dreame map edit-archive --apply` and the editor do.

use crate::fileserve::{FileServer, Request};
use crate::mapedit::{self, MapAccess, Saved};
use crate::recovery::{self, Entry, EntryKind, JobReport, Wait};
use crate::CloudRobot;
use anyhow::{anyhow, bail, Context, Result};
use dreame_cloud::{CloudClient, DeviceInfo};
use dreame_map::archive::Archive;
use dreame_map::roomreg::{self, Added, Removed, RoomMeta};
use dreame_map::segmap::{Axis, Changes, Direction, Segmentation};
use dreame_map::Map;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Picks the new backup from the recovery list after a backup: an entry of `map_id` whose
/// object name and time were not in the list before (the robot may reuse a name). The newest
/// wins if there are several.
pub fn new_backup<'a>(before: &[Entry], after: &'a [Entry], map_id: u32) -> Option<&'a Entry> {
    after
        .iter()
        .filter(|e| e.map_id == map_id && e.kind == EntryKind::Backup)
        .filter(|e| !before.iter().any(|b| b.objname == e.objname && b.time == e.time))
        .max_by_key(|e| e.time)
}

/// Backs up `map_id` and downloads the new backup's archive.
pub async fn fresh_backup(c: &CloudClient, dev: &DeviceInfo, map_id: u32, wait: Wait) -> Result<(Entry, Vec<u8>)> {
    let before = recovery::list(c, dev).await.unwrap_or_default();
    let r = recovery::backup(c, dev, map_id, wait).await?;
    if !r.succeeded() {
        bail!("the backup did not succeed (states seen: {:?})", r.seen);
    }
    let after = recovery::list(c, dev).await?;
    let e = new_backup(&before, &after, map_id)
        .ok_or_else(|| anyhow!("the backup succeeded but no new backup entry is in the recovery list"))?
        .clone();
    let bytes = download(c, dev, &e).await?;
    Ok((e, bytes))
}

/// Downloads a recovery entry's archive.
pub async fn download(c: &CloudClient, dev: &DeviceInfo, e: &Entry) -> Result<Vec<u8>> {
    let url = recovery::archive_url(c, dev, e).await?;
    Ok(c.fetch_bytes(&url).await?)
}

/// Applies `edit` to an archive and returns the new archive file: the segmentation images, and
/// for [`GridEdit::NewRoom`] the room registries too. Every other member stays as it was.
pub fn edit_archive(tbz2: &[u8], edit: &GridEdit) -> Result<(Vec<u8>, Changes, Option<Registered>)> {
    let mut a = Archive::decode(tbz2).context("decode the archive")?;
    let mut s = Segmentation::read(&a).context("read the room grid")?;
    let ch = edit.run(&mut s)?;
    if ch.cells.is_empty() {
        bail!("the edit changed no cell");
    }
    s.write(&mut a)?;
    let lists = match (edit.new_room(), edit) {
        (Some((new, parent, meta)), _) => Some(Registered::Added(roomreg::add_room(&mut a, &s, new, parent, meta).context("register the new room")?)),
        (None, GridEdit::RemoveRoom { room, into }) => {
            Some(Registered::Removed(roomreg::remove_room(&mut a, &s, *room, *into).context("take the room out of the room lists")?))
        }
        _ => None,
    };
    Ok((a.encode()?, ch, lists))
}

/// How an edit changed the archive's room lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    Added(Added),
    Removed(Removed),
}

impl std::fmt::Display for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added(r) => write!(f, "new room registered: neighbours {:?}, cleaning order place {}, in {:?}", r.neighbours, r.order, r.members),
            Self::Removed(r) => write!(f, "room taken out of the room lists (cleaning order place {:?}), in {:?}", r.order, r.members),
        }
    }
}

/// The entry to restore before the edited archive: the oldest original of `map_id`. It differs
/// from the robot's current map in many cells, so the robot loads it, and then also loads the
/// edited archive. Any entry whose map differs from the current one would do.
pub fn prime_entry(entries: &[Entry], map_id: u32) -> Option<&Entry> {
    entries.iter().filter(|e| e.map_id == map_id && e.kind == EntryKind::Original).min_by_key(|e| e.time)
}

/// The result of [`serve_and_restore`].
pub struct Served {
    pub job: JobReport,
    /// Every request that reached the file server.
    pub requests: Vec<Request>,
}

/// The robot's address on the local network, as the cloud reports it.
pub async fn robot_ip(c: &CloudClient, dev: &DeviceInfo) -> Result<IpAddr> {
    let ip = c.otc_info(&dev.did).await?.local_ip.ok_or_else(|| anyhow!("the cloud does not report the robot's address"))?;
    ip.parse().map_err(|_| anyhow!("the robot's address does not parse"))
}

/// Starts the file server for the robot on `port`, with nothing to serve yet. Start it before the
/// first write to the robot: on Windows the first listen on a port can open a firewall prompt,
/// and the robot's download fails while the prompt is unanswered.
pub async fn start_server(c: &CloudClient, dev: &DeviceInfo, name: &str, port: u16) -> Result<FileServer> {
    let ip = robot_ip(c, dev).await?;
    FileServer::start(Vec::new(), name, ip, port).await
}

/// Serves `body` from `server`, only to the robot, and restores `map_id` from it. The wait also
/// ends once `done` returns true (see [`recovery::restore_until`]).
pub async fn serve_and_restore<F, Fut>(c: &CloudClient, dev: &DeviceInfo, map_id: u32, server: &FileServer, body: Vec<u8>, wait: Wait, done: F) -> Result<Served>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    server.set_body(body);
    let job = recovery::restore_until(c, dev, map_id, server.url(), wait, done).await?;
    Ok(Served { job, requests: server.requests() })
}

/// One edit of the room grid images.
#[derive(Debug, Clone, PartialEq)]
pub enum GridEdit {
    /// Grow `room` into its neighbour `from` by up to `cells` cells in `direction`
    /// ([`Segmentation::grow`]).
    Grow { room: u8, from: u8, direction: Direction, cells: u32 },
    /// Move the cells at these points (world mm) to `room`, where they belong to another room
    /// ([`Segmentation::paint`]). Lasts through a cleaning run when built from a fresh backup
    /// (W21, W22).
    Paint { room: u8, points: Vec<(f64, f64)> },
    /// Move the border of neighbour rooms `a` and `b` to a line through `line`'s points (world mm;
    /// more than two points make a bent line; [`Segmentation::move_border`]). Like a paint, room
    /// ids, names and settings stay.
    Border { a: u8, b: u8, line: Vec<(f64, f64)> },
    /// Give the cells of `room` at or beyond `mm` on `axis` the unused id `new`
    /// ([`Segmentation::split`]). The robot re-segments such a room after its next run (W24).
    Split { room: u8, new: u8, axis: Axis, mm: f64 },
    /// Like `Split`, and also register `new` as a room of its own ([`roomreg::add_room`]): in
    /// `seg_info` with `room`'s type and floor, last in the cleaning order, in the neighbour
    /// table and the AI room and floor lists. `meta` can give it a name and a floor. [`apply`]
    /// gives it `room`'s cleaning settings.
    NewRoom { room: u8, new: u8, axis: Axis, mm: f64, meta: RoomMeta },
    /// Move the cells at these points (world mm) to the unused id `new`
    /// ([`Segmentation::paint_new`]) and register it like `NewRoom`, with `parent`'s type, floor
    /// and settings.
    PaintNewRoom { new: u8, parent: u8, points: Vec<(f64, f64)>, meta: RoomMeta },
    /// Move every cell of `room` to its neighbour `into` ([`Segmentation::remove`]) and take
    /// `room` out of the room lists ([`roomreg::remove_room`]).
    RemoveRoom { room: u8, into: u8 },
    /// Set or clear the wall bit on a line (world mm) ([`Segmentation::set_wall`]). The robot
    /// ignores these walls and clears them after its next run (W23).
    Wall { from: (f64, f64), to: (f64, f64), wall: bool },
}

impl GridEdit {
    /// Applies the edit to decoded grid images.
    pub fn run(&self, s: &mut Segmentation) -> dreame_map::Result<Changes> {
        match self {
            Self::Grow { room, from, direction, cells } => s.grow(*room, *from, *direction, *cells),
            Self::Paint { room, points } => {
                let cells = s.cells_at_mm(points);
                s.paint(*room, &cells)
            }
            Self::Border { a, b, line } => s.move_border(*a, *b, line),
            Self::PaintNewRoom { new, points, .. } => {
                let cells = s.cells_at_mm(points);
                s.paint_new(*new, &cells)
            }
            Self::RemoveRoom { room, into } => s.remove(*room, *into),
            Self::Split { room, new, axis, mm } | Self::NewRoom { room, new, axis, mm, .. } => {
                let (x, y) = s.cell_at_mm(*mm, *mm);
                s.split(*room, *new, *axis, if *axis == Axis::X { x } else { y })
            }
            Self::Wall { from, to, wall } => {
                let cells = s.line(s.cell_at_mm(from.0, from.1), s.cell_at_mm(to.0, to.1));
                Ok(s.set_wall(&cells, *wall))
            }
        }
    }

    /// The new room and the room whose type, floor and settings it takes, for the edits that
    /// add a room.
    pub fn new_room(&self) -> Option<(u8, u8, &RoomMeta)> {
        match self {
            Self::NewRoom { room, new, meta, .. } => Some((*new, *room, meta)),
            Self::PaintNewRoom { new, parent, meta, .. } => Some((*new, *parent, meta)),
            _ => None,
        }
    }

    /// The rooms whose size changes.
    pub fn rooms(&self) -> Vec<u8> {
        match self {
            Self::Grow { room, from, .. } => vec![*room, *from],
            Self::Paint { room, .. } => vec![*room],
            Self::Border { a, b, .. } => vec![*a, *b],
            Self::Split { room, new, .. } | Self::NewRoom { room, new, .. } => vec![*room, *new],
            Self::PaintNewRoom { new, parent, .. } => vec![*parent, *new],
            Self::RemoveRoom { room, into } => vec![*room, *into],
            Self::Wall { .. } => Vec::new(),
        }
    }
}

/// Room sizes in an archive's grid image for `rooms`, as `room 7 3125 cells, ...`.
pub fn room_sizes(tbz2: &[u8], rooms: &[u8]) -> Result<String> {
    let a = Archive::decode(tbz2)?;
    let sizes = Segmentation::read(&a)?.room_sizes();
    Ok(rooms.iter().map(|r| format!("room {r} {} cells", sizes.get(r).copied().unwrap_or(0))).collect::<Vec<_>>().join(", "))
}

/// Settings for [`apply`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Where the backup, the edited archive and the snapshots go.
    pub dir: PathBuf,
    /// Prefix of the file names.
    pub stamp: String,
    /// TCP port the edited archive is served on.
    pub port: u16,
    /// Restore the oldest original entry before the edited archive.
    pub prime: bool,
    /// Edit this archive instead of a fresh backup. It may be older than the map.
    pub archive: Option<Vec<u8>>,
}

/// What [`apply`] did.
#[derive(Debug)]
pub struct Report {
    /// The fresh backup, the revert file. `None` when [`Options::archive`] was given.
    pub backup_file: Option<PathBuf>,
    pub edited_file: PathBuf,
    pub before_file: PathBuf,
    /// The live frame before the edit, with the per-room settings.
    pub before_live_file: PathBuf,
    pub after_file: PathBuf,
    pub changes: Changes,
    /// Every request that reached the file server.
    pub requests: Vec<Request>,
    /// The result code of the settings write, or why it failed.
    pub settings: std::result::Result<Option<i64>, String>,
    /// What changed in the saved map ([`mapedit::diff`]).
    pub diff: Vec<String>,
}

impl Report {
    /// How to undo the edit.
    pub fn revert_hint(&self) -> String {
        revert_hint(self.backup_file.as_deref(), &self.before_live_file)
    }
}

fn revert_hint(backup: Option<&Path>, live: &Path) -> String {
    match backup {
        Some(f) => format!(
            "to revert: `dreame map restore-file {} --apply`, then `dreame map edit restore-settings {} --apply`",
            f.display(),
            live.display()
        ),
        None => format!("to revert, restore a backup, then `dreame map edit restore-settings {} --apply`", live.display()),
    }
}

/// Whether the saved map's 6.8 pointer differs from `old`. A read error counts as no change.
async fn saved_moved(robot: &CloudRobot, old: &str) -> Result<bool> {
    Ok(robot.saved_pointer().await.map(|p| p != old).unwrap_or(false))
}

/// Reads the saved map until its 6.8 pointer differs from `old`, for up to `secs` seconds.
async fn wait_saved_change(robot: &CloudRobot, old: &str, secs: u64) -> Result<Saved> {
    let start = std::time::Instant::now();
    loop {
        let s = mapedit::read_saved(robot).await?;
        if s.pointer != old || start.elapsed() > std::time::Duration::from_secs(secs) {
            return Ok(s);
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

fn map_id_u32(m: &Map) -> Result<u32> {
    u32::try_from(m.map_id).map_err(|_| anyhow!("the saved map has no valid map id ({})", m.map_id))
}

fn job_line(what: &str, r: &JobReport) -> String {
    let end = match r.last() {
        _ if r.done => "loaded".to_string(),
        Some(recovery::JobState::Success) => "succeeded".to_string(),
        Some(recovery::JobState::Failed(n)) => format!("FAILED (code {n})"),
        _ => "did not finish within the wait".to_string(),
    };
    format!("{what} {end} (status before {:?}, seen {:?})", r.before, r.seen)
}

/// Restores `body` over `map_id` from `server` and waits for the saved map to change from
/// `pointer`. With `prime`, restores the oldest original entry first, as the robot skips an
/// archive close to the map it holds. `hint` says how to undo, for the errors. Returns the
/// restore's report and the saved map read back.
#[allow(clippy::too_many_arguments)]
async fn load_archive(
    robot: &CloudRobot,
    server: &FileServer,
    map_id: u32,
    pointer: &str,
    body: Vec<u8>,
    prime: bool,
    hint: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<(Served, Saved)> {
    let (c, dev) = (&*robot.client, &robot.device);
    let mut pointer = pointer.to_string();
    if prime {
        let l = recovery::list(c, dev).await?;
        let e = prime_entry(&l, map_id).ok_or_else(|| anyhow!("no original entry of map {map_id} to restore first"))?.clone();
        progress(&format!("restoring the original entry (time {}) first", e.time));
        let url = recovery::archive_url(c, dev, &e).await?;
        let old = pointer.clone();
        let r = recovery::restore_until(c, dev, map_id, &url, Wait::default(), || saved_moved(robot, &old)).await?;
        progress(&job_line("restore of the original", &r));
        if !r.succeeded() {
            bail!("stopped before the restore from this machine; {hint}");
        }
        pointer = wait_saved_change(robot, &pointer, 60).await?.pointer;
    }
    progress(&format!("restoring the archive from this machine, port {}", server.port()));
    let old = pointer.clone();
    let served = serve_and_restore(c, dev, map_id, server, body, Wait::default(), || saved_moved(robot, &old)).await?;
    progress(&job_line("restore of the archive", &served.job));
    // The robot can report a failure and still load the archive on a retry (after a late
    // firewall answer, for one), so the saved map decides.
    let s = wait_saved_change(robot, &pointer, 60).await?;
    if s.pointer == pointer {
        let why = if server.requests().is_empty() { " (no request reached this machine: check the firewall)" } else { "" };
        bail!("the archive was not loaded{why}; {hint}");
    }
    if !served.job.succeeded() {
        progress("the restore reported a failure, but the saved map changed: going on");
    }
    Ok((served, s))
}

/// Sends the per-room settings of `snapshot`, which a restore drops. Returns the result code of
/// the write, or why it failed.
async fn put_back_settings(robot: &CloudRobot, snapshot: Map, dir: &Path, stamp: &str) -> std::result::Result<Option<i64>, String> {
    let edit = mapedit::Edit::RestoreSettings(Box::new(snapshot));
    mapedit::apply(robot, &edit, dir, &format!("{stamp}-settings"), mapedit::Wait::default())
        .await
        .map(|r| r.code)
        .map_err(|e| format!("{e:#}"))
}

/// Settings for [`restore_file`].
#[derive(Debug, Clone)]
pub struct RestoreOptions {
    /// Where the snapshots before and after go.
    pub dir: PathBuf,
    /// Prefix of the file names.
    pub stamp: String,
    /// TCP port the archive is served on.
    pub port: u16,
    /// Restore the oldest original entry first ([`prime_entry`]).
    pub prime: bool,
}

/// What [`restore_file`] did.
#[derive(Debug)]
pub struct RestoreReport {
    pub before_file: PathBuf,
    /// The live frame before the restore, with the per-room settings.
    pub before_live_file: PathBuf,
    pub after_file: PathBuf,
    /// Every request that reached the file server.
    pub requests: Vec<Request>,
    /// The result code of the settings write, or why it failed. `Ok(None)` also when no room of
    /// the restored map had settings to put back.
    pub settings: std::result::Result<Option<i64>, String>,
    /// What changed in the saved map ([`mapedit::diff`]).
    pub diff: Vec<String>,
}

/// Checks that `archive` is a map backup: a bzip2 tar that holds the room grid.
pub fn check_archive(archive: &[u8]) -> Result<()> {
    let a = Archive::decode(archive).context("the file is not a map backup")?;
    Segmentation::read(&a).context("the file is not a map backup")?;
    Ok(())
}

/// Replaces the current saved map with an archive from this machine, such as a backup that
/// [`fresh_backup`] downloaded. Then puts back the per-room settings the map had before, for the
/// rooms the restored map still has. `progress` gets one line per step. Writes to the robot; the
/// caller checks that the user approved it.
pub async fn restore_file(robot: &CloudRobot, archive: Vec<u8>, opts: &RestoreOptions, progress: &mut dyn FnMut(&str)) -> Result<RestoreReport> {
    let (c, dev) = (&*robot.client, &robot.device);
    check_archive(&archive)?;
    std::fs::create_dir_all(&opts.dir)?;
    let file = |name: &str| opts.dir.join(format!("{}-{name}", opts.stamp));
    if let Some(b) = robot.busy().await? {
        bail!("the robot is busy ({b}); restore only when it is idle and docked");
    }
    let saved = mapedit::read_saved(robot).await?;
    let map_id = map_id_u32(&saved.map)?;
    // Listen first: a firewall prompt then shows before anything on the robot changes.
    let server = start_server(c, dev, &format!("{}-{map_id}.tbz2", opts.stamp), opts.port).await?;
    progress(&format!(
        "serving on port {}; if Windows asks whether to allow network access, allow it now",
        server.port()
    ));
    let before_file = file("restore-before.txt");
    std::fs::write(&before_file, &saved.envelope)?;
    let before_live_file = file("restore-before-live.txt");
    std::fs::write(&before_live_file, &saved.live_envelope)?;
    let hint = format!("the room settings before are in {}", before_live_file.display());
    let (served, loaded) = load_archive(robot, &server, map_id, &saved.pointer, archive, opts.prime, &hint, progress).await?;
    // Settings follow room ids; rooms the restored map lacks have none to get.
    let mut snapshot = saved.map.clone();
    snapshot.rooms.retain(|id, _| loaded.map.rooms.contains_key(id));
    let settings = if snapshot.rooms.values().any(|r| r.settings.is_some()) {
        progress("putting back the per-room settings");
        put_back_settings(robot, snapshot, &opts.dir, &opts.stamp).await
    } else {
        Ok(None)
    };
    let after = mapedit::read_saved(robot).await?;
    let after_file = file("restore-after.txt");
    std::fs::write(&after_file, &after.envelope)?;
    Ok(RestoreReport {
        before_file,
        before_live_file,
        after_file,
        requests: served.requests,
        settings,
        diff: mapedit::diff(&saved.map, &after.map),
    })
}

/// Runs a grid edit on the robot: fresh backup (unless [`Options::archive`]), edit, restore of
/// the oldest original (if [`Options::prime`]), restore of the edited archive from this machine,
/// then the per-room settings put back. `progress` gets one line per step. Writes to the robot;
/// the caller checks that the user approved it.
pub async fn apply(robot: &CloudRobot, edit: &GridEdit, opts: &Options, progress: &mut dyn FnMut(&str)) -> Result<Report> {
    let (c, dev) = (&*robot.client, &robot.device);
    std::fs::create_dir_all(&opts.dir)?;
    let file = |name: &str| opts.dir.join(format!("{}-{name}", opts.stamp));
    if let Some(b) = robot.busy().await? {
        bail!("the robot is busy ({b}); edit only when it is idle and docked");
    }
    let saved = mapedit::read_saved(robot).await?;
    let map_id = map_id_u32(&saved.map)?;
    // Listen first: a firewall prompt then shows before anything on the robot changes.
    let server = start_server(c, dev, &format!("{}-{map_id}.tbz2", opts.stamp), opts.port).await?;
    progress(&format!(
        "serving on port {}; if Windows asks whether to allow network access, allow it now",
        server.port()
    ));
    let before_file = file("before.txt");
    std::fs::write(&before_file, &saved.envelope)?;
    let before_live_file = file("before-live.txt");
    std::fs::write(&before_live_file, &saved.live_envelope)?;
    let (source, backup_file) = match &opts.archive {
        Some(a) => (a.clone(), None),
        None => {
            progress(&format!("backing up map {map_id}"));
            let (e, bytes) = fresh_backup(c, dev, map_id, Wait::default()).await?;
            let f = file("backup.tbz2");
            std::fs::write(&f, &bytes)?;
            progress(&format!("backup taken at {} ({} bytes), saved as {}", e.time, bytes.len(), f.display()));
            (bytes, Some(f))
        }
    };
    let (edited, changes, lists) = edit_archive(&source, edit)?;
    progress(&format!("edit: {} cells in the grid image, {:?} in the plain images", changes.cells.len(), changes.plain));
    if let Some(r) = &lists {
        progress(&r.to_string());
    }
    if !edit.rooms().is_empty() {
        progress(&format!("before: {}", room_sizes(&source, &edit.rooms())?));
        progress(&format!("after:  {}", room_sizes(&edited, &edit.rooms())?));
    }
    let edited_file = file("edited.tbz2");
    std::fs::write(&edited_file, &edited)?;
    let hint = revert_hint(backup_file.as_deref(), &before_live_file);
    load_archive(robot, &server, map_id, &saved.pointer, edited, opts.prime, &hint, progress).await?;
    progress("putting back the per-room settings");
    // A new room gets the settings of the room it came from; a removed room has none to get.
    let mut snapshot = saved.map.clone();
    if let Some((new, parent, _)) = edit.new_room() {
        if let Some(mut r) = snapshot.rooms.get(&u32::from(parent)).cloned() {
            r.id = u32::from(new);
            r.name = None;
            snapshot.rooms.insert(r.id, r);
        }
    }
    if let GridEdit::RemoveRoom { room, .. } = *edit {
        snapshot.rooms.remove(&u32::from(room));
    }
    let settings = put_back_settings(robot, snapshot, &opts.dir, &opts.stamp).await;
    progress(
        "note: the live frame shows 0 as every room's cleaning-order position until the next cleaning run; \
         the saved map keeps the order (W23, W26)",
    );
    let after = mapedit::read_saved(robot).await?;
    let after_file = file("after.txt");
    std::fs::write(&after_file, &after.envelope)?;
    Ok(Report {
        backup_file,
        edited_file,
        before_file,
        before_live_file,
        after_file,
        changes,
        requests: server.requests(),
        settings,
        diff: mapedit::diff(&saved.map, &after.map),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(map_id: u32, kind: EntryKind, time: i64, obj: &str) -> Entry {
        Entry { map_id, kind, time, objname: obj.into(), thumbnail: None }
    }

    #[test]
    fn finds_the_new_backup() {
        let before = vec![e(1, EntryKind::Original, 10, "a"), e(1, EntryKind::Backup, 20, "b")];
        let mut after = before.clone();
        after.push(e(1, EntryKind::Backup, 30, "c"));
        after.push(e(2, EntryKind::Backup, 40, "d"));
        assert_eq!(new_backup(&before, &after, 1).unwrap().objname, "c");
        assert!(new_backup(&before, &before, 1).is_none());
    }

    #[test]
    fn primes_with_the_oldest_original() {
        let l = vec![e(1, EntryKind::Original, 30, "late"), e(1, EntryKind::Original, 10, "early"), e(1, EntryKind::Backup, 5, "b")];
        assert_eq!(prime_entry(&l, 1).unwrap().objname, "early");
        assert!(prime_entry(&l, 2).is_none());
    }

    #[test]
    fn rejects_an_empty_edit() {
        // A bad archive fails before the edit runs.
        let e = GridEdit::Wall { from: (0.0, 0.0), to: (50.0, 0.0), wall: true };
        assert!(edit_archive(b"not bzip2", &e).is_err());
    }
}
