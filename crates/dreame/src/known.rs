//! What a robot supports, saved to a file after discovery, so later starts skip discovery
//! (about 25 seconds). The file holds one robot: the device id and its
//! [`KnownCapabilities`]. A saved result is used only for the same device, model and firmware.

use dreame_core::DeviceInfo;
use dreame_model::{Capabilities, KnownCapabilities};
use serde_json::{json, Value};
use std::path::Path;

/// The saved result for `device`, if the file holds one for this device, model and firmware.
pub fn load(file: &Path, device: &DeviceInfo) -> Option<KnownCapabilities> {
    let text = std::fs::read_to_string(file).ok()?;
    let saved: Value = serde_json::from_str(&text).ok()?;
    let known: KnownCapabilities = serde_json::from_value(saved.get("known")?.clone()).ok()?;
    let same = saved.get("did").and_then(Value::as_str) == Some(device.did.as_str());
    (same && known.fits(&device.model, &device.firmware)).then_some(known)
}

/// Saves a discovery result for `device`. Only a complete result is worth saving: a partial
/// one would hide features until the next firmware update. Returns whether it was saved.
pub fn save(file: &Path, device: &DeviceInfo, caps: &Capabilities) -> bool {
    if !caps.complete() {
        return false;
    }
    let saved = json!({ "did": device.did, "known": caps.known(&device.firmware) });
    serde_json::to_string(&saved).is_ok_and(|text| std::fs::write(file, text).is_ok())
}
