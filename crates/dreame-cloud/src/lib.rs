//! Dreamehome cloud REST client for Dreame robot vacuums.
//!
//! The protocol is described in `docs/research/protocols.md`. The main entry
//! point is [`CloudClient`]: log in with a password or a stored refresh token,
//! list devices, read properties (relayed to the robot or from the cloud
//! cache), send commands and resolve OSS download URLs.
//!
//! Secrets (passwords, access and refresh tokens) are never logged, and every
//! `Debug` implementation in this crate redacts them.

pub mod auth;
pub mod client;
pub mod http;
pub mod region;
pub mod sign;

pub use auth::{MemoryTokenStore, StoredSession, TokenStore};
pub use client::{CachedProp, CloudClient, CloudConfig, OtcInfo};
pub use dreame_core::{ActionId, AppConstants, DeviceInfo, Error, PropId, PropResult, RawValue, Result};
pub use http::{HttpRequest, HttpResponse, HttpTransport, Method, ReqwestTransport};
pub use region::{Brand, Region};
pub use sign::Signer;
