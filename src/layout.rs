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
    /// Ignored entries stay in the list but are never updated by saving and
    /// never restored automatically (only via the per-entry button).
    #[serde(default)]
    pub ignored: bool,
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

    pub fn ignored_count(&self) -> usize {
        self.windows.iter().filter(|e| e.ignored).count()
    }

    /// Index of the entry with exactly this identity.
    pub fn position_of(&self, identity: &Identity) -> Option<usize> {
        self.windows.iter().position(|e| &e.identity == identity)
    }

    /// For every live window the saved entry to restore it to. Ignored
    /// entries take part in the matching (so a live window is not paired with
    /// a different entry) but are never returned.
    pub fn restore_plan(&self, live: &[Identity]) -> Vec<Option<usize>> {
        match_windows(&self.identities(), live)
            .into_iter()
            .map(|m| m.filter(|&si| !self.windows[si].ignored))
            .collect()
    }

    /// Index (into `live`) of the window that belongs to saved entry `idx`,
    /// regardless of the ignore flag. Used for the per-entry restore.
    pub fn live_index_for(&self, idx: usize, live: &[Identity]) -> Option<usize> {
        match_windows(&self.identities(), live)
            .iter()
            .position(|m| *m == Some(idx))
    }

    /// Merge a snapshot of live windows into the saved layout.
    ///
    /// Matched entries are updated in place, unmatched live windows are
    /// appended, saved entries without a live counterpart are kept (their
    /// application may simply be closed). Ignored entries are left untouched
    /// (and their live window is not added a second time). Returns true if
    /// anything changed.
    pub fn merge(&mut self, live: Vec<Entry>) -> bool {
        let before = self.windows.clone();
        let ids: Vec<Identity> = live.iter().map(|e| e.identity.clone()).collect();
        let matches = match_windows(&self.identities(), &ids);
        for (entry, m) in live.into_iter().zip(matches) {
            match m {
                Some(si) if self.windows[si].ignored => {}
                Some(si) => {
                    self.windows[si] = Entry {
                        ignored: false,
                        ..entry
                    }
                }
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
                // Drop the oldest entry, preferring ones that are not ignored.
                let victim = group
                    .iter()
                    .copied()
                    .find(|&j| !self.windows[j].ignored)
                    .unwrap_or(group[0]);
                self.windows.remove(victim);
            } else {
                i += 1;
            }
        }
        while self.windows.len() > MAX_TOTAL {
            let victim = self.windows.iter().position(|e| !e.ignored).unwrap_or(0);
            self.windows.remove(victim);
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
            ignored: false,
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

    #[test]
    fn old_layout_without_ignored_flag_still_loads() {
        let json = r#"{"version":1,"windows":[{"identity":{"exe_path":"C:\\e.exe",
            "class_name":"Cls","title":"T"},"placement":{"state":"normal",
            "restore_maximized":false,"rect":{"x":1,"y":2,"w":3,"h":4},
            "monitor":{"dpi":96,"work":{"x":0,"y":0,"w":10,"h":10}}}}]}"#;
        let l: Layout = serde_json::from_str(json).unwrap();
        assert_eq!(l.windows.len(), 1);
        assert!(!l.windows[0].ignored);
    }

    #[test]
    fn merge_keeps_ignored_entries_untouched() {
        let mut l = Layout::default();
        l.merge(vec![entry("A", 1), entry("B", 2)]);
        l.windows[0].ignored = true;
        // A moved: the ignored entry keeps its old position and flag and the
        // live window is not appended as a duplicate.
        assert!(!l.merge(vec![entry("A", 99), entry("B", 2)]));
        assert_eq!(l.windows.len(), 2);
        assert!(l.windows[0].ignored);
        assert_eq!(l.windows[0].placement.rect.x, 1);
        // Non-ignored entries are still updated and never become ignored.
        assert!(l.merge(vec![entry("B", 5)]));
        assert_eq!(l.windows[1].placement.rect.x, 5);
        assert!(!l.windows[1].ignored);
        assert_eq!(l.ignored_count(), 1);
    }

    #[test]
    fn restore_plan_skips_ignored_but_per_entry_lookup_does_not() {
        let mut l = Layout::default();
        l.merge(vec![entry("A", 1), entry("B", 2)]);
        l.windows[0].ignored = true;
        let live = vec![entry("B", 0).identity, entry("A", 0).identity];
        assert_eq!(l.restore_plan(&live), vec![Some(1), None]);
        assert_eq!(l.live_index_for(0, &live), Some(1));
        assert_eq!(l.live_index_for(1, &live), Some(0));
        assert_eq!(l.live_index_for(0, &live[..1]), None);
    }

    #[test]
    fn trim_prefers_dropping_non_ignored() {
        let mut l = Layout::default();
        for i in 0..(MAX_PER_GROUP + 1) {
            let t = format!("{}{}", char::from(b'a' + (i % 26) as u8), "#".repeat(i));
            l.windows.push(entry(&t, i as i32));
        }
        l.windows[0].ignored = true;
        l.trim();
        assert_eq!(l.windows.len(), MAX_PER_GROUP);
        assert!(l.windows[0].ignored);
        assert_eq!(l.windows[1].placement.rect.x, 2);
    }

    #[test]
    fn position_lookup_by_identity() {
        let mut l = Layout::default();
        l.merge(vec![entry("A", 1), entry("B", 2)]);
        assert_eq!(l.position_of(&entry("B", 0).identity), Some(1));
        assert_eq!(l.position_of(&entry("Z", 0).identity), None);
    }
}
