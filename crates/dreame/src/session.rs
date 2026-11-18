//! Credentials and the saved login session.
//!
//! Credentials come from the environment (`DREAME_USER_EMAIL`, `DREAME_PASSWORD`,
//! `DREAME_REGION`) or from the first `local/env` file found in the current directory or one of
//! its parents. After a login, the rotating refresh token is kept in `local/session.json` next to
//! that file, so later runs do not need the password. Values are never printed.

use anyhow::{anyhow, Result};
use dreame_cloud::{CloudClient, CloudConfig, DeviceInfo, Region, StoredSession, TokenStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Finds `local/env` in the current directory or a parent.
pub fn find_local_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("local");
        if cand.join("env").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Reads `KEY=VALUE` lines.
fn read_env_file(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect()
}

fn setting(name: &str, file: &[(String, String)]) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| file.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
        .filter(|v| !v.is_empty())
}

/// Keeps the session in a JSON file.
pub struct FileTokenStore(PathBuf);

impl FileTokenStore {
    /// A store at `path`, for example a file in an app's private folder.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> Option<StoredSession> {
        serde_json::from_str(&std::fs::read_to_string(&self.0).ok()?).ok()
    }
    fn save(&self, s: &StoredSession) {
        let Ok(text) = serde_json::to_string(s) else { return };
        if let Some(dir) = self.0.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        // The file holds a refresh token: readable by the owner only, where the system has modes.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        if let Ok(mut f) = opts.open(&self.0) {
            use std::io::Write;
            let _ = f.write_all(text.as_bytes());
        }
    }
    fn clear(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Credentials and the session store found from the current directory.
pub struct Session {
    env: Vec<(String, String)>,
    store: Option<Arc<FileTokenStore>>,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        let local = find_local_dir();
        let env = local.as_ref().map(|d| read_env_file(&d.join("env"))).unwrap_or_default();
        let store = local.map(|d| Arc::new(FileTokenStore(d.join("session.json"))));
        Self { env, store }
    }

    /// Whether a session file exists to forget or resume.
    pub fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// Deletes the saved session.
    pub fn forget(&self) {
        if let Some(s) = &self.store {
            s.clear();
        }
    }

    fn config(&self) -> CloudConfig {
        let cfg = CloudConfig::default();
        match &self.store {
            Some(s) => cfg.with_token_store(s.clone()),
            None => cfg,
        }
    }

    /// Logs in with the password.
    pub async fn login(&self) -> Result<CloudClient> {
        let user = setting("DREAME_USER_EMAIL", &self.env)
            .or_else(|| setting("DREAME_USER", &self.env))
            .ok_or_else(|| anyhow!("set DREAME_USER_EMAIL (or put it in local/env)"))?;
        let pass = setting("DREAME_PASSWORD", &self.env).ok_or_else(|| anyhow!("set DREAME_PASSWORD"))?;
        let regions: Vec<Region> =
            setting("DREAME_REGION", &self.env).map(|r| vec![Region::parse(&r)]).unwrap_or_default();
        Ok(CloudClient::login_any_with(self.config(), &regions, &user, &pass).await?)
    }

    /// Resumes the saved session, or logs in with the password.
    pub async fn client(&self) -> Result<CloudClient> {
        if self.store.as_ref().and_then(|s| s.load()).is_some() {
            match CloudClient::from_store(self.config()).await {
                Ok(c) => return Ok(c),
                Err(e) => eprintln!("saved session failed ({e}); logging in again"),
            }
        }
        self.login().await
    }
}

/// The device with id `did`, or the first vacuum on the account.
pub async fn pick_device(c: &CloudClient, did: Option<&str>) -> Result<DeviceInfo> {
    let devs = c.list_devices().await?;
    match did {
        Some(d) => devs.into_iter().find(|x| x.did == d).ok_or_else(|| anyhow!("device {d} not on this account")),
        None => devs.into_iter().find(|x| x.is_vacuum()).ok_or_else(|| anyhow!("no vacuum on this account")),
    }
}

/// `local/out/<sub>` in the `local/` directory, or `<sub>` in the current directory.
pub fn default_out_dir(sub: &str) -> PathBuf {
    find_local_dir().map(|l| l.join("out").join(sub)).unwrap_or_else(|| PathBuf::from(sub))
}

/// Seconds since the Unix epoch, for snapshot file names.
pub fn stamp() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
}
