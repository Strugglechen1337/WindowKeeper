//! Tray application: hidden message window, WinEvent hooks, timers, menu.

use crate::config::Config;
use crate::layout::{Entry, Layout};
use crate::matching::match_windows;
use crate::winapi::{self, hwnd_from};
use crate::{autostart, icon, logger, updater, APP_NAME, APP_VERSION};
use crate::{log_error, log_info, log_warn};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
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
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_WINEVENT: u32 = WM_APP + 2;
const WM_APP_QUIT: u32 = WM_APP + 3;

const TIMER_SAVE: usize = 1;
const TIMER_APPLY: usize = 2;
const TIMER_SCAN: usize = 3;
const TIMER_UPDATE: usize = 4;
const TIMER_WATCH_END: usize = 5;

const ID_SAVE: usize = 1001;
const ID_RESTORE: usize = 1002;
const ID_AUTOSAVE: usize = 1003;
const ID_AUTOSTART: usize = 1004;
const ID_UPDATE: usize = 1005;
const ID_QUIT: usize = 1006;

const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
const EVENT_SYSTEM_MOVESIZEEND: u32 = 0x000B;
const EVENT_SYSTEM_MINIMIZESTART: u32 = 0x0016;
const EVENT_SYSTEM_MINIMIZEEND: u32 = 0x0017;
const EVENT_OBJECT_DESTROY: u32 = 0x8001;
const EVENT_OBJECT_SHOW: u32 = 0x8002;
const EVENT_OBJECT_HIDE: u32 = 0x8003;
const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static LOC_PENDING: AtomicBool = AtomicBool::new(false);
static UPDATE_RUNNING: AtomicBool = AtomicBool::new(false);

struct App {
    hwnd: HWND,
    exe: PathBuf,
    cfg: Config,
    cfg_path: PathBuf,
    layout: Layout,
    layout_path: PathBuf,
    last_saved: String,
    watch_until: Option<Instant>,
    handled: HashSet<isize>,
    pending: HashMap<isize, (Instant, u32)>,
    suppress_until: Instant,
    save_armed: bool,
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

        let hicon = create_icon();
        let first_run = !cfg.first_run_done;
        let watch_minutes = cfg.restore_watch_minutes;
        let scan_ms = cfg.scan_interval_secs * 1000;
        let update_due = cfg.check_updates_on_start
            && updater::check_due(cfg.last_update_check, logger::now_secs());
        let app = App {
            hwnd,
            exe,
            cfg,
            cfg_path,
            layout,
            layout_path,
            last_saved: String::new(),
            watch_until: None,
            handled: HashSet::new(),
            pending: HashMap::new(),
            suppress_until: Instant::now(),
            save_armed: false,
            hicon,
            hooks: Vec::new(),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        };
        APP.with(|a| *a.borrow_mut() = Some(app));

        with_app(|app| {
            app.last_saved = app.layout.to_json();
            tray_add(app);
            install_hooks(app);
            SetTimer(app.hwnd, TIMER_SCAN, scan_ms, None);
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
                     Rechtsklick auf das Symbol öffnet das Menü.",
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
    n.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    n.uCallbackMessage = WM_APP_TRAY;
    n.hIcon = app.hicon;
    copy_wide(&mut n.szTip, "WindowKeeper – Fensterlayout merken");
    n
}

fn tray_add(app: &App) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid(app));
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

fn show_menu(hwnd: HWND) {
    let Some((auto_save, auto_start)) = with_app(|a| (a.cfg.auto_save, autostart::is_enabled()))
    else {
        return;
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let add = |id: usize, flags: MENU_ITEM_FLAGS, text: &str| {
            let t = wide(text);
            let _ = AppendMenuW(menu, MF_STRING | flags, id, PCWSTR(t.as_ptr()));
        };
        add(ID_SAVE, MENU_ITEM_FLAGS(0), "Layout jetzt speichern");
        add(
            ID_RESTORE,
            MENU_ITEM_FLAGS(0),
            "Layout jetzt wiederherstellen",
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(
            ID_AUTOSAVE,
            check(auto_save),
            "Automatisch speichern an/aus",
        );
        add(
            ID_AUTOSTART,
            check(auto_start),
            "Mit Windows starten an/aus",
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        add(ID_UPDATE, MENU_ITEM_FLAGS(0), "Nach Updates suchen");
        add(ID_QUIT, MENU_ITEM_FLAGS(0), "Beenden");

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
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
            on_command(hwnd, cmd.0 as usize);
        }
    }
}

fn on_command(hwnd: HWND, id: usize) {
    match id {
        ID_SAVE => {
            with_app(|app| {
                let n = snapshot(app, true);
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
        ID_AUTOSAVE => {
            with_app(|app| {
                app.cfg.auto_save = !app.cfg.auto_save;
                save_config(app);
                log_info!("Automatisch speichern: {}", app.cfg.auto_save);
            });
        }
        ID_AUTOSTART => {
            with_app(|app| {
                let want = !autostart::is_enabled();
                match autostart::set(want, &app.exe) {
                    Ok(()) => log_info!("Autostart: {want}"),
                    Err(e) => {
                        log_error!("Autostart fehlgeschlagen: {e}");
                        balloon(app, "Autostart konnte nicht geändert werden.");
                    }
                }
            });
        }
        ID_UPDATE => start_update_check(true),
        ID_QUIT => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        _ => {}
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
    if event == EVENT_OBJECT_LOCATIONCHANGE && LOC_PENDING.swap(true, Ordering::SeqCst) {
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
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_MOVESIZEEND,
        EVENT_SYSTEM_MINIMIZESTART,
        EVENT_SYSTEM_MINIMIZEEND,
        EVENT_OBJECT_DESTROY,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_HIDE,
        EVENT_OBJECT_LOCATIONCHANGE,
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
            arm_save(app, false);
        }
        EVENT_SYSTEM_MOVESIZEEND => {
            // The user took over this window.
            app.handled.insert(id);
            app.pending.remove(&id);
            arm_save(app, true);
        }
        EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE => {
            if event == EVENT_OBJECT_DESTROY {
                app.handled.remove(&id);
            }
            app.pending.remove(&id);
        }
        _ => arm_save(app, false),
    }
}

// ------------------------------------------------------------ save/restore ----

/// Schedule an automatic save. `restart` re-arms (debounce), otherwise an
/// already pending save is kept (throttle).
fn arm_save(app: &mut App, restart: bool) {
    if !app.cfg.auto_save || Instant::now() < app.suppress_until {
        return;
    }
    if restart || !app.save_armed {
        app.save_armed = true;
        unsafe {
            SetTimer(app.hwnd, TIMER_SAVE, app.cfg.debounce_ms, None);
        }
    }
}

fn write_layout(app: &mut App) {
    let json = app.layout.to_json();
    if json == app.last_saved {
        return;
    }
    match app.layout.save(&app.layout_path) {
        Ok(()) => app.last_saved = json,
        Err(e) => log_error!("layout.json konnte nicht gespeichert werden: {e}"),
    }
}

/// Capture the current windows into the layout. Automatic runs skip windows
/// that still wait for their restore so the saved positions are not clobbered.
/// Returns the number of captured windows.
fn snapshot(app: &mut App, manual: bool) -> usize {
    let mons = winapi::monitors();
    if mons.is_empty() {
        return 0;
    }
    let mut live = winapi::enumerate(&mons);
    let watching = app.watch_until.is_some_and(|t| Instant::now() < t);
    if !manual && watching {
        let ids: Vec<_> = app.layout.identities();
        let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
        let matches = match_windows(&ids, &live_ids);
        let mut keep = matches.iter();
        live.retain(|l| {
            let m = keep.next().copied().flatten();
            m.is_none() || app.handled.contains(&l.hwnd)
        });
    }
    let n = live.len();
    let entries: Vec<Entry> = live.into_iter().map(|l| l.entry).collect();
    app.layout.merge(entries);
    write_layout(app);
    n
}

/// Apply the saved layout to all matching windows. Returns how many were applied.
fn restore(app: &mut App, manual: bool) -> usize {
    let mons = winapi::monitors();
    if mons.is_empty() || app.layout.is_empty() {
        return 0;
    }
    let live = winapi::enumerate(&mons);
    let saved_ids = app.layout.identities();
    let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
    let matches = match_windows(&saved_ids, &live_ids);
    let mut n = 0;
    for (l, m) in live.iter().zip(&matches) {
        app.handled.insert(l.hwnd);
        if let Some(si) = m {
            if winapi::apply(l.hwnd, &app.layout.windows[*si].placement, &mons) {
                n += 1;
            }
        }
    }
    app.suppress_until = Instant::now() + Duration::from_millis(2500);
    if manual {
        log_info!("Manuelle Wiederherstellung: {n} Fenster");
    }
    n
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
        let saved_ids = app.layout.identities();
        let live_ids: Vec<_> = live.iter().map(|l| l.entry.identity.clone()).collect();
        let matches = match_windows(&saved_ids, &live_ids);
        for id in due {
            let (_, attempts) = app.pending.remove(&id).unwrap_or((now, 0));
            let found = live.iter().position(|l| l.hwnd == id);
            match found.and_then(|i| matches[i]) {
                Some(si) => {
                    if winapi::apply(id, &app.layout.windows[si].placement, &mons) {
                        log_info!(
                            "Spätes Fenster wiederhergestellt: {}",
                            app.layout.windows[si].identity.title
                        );
                    }
                    app.handled.insert(id);
                    app.suppress_until = now + Duration::from_millis(1500);
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
    }
    if app.pending.is_empty() {
        unsafe {
            let _ = KillTimer(app.hwnd, TIMER_APPLY);
        }
    }
}

// --------------------------------------------------------------- updater ----

fn start_update_check(manual: bool) {
    if UPDATE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(exe) = with_app(|app| {
        app.cfg.last_update_check = logger::now_secs();
        save_config(app);
        app.exe.clone()
    }) else {
        UPDATE_RUNNING.store(false, Ordering::SeqCst);
        return;
    };
    std::thread::spawn(move || {
        update_flow(manual, &exe);
        UPDATE_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn update_flow(manual: bool, exe: &std::path::Path) {
    let update = match updater::check(APP_VERSION) {
        Ok(Some(u)) => u,
        Ok(None) => {
            log_info!("Kein Update verfügbar");
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
            if manual {
                message_box(&e, APP_NAME, MB_OK | MB_ICONWARNING);
            }
            return;
        }
    };
    let q = format!(
        "Eine neue Version von WindowKeeper ist verfügbar: {} (installiert: {APP_VERSION}).\n\n\
         Jetzt herunterladen und installieren? WindowKeeper startet danach neu.",
        update.version
    );
    if message_box(&q, "WindowKeeper – Update", MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
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
            let m = (lp.0 as u32) & 0xFFFF;
            if m == WM_RBUTTONUP || m == WM_LBUTTONUP || m == WM_CONTEXTMENU {
                show_menu(hwnd);
            }
            Some(LRESULT(0))
        }
        WM_APP_WINEVENT => {
            let event = wp.0 as u32;
            if event == EVENT_OBJECT_LOCATIONCHANGE {
                LOC_PENDING.store(false, Ordering::SeqCst);
            }
            with_app(|app| on_win_event(app, event, lp.0));
            Some(LRESULT(0))
        }
        WM_TIMER => {
            let id = wp.0;
            with_app(|app| match id {
                TIMER_SAVE => {
                    unsafe {
                        let _ = KillTimer(app.hwnd, TIMER_SAVE);
                    }
                    app.save_armed = false;
                    if app.cfg.auto_save && Instant::now() >= app.suppress_until {
                        snapshot(app, false);
                    }
                }
                TIMER_SCAN => {
                    if app.cfg.auto_save && Instant::now() >= app.suppress_until {
                        snapshot(app, false);
                    }
                }
                TIMER_APPLY => process_pending(app),
                TIMER_WATCH_END => {
                    unsafe {
                        let _ = KillTimer(app.hwnd, TIMER_WATCH_END);
                    }
                    app.watch_until = None;
                    app.pending.clear();
                    log_info!("Beobachtung später Fenster beendet");
                }
                _ => {}
            });
            if id == TIMER_UPDATE {
                unsafe {
                    let _ = KillTimer(hwnd, TIMER_UPDATE);
                }
                start_update_check(false);
            }
            Some(LRESULT(0))
        }
        WM_DISPLAYCHANGE => {
            // Windows rearranges windows when monitors change: do not save that.
            with_app(|app| app.suppress_until = Instant::now() + Duration::from_secs(5));
            Some(LRESULT(0))
        }
        WM_APP_QUIT | WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            with_app(|app| {
                if app.cfg.auto_save {
                    snapshot(app, false);
                }
                unsafe {
                    for h in app.hooks.drain(..) {
                        let _ = UnhookWinEvent(h);
                    }
                    let _ = Shell_NotifyIconW(NIM_DELETE, &nid(app));
                    let _ = DestroyIcon(app.hicon);
                }
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
