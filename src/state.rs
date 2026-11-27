//! Small run state persisted next to the config (`data/state.json`):
//! when the layout was last saved / restored.

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunState {
    /// Unix seconds of the last save (0 = never).
    pub last_save_at: u64,
    /// Unix seconds of the last restore (0 = never).
    pub last_restore_at: u64,
    /// True once the user saved manually: that layout is then the fixed
    /// restore source and automatic saves go to a separate fallback layout.
    pub pinned: bool,
}

impl RunState {
    /// Missing or broken files simply yield "never".
    pub fn load(path: &Path) -> RunState {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        crate::paths::write_atomic(path, json.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_never_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!("wk-state-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("state.json");
        assert_eq!(RunState::load(&p), RunState::default());
        let s = RunState {
            last_save_at: 5,
            last_restore_at: 9,
            pinned: true,
        };
        s.save(&p).unwrap();
        assert_eq!(RunState::load(&p), s);
        std::fs::write(&p, "{\"last_save_at\": 3}").unwrap();
        assert_eq!(RunState::load(&p).last_restore_at, 0);
        std::fs::write(&p, "garbage").unwrap();
        assert_eq!(RunState::load(&p), RunState::default());
        std::fs::remove_dir_all(&dir).ok();
    }
}
