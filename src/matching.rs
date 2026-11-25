//! Window identity and matching of saved entries against live windows.

use serde::{Deserialize, Serialize};

/// Stable identity of a window: executable path + class name (+ title).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub exe_path: String,
    pub class_name: String,
    #[serde(default)]
    pub title: String,
}

/// Window classes shared by many unrelated windows; the title decides there.
const GENERIC_CLASSES: &[&str] = &[
    "",
    "#32770",
    "ApplicationFrameWindow",
    "Window",
    "MainWindow",
    "SunAwtFrame",
    "TkTopLevel",
];

pub fn is_generic_class(class: &str) -> bool {
    GENERIC_CLASSES
        .iter()
        .any(|g| g.eq_ignore_ascii_case(class))
}

fn norm_path(p: &str) -> String {
    p.replace('/', "\\").to_lowercase()
}

/// Normalize a title for comparison (case, whitespace, "modified" markers).
pub fn norm_title(t: &str) -> String {
    t.trim()
        .trim_start_matches(['*', '•', '●', ' '])
        .trim()
        .to_lowercase()
}

fn similarity(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    prefix + suffix.min(a.len().min(b.len()) - prefix)
}

fn same_group(a: &Identity, b: &Identity) -> bool {
    norm_path(&a.exe_path) == norm_path(&b.exe_path)
        && a.class_name.eq_ignore_ascii_case(&b.class_name)
}

/// For every live window return the index of the saved identity it matches.
///
/// Rules: executable + class must agree. Within such a group an exact
/// (normalized) title match wins. For non generic classes the remaining
/// windows are paired by best title similarity, then by order. For generic
/// classes only title matches count. Every saved entry is used at most once.
pub fn match_windows(saved: &[Identity], live: &[Identity]) -> Vec<Option<usize>> {
    let mut result: Vec<Option<usize>> = vec![None; live.len()];
    let mut used = vec![false; saved.len()];

    // Pass A: exact title within the same group.
    for (li, l) in live.iter().enumerate() {
        let lt = norm_title(&l.title);
        let hit = saved
            .iter()
            .enumerate()
            .find(|(si, s)| !used[*si] && same_group(s, l) && norm_title(&s.title) == lt);
        if let Some((si, _)) = hit {
            used[si] = true;
            result[li] = Some(si);
        }
    }

    // Pass B: non-generic classes, best similarity then order.
    for (li, l) in live.iter().enumerate() {
        if result[li].is_some() || is_generic_class(&l.class_name) {
            continue;
        }
        let lt = norm_title(&l.title);
        let mut best: Option<(usize, usize)> = None;
        for (si, s) in saved.iter().enumerate() {
            if used[si] || !same_group(s, l) {
                continue;
            }
            let score = similarity(&lt, &norm_title(&s.title));
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((si, score));
            }
        }
        if let Some((si, _)) = best {
            used[si] = true;
            result[li] = Some(si);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(exe: &str, class: &str, title: &str) -> Identity {
        Identity {
            exe_path: exe.into(),
            class_name: class.into(),
            title: title.into(),
        }
    }

    #[test]
    fn single_window_matches_despite_title_change() {
        let saved = [id("C:\\a\\app.exe", "AppWnd", "Doc1 - App")];
        let live = [id("c:/a/APP.exe", "appwnd", "Doc2 - App")];
        assert_eq!(match_windows(&saved, &live), vec![Some(0)]);
    }

    #[test]
    fn different_exe_or_class_never_match() {
        let saved = [id("C:\\a.exe", "X", "t")];
        assert_eq!(
            match_windows(&saved, &[id("C:\\b.exe", "X", "t")]),
            vec![None]
        );
        assert_eq!(
            match_windows(&saved, &[id("C:\\a.exe", "Y", "t")]),
            vec![None]
        );
    }

    #[test]
    fn exact_titles_win_over_order() {
        let saved = [
            id("e.exe", "Cls", "One"),
            id("e.exe", "Cls", "Two"),
            id("e.exe", "Cls", "Three"),
        ];
        let live = [id("e.exe", "Cls", "Three"), id("e.exe", "Cls", "One")];
        assert_eq!(match_windows(&saved, &live), vec![Some(2), Some(0)]);
    }

    #[test]
    fn similarity_picks_closest_title() {
        let saved = [
            id("e.exe", "Cls", "Notes - Editor"),
            id("e.exe", "Cls", "Report.txt - Editor"),
        ];
        let live = [id("e.exe", "Cls", "Report2.txt - Editor")];
        assert_eq!(match_windows(&saved, &live), vec![Some(1)]);
    }

    #[test]
    fn generic_class_requires_title() {
        let saved = [id("host.exe", "ApplicationFrameWindow", "Rechner")];
        let live = [
            id("host.exe", "ApplicationFrameWindow", "Einstellungen"),
            id("host.exe", "ApplicationFrameWindow", "Rechner"),
        ];
        assert_eq!(match_windows(&saved, &live), vec![None, Some(0)]);
    }

    #[test]
    fn saved_entries_are_used_once() {
        let saved = [id("e.exe", "Cls", "A")];
        let live = [id("e.exe", "Cls", "A"), id("e.exe", "Cls", "B")];
        assert_eq!(match_windows(&saved, &live), vec![Some(0), None]);
    }

    #[test]
    fn modified_marker_is_ignored() {
        assert_eq!(norm_title("*Untitled "), norm_title("untitled"));
    }
}
