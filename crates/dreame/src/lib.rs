//! Facade over the Dreame crates for apps: the login session, a typed [`Robot`] (status,
//! capabilities, live events, commands), and the map edit flow (build, snapshot, apply, read
//! back, diff).
//!
//! The CLI and the desktop editor both use it, so a map edit takes the same tested path in both.

pub mod archiveedit;
pub mod camera;
pub mod cloud;
pub mod control;
pub mod fileserve;
pub mod history;
pub mod live;
pub mod known;
pub mod mapedit;
pub mod maplist;
pub mod phase;
pub mod recovery;
pub mod robot;
pub mod session;
pub mod shapes;
pub mod watch;
pub mod settings;

pub use cloud::CloudRobot;
pub use robot::{Event, Events, Robot};
pub use session::Session;
pub use settings::Setting;
