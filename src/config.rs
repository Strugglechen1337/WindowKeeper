//! User configuration (TOML) with defaults and validation.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Save the layout automatically when windows are moved/changed.
    pub auto_save: bool,
    /// How long (minutes) late-appearing windows are restored after start.
    pub restore_watch_minutes: u32,
    /// Debounce / throttle for automatic saves in milliseconds.
    pub debounce_ms: u32,
    /// Interval of the conservative backup scan in seconds.
    pub scan_interval_secs: u32,
    /// Check for updates at startup (at most once per day).
    pub check_updates_on_start: bool,
    /// Unix timestamp (seconds) of the last update check.
    pub last_update_check: u64,
    /// The "running in tray" hint was shown already.
    pub first_run_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_save: true,
            restore_watch_minutes: 5,
            debounce_ms: 1500,
            scan_interval_secs: 30,
            check_updates_on_start: true,
            last_update_check: 0,
            first_run_done: false,
        }
    }
}

impl Config {
    /// Bring values into sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.restore_watch_minutes = self.restore_watch_minutes.min(24 * 60);
        self.debounce_ms = self.debounce_ms.clamp(200, 60_000);
        self.scan_interval_secs = self.scan_interval_secs.clamp(5, 3600);
        self
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
        assert!(c.auto_save);
        assert_eq!(c.restore_watch_minutes, 5);
        assert!(c.check_updates_on_start);
    }

    #[test]
    fn roundtrip() {
        let c = Config {
            auto_save: false,
            last_update_check: 42,
            ..Config::default()
        };
        assert_eq!(Config::from_toml(&c.to_toml()).unwrap(), c);
    }

    #[test]
    fn partial_file_uses_defaults_and_ignores_unknown() {
        let c = Config::from_toml("auto_save = false\nfoo = 1\n").unwrap();
        assert!(!c.auto_save);
        assert_eq!(c.restore_watch_minutes, 5);
    }

    #[test]
    fn values_are_sanitized() {
        let c = Config::from_toml("debounce_ms = 1\nscan_interval_secs = 0\n").unwrap();
        assert_eq!(c.debounce_ms, 200);
        assert_eq!(c.scan_interval_secs, 5);
    }

    #[test]
    fn missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("wk-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        assert_eq!(Config::load_or_default(&p), (Config::default(), false));
        std::fs::write(&p, "auto_save = [").unwrap();
        assert_eq!(Config::load_or_default(&p), (Config::default(), true));
        assert!(dir.join("config.toml.bad").exists());
        let c = Config {
            debounce_ms: 999,
            ..Config::default()
        };
        c.save(&p).unwrap();
        assert_eq!(Config::load_or_default(&p).0, c);
        std::fs::remove_dir_all(&dir).ok();
    }
}
