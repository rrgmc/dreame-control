//! Pictures from the robot camera: the obstacles and pets it recognised, with their photos.
//!
//! The official app's "take photo" grabs a frame from the live Tencent video on the phone, so
//! it is not available here. The robot's own photos are the obstacle and pet photos listed in
//! the live map. See docs/research/remote-and-camera.md.

use crate::cloud::{cached_text, live_envelope, CloudRobot};
use anyhow::{Context, Result};
use dreame_core::PropId;
use dreame_map::obstacle::{self, Obstacle, PhotoRef};
use dreame_map::Frame;

/// The obstacles in the current live map (6.3). Read-only.
pub async fn obstacles(robot: &CloudRobot) -> Result<Vec<Obstacle>> {
    let pointer = cached_text(&robot.client, &robot.device, PropId::new(6, 3)).await?;
    let envelope = live_envelope(&robot.client, &robot.device, &pointer).await?;
    let frame = Frame::decode(&envelope, None, None).context("decode the live frame (6.3)")?;
    Ok(frame.trailer.extra.get("ai_obstacle").map(obstacle::parse).unwrap_or_default())
}

/// Downloads and decrypts one obstacle photo; returns JPEG bytes. Read-only. Fails when the
/// robot has not uploaded the photo; [`crate::Robot::upload_obstacle_photo`] asks it to.
pub async fn photo(robot: &CloudRobot, p: &PhotoRef) -> Result<Vec<u8>> {
    let c = &robot.client;
    let url = c
        .permanent_download_url(&robot.device.did, &robot.device.model, &p.object())
        .await
        .context("find the photo in cloud storage")?;
    let data = c.fetch_bytes(&url).await.context("download the photo")?;
    Ok(p.decrypt(&data)?)
}
