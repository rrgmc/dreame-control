//! Device model for Dreame robot vacuums: the MIoT profile, value codecs,
//! open enums, capability discovery, a typed status snapshot and command
//! builders.
//!
//! The reference is `docs/research/device-model.md`. Enum tables and the
//! property id table are ported from Tasshack's dreame-vacuum (MIT License).
//!
//! - [`profile`]: every known property, action and event, with per-model
//!   overrides.
//! - [`enums`]: open enums (`Known(T)` or `Unknown(i64)`); labels are hints.
//! - [`codec`]: composite values such as 4.23, 4.50, 4.22, DND and schedules.
//! - [`discovery`]: read the profile from the robot and derive capabilities.
//! - [`status`]: a typed snapshot from property values of any source.
//! - [`commands`]: builders for actions and property writes. Nothing in
//!   this crate sends them.

pub mod codec;
pub mod commands;
pub mod discovery;
pub mod enums;
pub mod profile;
pub mod status;

pub use commands::Command;
pub use discovery::{discover, Capabilities, DiscoverOptions, Feature, KnownCapabilities, Presence, Probe, PropertyReader};
pub use enums::{CodeEnum, Open};
pub use profile::Profile;
pub use status::{DecodeContext, Status};
