//! `dreame`: a command-line tool for Dreame robot vacuums.
//!
//! Commands that write to the robot (map edits, backups, restores, settings and controls) do so
//! only with `--apply`. Credentials and the saved session are handled by
//! [`dreame::Session`] (see its module for where they come from).

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use dreame::cloud::{cached_text, live_envelope, saved_maps};
use dreame::session::{default_out_dir, pick_device, stamp};
use dreame::mapedit::MapAccess;
use dreame::{archiveedit, mapedit, recovery, CloudRobot, Robot, Session};
use dreame_cloud::{CloudClient, DeviceInfo, PropId};
use dreame_map::render::{save_png, RenderOptions};
use dreame_map::{Frame, Map};
use std::path::PathBuf;
use std::sync::Arc;

/// Default port for `map restore-file`, so a single inbound firewall rule can cover every build.
const RESTORE_PORT: u16 = 8770;

#[derive(Parser)]
#[command(name = "dreame", about = "Tool for Dreame robot vacuums. Commands that write to the robot send nothing without `--apply`.")]
struct Cli {
    /// Device id; defaults to the first vacuum on the account.
    #[arg(long, global = true)]
    did: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

// Parsed once per run, so the size of the largest variant (`Map`) does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Cmd {
    /// Log in with the password and save the session.
    Login,
    /// Forget the saved session.
    Logout,
    /// List devices on the account.
    Devices,
    /// Past cleaning jobs from the cloud's event history (4.e1). Read-only.
    History {
        /// How many jobs to list.
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Print the shape of the reply (keys and value types, no values) instead.
        #[arg(long)]
        shape: bool,
        /// The `from` time sent, in Unix seconds (default: 2023-06-17).
        #[arg(long)]
        from: Option<i64>,
        /// With --shape: a property's history, such as `6.3`, instead of the cleaning records.
        #[arg(long)]
        key: Option<String>,
        /// Also download the newest job's map and say what it holds.
        #[arg(long)]
        map: bool,
    },
    /// Read properties, for example `dreame props 2.1 3.1`.
    Props {
        ids: Vec<String>,
        /// Read the cloud cache instead of asking the robot.
        #[arg(long)]
        cached: bool,
    },
    /// List the obstacles and pets in the live map, and optionally save their photos. Read-only.
    Obstacles {
        /// Save each photo in the cloud as `obstacle-<index>.jpg` in this directory.
        #[arg(long)]
        save: Option<PathBuf>,
    },
    /// Show the robot's status, decoded with the device model.
    Status {
        /// Also print every raw value read.
        #[arg(long)]
        raw: bool,
        /// First probe which properties the robot has and print the derived features
        /// (read-only, about 30 relayed calls).
        #[arg(long)]
        discover: bool,
    },
    /// Print live updates from the robot (MQTT push) until stopped or the time runs out.
    Watch {
        /// Stop after this many seconds (0 = run until Ctrl+C).
        #[arg(long, default_value_t = 0)]
        seconds: u64,
        /// Also print telemetry (99.98) in full instead of summarising it. Map frames (6.1) are
        /// always shown as one line per decoded frame.
        #[arg(long)]
        all: bool,
    },
    /// Download, decode and render a map; `map snapshot` and `map edit` work on the saved map.
    #[command(args_conflicts_with_subcommands = true)]
    Map {
        #[command(subcommand)]
        action: Option<MapCmd>,
        /// Which map: `saved` (6.8 list) or `current` (last frame, 6.3).
        #[arg(long, default_value = "saved")]
        which: String,
        /// Saved map id (one per floor, from `map list`); defaults to the robot's current map.
        #[arg(long)]
        map: Option<i64>,
        /// PNG output path.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Pixels per grid cell in the PNG.
        #[arg(long, default_value_t = 4)]
        scale: u32,
        /// Draw the vector floor plan over the grid.
        #[arg(long)]
        floor_plan: bool,
        /// Print room names.
        #[arg(long)]
        names: bool,
        /// Also write the raw envelope text here.
        #[arg(long)]
        save_envelope: Option<PathBuf>,
    },
    /// Control the robot: start, pause, stop, dock, clean rooms or a zone. Prints the command;
    /// nothing is sent without `--apply`.
    Control {
        #[command(subcommand)]
        action: ControlCmd,
        /// Send the command to the robot. This makes it move.
        #[arg(long, global = true)]
        apply: bool,
    },
    /// List the robot's settings and their current values. Read-only.
    Settings,
    /// Change one setting. Prints the current value and the writes; nothing is sent without
    /// `--apply`, which then reads the setting back.
    Set {
        #[command(subcommand)]
        setting: SetCmd,
        /// Send the writes to the robot.
        #[arg(long, global = true)]
        apply: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
enum OnOff {
    On,
    Off,
}

impl OnOff {
    fn on(self) -> bool {
        self == Self::On
    }
}

#[derive(Subcommand, Debug, PartialEq)]
enum SetCmd {
    /// Speaker volume, 0 to 100.
    Volume { level: u8 },
    ChildLock { state: OnOff },
    CarpetBoost { state: OnOff },
    /// Resume the job after recharging.
    ResumeCleaning { state: OnOff },
    /// Clean each room with its own settings.
    CustomizedCleaning { state: OnOff },
    /// Wash the mop during the job.
    SelfClean { state: OnOff },
    /// Mop drying time in hours.
    DryingTime { hours: u8 },
    /// low, medium or high.
    CarpetSensitivity { level: String },
    /// Cleans between automatic bin emptying.
    AutoEmptyFrequency { cleans: u8 },
    /// Do not disturb; start and end default to the current window (HH:MM).
    Dnd {
        state: OnOff,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// Off-peak charging; start and end default to the current window (HH:MM).
    OffPeak {
        state: OnOff,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
    },
    /// One AI detection switch by name, for example "pet detection".
    Ai { name: String, state: OnOff },
    /// One entry of the settings store (4.50) by key, for example MeticulousTwist -1.
    #[command(allow_negative_numbers = true)]
    AutoSwitch { key: String, value: i64 },
}

impl SetCmd {
    /// The setting, with unset times taken from the robot's current values in `s`.
    fn to_setting(&self, s: &dreame_model::Status) -> Result<dreame::Setting> {
        use dreame::Setting as S;
        use dreame_model::codec::HhMm;
        let time = |given: &Option<String>, current: Option<HhMm>, what: &str| -> Result<HhMm> {
            match given {
                Some(t) => HhMm::parse(t).map_err(|e| anyhow!("{what}: {e}")),
                None => current.ok_or_else(|| anyhow!("give --{what}; the current value is not known")),
            }
        };
        Ok(match self {
            Self::Volume { level } => S::Volume(*level),
            Self::ChildLock { state } => S::ChildLock(state.on()),
            Self::CarpetBoost { state } => S::CarpetBoost(state.on()),
            Self::ResumeCleaning { state } => S::ResumeCleaning(state.on()),
            Self::CustomizedCleaning { state } => S::CustomizedCleaning(state.on()),
            Self::SelfClean { state } => S::SelfClean(state.on()),
            Self::DryingTime { hours } => S::DryingTimeHours(*hours),
            Self::CarpetSensitivity { level } => {
                use dreame_model::enums::CarpetSensitivity as C;
                S::CarpetSensitivity(match level.to_ascii_lowercase().as_str() {
                    "low" => C::Low,
                    "medium" => C::Medium,
                    "high" => C::High,
                    other => bail!("carpet sensitivity is low, medium or high, not {other}"),
                })
            }
            Self::AutoEmptyFrequency { cleans } => S::AutoEmptyFrequency(*cleans),
            Self::Dnd { state, start, end } => S::Dnd {
                enabled: state.on(),
                start: time(start, s.dnd.start, "start")?,
                end: time(end, s.dnd.end, "end")?,
            },
            Self::OffPeak { state, start, end } => S::OffPeak {
                enabled: state.on(),
                start: time(start, s.off_peak.as_ref().map(|o| o.start), "start")?,
                end: time(end, s.off_peak.as_ref().map(|o| o.end), "end")?,
            },
            Self::Ai { name, state } => {
                let bit = dreame::settings::ai_bit(name).ok_or_else(|| {
                    let names: Vec<_> = dreame_model::codec::ai_obstacle::AiFlags::NAMES.iter().map(|(_, n)| *n).collect();
                    anyhow!("no AI switch named {name:?}; one of {names:?}")
                })?;
                S::AiFlag { bit, on: state.on() }
            }
            Self::AutoSwitch { key, value } => S::AutoSwitch { key: key.clone(), value: *value },
        })
    }
}

/// Passes, suction and water for a clean; unset values come from the room's stored settings.
#[derive(clap::Args, Clone, Copy)]
struct CleanArgs {
    #[arg(long)]
    passes: Option<u32>,
    /// 0 quiet, 1 standard, 2 strong, 3 turbo.
    #[arg(long)]
    suction: Option<i64>,
    #[arg(long)]
    water: Option<i64>,
}

impl From<CleanArgs> for dreame::control::CleanOptions {
    fn from(a: CleanArgs) -> Self {
        Self { passes: a.passes, suction: a.suction, water: a.water }
    }
}

#[derive(Subcommand)]
enum ControlCmd {
    /// Start or resume a full clean (2.a1).
    Start,
    /// Pause (2.a2).
    Pause,
    /// Stop the job (4.a2).
    Stop,
    /// Return to the dock (3.a1).
    Dock,
    /// Clean the given rooms, in this order (4.a1, status 18).
    CleanRooms {
        #[arg(required = true)]
        ids: Vec<u32>,
        #[command(flatten)]
        opts: CleanArgs,
    },
    /// Clean one rectangle in world mm (4.a1, status 19).
    #[command(allow_negative_numbers = true)]
    CleanZone {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        #[command(flatten)]
        opts: CleanArgs,
    },
    /// Wait until the current run ends (docked, paused, error, or stopped) and print how it
    /// ended, with every error and fault code seen. Read-only; follows the live updates.
    Wait {
        /// Give up after this many minutes.
        #[arg(long, default_value_t = 90)]
        timeout_min: u64,
    },
    /// Drive by hand (4.15): repeat one step every second for `seconds`, then stop. The robot
    /// enters remote control on the first step.
    Remote {
        direction: RemoteDir,
        #[arg(long, default_value_t = 1.0)]
        seconds: f64,
    },
    /// Ask the robot to upload the photo of obstacle `index` from `dreame obstacles` (4.a6).
    UploadPhoto { index: usize },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
enum RemoteDir {
    Forward,
    Left,
    Right,
    TurnAround,
    Stop,
}

impl RemoteDir {
    fn step(self) -> dreame_model::commands::RemoteMove {
        use dreame_model::commands::RemoteMove;
        match self {
            Self::Forward => RemoteMove::FORWARD,
            Self::Left => RemoteMove::LEFT,
            Self::Right => RemoteMove::RIGHT,
            Self::TurnAround => RemoteMove::TURN_AROUND,
            Self::Stop => RemoteMove::STOP,
        }
    }
}

#[derive(Subcommand)]
enum MapCmd {
    /// List the saved maps (one per floor) with their ids and room counts. Read-only.
    List {
        /// Also print the custom map names.
        #[arg(long)]
        names: bool,
    },
    /// Save a saved map (envelope text) to a file. Read-only.
    Snapshot {
        /// Output directory; defaults to `local/out/snapshots`.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Saved map id (from `map list`); defaults to the robot's current map.
        #[arg(long)]
        map: Option<i64>,
    },
    /// Build a map edit from the freshest saved map and print it. Nothing is sent without
    /// `--apply`. On a map that is not the robot's current one, only edits that name their map
    /// (room names, types, merge, split, hide, floor) are allowed.
    Edit {
        #[command(subcommand)]
        edit: EditCmd,
        /// Saved map id (from `map list`); defaults to the robot's current map.
        #[arg(long, global = true)]
        map: Option<i64>,
        /// Send the edit to the robot, then read the saved map back and show what changed.
        #[arg(long, global = true)]
        apply: bool,
        /// Directory for the before and after snapshots; defaults to `local/out/edits`.
        #[arg(long, global = true)]
        dir: Option<PathBuf>,
    },
    /// List the recovery list (6.9): earlier versions and backups of each saved map. Read-only.
    Recovery,
    /// Download the archive of one recovery entry (index from `map recovery`). Read-only.
    FetchArchive {
        index: usize,
        /// Output file; defaults to `local/out/recovery/<stamp>-<index>.tbz2`.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Back up a saved map to the cloud (action 6.3). Adds a recovery entry; the map is not
    /// changed. Nothing is sent without `--apply`.
    Backup {
        /// Map id; defaults to the current saved map.
        map_id: Option<u32>,
        #[arg(long)]
        apply: bool,
    },
    /// Ask the robot to upload a full map frame (action 6.1), wait for it, and print each room's
    /// cleaning settings. Does not change the map. Nothing is sent without `--apply`.
    RequestFrame {
        #[arg(long)]
        apply: bool,
        /// Ask for a forced I-frame with a synchronous reply (`req_type` 1, `force_type` 1); the
        /// reply names the uploaded frame. Without it the robot pushes the frame over MQTT, which
        /// a sleeping robot may not do.
        #[arg(long)]
        force: bool,
    },
    /// Replace a saved map with an archive file from this machine. The CLI serves the file over
    /// HTTP on the local network, only to the robot, and gives the robot that URL through 6.10.
    /// Rewrites the saved map. Nothing is sent without `--apply`.
    RestoreFile {
        archive: PathBuf,
        /// Map id; defaults to the current saved map.
        #[arg(long)]
        map_id: Option<u32>,
        #[arg(long)]
        apply: bool,
        /// Directory for the before and after snapshots; defaults to `local/out/edits`.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// TCP port to serve the file on. Fixed by default so one firewall rule for this port
        /// covers every build; 0 picks a free port.
        #[arg(long, default_value_t = RESTORE_PORT)]
        port: u16,
    },
    /// Edit the room grid through a backup and restore. With `--apply`: back up the map,
    /// download the backup (kept as a revert file), edit its segmentation images, restore the
    /// oldest original entry first (the robot skips an archive of the map it holds), restore the
    /// edited archive from this machine, then put back the per-room settings. Without
    /// `--apply`, only edits an archive (`--archive`, or the newest backup in the recovery list)
    /// and writes the result.
    EditArchive {
        #[command(subcommand)]
        edit: ArchiveEditCmd,
        /// Send the edit to the robot. Rewrites the saved map.
        #[arg(long, global = true)]
        apply: bool,
        /// Edit this archive file instead of a fresh backup.
        #[arg(long, global = true)]
        archive: Option<PathBuf>,
        /// Skip the restore of the original entry before the edited archive.
        #[arg(long, global = true)]
        no_prime: bool,
        /// Directory for archives and snapshots; defaults to `local/out/recovery`.
        #[arg(long, global = true)]
        dir: Option<PathBuf>,
        /// TCP port to serve the edited archive on (see `restore-file`).
        #[arg(long, global = true, default_value_t = RESTORE_PORT)]
        port: u16,
    },
    /// Replace a saved map with one recovery entry (index from `map recovery`), through 6.10.
    /// Rewrites the saved map. Nothing is sent without `--apply`.
    Restore {
        index: usize,
        #[arg(long)]
        apply: bool,
        /// Directory for the before and after snapshots; defaults to `local/out/edits`.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

/// An edit to the room grid images of an archive.
#[derive(Subcommand)]
enum ArchiveEditCmd {
    /// Grow room ROOM into its neighbour FROM by CELLS grid cells (50 mm each) in DIRECTION
    /// (up, down, left or right on the grid image), along their whole shared border.
    Grow { room: u8, from: u8, direction: dreame_map::segmap::Direction, cells: u32 },
    /// Move the cells inside the rectangle (X0, Y0)-(X1, Y1), in world mm, that belong to
    /// another room to ROOM. Cells outside every room stay outside.
    #[command(allow_negative_numbers = true)]
    Paint { room: u8, x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Move the border of neighbour rooms A and B to a line through the points X0 Y0 X1 Y1
    /// [X2 Y2 ...], in world mm; more than two points draw a bent line. The line must cross both
    /// rooms from edge to edge. Room ids, names and settings stay.
    #[command(allow_negative_numbers = true)]
    Border {
        a: u8,
        b: u8,
        #[arg(num_args = 4.., required = true, value_name = "X Y")]
        points: Vec<f64>,
    },
    /// Give the cells of ROOM at or beyond MM (world mm) on AXIS (x or y) the unused room id
    /// NEW. Only the grid images change; the room metadata gets no entry for NEW. The robot
    /// re-segments such a room after its next cleaning run (W24).
    #[command(allow_negative_numbers = true)]
    Split { room: u8, new: u8, axis: dreame_map::segmap::Axis, mm: f64 },
    /// Like `split`, and also register NEW as a room of its own: in the room metadata
    /// (`seg_info`) with ROOM's type and floor, last in the cleaning order, in the neighbour table
    /// and the AI room list. It gets ROOM's cleaning settings. Research (W29).
    #[command(allow_negative_numbers = true)]
    NewRoom {
        room: u8,
        new: u8,
        axis: dreame_map::segmap::Axis,
        mm: f64,
        #[command(flatten)]
        meta: NewRoomMeta,
    },
    /// Move the cells inside the rectangle (X0, Y0)-(X1, Y1), in world mm, to the unused room id
    /// NEW and register it like `new-room`, with PARENT's type, floor and settings.
    #[command(allow_negative_numbers = true)]
    PaintNewRoom {
        new: u8,
        parent: u8,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        #[command(flatten)]
        meta: NewRoomMeta,
    },
    /// Move every cell of ROOM to its neighbour INTO and take ROOM out of the room lists
    /// (`seg_info`, cleaning order, neighbour table, AI room list, hidden rooms). Research (W31).
    RemoveRoom { room: u8, into: u8 },
    /// Mark the grid cells on a straight line from (X0, Y0) to (X1, Y1), in world mm, as wall.
    /// Room ids and carpet flags stay. The robot drives through these walls and clears them
    /// after its next cleaning run (W23); block a door with a virtual wall instead.
    #[command(allow_negative_numbers = true)]
    Wall {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        /// Clear the wall bit instead of setting it.
        #[arg(long)]
        clear: bool,
    },
}

/// Name and floor of a new room; without them it takes the parent's type and floor.
#[derive(clap::Args, Clone, Default)]
struct NewRoomMeta {
    /// Custom name of the new room.
    #[arg(long)]
    name: Option<String>,
    /// Floor of the new room: wood or tile.
    #[arg(long)]
    floor: Option<String>,
    /// Grain direction of a wood floor: 0 or 90.
    #[arg(long, default_value_t = 0)]
    direction: i64,
}

impl NewRoomMeta {
    fn to_meta(&self) -> Result<dreame_map::roomreg::RoomMeta> {
        use dreame_map::roomreg::Floor;
        let floor = match self.floor.as_deref() {
            None => None,
            Some("wood") => Some(Floor::Wood { direction: self.direction }),
            Some("tile") => Some(Floor::Tile),
            Some(other) => bail!("the floor is wood or tile, not {other}"),
        };
        Ok(dreame_map::roomreg::RoomMeta { name: self.name.clone(), floor })
    }
}

impl ArchiveEditCmd {
    fn to_edit(&self) -> Result<archiveedit::GridEdit> {
        use archiveedit::GridEdit;
        if let Self::Border { a, b, points } = self {
            if points.len() % 2 != 0 {
                bail!("the border line needs x and y for each point; got {} numbers", points.len());
            }
            let line = points.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect();
            return Ok(GridEdit::Border { a: *a, b: *b, line });
        }
        Ok(match *self {
            Self::Grow { room, from, direction, cells } => GridEdit::Grow { room, from, direction, cells },
            Self::Paint { room, x0, y0, x1, y1 } => GridEdit::Paint { room, points: cell_centres(x0, y0, x1, y1) },
            Self::Border { .. } => unreachable!("handled above"),
            Self::Split { room, new, axis, mm } => GridEdit::Split { room, new, axis, mm },
            Self::NewRoom { room, new, axis, mm, ref meta } => GridEdit::NewRoom { room, new, axis, mm, meta: meta.to_meta()? },
            Self::PaintNewRoom { new, parent, x0, y0, x1, y1, ref meta } => {
                GridEdit::PaintNewRoom { new, parent, points: cell_centres(x0, y0, x1, y1), meta: meta.to_meta()? }
            }
            Self::RemoveRoom { room, into } => GridEdit::RemoveRoom { room, into },
            Self::Wall { x0, y0, x1, y1, clear } => GridEdit::Wall { from: (x0, y0), to: (x1, y1), wall: !clear },
        })
    }
}

/// The centres of the 50 mm grid cells whose centre lies in a rectangle (world mm).
fn cell_centres(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    let axis = |a: f64, b: f64| {
        let (lo, hi) = (a.min(b), a.max(b));
        let first = ((lo - 25.0) / 50.0).ceil() as i64;
        let last = ((hi - 25.0) / 50.0).floor() as i64;
        (first..=last).map(|k| k as f64 * 50.0 + 25.0).collect::<Vec<_>>()
    };
    let ys = axis(y0, y1);
    axis(x0, x1).into_iter().flat_map(|x| ys.iter().map(move |&y| (x, y))).collect()
}

#[derive(Subcommand)]
enum EditCmd {
    /// Give a room a custom name.
    RenameRoom { id: u32, name: String },
    /// Add a no-go zone (world mm).
    #[command(allow_negative_numbers = true)]
    AddNoGo { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a no-go zone by its index in the dry-run listing.
    RemoveNoGo { index: usize },
    /// Add a no-mop zone (world mm).
    #[command(allow_negative_numbers = true)]
    AddNoMop { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a no-mop zone by its index in the dry-run listing.
    RemoveNoMop { index: usize },
    /// Add a virtual wall (world mm).
    #[command(allow_negative_numbers = true)]
    AddWall { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a virtual wall by its index in the dry-run listing.
    RemoveWall { index: usize },
    /// Add a rectangular carpet (world mm); corners snap to cell corners.
    #[command(allow_negative_numbers = true)]
    AddCarpet { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a carpet by its index in the dry-run listing.
    RemoveCarpet { index: usize },
    /// Add an area where detected carpet is ignored (world mm).
    #[command(allow_negative_numbers = true)]
    AddIgnoredCarpet { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove an ignored-carpet area by its index in the dry-run listing.
    RemoveIgnoredCarpet { index: usize },
    /// Add a virtual threshold (world mm).
    #[command(allow_negative_numbers = true)]
    AddThreshold { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a virtual threshold by its index in the dry-run listing.
    RemoveThreshold { index: usize },
    /// Add an impassable threshold (world mm).
    #[command(allow_negative_numbers = true)]
    AddImpassableThreshold { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove an impassable threshold by its index in the dry-run listing.
    RemoveImpassableThreshold { index: usize },
    /// Add a curtain line (world mm).
    #[command(allow_negative_numbers = true)]
    AddCurtain { x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Remove a curtain by its index in the dry-run listing.
    RemoveCurtain { index: usize },
    /// Set every room's name and type back to a snapshot of the same map (undo for rename-room).
    RestoreNames { snapshot: PathBuf },
    /// Send every room's cleaning settings from a live-frame snapshot (`*-live.txt`), for example
    /// after a map restore dropped them.
    RestoreSettings { snapshot: PathBuf },
    /// Set the full list of hidden rooms; give no ids to show every room.
    HideRooms { ids: Vec<u32> },
    /// Set the cleaning order: every visible room id, once, in order.
    CleaningOrder {
        #[arg(required = true)]
        ids: Vec<u32>,
    },
    /// Change one room's cleaning settings; other rooms are resent unchanged.
    RoomSettings {
        id: u32,
        #[arg(long)]
        suction: Option<i64>,
        #[arg(long)]
        water: Option<i64>,
        #[arg(long)]
        times: Option<i64>,
        #[arg(long)]
        clean_type: Option<i64>,
        #[arg(long)]
        mop_effect: Option<i64>,
    },
    /// Merge two neighbour rooms. Room ids change afterwards.
    MergeRooms { a: u32, b: u32 },
    /// Split a room along a line (world mm). Both ends must lie outside the room's floor.
    #[command(allow_negative_numbers = true)]
    SplitRoom { id: u32, x0: f64, y0: f64, x1: f64, y1: f64 },
    /// Move the boundary between neighbour rooms A and B to a line (world mm): merge, split, then
    /// restore both rooms' names, floors, settings and cleaning order. Several writes.
    #[command(allow_negative_numbers = true)]
    MoveBoundary { a: u32, b: u32, x0: f64, y0: f64, x1: f64, y1: f64 },
}

impl EditCmd {
    fn to_edit(&self) -> Result<mapedit::Edit> {
        use dreame_map::model::{Line, Rect};
        use mapedit::Edit;
        Ok(match *self {
            Self::RestoreNames { ref snapshot } => Edit::RestoreNames(Box::new(mapedit::read_snapshot(snapshot)?)),
            Self::RestoreSettings { ref snapshot } => Edit::RestoreSettings(Box::new(mapedit::read_snapshot(snapshot)?)),
            Self::RenameRoom { id, ref name } => Edit::RenameRoom { id, name: name.clone() },
            Self::HideRooms { ref ids } => Edit::HideRooms(ids.clone()),
            Self::CleaningOrder { ref ids } => Edit::CleaningOrder(ids.clone()),
            Self::AddNoGo { x0, y0, x1, y1 } => Edit::AddNoGo(Rect::new(x0, y0, x1, y1)),
            Self::RemoveNoGo { index } => Edit::RemoveNoGo(index),
            Self::AddNoMop { x0, y0, x1, y1 } => Edit::AddNoMop(Rect::new(x0, y0, x1, y1)),
            Self::RemoveNoMop { index } => Edit::RemoveNoMop(index),
            Self::AddWall { x0, y0, x1, y1 } => Edit::AddWall(Line::new(x0, y0, x1, y1)),
            Self::RemoveWall { index } => Edit::RemoveWall(index),
            Self::AddCarpet { x0, y0, x1, y1 } => Edit::AddCarpet(Rect::new(x0, y0, x1, y1)),
            Self::RemoveCarpet { index } => Edit::RemoveCarpet(index),
            Self::AddIgnoredCarpet { x0, y0, x1, y1 } => Edit::AddIgnoredCarpet(Rect::new(x0, y0, x1, y1)),
            Self::RemoveIgnoredCarpet { index } => Edit::RemoveIgnoredCarpet(index),
            Self::AddThreshold { x0, y0, x1, y1 } => Edit::AddThreshold(Line::new(x0, y0, x1, y1)),
            Self::RemoveThreshold { index } => Edit::RemoveThreshold(index),
            Self::AddImpassableThreshold { x0, y0, x1, y1 } => Edit::AddImpassableThreshold(Line::new(x0, y0, x1, y1)),
            Self::RemoveImpassableThreshold { index } => Edit::RemoveImpassableThreshold(index),
            Self::AddCurtain { x0, y0, x1, y1 } => Edit::AddCurtain(Line::new(x0, y0, x1, y1)),
            Self::RemoveCurtain { index } => Edit::RemoveCurtain(index),
            Self::RoomSettings { id, suction, water, times, clean_type, mop_effect } => {
                Edit::RoomSettings { id, suction, water, times, clean_type, mop_effect }
            }
            Self::MergeRooms { a, b } => Edit::MergeRooms(a, b),
            Self::SplitRoom { id, x0, y0, x1, y1 } => Edit::SplitRoom { id, line: Line::new(x0, y0, x1, y1) },
            Self::MoveBoundary { a, b, x0, y0, x1, y1 } => Edit::MoveBoundary { a, b, line: Line::new(x0, y0, x1, y1) },
        })
    }
}

/// The shape of a JSON value: keys and types, with strings as their length only, so ids and
/// object names are never printed.
fn shape_of(v: &serde_json::Value) -> String {
    use serde_json::Value as V;
    match v {
        V::Null => "null".into(),
        V::Bool(_) => "bool".into(),
        V::Number(n) => format!("number({})", if n.is_i64() { "int" } else { "float" }),
        V::String(s) => match serde_json::from_str::<V>(s) {
            Ok(inner @ (V::Object(_) | V::Array(_))) => format!("json-string {}", shape_of(&inner)),
            _ => format!("string({})", s.len()),
        },
        V::Array(a) => format!("[{}]", a.iter().map(shape_of).collect::<Vec<_>>().join(", ")),
        V::Object(o) => format!("{{{}}}", o.iter().map(|(k, x)| format!("{k}: {}", shape_of(x))).collect::<Vec<_>>().join(", ")),
    }
}

fn parse_ids(ids: &[String]) -> Result<Vec<PropId>> {
    ids.iter().map(|s| s.parse::<PropId>().map_err(|e| anyhow!("{e}"))).collect()
}

fn last_segment(objname: &str) -> &str {
    objname.rsplit('/').next().unwrap_or(objname)
}

fn print_entries(l: &[recovery::Entry]) {
    for (i, e) in l.iter().enumerate() {
        println!(
            "{i}: map {} {} time {} object .../{}{}",
            e.map_id,
            e.kind,
            e.time,
            last_segment(&e.objname),
            if e.thumbnail.is_some() { " (thumbnail)" } else { "" }
        );
    }
}

fn print_job(what: &str, r: &recovery::JobReport) {
    println!("{what} status before: {:?}; seen: {:?}{}", r.before, r.seen, if r.done { "; the saved map changed" } else { "" });
    match r.last() {
        _ if r.done => println!("{what} loaded"),
        Some(recovery::JobState::Success) => println!("{what} succeeded"),
        Some(recovery::JobState::Failed(n)) => println!("{what} FAILED (code {n})"),
        _ => println!("{what} did not finish within the wait; check again with `dreame map recovery`"),
    }
}

/// Prints the requests that reached the file server.
fn print_requests(reqs: &[dreame::fileserve::Request], succeeded: bool) {
    println!("requests to this machine: {}", reqs.len());
    if reqs.is_empty() && !succeeded {
        println!(
            "no request reached this machine: check that the firewall allows inbound connections \
             for this program (rules on Windows apply per program file, so each build path needs one)"
        );
    }
    for q in reqs {
        println!(
            "  {} {} -> {} ({} bytes){}{}{}",
            q.method,
            q.path,
            q.status,
            q.body_bytes,
            if q.from_peer { "" } else { " from another address" },
            q.range.as_ref().map(|r| format!("; range {r}")).unwrap_or_default(),
            q.user_agent.as_ref().map(|a| format!("; agent {a}")).unwrap_or_default()
        );
    }
}

/// `map edit-archive`; see [`MapCmd::EditArchive`].
async fn cmd_edit_archive(robot: &CloudRobot, edit: ArchiveEditCmd, apply: bool, archive: Option<PathBuf>, no_prime: bool, dir: Option<PathBuf>, port: u16) -> Result<()> {
    let (c, dev) = (&*robot.client, &robot.device);
    let edit = edit.to_edit()?;
    let dir = dir.unwrap_or_else(|| default_out_dir("recovery"));
    let archive = match &archive {
        Some(p) => Some(std::fs::read(p).with_context(|| format!("read {}", p.display()))?),
        None => None,
    };
    if apply {
        let opts = archiveedit::Options { dir, stamp: stamp(), port, prime: !no_prime, archive };
        let r = archiveedit::apply(robot, &edit, &opts, &mut |line| println!("{line}")).await?;
        print_requests(&r.requests, true);
        if let Some(f) = &r.backup_file {
            println!("backup:  {}", f.display());
        }
        println!("edited archive: {}", r.edited_file.display());
        println!("before: {}", r.before_file.display());
        println!("after:  {}", r.after_file.display());
        match &r.settings {
            Ok(code) => println!("per-room settings put back: result code {code:?}"),
            Err(e) => println!("per-room settings NOT put back ({e}); run `dreame map edit restore-settings {} --apply`", r.before_live_file.display()),
        }
        if r.diff.is_empty() {
            println!("no change seen in the saved map");
        }
        for line in &r.diff {
            println!("  {line}");
        }
        println!("{}", r.revert_hint());
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    let source = match archive {
        Some(a) => a,
        None => {
            let id = mapedit::read_saved(robot).await?.map.map_id;
            let map_id = u32::try_from(id).map_err(|_| anyhow!("the saved map has no valid map id ({id})"))?;
            let l = recovery::list(c, dev).await?;
            let e = l
                .iter()
                .filter(|e| e.map_id == map_id && e.kind == recovery::EntryKind::Backup)
                .max_by_key(|e| e.time)
                .ok_or_else(|| anyhow!("no backup of map {map_id} in the recovery list; pass --archive"))?;
            println!("source: newest backup .../{} (time {}); it may be older than the current map", last_segment(&e.objname), e.time);
            archiveedit::download(c, dev, e).await?
        }
    };
    let (edited, ch, added) = archiveedit::edit_archive(&source, &edit)?;
    println!("grid image: {} cells changed; plain images: {:?} cells", ch.cells.len(), ch.plain);
    if let Some(r) = added {
        println!("{r}");
    }
    if !edit.rooms().is_empty() {
        println!("before: {}", archiveedit::room_sizes(&source, &edit.rooms())?);
        println!("after:  {}", archiveedit::room_sizes(&edited, &edit.rooms())?);
    }
    let out = dir.join(format!("{}-edited.tbz2", stamp()));
    std::fs::write(&out, &edited)?;
    println!("edited archive: {}", out.display());
    println!("dry run; add --apply to send");
    Ok(())
}

/// Restores `map_id` from `url` (6.10) with before and after snapshots. The live frame is saved
/// too: a restore drops the per-room settings, and `map edit restore-settings` puts them back
/// from that file.
async fn run_restore(robot: &CloudRobot, map_id: u32, url: &str, dir: Option<PathBuf>, server: Option<&dreame::fileserve::FileServer>) -> Result<()> {
    let (c, dev) = (&*robot.client, &robot.device);
    if let Some(b) = robot.busy().await? {
        bail!("the robot is busy ({b}); restore only when it is idle and docked");
    }
    let before = mapedit::read_saved(robot).await?;
    let dir = dir.unwrap_or_else(|| default_out_dir("edits"));
    std::fs::create_dir_all(&dir)?;
    let st = stamp();
    let bf = dir.join(format!("{st}-restore-before.txt"));
    std::fs::write(&bf, &before.envelope)?;
    let bl = dir.join(format!("{st}-restore-before-live.txt"));
    std::fs::write(&bl, &before.live_envelope)?;
    println!("before: {}", bf.display());
    println!("before (live frame, for restore-settings): {}", bl.display());
    let r = recovery::restore(c, dev, map_id, url, recovery::Wait::default()).await?;
    print_job("restore", &r);
    if let Some(s) = server {
        print_requests(&s.requests(), r.succeeded());
    }
    // Give the robot time to re-save and upload the map before reading it back.
    let start = std::time::Instant::now();
    let mut after = mapedit::read_saved(robot).await?;
    while after.pointer == before.pointer && start.elapsed() < std::time::Duration::from_secs(60) {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        after = mapedit::read_saved(robot).await?;
    }
    let af = dir.join(format!("{st}-restore-after.txt"));
    std::fs::write(&af, &after.envelope)?;
    println!("after:  {}", af.display());
    println!("6.8 (saved map) changed: {}", after.pointer != before.pointer);
    let changes = mapedit::diff(&before.map, &after.map);
    if changes.is_empty() {
        println!("no change seen in the saved map");
    }
    for line in changes {
        println!("  {line}");
    }
    Ok(())
}

async fn recovery_entry(c: &CloudClient, dev: &DeviceInfo, index: usize) -> Result<recovery::Entry> {
    let l = recovery::list(c, dev).await?;
    let n = l.len();
    l.into_iter().nth(index).ok_or_else(|| anyhow!("no recovery entry {index}; the list has {n}"))
}

async fn cmd_map_action(c: CloudClient, dev: DeviceInfo, action: MapCmd) -> Result<()> {
    let robot = CloudRobot::new(Arc::new(c), dev);
    let (c, dev) = (&*robot.client, &robot.device);
    match action {
        MapCmd::Recovery => {
            println!("backup status (6.14): {:?}", recovery::backup_state(c, dev).await?);
            println!("restore status (6.11): {:?}", recovery::restore_state(c, dev).await?);
            print_entries(&recovery::list(c, dev).await?);
        }
        MapCmd::FetchArchive { index, out } => {
            let e = recovery_entry(c, dev, index).await?;
            let url = recovery::archive_url(c, dev, &e).await?;
            let bytes = c.fetch_bytes(&url).await?;
            let out = out.unwrap_or_else(|| default_out_dir("recovery").join(format!("{}-{index}.tbz2", stamp())));
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&out, &bytes)?;
            println!("{} .../{}: {} bytes; wrote {}", e.kind, last_segment(&e.objname), bytes.len(), out.display());
        }
        MapCmd::Backup { map_id, apply } => {
            let map_id = match map_id {
                Some(m) => m,
                None => {
                    let id = mapedit::read_saved(&robot).await?.map.map_id;
                    u32::try_from(id).map_err(|_| anyhow!("the saved map has no valid map id ({id})"))?
                }
            };
            println!("action 6.3 in [{{piid 4, value \"{map_id}\"}}]: back up map {map_id}");
            if !apply {
                println!("dry run; add --apply to send");
                return Ok(());
            }
            if let Some(b) = robot.busy().await? {
                bail!("the robot is busy ({b}); back up when it is idle");
            }
            let r = recovery::backup(c, dev, map_id, recovery::Wait::default()).await?;
            println!("action result: {}", r.result);
            print_job("backup", &r);
            print_entries(&recovery::list(c, dev).await?);
        }
        MapCmd::RequestFrame { apply, force } => {
            let info = if force {
                serde_json::json!({"req_type": 1, "frame_type": "I", "force_type": 1})
            } else {
                serde_json::json!({"frame_type": "I"})
            };
            let cmd = dreame_model::commands::request_map_with(info.clone());
            println!("action 6.1 with frame info {info}: request a full map frame");
            if !apply {
                println!("dry run; add --apply to send");
                return Ok(());
            }
            if force {
                let r = dreame::control::send(c, dev, &cmd).await?;
                let out = r["out"].as_array().cloned().unwrap_or_default();
                let piid = |n: u64| {
                    out.iter().find(|p| p["piid"] == n).and_then(|p| p["value"].as_str()).filter(|s| !s.is_empty()).map(str::to_string)
                };
                println!("result code {}; object (piid 3) {}; inline map (piid 1) {}", r["code"], piid(3).is_some(), piid(1).is_some());
                let env = match (piid(1), piid(3)) {
                    (Some(inline), _) => inline,
                    (None, Some(obj)) => live_envelope(c, dev, &obj).await?,
                    (None, None) => bail!("the reply holds no map"),
                };
                let f = Frame::decode(&env, None, None)?;
                let h = &f.header;
                println!("map frame {:?} map {} frame {}", h.frame_type, h.map_id, h.frame_id);
                match &f.trailer.cleanset {
                    None => println!("the frame has no per-room settings (cleanset)"),
                    Some(cs) => {
                        for (id, s) in &cs.value {
                            println!("  room {id:>2}: settings {s:?}");
                        }
                    }
                }
                return Ok(());
            }
            // The robot pushes the frame over MQTT (6.1), so listen before asking.
            let live = Robot::cloud(robot.client.clone(), dev.clone());
            let mut events = live.events()?;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let r = dreame::control::send(c, dev, &cmd).await?;
            println!("action result: {r}");
            let wait = tokio::time::sleep(std::time::Duration::from_secs(60));
            tokio::pin!(wait);
            let frame = loop {
                tokio::select! {
                    _ = &mut wait => break None,
                    e = events.next() => match e {
                        None => break None,
                        Some(dreame::Event::MapFrame(f)) => break Some(f),
                        Some(_) => {}
                    },
                }
            };
            let Some(f) = frame else {
                bail!("no map frame arrived within 60 s");
            };
            let h = &f.header;
            println!("map frame {:?} map {} frame {}", h.frame_type, h.map_id, h.frame_id);
            match &f.trailer.cleanset {
                None => println!("the frame has no per-room settings (cleanset)"),
                Some(cs) => {
                    for (id, s) in &cs.value {
                        println!("  room {id:>2}: settings {s:?}");
                    }
                }
            }
        }
        MapCmd::Restore { index, apply, dir } => {
            let e = recovery_entry(c, dev, index).await?;
            println!("restore map {} from {} entry .../{} (time {})", e.map_id, e.kind, last_segment(&e.objname), e.time);
            if !apply {
                println!("dry run; add --apply to send");
                return Ok(());
            }
            let url = recovery::archive_url(c, dev, &e).await?;
            run_restore(&robot, e.map_id, &url, dir, None).await?;
        }
        MapCmd::EditArchive { edit, apply, archive, no_prime, dir, port } => {
            Box::pin(cmd_edit_archive(&robot, edit, apply, archive, no_prime, dir, port)).await?;
        }
        MapCmd::RestoreFile { archive, map_id, apply, dir, port } => {
            let body = std::fs::read(&archive).with_context(|| format!("read {}", archive.display()))?;
            let map_id = match map_id {
                Some(m) => m,
                None => {
                    let id = mapedit::read_saved(&robot).await?.map.map_id;
                    u32::try_from(id).map_err(|_| anyhow!("the saved map has no valid map id ({id})"))?
                }
            };
            println!("restore map {map_id} from {} ({} bytes), served from this machine over HTTP", archive.display(), body.len());
            if !apply {
                println!("dry run; add --apply to send");
                return Ok(());
            }
            let ip = c.otc_info(&dev.did).await?.local_ip.ok_or_else(|| anyhow!("the cloud does not report the robot's address"))?;
            let ip: std::net::IpAddr = ip.parse().map_err(|_| anyhow!("the robot's address does not parse"))?;
            let name = format!("{}-{map_id}.tbz2", stamp());
            let server = dreame::fileserve::FileServer::start(body, &name, ip, port).await?;
            println!("serving on the local network, port {} (address not shown)", server.port());
            run_restore(&robot, map_id, server.url(), dir, Some(&server)).await?;
        }
        MapCmd::List { names } => {
            let maps = mapedit::read_saved_maps(&robot).await?;
            println!("saved maps: {} (current id {:?})", maps.maps.len(), maps.current);
            for m in &maps.maps {
                let map = Map::from_frame(&Frame::decode(&m.envelope, None, None)?, mapedit::MAP_VERSION, true)?;
                let current = if maps.current_map().is_some_and(|c| c.map_id == m.map_id) { ", current" } else { "" };
                let name = match (names, m.name.is_empty()) {
                    (_, true) => String::new(),
                    (true, false) => format!(", name {:?}", m.name),
                    (false, false) => ", named".to_string(),
                };
                println!("  map {}: {} rooms, rotation {}{name}{current}", m.map_id, map.rooms.len(), m.angle);
            }
        }
        MapCmd::Snapshot { dir, map } => {
            let saved = mapedit::read_saved_map(&robot, map).await?;
            let dir = dir.unwrap_or_else(|| default_out_dir("snapshots"));
            std::fs::create_dir_all(&dir)?;
            let p = dir.join(format!("{}-saved.txt", stamp()));
            std::fs::write(&p, &saved.envelope)?;
            println!("map_id {}: {} rooms; wrote {}", saved.map.map_id, saved.map.rooms.len(), p.display());
        }
        MapCmd::Edit { edit, map, apply, dir } => {
            let e = edit.to_edit()?;
            if apply {
                let dir = dir.unwrap_or_else(|| default_out_dir("edits"));
                let r = mapedit::apply_to(&robot, map, &e, &dir, &stamp(), mapedit::Wait::default()).await?;
                println!(
                    "result code {:?}; 6.8 (saved map) changed: {}; 6.3 (live frame) changed: {}",
                    r.code, r.pointer_changed, r.live_changed
                );
                // CloudRobot drops the echoed device id from the result.
                println!("action result: {}", r.result);
                println!("before: {}", r.before_file.display());
                println!("after:  {}", r.after_file.display());
                if r.changes.is_empty() {
                    println!("no change seen in the saved map");
                }
                for line in &r.steps {
                    println!("step: {line}");
                }
                for line in &r.changes {
                    println!("  {line}");
                }
            } else {
                let saved = mapedit::read_saved_map(&robot, map).await?;
                saved.check_fully_read()?;
                let m = &saved.map;
                let r = &m.restrictions;
                println!("map_id {}{}: {} rooms", m.map_id, if saved.current { "" } else { " (not the current map)" }, m.rooms.len());
                for room in m.rooms.values() {
                    let kind = match room.room_type {
                        None => "no name or type".to_string(),
                        Some(0) => "custom name".to_string(),
                        Some(t) => format!("type {t} index {}", room.index.unwrap_or(0)),
                    };
                    println!("  room {:>2}: {kind}{}", room.id, if room.hidden { ", hidden" } else { "" });
                }
                for (i, z) in r.no_go.iter().enumerate() {
                    println!("  no-go {i}: {:?}", z.to_quad());
                }
                for (i, z) in r.no_mop.iter().enumerate() {
                    println!("  no-mop {i}: {:?}", z.to_quad());
                }
                for (i, w) in r.virtual_walls.iter().enumerate() {
                    println!("  wall {i}: {:?}", w.to_quad());
                }
                for (i, c) in m.carpet_areas.iter().enumerate() {
                    let shape = if c.shape.is_ellipse() { "ellipse" } else { "rect" };
                    println!("  carpet {i}: id {} {shape} {:?} type {:?}", c.id, c.shape.to_quad(), c.kind);
                }
                for (i, z) in m.ignored_carpet.iter().enumerate() {
                    println!("  ignored-carpet {i}: {:?}", z.to_quad());
                }
                for (i, l) in r.thresholds.iter().enumerate() {
                    println!("  threshold {i}: {:?}", l.to_quad());
                }
                for (i, l) in r.impassable_thresholds.iter().enumerate() {
                    println!("  impassable-threshold {i}: {:?}", l.to_quad());
                }
                for (i, l) in r.curtains.iter().enumerate() {
                    println!("  curtain {i}: {:?}", l.to_quad());
                }
                let steps = mapedit::steps(m, &e)?;
                mapedit::check_target(&saved, &e, &steps)?;
                for (label, cmd) in &steps {
                    println!("command {label} (names hidden): {}", mapedit::redacted(cmd));
                }
                if matches!(e, mapedit::Edit::MoveBoundary { .. }) {
                    println!("then: names, floors, settings and order are restored from what the split returns");
                }
                println!("dry run: nothing sent; add --apply to send it");
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_map(
    c: &CloudClient,
    dev: &DeviceInfo,
    which: &str,
    map_id: Option<i64>,
    out: Option<PathBuf>,
    scale: u32,
    floor_plan: bool,
    names: bool,
    save_envelope: Option<PathBuf>,
) -> Result<()> {
    // Map version 2 matches r2579a; models with MAP_V2 need 3. Capability
    // detection will choose this once the device model crate exists.
    let version = 2u8;
    let (envelope, saved) = match which {
        "saved" => {
            let ptr = cached_text(c, dev, PropId::new(6, 8)).await?;
            let maps = saved_maps(c, dev, &ptr).await?;
            println!("saved maps: {:?} (current id {:?})", maps.ids(), maps.current);
            (maps.pick(map_id)?.envelope.clone(), true)
        }
        "current" if map_id.is_some() => bail!("--map picks a saved map; it does not go with --which current"),
        "current" => {
            let ptr = cached_text(c, dev, PropId::new(6, 3)).await?;
            (live_envelope(c, dev, &ptr).await?, false)
        }
        other => bail!("--which must be saved or current, not {other}"),
    };
    if let Some(p) = save_envelope {
        std::fs::write(&p, &envelope)?;
    }
    let frame = Frame::decode(&envelope, None, None)?;
    let map = if saved { Map::from_frame(&frame, version, true)? } else { Map::from_live(&frame, version)? };
    let h = &frame.header;
    println!(
        "map_id {} frame {} {:?}: {} x {} cells of {} mm; {} rooms ({} hidden); {} carpets; {} furniture; {} path points",
        h.map_id,
        h.frame_id,
        h.frame_type,
        map.grid.width,
        map.grid.height,
        h.grid_size,
        map.rooms.len(),
        map.rooms.values().filter(|r| r.hidden).count(),
        map.carpets.len() + map.carpet_areas.len(),
        map.furniture.len(),
        map.path.points.len(),
    );
    if names {
        for r in map.rooms.values() {
            println!(
                "  room {:>2}: {:<20} type {:?} order {:?}{}",
                r.id,
                r.name.as_deref().unwrap_or("-"),
                r.room_type,
                r.order,
                if r.hidden { " (hidden)" } else { "" }
            );
        }
    }
    if let Some(out) = out {
        let opts = RenderOptions { scale, floor_plan, ..RenderOptions::default() };
        save_png(&map, &opts, &out)?;
        println!("wrote {}", out.display());
    }
    Ok(())
}

async fn cmd_control(c: CloudClient, dev: DeviceInfo, action: ControlCmd, apply: bool) -> Result<()> {
    use dreame::control;
    use dreame_model::commands;
    let robot = Robot::cloud(Arc::new(c), dev);
    let cmd = match action {
        ControlCmd::Wait { timeout_min } => {
            let opts = dreame::watch::WaitRun { timeout: std::time::Duration::from_secs(timeout_min * 60), ..Default::default() };
            let r = dreame::watch::wait_run_end(&robot, opts, &mut |l| println!("{l}")).await?;
            println!("run ended: {}", r.end);
            if r.watch.codes.is_empty() {
                println!("no error or fault codes seen");
            } else {
                let codes: Vec<String> = r.watch.codes.iter().map(|c| format!("{} ({c})", c.code())).collect();
                println!("codes seen: {}", codes.join(", "));
            }
            return Ok(());
        }
        ControlCmd::Remote { direction, seconds } => return remote(&robot, direction, seconds, apply).await,
        ControlCmd::UploadPhoto { index } => {
            let obs = robot.obstacles().await?;
            let o = obs.get(index).ok_or_else(|| anyhow!("no obstacle {index}; there are {}", obs.len()))?;
            let p = o.photo.as_ref().ok_or_else(|| anyhow!("obstacle {index} has no photo"))?;
            commands::upload_obstacle_photo(&p.file, &p.pwd, &p.object())
        }
        ControlCmd::Start => commands::start(),
        ControlCmd::Pause => commands::pause(),
        ControlCmd::Stop => commands::stop(),
        ControlCmd::Dock => commands::dock(),
        ControlCmd::CleanRooms { ids, opts } => control::clean_rooms(&robot.saved_map().await?, &ids, opts.into())?,
        ControlCmd::CleanZone { x0, y0, x1, y1, opts } => {
            let zone = dreame_map::model::Rect::new(x0, y0, x1, y1);
            control::clean_zone(&robot.saved_map().await?, zone, opts.into())?
        }
    };
    println!("command: {}", control::describe(&cmd));
    if !apply {
        println!("dry run: nothing sent; add --apply to send it");
        return Ok(());
    }
    // Robot::send fails when the robot does not accept the command.
    let r = robot.send(&cmd).await?;
    println!("result: {r}");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    println!("status 5 s later:");
    print_status(&robot.refresh().await?, &robot.cloud_link().device, false);
    Ok(())
}

/// Drives one direction for `seconds`, one step a second, then sends a stop.
async fn remote(robot: &Robot, dir: RemoteDir, seconds: f64, apply: bool) -> Result<()> {
    use dreame_model::commands::{remote_move, RemoteMove, REMOTE_REPEAT_MS};
    let step = dir.step();
    // A turn-around and a stop are single steps; the others repeat.
    let repeat = !(step.is_stop() || step == RemoteMove::TURN_AROUND);
    let steps = if repeat { (seconds * 1000.0 / REMOTE_REPEAT_MS as f64).ceil().max(1.0) as u32 } else { 1 };
    println!("command: {} x{steps}", dreame::control::describe(&remote_move(step, true, 0)));
    if repeat {
        println!("then: {}", dreame::control::describe(&remote_move(RemoteMove::STOP, false, 0)));
    }
    if !apply {
        println!("dry run: nothing sent; add --apply to send it");
        return Ok(());
    }
    for i in 0..steps {
        let r = robot.remote_move(step, i == 0).await?;
        println!("step {}: {r}", i + 1);
        if i + 1 < steps {
            tokio::time::sleep(std::time::Duration::from_millis(REMOTE_REPEAT_MS)).await;
        }
    }
    if repeat {
        tokio::time::sleep(std::time::Duration::from_millis(REMOTE_REPEAT_MS)).await;
        println!("stop: {}", robot.remote_move(RemoteMove::STOP, false).await?);
    }
    for (label, wait) in [("now", 0), ("10 s later", 10)] {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        let s = robot.read(&[PropId::new(2, 1), PropId::new(4, 1)]).await?;
        println!("{label}: state (2.1) {:?}, work mode (4.1) {:?}", s.state, s.status);
    }
    Ok(())
}

async fn cmd_obstacles(c: CloudClient, dev: DeviceInfo, save: Option<PathBuf>) -> Result<()> {
    let robot = Robot::cloud(Arc::new(c), dev);
    let obs = robot.obstacles().await?;
    println!("{} obstacles in the live map", obs.len());
    if let Some(dir) = &save {
        std::fs::create_dir_all(dir)?;
    }
    for (i, o) in obs.iter().enumerate() {
        let photo = match &o.photo {
            None => "no photo".to_string(),
            Some(p) => format!("photo, upload status {:?}", p.upload_status),
        };
        let pet = if o.is_pet() { " (pet)" } else { "" };
        println!("{i}: type {}{pet} at ({:.0}, {:.0}) mm, confidence {:.2}, {photo}", o.kind, o.x, o.y, o.confidence);
        if let (Some(dir), Some(p)) = (&save, &o.photo) {
            match robot.obstacle_photo(p).await {
                Ok(jpeg) => {
                    let path = dir.join(format!("obstacle-{i}.jpg"));
                    std::fs::write(&path, &jpeg)?;
                    println!("   saved {} ({} bytes)", path.display(), jpeg.len());
                }
                Err(e) => println!("   photo not fetched: {e:#}"),
            }
        }
    }
    Ok(())
}

async fn cmd_settings(c: CloudClient, dev: DeviceInfo) -> Result<()> {
    let robot = Robot::cloud(Arc::new(c), dev);
    robot.refresh().await?;
    // Settings outside the core services come from the cache unless read live.
    let s = robot.read(&dreame::settings::all_properties()).await?;
    for setting in dreame::settings::known(&s) {
        let ids: Vec<String> = setting.properties().iter().map(ToString::to_string).collect();
        println!("{:<30} {:<12} {}", setting.name(), ids.join(","), setting.current(&s).unwrap_or_else(|| "-".into()));
    }
    Ok(())
}

async fn cmd_set(c: CloudClient, dev: DeviceInfo, cmd: &SetCmd, apply: bool) -> Result<()> {
    let mut robot = Robot::cloud(Arc::new(c), dev);
    // Discovery first, so a property the robot lacks is refused before anything is sent.
    robot.discover(&Default::default()).await?;
    robot.refresh().await?;
    // The setting's own properties live, since refresh reads only the core services live.
    let props = cmd.to_setting(&robot.status())?.properties();
    let before = robot.read(&props).await?;
    let setting = cmd.to_setting(&before)?;
    if let Some(caps) = robot.capabilities() {
        if let Some(id) = setting.properties().into_iter().find(|id| !caps.present(*id)) {
            bail!("this robot has no property {id} ({}); nothing to change", setting.name());
        }
    }
    println!("{}: now {}", setting.name(), setting.current(&before).unwrap_or_else(|| "unknown".into()));
    for w in setting.commands(&before)? {
        println!("write: {}", dreame::control::describe(&w));
    }
    if !apply {
        println!("dry run: nothing sent; add --apply to send it");
        return Ok(());
    }
    for r in robot.set(&setting).await? {
        println!("result: {r}");
    }
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let after = robot.read(&props).await?;
    println!("{}: read back {}", setting.name(), setting.current(&after).unwrap_or_else(|| "unknown".into()));
    Ok(())
}

async fn cmd_status(c: CloudClient, dev: DeviceInfo, raw: bool, discover: bool) -> Result<()> {
    let mut robot = Robot::cloud(Arc::new(c), dev);
    if discover {
        let caps = robot.discover(&Default::default()).await?;
        let (present, ..) = caps.counts();
        println!("discovery: {present} properties present");
        for (f, on) in &caps.features {
            println!("  feature {f:?}: {on}");
        }
    }
    let s = robot.refresh().await?;
    print_status(&s, &robot.cloud_link().device, raw);
    Ok(())
}

fn print_status(s: &dreame_model::Status, dev: &DeviceInfo, raw: bool) {
    let show = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    println!("model     {}  fw {}", dev.model, dev.firmware);
    println!("state     {}", show(s.state.map(|x| x.to_string())));
    println!("status    {}", show(s.status.map(|x| x.to_string())));
    println!("task      {} / {}", show(s.task_status.map(|x| x.to_string())), show(s.task_type.map(|x| x.to_string())));
    println!("battery   {}%  charging {}", show(s.battery.map(|x| x.to_string())), show(s.charging.map(|x| x.to_string())));
    println!("error     {}  active faults {:?}", show(s.error.map(|x| x.to_string())), s.active_faults().iter().map(|f| f.to_string()).collect::<Vec<_>>());
    println!("last job  {} min, {} m2", show(s.cleaning_time_min.map(|x| x.to_string())), show(s.cleaned_area_m2.map(|x| x.to_string())));
    println!("suction   {}  wetness {}", show(s.suction.map(|x| x.to_string())), show(s.wetness.map(|x| x.to_string())));
    println!(
        "dnd       {} {}-{}",
        show(s.dnd.enabled.map(|x| x.to_string())),
        show(s.dnd.start.map(|x| x.to_string())),
        show(s.dnd.end.map(|x| x.to_string()))
    );
    for (k, life) in &s.consumables {
        println!("consumable {:?}: {}% left, {} h", k, show(life.percent.map(|x| x.to_string())), show(life.time_left.map(|x| x.to_string())));
    }
    println!("station   {:?}", s.station.self_wash_base.map(|x| x.to_string()));
    println!("cleans    {}", show(s.totals.cleaning_count.map(|x| x.to_string())));
    println!("values    {} read, {} decode errors", s.raw.len(), s.decode_errors.len());
    if raw {
        for (id, v) in &s.raw {
            println!("  {id} = {}", v.to_json());
        }
    }
}

/// Short text for a pushed value; long telemetry blobs are summarised.
fn show_value(id: PropId, v: &dreame_core::RawValue, all: bool) -> String {
    let text = match v.to_json() {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    if !all && id == PropId::new(99, 98) {
        return format!("<{} chars>", text.len());
    }
    text
}

async fn cmd_watch(c: CloudClient, dev: DeviceInfo, seconds: u64, all: bool) -> Result<()> {
    use dreame::Event;
    let robot = Robot::cloud(Arc::new(c), dev);
    let mut events = robot.events()?;
    let start = std::time::Instant::now();
    let deadline = async {
        if seconds == 0 {
            std::future::pending::<()>().await
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(seconds)).await
        }
    };
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = tokio::signal::ctrl_c() => break,
            event = events.next() => {
                let Some(event) = event else { break };
                let t = start.elapsed().as_secs();
                match event {
                    Event::Status { changed, status } => {
                        for id in changed {
                            let v = status.raw.get(&id).map(|v| show_value(id, v, all)).unwrap_or_default();
                            println!("[{t:>5}s] {id} = {v}");
                        }
                    }
                    Event::MapFrame(f) => {
                        let h = &f.header;
                        println!("[{t:>5}s] map frame {:?} map {} frame {} robot ({}, {}) angle {}", h.frame_type, h.map_id, h.frame_id, h.robot.x, h.robot.y, h.robot.angle);
                    }
                    Event::RobotEvent { siid, eiid, arguments } => {
                        let args: Vec<String> = arguments.iter().map(|(p, v)| format!("{p}={}", v.to_json())).collect();
                        println!("[{t:>5}s] event {siid}.e{eiid} {}", args.join(" "));
                    }
                    Event::Connection(state) => eprintln!("[mqtt] {state:?}"),
                }
            }
        }
    }
    events.stop().await;
    Ok(())
}

fn main() -> Result<()> {
    // The command futures are large; debug builds overflow the 1 MB main-thread stack on Windows.
    // Run the runtime on a thread with a larger stack instead.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(16 * 1024 * 1024)
                .build()?
                .block_on(run())
        })?
        .join()
        .map_err(|_| anyhow!("the CLI thread panicked"))?
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Session::new();
    match cli.cmd {
        Cmd::Login => {
            let c = ctx.login().await?;
            println!(
                "logged in: region {}, country {}; session saved: {}",
                c.region().code().unwrap_or("custom"),
                c.country().unwrap_or_default(),
                ctx.has_store()
            );
        }
        Cmd::Logout => {
            ctx.forget();
            println!("session removed");
        }
        Cmd::Devices => {
            let c = ctx.client().await?;
            for d in c.list_devices().await? {
                println!("{}  {}  fw {}  {}  {}", d.did, d.model, d.firmware, if d.online { "online" } else { "offline" }, d.name);
            }
        }
        Cmd::History { limit, shape, from, key, map } => {
            let since = from.unwrap_or(1_687_019_188);
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            // From 2023-06-17, as early as the cloud keeps history.
            if shape {
                // `--key 6.3` looks at a property's history instead of the cleaning records.
                let (kind, siid, iid) = match &key {
                    Some(k) => {
                        let id = parse_ids(std::slice::from_ref(k))?.into_iter().next().ok_or_else(|| anyhow!("bad key"))?;
                        ("piid", id.siid, id.piid)
                    }
                    None => ("eiid", 4, 1),
                };
                let reply = c.history_reply(&dev.did, kind, siid, iid, since, limit).await?;
                println!("{}", shape_of(&reply));
                return Ok(());
            }
            let entries = c.event_history(&dev.did, 4, 1, since, limit).await?;
            println!("{} entries", entries.len());
            for e in &entries {
                match dreame::history::record_from_entry(e) {
                    Ok(r) => println!("{}", dreame::history::summary(&r)),
                    Err(err) => println!("undecoded entry: {err}"),
                }
            }
            if map {
                // The newest job's map: how many rooms and path points it holds.
                let robot = dreame::CloudRobot::new(std::sync::Arc::new(c), dev);
                let jobs = dreame::history::jobs(&robot, 1).await?;
                let Some(job) = jobs.first() else { bail!("no jobs") };
                match dreame::history::job_map(&robot, job).await? {
                    Some(m) => println!("newest job's map: {} rooms, {} path points, {} active rooms", m.rooms.len(), m.path.points.len(), m.active_rooms.len()),
                    None => println!("the newest job names no map"),
                }
            }
        }
        Cmd::Props { ids, cached } => {
            let ids = parse_ids(&ids)?;
            if ids.is_empty() {
                bail!("give property ids such as 2.1 3.1");
            }
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            if cached {
                for p in c.cached_properties(&dev.did, &ids).await? {
                    println!("{} = {}", p.id, p.value.to_json());
                }
            } else {
                for p in c.get_properties(&dev, &ids).await? {
                    match p.value {
                        Some(v) if p.code == 0 => println!("{} = {}", p.id, v.to_json()),
                        _ => println!("{} code {}", p.id, p.code),
                    }
                }
            }
        }
        Cmd::Status { raw, discover } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_status(c, dev, raw, discover).await?;
        }
        Cmd::Watch { seconds, all } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_watch(c, dev, seconds, all).await?;
        }
        Cmd::Obstacles { save } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_obstacles(c, dev, save).await?;
        }
        Cmd::Control { action, apply } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_control(c, dev, action, apply).await?;
        }
        Cmd::Settings => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_settings(c, dev).await?;
        }
        Cmd::Set { setting, apply } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            cmd_set(c, dev, &setting, apply).await?;
        }
        Cmd::Map { action, which, map, out, scale, floor_plan, names, save_envelope } => {
            let c = ctx.client().await?;
            let dev = pick_device(&c, cli.did.as_deref()).await?;
            match action {
                // Boxed: the future is too large for the 1 MB main-thread stack on Windows.
                Some(a) => Box::pin(cmd_map_action(c, dev, a)).await?,
                None => cmd_map(&c, &dev, &which, map, out, scale, floor_plan, names, save_envelope).await?,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_centres_in_a_rectangle() {
        // x from 0 to 100 mm holds the centres 25 and 75; y from 20 to 30 holds 25.
        assert_eq!(cell_centres(100.0, 30.0, 0.0, 20.0), [(25.0, 25.0), (75.0, 25.0)]);
        assert!(cell_centres(0.0, 0.0, 10.0, 10.0).is_empty());
    }
    use clap::CommandFactory;

    #[test]
    fn cli_parses() {
        Cli::command().debug_assert();
        // The plain map command keeps its flags.
        let c = Cli::try_parse_from(["dreame", "map", "--which", "current", "--names"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: None, ref which, names: true, .. } if which == "current"));
        // Edits are dry runs unless --apply is given.
        let c = Cli::try_parse_from(["dreame", "map", "edit", "add-no-go", "0", "0", "500", "500"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Edit { apply: false, map: None, .. }), .. }));
        // Multi-floor: list the maps; pick one for render, snapshot and edit.
        let c = Cli::try_parse_from(["dreame", "map", "list"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::List { names: false }), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "--map", "2", "--out", "m.png"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: None, map: Some(2), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "snapshot", "--map", "2"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Snapshot { map: Some(2), .. }), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "hide-rooms", "3", "--map", "2"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Edit { map: Some(2), apply: false, .. }), .. }));
        // Backup and restore are dry runs unless --apply is given.
        let c = Cli::try_parse_from(["dreame", "map", "backup"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Backup { map_id: None, apply: false }), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "edit-archive", "grow", "10", "12", "up", "10"]).unwrap();
        assert!(matches!(
            c.cmd,
            Cmd::Map {
                action: Some(MapCmd::EditArchive {
                    edit: ArchiveEditCmd::Grow { room: 10, from: 12, direction: dreame_map::segmap::Direction::Up, cells: 10 },
                    apply: false,
                    no_prime: false,
                    port: RESTORE_PORT,
                    ..
                }),
                ..
            }
        ));
        let c = Cli::try_parse_from(["dreame", "map", "restore", "2", "--apply"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Restore { index: 2, apply: true, .. }), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "remove-no-go", "0", "--apply"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Map { action: Some(MapCmd::Edit { apply: true, .. }), .. }));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "room-settings", "3", "--suction", "2"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert!(matches!(edit.to_edit().unwrap(), mapedit::Edit::RoomSettings { id: 3, suction: Some(2), water: None, .. }));
        // Negative coordinates parse as numbers, not flags.
        let c = Cli::try_parse_from(["dreame", "map", "edit", "add-wall", "-500", "-500", "0", "-20"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert!(matches!(edit, EditCmd::AddWall { x0: -500.0, y1: -20.0, .. }));
        // Hiding takes the full list, which may be empty; the order needs at least one id.
        let c = Cli::try_parse_from(["dreame", "map", "edit", "hide-rooms"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert_eq!(edit.to_edit().unwrap(), mapedit::Edit::HideRooms(vec![]));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "cleaning-order", "3", "1", "2"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert_eq!(edit.to_edit().unwrap(), mapedit::Edit::CleaningOrder(vec![3, 1, 2]));
        assert!(Cli::try_parse_from(["dreame", "map", "edit", "cleaning-order"]).is_err());
        let c = Cli::try_parse_from(["dreame", "map", "edit", "split-room", "4", "-100", "0", "-100", "900"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert_eq!(
            edit.to_edit().unwrap(),
            mapedit::Edit::SplitRoom { id: 4, line: dreame_map::model::Line::new(-100.0, 0.0, -100.0, 900.0) }
        );
        let c = Cli::try_parse_from(["dreame", "map", "edit", "merge-rooms", "3", "5"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert_eq!(edit.to_edit().unwrap(), mapedit::Edit::MergeRooms(3, 5));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "move-boundary", "3", "5", "-100", "0", "-100", "900"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert!(matches!(edit.to_edit().unwrap(), mapedit::Edit::MoveBoundary { a: 3, b: 5, .. }));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "add-carpet", "-500", "0", "500", "900"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert!(matches!(edit.to_edit().unwrap(), mapedit::Edit::AddCarpet(_)));
        let c = Cli::try_parse_from(["dreame", "map", "edit", "remove-impassable-threshold", "2"]).unwrap();
        let Cmd::Map { action: Some(MapCmd::Edit { edit, .. }), .. } = c.cmd else { panic!("not an edit") };
        assert_eq!(edit.to_edit().unwrap(), mapedit::Edit::RemoveImpassableThreshold(2));
    }

    #[test]
    fn control_parses_and_defaults_to_dry_run() {
        let c = Cli::try_parse_from(["dreame", "control", "dock"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Control { action: ControlCmd::Dock, apply: false }));
        let c = Cli::try_parse_from(["dreame", "control", "clean-rooms", "3", "5", "--passes", "2", "--apply"]).unwrap();
        let Cmd::Control { action: ControlCmd::CleanRooms { ids, opts }, apply: true } = c.cmd else { panic!("not a room clean") };
        assert_eq!(ids, [3, 5]);
        assert_eq!((opts.passes, opts.suction, opts.water), (Some(2), None, None));
        assert!(Cli::try_parse_from(["dreame", "control", "clean-rooms"]).is_err());
        let c = Cli::try_parse_from(["dreame", "control", "clean-zone", "-500", "0", "500", "1000", "--suction", "2"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Control { action: ControlCmd::CleanZone { x0: -500.0, .. }, apply: false }));
    }

    #[test]
    fn set_parses_and_defaults_to_dry_run() {
        use dreame_model::{DecodeContext, Status};
        let c = Cli::try_parse_from(["dreame", "set", "volume", "30"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Set { setting: SetCmd::Volume { level: 30 }, apply: false }));
        let c = Cli::try_parse_from(["dreame", "set", "ai", "pet detection", "on", "--apply"]).unwrap();
        let Cmd::Set { setting, apply: true } = c.cmd else { panic!("not a set") };
        let s = Status::new(DecodeContext::default());
        assert!(matches!(setting.to_setting(&s).unwrap(), dreame::Setting::AiFlag { on: true, .. }));
        let c = Cli::try_parse_from(["dreame", "set", "auto-switch", "MeticulousTwist", "-1"]).unwrap();
        assert!(matches!(c.cmd, Cmd::Set { setting: SetCmd::AutoSwitch { value: -1, .. }, .. }));
        // DND without times needs the current window.
        let c = Cli::try_parse_from(["dreame", "set", "dnd", "on"]).unwrap();
        let Cmd::Set { setting, .. } = c.cmd else { panic!("not a set") };
        assert!(setting.to_setting(&s).is_err());
        let c = Cli::try_parse_from(["dreame", "set", "dnd", "on", "--start", "23:00", "--end", "07:00"]).unwrap();
        let Cmd::Set { setting, .. } = c.cmd else { panic!("not a set") };
        assert!(matches!(setting.to_setting(&s).unwrap(), dreame::Setting::Dnd { enabled: true, .. }));
        assert!(Cli::try_parse_from(["dreame", "set", "child-lock", "maybe"]).is_err());
    }
}
