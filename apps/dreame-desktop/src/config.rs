//! Where the app keeps its files, and the small preferences it remembers.
//!
//! The files live in the system's per-user app folder: `%APPDATA%` on Windows,
//! `~/Library/Application Support` on macOS, and `$XDG_CONFIG_HOME` (usually `~/.config`) on
//! Linux. The password is never stored; the session file holds a refresh token.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub dir: PathBuf,
    /// The cloud session (refresh token).
    pub session: PathBuf,
    pub prefs: PathBuf,
    /// Problems worth keeping for support, one line each. Never holds secrets.
    pub log: PathBuf,
    /// What the robot supports, found once per firmware, so later starts skip discovery.
    pub capabilities: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        let dir = directories::ProjectDirs::from("org", "dreame-control", "Dreame Control")
            .map(|p| p.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("dreame-control"));
        if let Some(old) = directories::ProjectDirs::from("org", "dreame-open", "Dreame Open") {
            adopt_old_dir(old.config_dir(), &dir);
        }
        Self::in_dir(dir)
    }

    pub fn in_dir(dir: PathBuf) -> Self {
        Self {
            session: dir.join("session.json"),
            prefs: dir.join("prefs.json"),
            log: dir.join("log.txt"),
            capabilities: dir.join("capabilities.json"),
            dir,
        }
    }
}

/// Moves the folder of the app's earlier name, "Dreame Open", to `new`, so an update keeps the
/// session and preferences. Does nothing when `new` already exists or `old` does not.
fn adopt_old_dir(old: &std::path::Path, new: &std::path::Path) {
    if new.exists() || !old.is_dir() {
        return;
    }
    if let Some(parent) = new.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::rename(old, new);
}

/// Appends one line, with the time in seconds since 1970, to the log file.
pub fn log_line(path: &std::path::Path, line: &str) {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{t} {line}");
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    /// The account email, to fill the sign-in form.
    pub email: Option<String>,
    /// The cloud region code; `None` tries every region.
    pub region: Option<String>,
    /// The robot chosen last, by device id.
    pub device: Option<String>,
    /// Show the advanced map editing tools.
    #[serde(default)]
    pub advanced: bool,
    /// Send system notifications while the app is in the background.
    #[serde(default = "yes")]
    pub notify: bool,
    /// The language as a tag such as `pt-BR`; `None` follows the system.
    #[serde(default)]
    pub language: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self { email: None, region: None, device: None, advanced: false, notify: true, language: None }
    }
}

impl Prefs {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(&paths.prefs).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        let _ = std::fs::create_dir_all(&paths.dir);
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&paths.prefs, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_folder_is_adopted_once() {
        let root = std::env::temp_dir().join(format!("dreame-adopt-{}", std::process::id()));
        let (old, new) = (root.join("old"), root.join("new"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("prefs.json"), "{}").unwrap();
        adopt_old_dir(&old, &new);
        assert!(new.join("prefs.json").is_file() && !old.exists());
        std::fs::create_dir_all(&old).unwrap();
        adopt_old_dir(&old, &new);
        assert!(old.exists(), "an existing new folder is never replaced");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn prefs_round_trip() {
        let dir = std::env::temp_dir().join(format!("dreame-desktop-test-{}", std::process::id()));
        let paths = Paths::in_dir(dir.clone());
        assert_eq!(Prefs::load(&paths), Prefs::default());
        let p = Prefs {
            email: Some("a@b.c".into()),
            region: Some("eu".into()),
            device: Some("1".into()),
            advanced: true,
            notify: false,
            language: Some("pt-BR".into()),
        };
        p.save(&paths);
        assert_eq!(Prefs::load(&paths), p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prefs_from_before_the_language_load() {
        let old = r#"{"email":"a@b.c","region":null,"device":null,"advanced":false,"notify":true}"#;
        let p: Prefs = serde_json::from_str(old).unwrap();
        assert_eq!(p.language, None);
        assert_eq!(p.email.as_deref(), Some("a@b.c"));
    }
}
