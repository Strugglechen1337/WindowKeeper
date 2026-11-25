//! Platform independent model of the GUI: what the tray thread publishes
//! (`Snapshot`), what the UI asks for (`UiCommand`) and the formatting /
//! filtering helpers. The Win32/egui code only renders and forwards this.

use crate::config::SettingsEdit;
use crate::geometry::{Placement, ShowState};
use crate::layout::Entry;
use crate::matching::Identity;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

// ------------------------------------------------------------- snapshot ----

/// State of the update check as shown on the info page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(String),
    Failed(String),
}

impl UpdateStatus {
    pub fn label(&self) -> String {
        match self {
            UpdateStatus::Unknown => "Noch nicht geprüft".into(),
            UpdateStatus::Checking => "Prüfe …".into(),
            UpdateStatus::UpToDate => "Aktuell".into(),
            UpdateStatus::Available(v) => format!("Update verfügbar: Version {v}"),
            UpdateStatus::Failed(e) => format!("Fehler: {e}"),
        }
    }
}

/// A short message shown in the GUI (e.g. "app is not open").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Increases with every notice so the UI can tell new ones apart.
    pub seq: u64,
    pub text: String,
}

/// Everything the GUI displays; published by the tray thread.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub settings: SettingsEdit,
    pub autostart: bool,
    pub last_save_at: u64,
    pub last_restore_at: u64,
    pub last_update_check: u64,
    pub watching: bool,
    pub entries: Vec<Entry>,
    pub update: UpdateStatus,
    pub notice: Option<Notice>,
}

impl Snapshot {
    pub fn ignored_count(&self) -> usize {
        self.entries.iter().filter(|e| e.ignored).count()
    }

    pub fn restorable_count(&self) -> usize {
        self.entries.len() - self.ignored_count()
    }
}

// ------------------------------------------------------------- commands ----

/// Requests from the UI thread, executed on the tray thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCommand {
    SaveLayout,
    RestoreAll,
    RestoreEntry(Identity),
    SetIgnored(Identity, bool),
    DeleteEntry(Identity),
    SetAutostart(bool),
    SetSettings(SettingsEdit),
    CheckUpdates,
}

// --------------------------------------------------------------- bridge ----

type Callback = Arc<dyn Fn() + Send + Sync>;

/// Thread-safe meeting point of the tray thread and the UI thread.
pub struct Bridge {
    snapshot: Mutex<Snapshot>,
    commands: Mutex<VecDeque<UiCommand>>,
    wake_tray: Callback,
    repaint: Mutex<Option<Callback>>,
    ui_open: AtomicBool,
    version: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Bridge {
    /// `wake_tray` is called after a command was queued (PostMessage).
    pub fn new(wake_tray: impl Fn() + Send + Sync + 'static) -> Arc<Bridge> {
        Arc::new(Bridge {
            snapshot: Mutex::new(Snapshot::default()),
            commands: Mutex::new(VecDeque::new()),
            wake_tray: Arc::new(wake_tray),
            repaint: Mutex::new(None),
            ui_open: AtomicBool::new(false),
            version: AtomicU64::new(0),
        })
    }

    /// UI side: queue a command for the tray thread.
    pub fn send(&self, cmd: UiCommand) {
        lock(&self.commands).push_back(cmd);
        (self.wake_tray)();
    }

    /// Tray side: take all queued commands.
    pub fn take_commands(&self) -> Vec<UiCommand> {
        lock(&self.commands).drain(..).collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// Increases with every snapshot change (lets the UI skip needless clones).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Modify the snapshot and wake the UI.
    pub fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        f(&mut lock(&self.snapshot));
        self.version.fetch_add(1, Ordering::SeqCst);
        self.request_repaint();
    }

    /// Show a notice (replaces the previous one).
    pub fn notify(&self, text: impl Into<String>) {
        let text = text.into();
        self.update(|s| {
            let seq = s.notice.as_ref().map_or(1, |n| n.seq + 1);
            s.notice = Some(Notice { seq, text });
        });
    }

    pub fn set_repaint(&self, f: Option<Callback>) {
        *lock(&self.repaint) = f;
    }

    pub fn request_repaint(&self) {
        let cb = lock(&self.repaint).clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// True while a GUI window exists (the tray thread only publishes then).
    pub fn is_ui_open(&self) -> bool {
        self.ui_open.load(Ordering::SeqCst)
    }

    pub fn set_ui_open(&self, open: bool) {
        self.ui_open.store(open, Ordering::SeqCst);
    }
}

// ----------------------------------------------------------- formatting ----

/// Short sentence for the overview page.
pub fn status_sentence(watching: bool, auto_save: bool) -> &'static str {
    match (watching, auto_save) {
        (true, _) => {
            "WindowKeeper beobachtet spät startende Fenster und platziert sie automatisch."
        }
        (false, true) => "Bereit – Änderungen an Fenstern werden automatisch gespeichert.",
        (false, false) => "Bereit – automatisches Speichern ist ausgeschaltet.",
    }
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Days since 1970-01-01 of a civil date (inverse of `civil_from_days`).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// German date/time, e.g. `25.11.2026, 22:31 Uhr`; `Noch nie` for 0.
/// `utc_offset_secs` is the local offset to UTC at that moment.
pub fn format_datetime_de(unix: u64, utc_offset_secs: i64) -> String {
    if unix == 0 {
        return "Noch nie".into();
    }
    let local = unix as i64 + utc_offset_secs;
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    let rem = local.rem_euclid(86_400);
    format!(
        "{d:02}.{m:02}.{y:04}, {:02}:{:02} Uhr",
        rem / 3600,
        rem % 3600 / 60
    )
}

/// File name of a Windows (or POSIX) path.
pub fn exe_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

pub fn state_label(s: ShowState) -> &'static str {
    match s {
        ShowState::Normal => "Normal",
        ShowState::Maximized => "Maximiert",
        ShowState::Minimized => "Minimiert",
    }
}

/// `x, y · Breite × Höhe`
pub fn position_text(p: &Placement) -> String {
    let r = &p.rect;
    format!("{}, {}  ·  {} × {}", r.x, r.y, r.w, r.h)
}

/// Human readable monitor name (GDI name like `\\.\DISPLAY1` -> `Monitor 1`).
pub fn monitor_name(p: &Placement) -> String {
    let m = &p.monitor;
    let digits: String = m
        .gdi_name
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let base = if digits.is_empty() {
        "Unbekannter Monitor".to_string()
    } else {
        format!("Monitor {digits}")
    };
    if m.adapter.trim().is_empty() {
        base
    } else {
        format!("{base} ({})", m.adapter.trim())
    }
}

/// Case-insensitive search over exe name, title and monitor.
pub fn matches_filter(e: &Entry, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    [
        exe_file_name(&e.identity.exe_path),
        &e.identity.title,
        &monitor_name(&e.placement),
        state_label(e.placement.state),
    ]
    .iter()
    .any(|f| f.to_lowercase().contains(&q))
}

/// Maximum number of seconds a [`Notice`] stays visible in the GUI.
pub const NOTICE_SECS: f64 = 6.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{MonitorRef, Rect};
    use std::sync::atomic::AtomicUsize;

    fn entry(exe: &str, title: &str) -> Entry {
        Entry {
            identity: Identity {
                exe_path: exe.into(),
                class_name: "C".into(),
                title: title.into(),
            },
            placement: Placement {
                state: ShowState::Maximized,
                restore_maximized: false,
                rect: Rect::new(-10, 20, 800, 600),
                monitor: MonitorRef {
                    gdi_name: "\\\\.\\DISPLAY2".into(),
                    adapter: "Test GPU".into(),
                    ..MonitorRef::default()
                },
            },
            ignored: false,
        }
    }

    #[test]
    fn german_datetime_formatting() {
        assert_eq!(format_datetime_de(0, 3600), "Noch nie");
        // 2026-11-25 21:31:00 UTC
        let t = 1_795_642_260;
        assert_eq!(format_datetime_de(t, 0), "25.11.2026, 21:31 Uhr");
        assert_eq!(format_datetime_de(t, 3600), "25.11.2026, 22:31 Uhr");
        // Crossing midnight and negative offsets.
        assert_eq!(format_datetime_de(t, 3 * 3600), "26.11.2026, 00:31 Uhr");
        assert_eq!(
            format_datetime_de(86_400 + 60, -120),
            "01.01.1970, 23:59 Uhr"
        );
    }

    #[test]
    fn civil_roundtrip() {
        for days in [-1000, 0, 59, 60, 10_957, 20_417, 40_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn file_names_and_labels() {
        assert_eq!(exe_file_name("C:\\Program Files\\App\\app.exe"), "app.exe");
        assert_eq!(exe_file_name("/usr/bin/x"), "x");
        assert_eq!(exe_file_name("plain.exe"), "plain.exe");
        assert_eq!(state_label(ShowState::Minimized), "Minimiert");
        let e = entry("C:\\a\\app.exe", "T");
        assert_eq!(position_text(&e.placement), "-10, 20  ·  800 × 600");
        assert_eq!(monitor_name(&e.placement), "Monitor 2 (Test GPU)");
        let mut bare = e.placement.clone();
        bare.monitor = MonitorRef::default();
        assert_eq!(monitor_name(&bare), "Unbekannter Monitor");
    }

    #[test]
    fn filter_matches_name_title_monitor_state() {
        let e = entry("C:\\Tools\\Notepad.exe", "Notizen – Entwurf");
        assert!(matches_filter(&e, ""));
        assert!(matches_filter(&e, "  notepad "));
        assert!(matches_filter(&e, "entwurf"));
        assert!(matches_filter(&e, "monitor 2"));
        assert!(matches_filter(&e, "maximiert"));
        // The folder is not searched, only the file name.
        assert!(!matches_filter(&e, "tools"));
    }

    #[test]
    fn status_sentences() {
        assert!(status_sentence(true, false).contains("beobachtet"));
        assert!(status_sentence(false, true).contains("automatisch gespeichert"));
        assert!(status_sentence(false, false).contains("ausgeschaltet"));
    }

    #[test]
    fn update_labels() {
        assert_eq!(UpdateStatus::UpToDate.label(), "Aktuell");
        assert!(UpdateStatus::Available("1.2.3".into())
            .label()
            .contains("1.2.3"));
        assert!(UpdateStatus::Failed("x".into())
            .label()
            .starts_with("Fehler"));
    }

    #[test]
    fn snapshot_counts() {
        let mut s = Snapshot {
            entries: vec![entry("a.exe", "1"), entry("b.exe", "2")],
            ..Snapshot::default()
        };
        s.entries[1].ignored = true;
        assert_eq!(s.ignored_count(), 1);
        assert_eq!(s.restorable_count(), 1);
    }

    #[test]
    fn bridge_queues_commands_wakes_and_notifies() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let b = Bridge::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        b.send(UiCommand::SaveLayout);
        b.send(UiCommand::SetAutostart(true));
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert_eq!(
            b.take_commands(),
            vec![UiCommand::SaveLayout, UiCommand::SetAutostart(true)]
        );
        assert!(b.take_commands().is_empty());

        let repaints = Arc::new(AtomicUsize::new(0));
        let r = repaints.clone();
        b.set_repaint(Some(Arc::new(move || {
            r.fetch_add(1, Ordering::SeqCst);
        })));
        b.notify("a");
        b.notify("b");
        let n = b.snapshot().notice.unwrap();
        assert_eq!((n.seq, n.text.as_str()), (2, "b"));
        assert_eq!(repaints.load(Ordering::SeqCst), 2);
        b.set_repaint(None);
        b.update(|s| s.autostart = true);
        assert_eq!(repaints.load(Ordering::SeqCst), 2);
        assert!(b.snapshot().autostart);
        assert_eq!(b.version(), 3);

        assert!(!b.is_ui_open());
        b.set_ui_open(true);
        assert!(b.is_ui_open());
    }
}
