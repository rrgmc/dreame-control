//! Robot access through the Dreamehome cloud: cached properties, map downloads, and the
//! [`MapAccess`](crate::mapedit::MapAccess) implementation the edit flow uses.

use crate::mapedit::MapAccess;
use crate::maplist::{self, SavedMaps, Source};
use anyhow::{anyhow, Context, Result};
use dreame_cloud::{CloudClient, DeviceInfo, PropId};
use dreame_map::edit::EditCommand;
use serde_json::Value;
use std::sync::Arc;

/// A property's cached value as text (strings unquoted).
pub async fn cached_text(c: &CloudClient, dev: &DeviceInfo, id: PropId) -> Result<String> {
    let v = c.cached_properties(&dev.did, &[id]).await?;
    let p = v.into_iter().next().ok_or_else(|| anyhow!("property {id} not in the cloud cache"))?;
    Ok(match p.value.to_json() {
        Value::String(s) => s,
        other => other.to_string(),
    })
}

/// Downloads an object (map frame, map list) by name.
pub async fn download_object(c: &CloudClient, dev: &DeviceInfo, object: &str) -> Result<String> {
    let url = c.download_url(&dev.did, &dev.model, object).await?;
    Ok(c.fetch_text(&url).await?)
}

/// Downloads the saved-map list that the 6.8 pointer names, and the envelope of every map in it.
pub async fn saved_maps(c: &CloudClient, dev: &DeviceInfo, pointer: &str) -> Result<SavedMaps> {
    let list = maplist::parse(&download_object(c, dev, &maplist::pointer_object(pointer)?).await?)?;
    let mut envelopes = Vec::with_capacity(list.entries.len());
    for e in &list.entries {
        envelopes.push(match &e.source {
            Source::Inline(t) => t.clone(),
            Source::Object(o) => download_object(c, dev, o).await.context("download a saved map object")?,
        });
    }
    SavedMaps::new(list, envelopes)
}

/// Downloads the live map frame that the 6.3 value (`<object>[,<key>]`) names. A key, when
/// present, is appended to the envelope as the decoder expects.
pub async fn live_envelope(c: &CloudClient, dev: &DeviceInfo, pointer: &str) -> Result<String> {
    let (obj, key) = match pointer.split_once(',') {
        Some((o, k)) => (o, Some(k)),
        None => (pointer, None),
    };
    let text = download_object(c, dev, obj).await?;
    Ok(match key {
        Some(k) => format!("{},{k}", text.trim()),
        None => text,
    })
}

/// The robot behind the cloud client.
#[derive(Clone)]
pub struct CloudRobot {
    pub client: Arc<CloudClient>,
    pub device: DeviceInfo,
}

impl CloudRobot {
    pub fn new(client: Arc<CloudClient>, device: DeviceInfo) -> Self {
        Self { client, device }
    }
}

impl MapAccess for CloudRobot {
    async fn saved_pointer(&self) -> Result<String> {
        cached_text(&self.client, &self.device, PropId::new(6, 8)).await
    }
    async fn saved_maps(&self, pointer: &str) -> Result<SavedMaps> {
        saved_maps(&self.client, &self.device, pointer).await
    }
    async fn update_map(&self, cmd: &EditCommand) -> Result<Value> {
        let mut r = self.client.action(&self.device, dreame_core::ActionId::new(6, 2), vec![cmd.action_in()]).await?;
        // The result echoes the device id; drop it so no caller prints it by accident.
        if let Some(o) = r.as_object_mut() {
            o.remove("did");
        }
        Ok(r)
    }
    async fn busy(&self) -> Result<Option<String>> {
        use dreame_model::enums::{Open, TaskStatus};
        use dreame_model::status::{DecodeContext, Status};
        let id = PropId::new(4, 7);
        let r = self.client.get_properties(&self.device, &[id]).await?;
        let Some(v) = r.into_iter().find(|p| p.code == 0).and_then(|p| p.value) else {
            return Ok(Some("task status 4.7 could not be read".into()));
        };
        let s = Status::from_values([(&id, &v)], DecodeContext::default());
        Ok(match s.task_status {
            Some(Open::Known(TaskStatus::Completed)) => None,
            Some(t) => Some(t.to_string()),
            None => Some("task status 4.7 did not decode".into()),
        })
    }
    async fn live_pointer(&self) -> Result<String> {
        cached_text(&self.client, &self.device, PropId::new(6, 3)).await
    }
    async fn live_envelope(&self, pointer: &str) -> Result<String> {
        live_envelope(&self.client, &self.device, pointer).await
    }
}
