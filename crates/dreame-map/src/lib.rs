//! Offline map codec for Dreame robot vacuums.
//!
//! The format is described in `docs/research/maps.md`. The crate has no network code, so a desktop
//! editor can open saved files offline.
//!
//! Layers, from wire to editor:
//!
//! - [`envelope`]: text envelope (URL-safe base64, optional AES-256-CBC, zlib) to raw bytes.
//! - [`header`]: the 27-byte frame header.
//! - [`frame`]: header, raw pixel bytes and typed JSON trailer together.
//! - [`grid`]: pixel encodings A to E and Wi-Fi, decoded into a classified grid.
//! - [`trailer`]: serde structs for the JSON trailer; unknown keys survive a round trip.
//! - [`path`]: the `tr` path string.
//! - [`merge`]: I-frame and P-frame sequencing ([`merge::MapState`]).
//! - [`model`]: the editable [`model::Map`] in world millimetres.
//! - [`render`]: RGBA raster and PNG export.
//! - [`edit`]: typed builders for the action 6.2 piid-4 edit payloads.
//! - [`archive`]: recovery archives (bzip2 tar), edited member by member.
//! - [`segmap`]: the room grid images in an archive, and edits to them.
//! - [`view`]: pan and zoom over a rendered map, for GUIs.

pub mod archive;
pub mod edit;
pub mod envelope;
pub mod frame;
pub mod grid;
pub mod header;
pub mod merge;
pub mod model;
pub mod obstacle;
pub mod path;
pub mod render;
pub mod roomreg;
pub mod segmap;
pub mod trailer;
pub mod view;

pub use dreame_core::{Error, Result};
pub use frame::Frame;
pub use grid::{Encoding, Grid, Pixel, PixelClass};
pub use header::{FrameType, Header, Pose};
pub use merge::MapState;
pub use model::Map;
pub use trailer::Trailer;

/// Builds a [`Error::Decode`] from any message.
pub(crate) fn decode_err(msg: impl Into<String>) -> Error {
    Error::Decode(msg.into())
}
