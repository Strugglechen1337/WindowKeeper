//! Rectangles, monitor descriptions and the monitor fallback / clamp logic.

use serde::{Deserialize, Serialize};

/// Rectangle in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Size of the overlap with `other` (0 when disjoint).
    pub fn intersection_size(&self, other: &Rect) -> (i32, i32) {
        let w = self.right().min(other.right()) - self.x.max(other.x);
        let h = self.bottom().min(other.bottom()) - self.y.max(other.y);
        (w.max(0), h.max(0))
    }
}

/// Window show state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowState {
    #[default]
    Normal,
    Maximized,
    Minimized,
}

/// A monitor as currently attached to the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorDesc {
    /// Stable device interface path (may be empty if unavailable).
    pub device_id: String,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub gdi_name: String,
    /// Adapter description string.
    pub adapter: String,
    /// Full monitor rectangle.
    pub rect: Rect,
    /// Work area (taskbar excluded).
    pub work: Rect,
    /// Effective DPI of the monitor.
    pub dpi: u32,
    pub primary: bool,
}

/// Monitor identity as saved together with a placement.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MonitorRef {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub gdi_name: String,
    #[serde(default)]
    pub adapter: String,
    /// DPI of the monitor at save time (96 = 100 %).
    pub dpi: u32,
    /// Work area of the monitor at save time.
    pub work: Rect,
}

impl MonitorRef {
    pub fn from_desc(m: &MonitorDesc) -> Self {
        Self {
            device_id: m.device_id.clone(),
            gdi_name: m.gdi_name.clone(),
            adapter: m.adapter.clone(),
            dpi: m.dpi,
            work: m.work,
        }
    }

    /// Whether `m` is the same physical display as this saved reference.
    pub fn matches(&self, m: &MonitorDesc) -> bool {
        if !self.device_id.is_empty() && !m.device_id.is_empty() {
            return self.device_id.eq_ignore_ascii_case(&m.device_id);
        }
        // No stable id available: fall back to GDI name + adapter.
        !self.gdi_name.is_empty()
            && self.gdi_name.eq_ignore_ascii_case(&m.gdi_name)
            && (self.adapter.is_empty() || self.adapter == m.adapter)
    }
}

/// Saved placement of a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub state: ShowState,
    /// A minimized window that returns to maximized when restored.
    #[serde(default)]
    pub restore_maximized: bool,
    /// Normal (restored) rectangle in screen coordinates.
    pub rect: Rect,
    pub monitor: MonitorRef,
}

/// Result of resolving a saved placement against the current monitors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub rect: Rect,
    /// `false` when the saved monitor is gone and the primary one was used.
    pub monitor_found: bool,
}

/// Scale a value from `from_dpi` to `to_dpi`, rounding to nearest.
pub fn scale_dpi(v: i32, to_dpi: u32, from_dpi: u32) -> i32 {
    if from_dpi == 0 || to_dpi == 0 || from_dpi == to_dpi {
        return v;
    }
    let num = v as i64 * to_dpi as i64;
    let den = from_dpi as i64;
    let half = if num >= 0 { den / 2 } else { -(den / 2) };
    ((num + half) / den) as i32
}

/// Fully clamp `r` into `area` (shrinking it if it does not fit).
pub fn clamp_into(r: Rect, area: Rect) -> Rect {
    let w = r.w.min(area.w).max(1);
    let h = r.h.min(area.h).max(1);
    let x = r.x.clamp(area.x, (area.right() - w).max(area.x));
    let y = r.y.clamp(area.y, (area.bottom() - h).max(area.y));
    Rect { x, y, w, h }
}

/// True if enough of the window is visible inside `work` for the user to grab it.
pub fn is_reachable(r: &Rect, work: &Rect) -> bool {
    let (iw, ih) = r.intersection_size(work);
    iw >= r.w.min(120) && ih >= r.h.min(60) && r.y >= work.y - 16 && r.y <= work.bottom() - 40
}

/// Find the saved monitor among the attached ones.
pub fn find_monitor<'a>(
    saved: &MonitorRef,
    monitors: &'a [MonitorDesc],
) -> Option<&'a MonitorDesc> {
    monitors.iter().find(|m| saved.matches(m))
}

/// Monitor that contains most of `r` (nearest one if there is no overlap).
pub fn monitor_for_rect<'a>(r: &Rect, monitors: &'a [MonitorDesc]) -> Option<&'a MonitorDesc> {
    let area = |m: &MonitorDesc| {
        let (w, h) = r.intersection_size(&m.rect);
        w as i64 * h as i64
    };
    let best = monitors.iter().max_by_key(|m| area(m))?;
    if area(best) > 0 {
        return Some(best);
    }
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let dist = |m: &MonitorDesc| {
        let dx = (cx - (m.rect.x + m.rect.w / 2)) as i64;
        let dy = (cy - (m.rect.y + m.rect.h / 2)) as i64;
        dx * dx + dy * dy
    };
    monitors.iter().min_by_key(|m| dist(m))
}

/// Compute the rectangle a saved placement should get now.
///
/// * Saved monitor present: position relative to that monitor's work area,
///   scaled for DPI changes; only corrected if the window would be unreachable.
/// * Saved monitor missing: placed on the primary monitor and fully clamped
///   into its work area.
pub fn resolve(p: &Placement, monitors: &[MonitorDesc]) -> Option<Resolved> {
    let primary = monitors
        .iter()
        .find(|m| m.primary)
        .or_else(|| monitors.first())?;
    let found = find_monitor(&p.monitor, monitors);
    let target = found.unwrap_or(primary);
    let rel_x = p.rect.x - p.monitor.work.x;
    let rel_y = p.rect.y - p.monitor.work.y;
    let rect = Rect {
        x: target.work.x + scale_dpi(rel_x, target.dpi, p.monitor.dpi),
        y: target.work.y + scale_dpi(rel_y, target.dpi, p.monitor.dpi),
        w: scale_dpi(p.rect.w, target.dpi, p.monitor.dpi).max(1),
        h: scale_dpi(p.rect.h, target.dpi, p.monitor.dpi).max(1),
    };
    let rect = if found.is_some() {
        if is_reachable(&rect, &target.work) {
            rect
        } else {
            clamp_into(rect, target.work)
        }
    } else {
        clamp_into(rect, target.work)
    };
    Some(Resolved {
        rect,
        monitor_found: found.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(id: &str, primary: bool, x: i32, dpi: u32) -> MonitorDesc {
        MonitorDesc {
            device_id: id.into(),
            gdi_name: format!("\\\\.\\DISPLAY{}", if primary { 1 } else { 2 }),
            adapter: "GPU".into(),
            rect: Rect::new(x, 0, 1920, 1080),
            work: Rect::new(x, 0, 1920, 1040),
            dpi,
            primary,
        }
    }

    fn placement(m: &MonitorDesc, rect: Rect) -> Placement {
        Placement {
            state: ShowState::Normal,
            restore_maximized: false,
            rect,
            monitor: MonitorRef::from_desc(m),
        }
    }

    #[test]
    fn same_monitor_keeps_rect() {
        let m = mon("A", true, 0, 96);
        let p = placement(&m, Rect::new(100, 50, 800, 600));
        let r = resolve(&p, &[m]).unwrap();
        assert_eq!(r.rect, Rect::new(100, 50, 800, 600));
        assert!(r.monitor_found);
    }

    #[test]
    fn missing_monitor_falls_back_to_primary_and_clamps() {
        let primary = mon("A", true, 0, 96);
        let second = mon("B", false, 1920, 96);
        let p = placement(&second, Rect::new(2500, 900, 800, 600));
        let r = resolve(&p, std::slice::from_ref(&primary)).unwrap();
        assert!(!r.monitor_found);
        assert!(r.rect.x >= primary.work.x && r.rect.right() <= primary.work.right());
        assert!(r.rect.y >= primary.work.y && r.rect.bottom() <= primary.work.bottom());
        assert_eq!(r.rect.w, 800);
        assert_eq!(r.rect.h, 600);
    }

    #[test]
    fn missing_monitor_shrinks_oversized_window() {
        let primary = mon("A", true, 0, 96);
        let second = mon("B", false, 1920, 96);
        let p = placement(&second, Rect::new(1920, 0, 5000, 3000));
        let r = resolve(&p, std::slice::from_ref(&primary)).unwrap();
        assert_eq!(r.rect, primary.work);
    }

    #[test]
    fn monitor_moved_in_arrangement_uses_relative_offset() {
        let old = mon("B", false, 1920, 96);
        let p = placement(&old, Rect::new(2020, 100, 500, 400));
        let primary = mon("A", true, 0, 96);
        let moved = mon("B", false, -1920, 96);
        let r = resolve(&p, &[primary, moved]).unwrap();
        assert_eq!(r.rect, Rect::new(-1820, 100, 500, 400));
    }

    #[test]
    fn unreachable_window_on_present_monitor_is_clamped() {
        let m = mon("A", true, 0, 96);
        let p = placement(&m, Rect::new(5000, 5000, 800, 600));
        let r = resolve(&p, std::slice::from_ref(&m)).unwrap();
        assert!(r.monitor_found);
        assert_eq!(r.rect.right(), m.work.right());
        assert_eq!(r.rect.bottom(), m.work.bottom());
    }

    #[test]
    fn partially_offscreen_window_is_left_alone() {
        let m = mon("A", true, 0, 96);
        let p = placement(&m, Rect::new(-7, 0, 1934, 1047));
        let r = resolve(&p, &[m]).unwrap();
        assert_eq!(r.rect, Rect::new(-7, 0, 1934, 1047));
    }

    #[test]
    fn dpi_change_scales_size_and_offset() {
        let old = mon("A", true, 0, 96);
        let p = placement(&old, Rect::new(100, 100, 800, 600));
        let new = mon("A", true, 0, 192);
        let r = resolve(&p, &[new]).unwrap();
        assert_eq!(r.rect, Rect::new(200, 200, 1600, 1200));
    }

    #[test]
    fn monitor_for_rect_prefers_overlap_then_nearest() {
        let a = mon("A", true, 0, 96);
        let b = mon("B", false, 1920, 96);
        let ms = [a, b];
        let m = monitor_for_rect(&Rect::new(1800, 0, 400, 300), &ms).unwrap();
        assert_eq!(m.device_id, "B");
        let m = monitor_for_rect(&Rect::new(-3000, 0, 100, 100), &ms).unwrap();
        assert_eq!(m.device_id, "A");
        assert!(monitor_for_rect(&Rect::new(0, 0, 1, 1), &[]).is_none());
    }

    #[test]
    fn no_monitors_gives_none() {
        let m = mon("A", true, 0, 96);
        let p = placement(&m, Rect::new(0, 0, 10, 10));
        assert!(resolve(&p, &[]).is_none());
    }

    #[test]
    fn monitor_matching_by_id_and_name_fallback() {
        let m = mon("A", true, 0, 96);
        let mut saved = MonitorRef::from_desc(&m);
        assert!(saved.matches(&m));
        saved.device_id = "other".into();
        assert!(!saved.matches(&m));
        saved.device_id.clear();
        assert!(saved.matches(&m));
        let mut m2 = m.clone();
        m2.device_id.clear();
        saved.gdi_name = "\\\\.\\DISPLAY9".into();
        assert!(!saved.matches(&m2));
    }

    #[test]
    fn scale_and_clamp_helpers() {
        assert_eq!(scale_dpi(100, 120, 96), 125);
        assert_eq!(scale_dpi(-100, 120, 96), -125);
        assert_eq!(scale_dpi(100, 96, 0), 100);
        let c = clamp_into(Rect::new(-50, -50, 100, 100), Rect::new(0, 0, 500, 500));
        assert_eq!(c, Rect::new(0, 0, 100, 100));
    }
}
