//! Dreamehome cloud MQTT push client for Dreame robot vacuums.
//!
//! The robot reports state changes through the Dreame MQTT broker. This crate
//! subscribes to the device topic and turns each message into [`Push`]
//! values. It never publishes: commands go through the REST API in
//! `dreame-cloud`. The protocol is described in
//! `docs/research/protocols.md`, section 11.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo(cloud: std::sync::Arc<dreame_cloud::CloudClient>) -> dreame_core::Result<()> {
//! use dreame_mqtt::{MqttClient, MqttConfig, Push};
//!
//! let dev = cloud.list_devices().await?.into_iter().find(|d| d.is_vacuum()).unwrap();
//! let cfg = MqttConfig::from_cloud(&dev, &cloud)?;
//! let (handle, mut pushes) = MqttClient::connect(cfg, cloud.clone());
//! while let Some(push) = pushes.recv().await {
//!     if let Push::PropertiesChanged(changes) = push {
//!         for (id, value) in changes {
//!             println!("{id} = {value:?}");
//!         }
//!     }
//! }
//! handle.stop().await;
//! # Ok(()) }
//! ```
//!
//! # Design
//!
//! - **Hand-written MQTT client.** The client needs only CONNECT, SUBSCRIBE
//!   with QoS 0, PINGREQ and DISCONNECT, and reads PUBLISH. That is about
//!   150 lines (module `packet`), already proven against the broker by the
//!   research spike. `rumqttc` was the planned choice, but it would bring its
//!   own rustls and tokio-rustls version pins, a publish path this read-only
//!   client must not use, and an event loop that keeps the password fixed,
//!   so every token change would mean rebuilding it. A small client makes
//!   the token-per-connect rule and the certificate handling explicit.
//! - **Pinned certificate check.** The broker's chain ends in a
//!   Dreame-private root that `rustls-webpki` cannot parse. [`DreamePinned`]
//!   checks the chain, the host name and the handshake signature itself,
//!   against that root only. Verification is never switched off.
//! - **Fresh token per connect.** A [`TokenSource`] supplies the password
//!   before every attempt, and a refreshed one after CONNACK 4 or 5.
//! - **Pure decoding.** [`decode_message`] has no I/O, so it is tested on
//!   synthetic and captured payloads.
//!
//! Nothing in this crate logs tokens, uids, device ids, topics or payloads.

mod client;
mod config;
mod decode;
mod packet;
mod tls;

pub use client::{ConnectionState, MqttClient, MqttHandle, PushStream};
pub use config::{random_client_id, MqttConfig, TokenFuture, TokenSource, DREAME_PORT, MOVA_PORT};
pub use decode::{decode_message, decode_value, Push};
pub use dreame_core::{Error, PropId, RawValue, Result};
pub use tls::{client_config, DreamePinned, DREAME_ROOT_CA_PEM};
