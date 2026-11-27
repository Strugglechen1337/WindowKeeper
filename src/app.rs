//! Tray application: hidden message window, WinEvent hooks, timers, menu.

use crate::config::Config;
use crate::geometry::Placement;
use crate::layout::{Entry, Layout};
use crate::matching::Identity;
use crate::state::RunState;
use crate::tray_event::{decode, TrayAction};
use crate::ui_model::{exe_file_name, Bridge, UiCommand, UpdateStatus};
use crate::winapi::{self, hwnd_from};
use crate::{autostart, gui, icon, logger, updater, APP_NAME, APP_VERSION};
use crate::{log_error, log_info, log_warn};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT,
    POINT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_WINEVENT: u32 = WM_APP + 2;
const WM_APP_QUIT: u32 = WM_APP + 3;
const WM_APP_UI: u32 = WM_APP + 4;
const WM_APP_COMMAND: u32 = WM_APP + 5;

const TIMER_APPLY: usize = 2;
const TIMER_UPDATE: usize = 4;
const TIMER_WATCH_END: usize = 5;
const TIMER_TRAY: usize = 6;
const TIMER_VERIFY: usize = 7;

/// Re-check delays (ms) after a restore: apps like Teams, Outlook or Edge
/// often move their window again after it appeared.
const VERIFY_DELAYS_MS: [u64; 4] = [1500, 3000, 6000, 12000];

const ID_OPEN: usize = 1000;
const ID_SAVE: usize = 1001;
const ID_RESTORE: usize = 1002;
const ID_AUTOSTART: usize = 1004;
const ID_UPDATE: usize = 1005;
const ID_QUIT: usize = 1006;

const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_HIDE: u32 = 0x8003;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);
/// True once the shell accepted NOTIFYICON_VERSION_4 for the tray icon.
static TRAY_V4: AtomicBool = AtomicBool::new(false);
/// True while the tray context menu runs its nested loop (ignore re-entrant clicks).
static MENU_OPEN: AtomicBool = AtomicBool::new(false);
static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

/// The channel to the GUI thread (created once in [`run`]).
fn bridge() -> &'static Arc<Bridge> {
    BRIDGE.get_or_init(|| {
        Bridge::new(|| {
            let main = MAIN_HWND.load(Ordering::SeqCst);
            if main != 0 {
                unsafe {
                    let _ = PostMessageW(hwnd_from(main), WM_APP_UI, WPARAM(0), LPARAM(0));
                }
            }
        })
    })
}

/// Who started an update check (decides which result dialogs are shown).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateSource {
    Auto,
    Tray,
    Gui,
}

struct App {
    hwnd: HWND,
    exe: PathBuf,
    cfg: Config,
    cfg_path: PathBuf,
    layout: Layout,
    layout_path: PathBuf,
    state: RunState,
    state_path: PathBuf,
    last_saved: String,
    watch_until: Option<Instant>,
    handled: HashSet<isize>,
    pending: HashMap<isize, (Instant, u32)>,
    /// Restored windows to re-check: due time, checks done, target placement.
    verify: HashMap<isize, (Instant, usize, Placement)>,
    hicon: HICON,
    hooks: Vec<HWINEVENTHOOK>,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| {
        let mut b = a.try_borrow_mut().ok()?;
        b.as_mut().map(f)
    })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide<const N: usize>(dst: &mut [u16; N], s: &str) {
    for (i, c) in s.encode_utf16().take(N - 1).enumerate() {
        dst[i] = c;
    }
}

fn message_box(text: &str, caption: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(text);
    let c = wide(caption);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            flags | MB_SETFOREGROUND | MB_TOPMOST,
        )
    }
}

// ---------------------------------------------------------------- entry ----

pub fn run() {
    winapi::set_dpi_awareness();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("WindowKeeper.exe"));
    let (data_dir, fell_back) = crate::paths::ensure_data_dir(&exe);
    logger::init(&data_dir.join("windowkeeper.log"));
    std::panic::set_hook(Box::new(|info| log_error!("panic: {info}")));
    log_info!("{APP_NAME} {APP_VERSION} gestartet ({})", exe.display());
    if fell_back {
        log_warn!(
            "Programmordner nicht beschreibbar, nutze {}",
            data_dir.display()
        );
    }

    let restarted = std::env::args().any(|a| a == "--restarted");
    let Some(_mutex) = acquire_single_instance(restarted) else {
        message_box(
            "WindowKeeper läuft bereits (Symbol im Infobereich der Taskleiste).",
            APP_NAME,
            MB_OK | MB_ICONINFORMATION,
        );
        return;
    };

    // Remove the previous EXE left over from an update (it may still be locked).
    let exe_for_cleanup = exe.clone();
    std::thread::spawn(move || {
        for _ in 0..30 {
            if updater::cleanup_old(&exe_for_cleanup) {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    let cfg_path = data_dir.join("config.toml");
    let (cfg, broken) = Config::load_or_default(&cfg_path);
    if broken {
        log_warn!("config.toml war ungültig und wurde als .bad gesichert");
    }
    if !cfg_path.exists() {
        let _ = cfg.save(&cfg_path);
    }
    let layout_path = data_dir.join("layout.json");
    let state_path = data_dir.join("state.json");
    let state = RunState::load(&state_path);
    let layout = match Layout::load(&layout_path) {
        Ok(l) => l,
        Err(e) => {
            if layout_path.exists() {
                log_warn!("layout.json unlesbar ({e}), wird gesichert");
                let _ = std::fs::rename(&layout_path, layout_path.with_extension("json.bad"));
            }
            Layout::default()
        }
    };
    if autostart::refresh_path(&exe) {
        log_info!("Autostart-Pfad aktualisiert");
    }

    unsafe {
        let Ok(hmod) = GetModuleHandleW(None) else {
            return;
        };
        let class = w!("WindowKeeperHiddenWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hmod.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!("WindowKeeper"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            hmod,
            None,
        ) else {
            log_error!("Fenster konnte nicht erstellt werden");
            return;
        };
        MAIN_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        let _ = bridge();
        let hicon = create_icon();
        let first_run = !cfg.first_run_done;
        let watch_minutes = cfg.restore_watch_minutes;
        let update_due = cfg.check_updates_on_start
            && updater::check_due(cfg.last_update_check, logger::now_secs());
        let app = App {
            hwnd,
            exe,
            cfg,
            cfg_path,
            layout,
            layout_path,
            state,
            state_path,
            last_saved: String::new(),
            watch_until: None,
            handled: HashSet::new(),
            pending: HashMap::new(),
            verify: HashMap::new(),
            hicon,
            hooks: Vec::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        };
        APP.with(|a| *a.borrow_mut() = Some(app));

        with_app(|app| {
            app.last_saved = app.layout.to_json();
            tray_add(app);
            install_hooks(app);
            SetTimer(app.hwnd, TIMER_TRAY, 30_000, None);
            // Startup: restore, then keep watching for late windows.
            let n = restore(app, false);
            log_info!("Start-Wiederherstellung: {n} Fenster");
            if watch_minutes > 0 && !app.layout.is_empty() {
                app.watch_until =
                    Some(Instant::now() + Duration::from_secs(watch_minutes as u64 * 60));
                SetTimer(app.hwnd, TIMER_WATCH_END, watch_minutes * 60_000, None);
            }
            if first_run {
                balloon(
                    app,
                    "WindowKeeper läuft im Infobereich der Taskleiste. \
                     Linksklick auf das Symbol öffnet die Übersicht, Rechtsklick das Menü.",
                );
                app.cfg.first_run_done = true;
                save_config(app);
            }
            if update_due {
                SetTimer(app.hwnd, TIMER_UPDATE, 15_000, None);
            }
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log_info!("{APP_NAME} beendet");
}

/// Create the single-instance mutex. After an update restart the previous
/// process may still be shutting down, so retry for a while.
fn acquire_single_instance(restarted: bool) -> Option<windows::Win32::Foundation::HANDLE> {
    let attempts = if restarted { 60 } else { 1 };
    for i in 0..attempts {
        unsafe {
            if let Ok(h) = CreateMutexW(None, true, w!("Local\\WindowKeeper_SingleInstance")) {
                if GetLastError() != ERROR_ALREADY_EXISTS {
                    return Some(h);
                }
                let _ = CloseHandle(h);
            }
        }
        if i + 1 < attempts {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    None
}

// ------------------------------------------------------------------ tray ----

fn create_icon() -> HICON {
    let rgba = icon::rgba();
    let mut bgra = rgba.clone();
    for px in bgra.chunks_mut(4) {
        px.swap(0, 2);
    }
    let and_mask = vec![0u8; (icon::SIZE * icon::SIZE / 8) as usize];
    unsafe {
        let hmod: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        CreateIcon(
            hmod,
            icon::SIZE as i32,
            icon::SIZE as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
        .unwrap_or_default()
    }
}

fn nid(app: &App) -> NOTIFYICONDATAW {
    let mut n = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: app.hwnd,
        uID: 1,
        ..Default::default()
    };
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    n.uCallbackMessage = WM_APP_TRAY;
    n.hIcon = app.hicon;
    copy_wide(&mut n.szTip, "WindowKeeper – Fensterlayout merken");
    n
}

fn tray_add(app: &App) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid(app));
        // Opt in to the modern callback protocol (WM_CONTEXTMENU / NIN_SELECT);
        // must be repeated after every NIM_ADD (e.g. after an Explorer restart).
        let mut n = nid(app);
        n.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let ok = Shell_NotifyIconW(NIM_SETVERSION, &n).as_bool();
        TRAY_V4.store(ok, Ordering::SeqCst);
    }
}

/// Make sure the icon still exists in the shell; re-add it if the shell lost it
/// (Explorer hiccup, missed `TaskbarCreated`, sleep/resume, remote session).
fn tray_check(app: &App) {
    let alive = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid(app)).as_bool() };
    if !alive {
        log_warn!("Tray-Symbol fehlt in der Shell – wird neu angelegt");
        tray_add(app);
    }
}

fn balloon(app: &App, text: &str) {
    let mut n = nid(app);
    n.uFlags |= NIF_INFO;
    n.dwInfoFlags = NIIF_INFO;
    copy_wide(&mut n.szInfoTitle, APP_NAME);
    copy_wide(&mut n.szInfo, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
    }
}

/// In `NOTIFYICON_VERSION_4` the icon position (screen coordinates) is packed
/// into `wParam` as signed 16-bit values. `None` if it looks unset.
fn tray_anchor(wparam: usize) -> Option<POINT> {
    let x = (wparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x != 0 || y != 0).then_some(POINT { x, y })
}

fn show_menu(hwnd: HWND, anchor: Option<POINT>) {
    // A menu is already open (nested loop): a second one would never appear.
    if MENU_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    show_menu_inner(hwnd, anchor);
    MENU_OPEN.store(false, Ordering::SeqCst);
}

fn show_menu_inner(hwnd: HWND, anchor: Option<POINT>) {
    // Never fail silently: if the state is busy, still show the menu.
    let auto_start = autostart::is_enabled();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            log_error!("Tray-Menü: CreatePopupMenu fehlgeschlagen");
            return;
        };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let add = |id: usize, flags: MENU_ITEM_FLAGS, text: &str| {
            let t = wide(text);
            let _ = AppendMenuW(menu, MF_STRING | flags, id, PCWSTR(t.as_ptr()));
        };
        add(ID_OPEN, MENU_ITEM_FLAGS(0), "Übersicht öffnen");
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(ID_SAVE, MENU_ITEM_FLAGS(0), "Layout jetzt speichern");
        add(
            ID_RESTORE,
            MENU_ITEM_FLAGS(0),
            "Layout jetzt wiederherstellen",
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(
            ID_AUTOSTART,
            check(auto_start),
            "Mit Windows starten an/aus",
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(ID_UPDATE, MENU_ITEM_FLAGS(0), "Nach Updates suchen");
        add(ID_QUIT, MENU_ITEM_FLAGS(0), "Beenden");

        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);
        let pt = anchor.unwrap_or_else(|| {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            pt
        });
        if !SetForegroundWindow(hwnd).as_bool() {
            log_warn!("Tray-Menü: SetForegroundWindow abgelehnt");
        }
        // The app state must not be borrowed here: the menu runs a nested loop.
        let cmd = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            None,
        );
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if cmd.0 > 0 {
            log_info!("Tray-Menü: Befehl {}", cmd.0);
            on_command(hwnd, cmd.0 as usize, 0);
        }
    }
}

/// Run a tray menu command. If the app state is momentarily borrowed (nested
/// message loop), retry via the message queue instead of silently dropping it.
fn on_command(hwnd: HWND, id: usize, tries: usize) {
    if id != ID_OPEN && id != ID_UPDATE && id != ID_QUIT && with_app(|_| ()).is_none() {
        if tries < 20 {
            log_warn!(
                "Tray-Befehl {id}: Zustand belegt, neuer Versuch {}",
                tries + 1
            );
            unsafe {
                let _ = PostMessageW(hwnd, WM_APP_COMMAND, WPARAM(id), LPARAM(tries as isize + 1));
            }
        } else {
            log_error!("Tray-Befehl {id} verworfen: Zustand dauerhaft belegt");
        }
        return;
    }
    match id {
        ID_OPEN => open_gui(),
        ID_SAVE => {
            with_app(|app| {
                let n = snapshot(app);
                balloon(app, &format!("Layout gespeichert ({n} Fenster)."));
            });
        }
        ID_RESTORE => {
            with_app(|app| {
                if app.layout.is_empty() {
                    balloon(app, "Es ist noch kein Layout gespeichert.");
                    return;
                }
                let n = restore(app, true);
                balloon(app, &format!("{n} Fenster wiederhergestellt."));
            });
        }
        ID_AUTOSTART => {
            with_app(|app| {
                let want = !autostart::is_enabled();
                if set_autostart(app, want) {
                    balloon(
                        app,
                        if want {
                            "Start mit Windows ist jetzt an."
                        } else {
                            "Start mit Windows ist jetzt aus."
                        },
                    );
                } else {
                    balloon(app, "Autostart konnte nicht geändert werden.");
                }
            });
        }
        ID_UPDATE => start_update_check(UpdateSource::Tray),
        ID_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
    }
    with_app(|app| publish(app, false));
}

fn set_autostart(app: &App, want: bool) -> bool {
    match autostart::set(want, &app.exe) {
        Ok(()) => {
            log_info!("Autostart: {want}");
            true
        }
        Err(e) => {
            log_error!("Autostart fehlgeschlagen: {e}");
            false
        }
    }
}

// ------------------------------------------------------------------- gui ----

/// Open the overview window or bring the open one to the front.
fn open_gui() {
    with_app(|app| publish(app, true));
    gui::open_or_focus(bridge().clone());
}

/// Copy the current state into the snapshot the GUI renders. Skipped while
/// no GUI window exists (unless `force`).
fn publish(app: &App, force: bool) {
    let b = bridge();
    if !force && !b.is_ui_open() {
        return;
    }
    let watching = app.watch_until.is_some_and(|t| Instant::now() < t);
    let autostart = autostart::is_enabled();
    b.update(|s| {
        s.settings = app.cfg.settings();
        s.autostart = autostart;
        s.last_save_at = app.state.last_save_at;
        s.last_restore_at = app.state.last_restore_at;
        s.last_update_check = app.cfg.last_update_check;
        s.watching = watching;
        s.entries = app.layout.windows.clone();
    });
}

fn handle_ui_command(cmd: UiCommand) {
    let b = bridge();
    match cmd {
        UiCommand::CheckUpdates => {
            start_update_check(UpdateSource::Gui);
            return;
        }
        UiCommand::SaveLayout => {
            with_app(|app| {
                let n = snapshot(app);
                b.notify(format!("Layout gespeichert ({n} Fenster)."));
            });
        }
        UiCommand::RestoreAll => {
            with_app(|app| {
                let n = restore(app, true);
                b.notify(format!("{n} Fenster wiederhergestellt."));
            });
        }
        UiCommand::RestoreEntry(id) => {
            with_app(|app| restore_entry(app, &id));
        }
        UiCommand::SetIgnored(id, ignored) => {
            with_app(|app| {
                if let Some(i) = app.layout.position_of(&id) {
                    app.layout.windows[i].ignored = ignored;
                    write_layout(app);
                }
            });
        }
        UiCommand::DeleteEntry(id) => {
            with_app(|app| {
                if let Some(i) = app.layout.position_of(&id) {
                    app.layout.windows.remove(i);
                    write_layout(app);
                }
            });
        }
        UiCommand::SetAutostart(want) => {
            with_app(|app| {
                if !set_autostart(app, want) {
                    b.notify("Autostart konnte nicht geändert werden.");
                }
            });
        }
        UiCommand::SetSettings(edit) => {
            with_app(|app| {
                if app.cfg.apply_settings(&edit) {
                    save_config(app);
                    log_info!("Einstellungen über die Oberfläche geändert");
                }
            });
        }
    }
    with_app(|app| publish(app, false));
}

/// Restore a single saved entry (also if it is ignored). Never starts programs.
fn restore_entry(app: &mut App, id: &Identity) {
    let b = bridge();
    let Some(idx) = app.layout.position_of(id) else {
        b.notify("Der Eintrag existiert nicht mehr.");
        return;
    };
    let name = exe_file_name(&id.exe_path).to_string();
    let mons = winapi::monitors();
    let live = winapi::enumerate(&mons);
    let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
    let Some(li) = app.layout.live_index_for(idx, &live_ids) else {
        b.notify(format!(
            "„{name}“ ist nicht geöffnet – es wurde nichts gestartet."
        ));
        return;
    };
    let hwnd = live[li].hwnd;
    let placement = app.layout.windows[idx].placement.clone();
    if winapi::apply(hwnd, &placement, &mons) {
        schedule_verify(app, hwnd, placement);
        app.handled.insert(hwnd);
        mark_restored(app);
        b.notify(format!("Fenster von „{name}“ wiederhergestellt."));
    } else {
        b.notify(format!(
            "Das Fenster von „{name}“ konnte nicht verschoben werden."
        ));
    }
}

fn mark_saved(app: &mut App) {
    app.state.last_save_at = logger::now_secs();
    save_state(app);
}

fn mark_restored(app: &mut App) {
    app.state.last_restore_at = logger::now_secs();
    save_state(app);
}

fn save_state(app: &App) {
    if let Err(e) = app.state.save(&app.state_path) {
        log_error!("state.json konnte nicht gespeichert werden: {e}");
    }
}

fn save_config(app: &App) {
    if let Err(e) = app.cfg.save(&app.cfg_path) {
        log_error!("config.toml konnte nicht gespeichert werden: {e}");
    }
}

// ----------------------------------------------------------------- hooks ----

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != 0 || id_child != 0 || hwnd.0.is_null() {
        return;
    }
    // Top-level windows only.
    if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CHILD.0 != 0 {
        return;
    }
    let main = MAIN_HWND.load(Ordering::SeqCst);
    if main != 0 {
        let _ = PostMessageW(
            hwnd_from(main),
            WM_APP_WINEVENT,
            WPARAM(event as usize),
            LPARAM(hwnd.0 as isize),
        );
    }
}

fn install_hooks(app: &mut App) {
    for ev in [
        EVENT_SYSTEM_MOVESIZEEND,
        EVENT_OBJECT_DESTROY,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_HIDE,
    ] {
        let h = unsafe {
            SetWinEventHook(
                ev,
                ev,
                HMODULE::default(),
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if h.0.is_null() {
            log_warn!("WinEvent-Hook {ev:#x} fehlgeschlagen");
        } else {
            app.hooks.push(h);
        }
    }
}

fn on_win_event(app: &mut App, event: u32, id: isize) {
    let now = Instant::now();
    match event {
        EVENT_OBJECT_SHOW => {
            let watching = app.watch_until.is_some_and(|t| now < t);
            if watching && !app.handled.contains(&id) {
                app.pending
                    .insert(id, (now + Duration::from_millis(600), 0));
                unsafe {
                    SetTimer(app.hwnd, TIMER_APPLY, 400, None);
                }
            }
        }
        EVENT_SYSTEM_MOVESIZEEND => {
            // The user took over this window.
            app.handled.insert(id);
            app.pending.remove(&id);
            app.verify.remove(&id);
        }
        EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE => {
            if event == EVENT_OBJECT_DESTROY {
                app.handled.remove(&id);
            }
            app.pending.remove(&id);
            app.verify.remove(&id);
        }
        _ => {}
    }
}

// ------------------------------------------------------------ save/restore ----

/// Write the layout if it changed. Returns true if the file was written.
fn write_layout(app: &mut App) -> bool {
    let json = app.layout.to_json();
    if json == app.last_saved {
        return false;
    }
    match app.layout.save(&app.layout_path) {
        Ok(()) => {
            app.last_saved = json;
            true
        }
        Err(e) => {
            log_error!("layout.json konnte nicht gespeichert werden: {e}");
            false
        }
    }
}

/// Capture the current windows into the saved layout (manual save only).
/// Returns the number of captured windows.
fn snapshot(app: &mut App) -> usize {
    let mons = winapi::monitors();
    if mons.is_empty() {
        return 0;
    }
    let live = winapi::enumerate(&mons);
    let n = live.len();
    let entries: Vec<Entry> = live.into_iter().map(|l| l.entry).collect();
    app.layout.merge(entries);
    write_layout(app);
    mark_saved(app);
    publish(app, false);
    n
}

/// Apply the saved layout to all matching windows. Returns how many were applied.
fn restore(app: &mut App, manual: bool) -> usize {
    let mons = winapi::monitors();
    if mons.is_empty() || app.layout.is_empty() {
        return 0;
    }
    let live = winapi::enumerate(&mons);
    let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
    let matches = app.layout.restore_plan(&live_ids);
    let mut n = 0;
    for (l, m) in live.iter().zip(&matches) {
        app.handled.insert(l.hwnd);
        if let Some(si) = m {
            let placement = app.layout.windows[*si].placement.clone();
            if winapi::apply(l.hwnd, &placement, &mons) {
                n += 1;
                schedule_verify(app, l.hwnd, placement);
            }
        }
    }
    if n > 0 {
        mark_restored(app);
    }
    if manual {
        log_info!("Manuelle Wiederherstellung: {n} Fenster");
    }
    publish(app, false);
    n
}

/// Re-check a restored window a few times and re-apply if the app moved it.
fn schedule_verify(app: &mut App, hwnd: isize, placement: Placement) {
    let due = Instant::now() + Duration::from_millis(VERIFY_DELAYS_MS[0]);
    app.verify.insert(hwnd, (due, 0, placement));
    unsafe {
        SetTimer(app.hwnd, TIMER_VERIFY, 500, None);
    }
}

fn process_verify(app: &mut App) {
    let now = Instant::now();
    let due: Vec<isize> = app
        .verify
        .iter()
        .filter(|(_, (t, _, _))| *t <= now)
        .map(|(k, _)| *k)
        .collect();
    if !due.is_empty() {
        let mons = winapi::monitors();
        for id in due {
            let Some((_, done, placement)) = app.verify.remove(&id) else {
                continue;
            };
            if !winapi::is_applied(id, &placement, &mons) {
                if winapi::apply(id, &placement, &mons) {
                    log_info!(
                        "Fenster nach dem Start erneut positioniert (Versuch {})",
                        done + 1
                    );
                }
            }
            let next = done + 1;
            if let Some(delay) = VERIFY_DELAYS_MS.get(next) {
                let wait = delay - VERIFY_DELAYS_MS[next - 1];
                app.verify
                    .insert(id, (now + Duration::from_millis(wait), next, placement));
            }
        }
    }
    if app.verify.is_empty() {
        unsafe {
            let _ = KillTimer(app.hwnd, TIMER_VERIFY);
        }
    }
}

/// Restore windows that appeared recently.
fn process_pending(app: &mut App) {
    let now = Instant::now();
    let due: Vec<isize> = app
        .pending
        .iter()
        .filter(|(_, (t, _))| *t <= now)
        .map(|(k, _)| *k)
        .collect();
    if !due.is_empty() {
        let mons = winapi::monitors();
        let live = winapi::enumerate(&mons);
        let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
        let matches = app.layout.restore_plan(&live_ids);
        let mut restored_any = false;
        for id in due {
            let (_, attempts) = app.pending.remove(&id).unwrap_or((now, 0));
            let found = live.iter().position(|l| l.hwnd == id);
            match found.and_then(|i| matches[i]) {
                Some(si) => {
                    let placement = app.layout.windows[si].placement.clone();
                    if winapi::apply(id, &placement, &mons) {
                        schedule_verify(app, id, placement);
                        restored_any = true;
                        log_info!(
                            "Spätes Fenster wiederhergestellt: {}",
                            app.layout.windows[si].identity.title
                        );
                    }
                    app.handled.insert(id);
                }
                None if found.is_some() && attempts < 3 => {
                    // Title/class may not be final yet; try again shortly.
                    app.pending
                        .insert(id, (now + Duration::from_millis(1000), attempts + 1));
                }
                None => {
                    app.handled.insert(id);
                }
            }
        }
        if restored_any {
            mark_restored(app);
            publish(app, false);
        }
    }
    if app.pending.is_empty() {
        unsafe {
            let _ = KillTimer(app.hwnd, TIMER_APPLY);
        }
    }
}

// --------------------------------------------------------------- updater ----

fn start_update_check(source: UpdateSource) {
    if UPDATE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(exe) = with_app(|app| {
        app.cfg.last_update_check = logger::now_secs();
        save_config(app);
        publish(app, false);
        app.exe.clone()
    }) else {
        UPDATE_RUNNING.store(false, Ordering::SeqCst);
        return;
    };
    bridge().update(|s| s.update = UpdateStatus::Checking);
    std::thread::spawn(move || {
        update_flow(source, &exe);
        UPDATE_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn set_update_status(status: UpdateStatus) {
    bridge().update(|s| s.update = status);
}

fn update_flow(source: UpdateSource, exe: &std::path::Path) {
    // The GUI shows results itself; message boxes are for the tray menu only.
    let manual = source == UpdateSource::Tray;
    let update = match updater::check(APP_VERSION) {
        Ok(Some(u)) => u,
        Ok(None) => {
            log_info!("Kein Update verfügbar");
            set_update_status(UpdateStatus::UpToDate);
            if manual {
                message_box(
                    &format!("Sie verwenden bereits die aktuelle Version ({APP_VERSION})."),
                    APP_NAME,
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            return;
        }
        Err(e) => {
            log_warn!("Update-Prüfung: {e}");
            set_update_status(UpdateStatus::Failed(e.clone()));
            if manual {
                message_box(&e, APP_NAME, MB_OK | MB_ICONWARNING);
            }
            return;
        }
    };
    set_update_status(UpdateStatus::Available(update.version.clone()));
    let q = format!(
        "Eine neue Version von WindowKeeper ist verfügbar: {} (installiert: {APP_VERSION}).\n\n\
         Jetzt herunterladen und installieren? WindowKeeper startet danach neu.",
        update.version
    );
    if message_box(&q, "WindowKeeper – Update", MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    set_update_status(UpdateStatus::Checking);
    let new_file = updater::new_path(exe);
    let result = updater::download_verified(&update, &new_file).and_then(|()| {
        updater::swap_in(exe, &new_file)?;
        std::process::Command::new(exe)
            .arg("--restarted")
            .spawn()
            .map_err(|e| format!("Neustart fehlgeschlagen: {e}"))?;
        Ok(())
    });
    match result {
        Ok(()) => {
            log_info!("Update auf {} installiert, Neustart", update.version);
            let main = MAIN_HWND.load(Ordering::SeqCst);
            unsafe {
                let _ = PostMessageW(hwnd_from(main), WM_APP_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        Err(e) => {
            log_error!("Update fehlgeschlagen: {e}");
            set_update_status(UpdateStatus::Failed(e.clone()));
            message_box(
                &format!("Das Update konnte nicht installiert werden:\n{e}"),
                APP_NAME,
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

// ---------------------------------------------------------------- wndproc ----

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(hwnd, msg, wp, lp)));
    match handled {
        Ok(Some(r)) => r,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn handle(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_APP_TRAY => {
            let v4 = TRAY_V4.load(Ordering::SeqCst);
            match decode(lp.0, v4) {
                TrayAction::ShowMenu => show_menu(hwnd, if v4 { tray_anchor(wp.0) } else { None }),
                TrayAction::OpenOverview => open_gui(),
                TrayAction::Ignore => {}
            }
            Some(LRESULT(0))
        }
        WM_APP_COMMAND => {
            on_command(hwnd, wp.0, lp.0 as usize);
            Some(LRESULT(0))
        }
        WM_APP_UI => {
            for cmd in bridge().take_commands() {
                handle_ui_command(cmd);
            }
            Some(LRESULT(0))
        }
        WM_APP_WINEVENT => {
            let event = wp.0 as u32;
            with_app(|app| on_win_event(app, event, lp.0));
            Some(LRESULT(0))
        }
        WM_TIMER => {
            let id = wp.0;
            with_app(|app| match id {
                TIMER_APPLY => process_pending(app),
                TIMER_TRAY => tray_check(app),
                TIMER_VERIFY => process_verify(app),
                TIMER_WATCH_END => {
                    unsafe {
                        let _ = KillTimer(app.hwnd, TIMER_WATCH_END);
                    }
                    app.watch_until = None;
                    app.pending.clear();
                    log_info!("Beobachtung später Fenster beendet");
                    publish(app, false);
                }
                _ => {}
            });
            if id == TIMER_UPDATE {
                unsafe {
                    let _ = KillTimer(hwnd, TIMER_UPDATE);
                }
                start_update_check(UpdateSource::Auto);
            }
            Some(LRESULT(0))
        }
        WM_APP_QUIT | WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            with_app(|app| unsafe {
                for h in app.hooks.drain(..) {
                    let _ = UnhookWinEvent(h);
                }
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid(app));
                let _ = DestroyIcon(app.hicon);
            });
            unsafe { PostQuitMessage(0) };
            Some(LRESULT(0))
        }
        m if m != 0 && with_app(|a| a.taskbar_created == m).unwrap_or(false) => {
            // Explorer restarted: re-add the tray icon.
            with_app(|app| tray_add(app));
            Some(LRESULT(0))
        }
        _ => None,
    }
}
