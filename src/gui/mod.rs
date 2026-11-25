//! The overview window (egui/eframe, OpenGL).
//!
//! Design: eframe runs on its own dedicated thread. That thread never touches
//! window hooks or placements – it only reads a [`Snapshot`] and posts
//! [`UiCommand`]s to the tray thread through the [`Bridge`] (PostMessage).
//! Closing the window ends only `run_native`; the tray app keeps running.
//! eframe reuses its event loop (thread-local), so the window can be opened
//! again and again.

mod icons;
mod theme;
mod view;

use crate::ui_model::{days_from_civil, format_datetime_de, Bridge};
use crate::{icon, log_error, log_info, APP_NAME};
use eframe::egui;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, FILETIME, HWND, LPARAM, SYSTEMTIME};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumThreadWindows, GetWindow, IsIconic, IsWindowVisible, MessageBoxW,
    SetForegroundWindow, ShowWindow, GW_OWNER, MB_ICONWARNING, MB_OK, SW_RESTORE, SW_SHOWNORMAL,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No window.
    Idle,
    /// Window requested, not created yet.
    Starting,
    /// Window exists.
    Live,
    /// Window is being closed.
    Closing,
}

struct Control {
    phase: Phase,
    /// Open requested while the previous window was closing.
    reopen: bool,
    thread_id: u32,
    tx: Option<Sender<()>>,
}

static CONTROL: Mutex<Control> = Mutex::new(Control {
    phase: Phase::Idle,
    reopen: false,
    thread_id: 0,
    tx: None,
});

fn control() -> MutexGuard<'static, Control> {
    CONTROL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open the window, or raise the already open one (never a second instance).
/// Called on the tray thread.
pub fn open_or_focus(bridge: Arc<Bridge>) {
    // Never hold the lock while calling into Win32 windows of the GUI thread:
    // those calls send messages to it and it may be waiting for this lock.
    let focus_thread = {
        let mut c = control();
        match c.phase {
            Phase::Starting => None, // already on its way
            Phase::Live => Some(c.thread_id),
            Phase::Closing => {
                c.reopen = true;
                None
            }
            Phase::Idle => {
                c.phase = Phase::Starting;
                bridge.set_ui_open(true);
                if c.tx.as_ref().is_none_or(|tx| tx.send(()).is_err()) {
                    let (tx, rx) = channel::<()>();
                    let _ = tx.send(());
                    c.tx = Some(tx);
                    std::thread::spawn(move || gui_thread(rx, bridge));
                }
                None
            }
        }
    };
    if let Some(thread_id) = focus_thread {
        focus(thread_id);
    }
}

fn gui_thread(rx: std::sync::mpsc::Receiver<()>, bridge: Arc<Bridge>) {
    control().thread_id = unsafe { GetCurrentThreadId() };
    while rx.recv().is_ok() {
        loop {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_window(bridge.clone())
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(e)) => report_failure(&e),
                Err(_) => report_failure("interner Fehler"),
            }
            let mut c = control();
            if c.reopen && c.phase != Phase::Idle {
                c.reopen = false;
                c.phase = Phase::Starting;
                continue;
            }
            c.reopen = false;
            c.phase = Phase::Idle;
            bridge.set_ui_open(false);
            bridge.set_repaint(None);
            break;
        }
    }
}

fn report_failure(e: &str) {
    log_error!("Oberfläche konnte nicht gestartet werden: {e}");
    let text: Vec<u16> = "Die Oberfläche konnte nicht gestartet werden.\n\
        Möglicherweise fehlt OpenGL-Unterstützung (Grafiktreiber).\n\
        WindowKeeper arbeitet weiterhin im Infobereich der Taskleiste."
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("WindowKeeper"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn run_window(bridge: Arc<Bridge>) -> Result<(), String> {
    let ico = icon::rgba();
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([900.0, 640.0])
        .with_min_inner_size([660.0, 460.0])
        .with_icon(egui::IconData {
            rgba: ico,
            width: icon::SIZE,
            height: icon::SIZE,
        });
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        event_loop_builder: Some(Box::new(|b| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            b.with_any_thread(true);
        })),
        ..Default::default()
    };
    log_info!("Übersicht geöffnet");
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(view::View::new(cc, bridge, set_phase)))),
    );
    log_info!("Übersicht geschlossen");
    result.map_err(|e| e.to_string())
}

fn set_phase_live() {
    control().phase = Phase::Live;
}

fn set_phase(live: bool) {
    if live {
        set_phase_live();
    } else {
        let mut c = control();
        if c.phase != Phase::Idle {
            c.phase = Phase::Closing;
        }
    }
}

// ------------------------------------------------------------ win32 bits ----

/// Raise the GUI window of `thread_id` (restoring it if minimized).
fn focus(thread_id: u32) {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let found = &mut *(lp.0 as *mut Option<HWND>);
        let owned = GetWindow(hwnd, GW_OWNER)
            .map(|o| !o.0.is_null())
            .unwrap_or(false);
        if found.is_none()
            && !owned
            && (IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool())
        {
            *found = Some(hwnd);
        }
        BOOL(1)
    }
    if thread_id == 0 {
        return;
    }
    let mut found: Option<HWND> = None;
    unsafe {
        let _ = EnumThreadWindows(thread_id, Some(cb), LPARAM(&mut found as *mut _ as isize));
        if let Some(hwnd) = found {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// Open a URL with the default browser.
pub(crate) fn open_url(url: &str) {
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Local time offset (seconds east of UTC) at the given moment.
fn local_offset_secs(unix: u64) -> i64 {
    let calc = || -> Option<i64> {
        let t = (unix + 11_644_473_600) * 10_000_000;
        let ft = FILETIME {
            dwLowDateTime: t as u32,
            dwHighDateTime: (t >> 32) as u32,
        };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        unsafe {
            FileTimeToSystemTime(&ft, &mut utc).ok()?;
            SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
        }
        let days = days_from_civil(
            i64::from(local.wYear),
            i64::from(local.wMonth),
            i64::from(local.wDay),
        );
        let secs = days * 86_400
            + i64::from(local.wHour) * 3600
            + i64::from(local.wMinute) * 60
            + i64::from(local.wSecond);
        Some(secs - unix as i64)
    };
    calc().unwrap_or(0)
}

/// German local date/time or "Noch nie".
pub(crate) fn format_local(unix: u64) -> String {
    format_datetime_de(unix, local_offset_secs(unix))
}
