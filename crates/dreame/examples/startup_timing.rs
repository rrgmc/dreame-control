//! Times each network step an app takes from start to a drawn map. Read-only.
//!
//! Run it from the main checkout, so that `local/` is found:
//! `cargo run -p dreame --example startup_timing`.

use dreame::mapedit::MapAccess;
use dreame::{Robot, Session};
use std::sync::Arc;
use std::time::Instant;

/// Prints the time since `t` and restarts it.
fn lap(t: &mut Instant, what: &str) {
    println!("{:>7.0} ms  {what}", t.elapsed().as_secs_f64() * 1000.0);
    *t = Instant::now();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let all = Instant::now();
    let t = &mut Instant::now();
    let client = Arc::new(Session::new().client().await?);
    lap(t, "resume the session (token refresh)");
    let device = dreame::session::pick_device(&client, None).await?;
    lap(t, "list devices");
    let mut robot = Robot::cloud(client, device);
    robot.refresh_cached().await?;
    lap(t, "status from the cloud cache");
    dreame::mapedit::read_saved(robot.cloud_link()).await?;
    lap(t, "saved map and live frame, together");
    println!("{:>7.0} ms  to a drawn map with saved capabilities\n", all.elapsed().as_secs_f64() * 1000.0);

    *t = Instant::now();
    robot.refresh().await?;
    lap(t, "status from the cache and the robot itself");
    robot.discover(&Default::default()).await?;
    lap(t, "discover capabilities");
    let link = robot.cloud_link();
    let p = link.saved_pointer().await?;
    lap(t, "saved map pointer (6.8)");
    link.saved_maps(&p).await?;
    lap(t, "saved map list download");
    let lp = link.live_pointer().await?;
    lap(t, "live map pointer (6.3)");
    link.live_envelope(&lp).await?;
    lap(t, "live map download");
    Ok(())
}
