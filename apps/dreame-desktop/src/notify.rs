//! System notifications: a finished job, an error, a robot that seems stuck. They use the
//! system's own notification centre on Windows, macOS and Linux. What to say comes from
//! [`dreame::phase::notices`].

pub use dreame::phase::{notices, Notice};

/// Shows a notice in the system's notification centre, on a thread of its own so a slow
/// notification service never holds up the window.
pub fn show(n: &Notice) {
    let n = n.clone();
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new().appname("Dreame Control").summary(&n.title).body(&n.body).show();
    });
}
