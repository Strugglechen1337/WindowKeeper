//! Persisted layout: saved window placements and the merge logic.

use crate::geometry::Placement;
use crate::matching::{match_windows, Identity};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of saved entries per exe+class group.
pub const MAX_PER_GROUP: usize = 12;
/// Maximum total number of saved entries.
pub const MAX_TOTAL: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub identity: Identity,
    pub placement: Placement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    #[serde(default)]
    pub windows: Vec<Entry>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            version: 1,
            windows: Vec::new(),
        }
    }
}

impl Layout {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn identities(&self) -> Vec<Identity> {
        self.windows.iter().map(|e| e.identity.clone()).collect()
    }

    /// Merge a snapshot of live windows into the saved layout.
    ///
    /// Matched entries are updated in place, unmatched live windows are
    /// appended, saved entries without a live counterpart are kept (their
    /// application may simply be closed). Returns true if anything changed.
    pub fn merge(&mut self, live: Vec<Entry>) -> bool {
        let before = self.windows.clone();
        let ids: Vec<Identity> = live.iter().map(|e| e.identity.clone()).collect();
        let matches = match_windows(&self.identities(), &ids);
        for (entry, m) in live.into_iter().zip(matches) {
            match m {
                Some(si) => self.windows[si] = entry,
                None => self.windows.push(entry),
            }
        }
        self.trim();
        self.windows != before
    }

    /// Enforce the per-group and total caps, dropping the oldest entries.
    fn trim(&mut self) {
        let mut i = 0;
        while i < self.windows.len() {
            let id = self.windows[i].identity.clone();
            let group: Vec<usize> = (0..self.windows.len())
                .filter(|&j| {
                    let o = &self.windows[j].identity;
                    o.exe_path.eq_ignore_ascii_case(&id.exe_path)
                        && o.class_name.eq_ignore_ascii_case(&id.class_name)
                })
                .collect();
            if group.len() > MAX_PER_GROUP {
                self.windows.remove(group[0]);
            } else {
                i += 1;
            }
        }
        while self.windows.len() > MAX_TOTAL {
            self.windows.remove(0);
        }
    }

    pub fn load(path: &Path) -> io::Result<Layout> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::paths::write_atomic(path, self.to_json().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{MonitorRef, Rect, ShowState};

    fn entry(title: &str, x: i32) -> Entry {
        Entry {
            identity: Identity {
                exe_path: "C:\\e.exe".into(),
                class_name: "Cls".into(),
                title: title.into(),
            },
            placement: Placement {
                state: ShowState::Normal,
                restore_maximized: false,
                rect: Rect::new(x, 0, 100, 100),
                monitor: MonitorRef::default(),
            },
        }
    }

    #[test]
    fn merge_updates_adds_and_keeps() {
        let mut l = Layout::default();
        assert!(l.merge(vec![entry("A", 1), entry("B", 2)]));
        assert_eq!(l.windows.len(), 2);
        // A moved, B closed (not in snapshot): B is kept.
        assert!(l.merge(vec![entry("A", 10)]));
        assert_eq!(l.windows.len(), 2);
        assert_eq!(l.windows[0].placement.rect.x, 10);
        assert_eq!(l.windows[1].identity.title, "B");
        // A new window of another class is appended.
        let mut other = entry("C", 3);
        other.identity.class_name = "Other".into();
        assert!(l.merge(vec![entry("A", 10), other.clone()]));
        assert_eq!(l.windows.len(), 3);
        // Same snapshot again: nothing changes.
        assert!(!l.merge(vec![entry("A", 10), other]));
    }

    #[test]
    fn group_cap_drops_oldest() {
        let mut l = Layout::default();
        for i in 0..(MAX_PER_GROUP + 5) {
            // Distinct titles with no similarity so each is a new entry.
            let t = format!("{}{}", char::from(b'a' + (i % 26) as u8), "#".repeat(i));
            l.windows.push(entry(&t, i as i32));
        }
        l.trim();
        assert_eq!(l.windows.len(), MAX_PER_GROUP);
        assert_eq!(l.windows[0].placement.rect.x, 5);
    }

    #[test]
    fn json_roundtrip_and_file_io() {
        let mut l = Layout::default();
        l.merge(vec![entry("A", 1)]);
        let back: Layout = serde_json::from_str(&l.to_json()).unwrap();
        assert_eq!(l, back);
        let dir = std::env::temp_dir().join(format!("wk-layout-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("layout.json");
        l.save(&p).unwrap();
        assert_eq!(Layout::load(&p).unwrap(), l);
        std::fs::remove_dir_all(&dir).ok();
    }
}
