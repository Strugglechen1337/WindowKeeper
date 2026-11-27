//! User configuration (TOML) with defaults and validation.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Upper bound of the late-window watch time (minutes).
pub const WATCH_MINUTES_MAX: u32 = 24 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// How long (minutes) late-appearing windows are restored after start.
    pub restore_watch_minutes: u32,
    /// Check for updates at startup (at most once per day).
    pub check_updates_on_start: bool,
    /// Unix timestamp (seconds) of the last update check.
    pub last_update_check: u64,
    /// The "running in tray" hint was shown already.
    pub first_run_done: bool,
}

/// The user editable subset of [`Config`] (what the settings page shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsEdit {
    pub restore_watch_minutes: u32,
    pub check_updates_on_start: bool,
}

impl Default for SettingsEdit {
    fn default() -> Self {
        Config::default().settings()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            restore_watch_minutes: 10,
            check_updates_on_start: true,
            last_update_check: 0,
            first_run_done: false,
        }
    }
}

impl Config {
    /// Bring values into sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.restore_watch_minutes = self.restore_watch_minutes.min(WATCH_MINUTES_MAX);
        self
    }

    /// Apply the values edited in the GUI (sanitized). Returns true if
    /// anything changed.
    pub fn apply_settings(&mut self, s: &SettingsEdit) -> bool {
        let before = self.clone();
        self.restore_watch_minutes = s.restore_watch_minutes;
        self.check_updates_on_start = s.check_updates_on_start;
        *self = self.clone().sanitized();
        *self != before
    }

    pub fn settings(&self) -> SettingsEdit {
        SettingsEdit {
            restore_watch_minutes: self.restore_watch_minutes,
            check_updates_on_start: self.check_updates_on_start,
        }
    }

    pub fn from_toml(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str::<Config>(text).map(Config::sanitized)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Load the config; a missing file yields defaults. A broken file is
    /// moved aside (`.bad`) and defaults are used. The bool tells whether the
    /// file was broken.
    pub fn load_or_default(path: &Path) -> (Config, bool) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Config::from_toml(&text) {
                Ok(c) => (c, false),
                Err(_) => {
                    let _ = std::fs::rename(path, path.with_extension("toml.bad"));
                    (Config::default(), true)
                }
            },
            Err(_) => (Config::default(), false),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::paths::write_atomic(path, self.to_toml().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.restore_watch_minutes, 10);
        assert!(c.check_updates_on_start);
    }

    #[test]
    fn roundtrip() {
        let c = Config {
            restore_watch_minutes: 3,
            last_update_check: 42,
            ..Config::default()
        };
        assert_eq!(Config::from_toml(&c.to_toml()).unwrap(), c);
    }

    #[test]
    fn partial_file_uses_defaults_and_ignores_unknown() {
        // Keys of removed settings (auto_save, ...) in old files are ignored.
        let c = Config::from_toml("auto_save = false\ndebounce_ms = 1\nfoo = 1\n").unwrap();
        assert_eq!(c.restore_watch_minutes, 10);
    }

    #[test]
    fn values_are_sanitized() {
        let c = Config::from_toml("restore_watch_minutes = 999999\n").unwrap();
        assert_eq!(c.restore_watch_minutes, WATCH_MINUTES_MAX);
    }

    #[test]
    fn missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("wk-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        assert_eq!(Config::load_or_default(&p), (Config::default(), false));
        std::fs::write(&p, "restore_watch_minutes = [").unwrap();
        assert_eq!(Config::load_or_default(&p), (Config::default(), true));
        assert!(dir.join("config.toml.bad").exists());
        let c = Config {
            restore_watch_minutes: 7,
            ..Config::default()
        };
        c.save(&p).unwrap();
        assert_eq!(Config::load_or_default(&p).0, c);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_settings_sanitizes_and_keeps_internal_fields() {
        let mut c = Config {
            last_update_check: 7,
            first_run_done: true,
            ..Config::default()
        };
        let mut e = c.settings();
        assert!(!c.apply_settings(&e));
        e.restore_watch_minutes = 100_000;
        e.check_updates_on_start = false;
        assert!(c.apply_settings(&e));
        assert_eq!(c.restore_watch_minutes, 24 * 60);
        assert!(!c.check_updates_on_start);
        assert_eq!(c.last_update_check, 7);
        assert!(c.first_run_done);
    }
}
