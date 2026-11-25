//! Decides which top-level windows are tracked (pure, testable).

/// Facts about a window collected by the OS layer.
#[derive(Debug, Clone, Default)]
pub struct WindowFacts {
    pub visible: bool,
    pub tool_window: bool,
    pub app_window_flag: bool,
    pub no_activate: bool,
    pub cloaked: bool,
    pub hung: bool,
    pub has_owner: bool,
    pub own_process: bool,
    pub class_name: String,
    pub title: String,
    pub width: i32,
    pub height: i32,
}

/// Shell and system classes that are never real application windows.
const IGNORED_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

pub fn should_track(f: &WindowFacts) -> bool {
    if !f.visible || f.cloaked || f.hung || f.own_process {
        return false;
    }
    if f.title.trim().is_empty() || f.width <= 0 || f.height <= 0 {
        return false;
    }
    if IGNORED_CLASSES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&f.class_name))
    {
        return false;
    }
    if f.tool_window && !f.app_window_flag {
        return false;
    }
    if f.no_activate && !f.app_window_flag {
        return false;
    }
    if f.has_owner && !f.app_window_flag {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> WindowFacts {
        WindowFacts {
            visible: true,
            class_name: "Notepad".into(),
            title: "x - Editor".into(),
            width: 800,
            height: 600,
            ..Default::default()
        }
    }

    #[test]
    fn normal_window_is_tracked() {
        assert!(should_track(&ok()));
    }

    #[test]
    fn oddities_are_ignored() {
        let mut f = ok();
        f.cloaked = true;
        assert!(!should_track(&f));
        let mut f = ok();
        f.tool_window = true;
        assert!(!should_track(&f));
        f.app_window_flag = true;
        assert!(should_track(&f));
        let mut f = ok();
        f.visible = false;
        assert!(!should_track(&f));
        let mut f = ok();
        f.width = 0;
        assert!(!should_track(&f));
        let mut f = ok();
        f.title = "  ".into();
        assert!(!should_track(&f));
        let mut f = ok();
        f.class_name = "Shell_TrayWnd".into();
        assert!(!should_track(&f));
        let mut f = ok();
        f.own_process = true;
        assert!(!should_track(&f));
        let mut f = ok();
        f.hung = true;
        assert!(!should_track(&f));
        let mut f = ok();
        f.has_owner = true;
        assert!(!should_track(&f));
    }
}
