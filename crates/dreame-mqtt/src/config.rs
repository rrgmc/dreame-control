//! Connection settings and token sources (protocols.md sections 11.1 and 11.2).

use dreame_cloud::{Brand, CloudClient};
use dreame_core::{DeviceInfo, Error, Result};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Broker port for Dreame when the device record has no `bindDomain`.
pub const DREAME_PORT: u16 = 19973;
/// Broker port for MOVA when the device record has no `bindDomain`.
pub const MOVA_PORT: u16 = 19974;

/// Where and as whom to connect. Holds no secrets: the password (the access
/// token) comes from a [`TokenSource`] on each connect. `Debug` still hides
/// the ids, because they identify the account and the robot.
#[derive(Clone, PartialEq, Eq)]
pub struct MqttConfig {
    /// Broker host name; also checked against the certificate.
    pub host: String,
    pub port: u16,
    /// Account id; the MQTT username.
    pub uid: String,
    /// Device id.
    pub did: String,
    /// Device model, for example `dreame.vacuum.r2579a`.
    pub model: String,
    /// Account region code, for example `us`.
    pub region: String,
    /// MQTT client id. [`random_client_id`] gives the usual form.
    pub client_id: String,
    /// Keep-alive sent in CONNECT. A PINGREQ goes out at half this interval.
    pub keep_alive: Duration,
}

impl fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("uid", &"<redacted>")
            .field("did", &"<redacted>")
            .field("model", &self.model)
            .field("region", &self.region)
            .field("keep_alive", &self.keep_alive)
            .finish_non_exhaustive()
    }
}

/// `p_` followed by 16 random hex digits, as node-dreame and the spike use.
pub fn random_client_id() -> String {
    format!("p_{:016x}", rand::random::<u64>())
}

impl MqttConfig {
    /// Builds the config from a device record, the account uid and the
    /// account region code. The broker comes from `bind_domain` (`host:port`).
    /// Without it, the Dreame fallback `app.mt.<region>.iot.dreame.tech:19973`
    /// is used.
    pub fn from_device(dev: &DeviceInfo, uid: impl Into<String>, region: impl Into<String>) -> Result<Self> {
        Self::from_device_for_brand(dev, uid, region, Brand::Dreame)
    }

    /// Like [`from_device`](Self::from_device), with the brand choosing the fallback host.
    pub fn from_device_for_brand(dev: &DeviceInfo, uid: impl Into<String>, region: impl Into<String>, brand: Brand) -> Result<Self> {
        let region = region.into().to_ascii_lowercase();
        let (host, port) = match dev.bind_domain.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(b) => match b.rsplit_once(':') {
                Some((h, p)) => (h.to_string(), p.parse().map_err(|_| Error::Decode("bad bindDomain port".into()))?),
                None => (b.to_string(), DREAME_PORT),
            },
            None => match brand {
                Brand::Mova => (format!("app.mt.{region}.iot.mova-tech.com"), MOVA_PORT),
                _ => (format!("app.mt.{region}.iot.dreame.tech"), DREAME_PORT),
            },
        };
        if region.is_empty() {
            return Err(Error::Decode("empty region".into()));
        }
        Ok(Self {
            host,
            port,
            uid: uid.into(),
            did: dev.did.clone(),
            model: dev.model.clone(),
            region,
            client_id: random_client_id(),
            keep_alive: Duration::from_secs(60),
        })
    }

    /// Builds the config from a device record and a logged-in cloud client.
    /// Fails for a client on a custom host, which has no region code.
    pub fn from_cloud(dev: &DeviceInfo, client: &CloudClient) -> Result<Self> {
        let region = client.region();
        let code = region.code().ok_or_else(|| Error::Unsupported("custom cloud host has no region code".into()))?;
        Self::from_device_for_brand(dev, client.uid(), code, client.brand())
    }

    /// Topics to subscribe: `/status/<did>/<uid>/<model>/<region>/`. Korean
    /// accounts also subscribe with region `sg` (protocols.md section 11.2).
    pub fn topics(&self) -> Vec<String> {
        let t = |r: &str| format!("/status/{}/{}/{}/{}/", self.did, self.uid, self.model, r);
        let mut v = vec![t(&self.region)];
        if self.region == "kr" {
            v.push(t("sg"));
        }
        v
    }
}

/// Future returned by [`TokenSource::token`].
pub type TokenFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// Supplies the access token used as the MQTT password.
///
/// The client calls it before every connect, so a reconnect always uses the
/// current token. `refresh` is true after the broker refused the previous
/// token (CONNACK 4 or 5); the source should then get a new one.
///
/// Implemented for `Arc<CloudClient>` and for closures
/// `Fn(bool) -> impl Future<Output = Result<String>>`.
pub trait TokenSource: Send + Sync + 'static {
    fn token(&self, refresh: bool) -> TokenFuture<'_>;
}

impl<F, Fut> TokenSource for F
where
    F: Fn(bool) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String>> + Send + 'static,
{
    fn token(&self, refresh: bool) -> TokenFuture<'_> {
        Box::pin(self(refresh))
    }
}

impl TokenSource for Arc<CloudClient> {
    /// Refreshes when asked to, or when less than 10 minutes remain.
    fn token(&self, refresh: bool) -> TokenFuture<'_> {
        Box::pin(async move {
            if refresh || self.expires_in() < Duration::from_secs(600) {
                self.refresh().await?;
            }
            Ok(self.access_token())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(bind: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            did: "1234567".into(),
            model: "dreame.vacuum.x".into(),
            bind_domain: bind.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn from_bind_domain() {
        let c = MqttConfig::from_device(&dev(Some("10000.mt.us.iot.dreame.tech:19973")), "42", "US").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("10000.mt.us.iot.dreame.tech", 19973));
        assert_eq!(c.topics(), vec!["/status/1234567/42/dreame.vacuum.x/us/".to_string()]);
        assert!(c.client_id.starts_with("p_") && c.client_id.len() == 18);
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("1234567") && !dbg.contains("\"42\""));
    }

    #[test]
    fn fallback_host_and_kr_topics() {
        let c = MqttConfig::from_device(&dev(None), "42", "kr").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("app.mt.kr.iot.dreame.tech", 19973));
        assert_eq!(c.topics().len(), 2);
        assert!(c.topics()[1].ends_with("/sg/"));
        let m = MqttConfig::from_device_for_brand(&dev(None), "42", "eu", Brand::Mova).unwrap();
        assert_eq!((m.host.as_str(), m.port), ("app.mt.eu.iot.mova-tech.com", 19974));
    }
}
