//! Renders the real overview GUI with clearly fake *sample data* and saves
//! PNG screenshots (dark + light, all four pages) for the README.
//!
//! Needs the `screenshots` feature and an X display with OpenGL
//! (see `scripts/export_screenshots.sh`, which starts Xvfb):
//!
//! ```text
//! cargo run --example export_screenshots --features screenshots --release [OUT_DIR]
//! ```
//!
//! Nothing here is measured on a real machine: the windows, times and the
//! update status are made up, and the GUI shows a "Beispieldaten" marker.

use eframe::egui::{self, ThemePreference};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windowkeeper::config::SettingsEdit;
use windowkeeper::geometry::{MonitorRef, Placement, Rect, ShowState};
use windowkeeper::gui::demo::{DemoPage, DemoView};
use windowkeeper::layout::Entry;
use windowkeeper::matching::Identity;
use windowkeeper::ui_model::{days_from_civil, Bridge, UpdateStatus};

const WINDOW_SIZE: [f32; 2] = [1000.0, 680.0];
/// Frames to let layout, fonts and textures settle before the screenshot.
const WARMUP_FRAMES: u32 = 8;
const PAGES: [(DemoPage, &str); 4] = [
    (DemoPage::Overview, "overview"),
    (DemoPage::Windows, "windows"),
    (DemoPage::Settings, "settings"),
    (DemoPage::Info, "info"),
];
const THEMES: [(ThemePreference, &str); 2] = [
    (ThemePreference::Dark, "dark"),
    (ThemePreference::Light, "light"),
];

/// Fixed demo time (2026-11-26, local time Europe/Berlin = UTC+1).
fn demo_time(h: i64, m: i64, s: i64) -> u64 {
    (days_from_civil(2026, 11, 26) * 86_400 + h * 3600 + m * 60 + s - 3600) as u64
}

/// Sample data: a 3440x1440 ultrawide with a 40 px taskbar.
fn demo_bridge() -> Arc<Bridge> {
    let monitor = MonitorRef {
        device_id: "DEMO".into(),
        gdi_name: "\\\\.\\DISPLAY1".into(),
        adapter: "Beispiel".into(),
        dpi: 96,
        work: Rect::new(0, 0, 3440, 1400),
    };
    let entry = |exe: &str, class: &str, title: &str, rect: Rect, ignored: bool| Entry {
        identity: Identity {
            exe_path: exe.into(),
            class_name: class.into(),
            title: title.into(),
        },
        placement: Placement {
            state: ShowState::Normal,
            restore_maximized: false,
            rect,
            monitor: monitor.clone(),
        },
        ignored,
    };
    let entries = vec![
        entry(
            "C:\\Program Files\\Mozilla Firefox\\firefox.exe",
            "MozillaWindowClass",
            "Startseite – Mozilla Firefox",
            Rect::new(0, 0, 1100, 1400),
            false,
        ),
        entry(
            "C:\\Users\\Marcel\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
            "Chrome_WidgetWin_1",
            "main.rs – windowkeeper – Visual Studio Code",
            Rect::new(1100, 0, 1500, 1400),
            false,
        ),
        entry(
            "C:\\Program Files\\Microsoft Office\\root\\Office16\\OUTLOOK.EXE",
            "rctrl_renwnd32",
            "Posteingang – Outlook",
            Rect::new(2600, 700, 840, 700),
            true,
        ),
        entry(
            "C:\\Windows\\explorer.exe",
            "CabinetWClass",
            "Dieser PC",
            Rect::new(2600, 0, 840, 700),
            false,
        ),
    ];
    let bridge = Bridge::new(|| {});
    bridge.update(|s| {
        s.settings = SettingsEdit {
            restore_watch_minutes: 10,
            check_updates_on_start: true,
        };
        s.autostart = true;
        s.watching = false;
        s.last_save_at = demo_time(9, 12, 40);
        s.last_restore_at = demo_time(8, 41, 12);
        s.last_update_check = demo_time(8, 41, 30);
        s.update = UpdateStatus::UpToDate;
        s.entries = entries;
    });
    bridge
}

struct Step {
    theme: ThemePreference,
    page: DemoPage,
    file: PathBuf,
}

struct Driver {
    view: DemoView,
    steps: Vec<Step>,
    idx: usize,
    frames: u32,
    /// Number of finished screenshots (saved or failed).
    done: Arc<AtomicUsize>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl eframe::App for Driver {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let Some(step) = self.steps.get(self.idx) else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        if self.frames == 0 {
            ctx.set_theme(step.theme);
            self.view.set_page(step.page);
        }
        eframe::App::ui(&mut self.view, ui, frame);
        self.frames += 1;
        if self.frames == WARMUP_FRAMES {
            let file = step.file.clone();
            let done = self.done.clone();
            let errors = self.errors.clone();
            ctx.request_screenshot(move |img| {
                if let Err(e) = save_png(&file, &img) {
                    errors
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(format!("{}: {e}", file.display()));
                }
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        if self.frames > WARMUP_FRAMES && self.done.load(Ordering::SeqCst) > self.idx {
            self.idx += 1;
            self.frames = 0;
        }
        ctx.request_repaint();
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.view.clear_color(visuals)
    }
}

fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let rgba: Vec<u8> = img
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    image::save_buffer(
        path,
        &rgba,
        img.size[0] as u32,
        img.size[1] as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

/// App icon as PNG: 32 px original and a crisp 4x (nearest neighbour) copy.
fn save_icons(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let n = windowkeeper::icon::SIZE;
    let px = windowkeeper::icon::rgba();
    let small = dir.join("icon.png");
    image::save_buffer(&small, &px, n, n, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
    let k = 4;
    let mut big = Vec::with_capacity(px.len() * (k * k) as usize);
    for y in 0..n * k {
        for x in 0..n * k {
            let i = (((y / k) * n + x / k) * 4) as usize;
            big.extend_from_slice(&px[i..i + 4]);
        }
    }
    let large = dir.join("icon-128.png");
    image::save_buffer(&large, &big, n * k, n * k, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(vec![small, large])
}

fn main() {
    let out_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "docs/screenshots".into()),
    );
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("Ausgabeordner {} nicht anlegbar: {e}", out_dir.display());
        std::process::exit(1);
    }
    let steps: Vec<Step> = THEMES
        .iter()
        .flat_map(|&(theme, tname)| {
            let out_dir = &out_dir;
            PAGES.iter().map(move |&(page, pname)| Step {
                theme,
                page,
                file: out_dir.join(format!("{pname}-{tname}.png")),
            })
        })
        .collect();
    let expected: Vec<PathBuf> = steps.iter().map(|s| s.file.clone()).collect();
    // Remove stale files so success really means "freshly rendered".
    for f in &expected {
        let _ = std::fs::remove_file(f);
    }

    let done = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let bridge = demo_bridge();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WindowKeeper")
            .with_inner_size(WINDOW_SIZE)
            .with_resizable(false)
            .with_position([0.0, 0.0]),
        ..Default::default()
    };
    let (d, e) = (done.clone(), errors.clone());
    let result = eframe::run_native(
        "WindowKeeper",
        options,
        Box::new(move |cc| {
            Ok(Box::new(Driver {
                view: DemoView::new(cc, bridge),
                steps,
                idx: 0,
                frames: 0,
                done: d,
                errors: e,
            }))
        }),
    );
    if let Err(e) = result {
        eprintln!(
            "Fenster/OpenGL konnte nicht gestartet werden: {e}\n\
             Läuft ein X-Server (Xvfb) mit GLX, und ist LIBGL_ALWAYS_SOFTWARE=1 gesetzt? \
             Siehe scripts/export_screenshots.sh."
        );
        std::process::exit(1);
    }

    let mut ok = true;
    for err in errors.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        eprintln!("Fehler beim Speichern: {err}");
        ok = false;
    }
    match save_icons(&out_dir) {
        Ok(files) => println!(
            "{}",
            files
                .iter()
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        ),
        Err(e) => {
            eprintln!("Symbol konnte nicht gespeichert werden: {e}");
            ok = false;
        }
    }
    for f in &expected {
        match std::fs::metadata(f) {
            Ok(m) if m.len() > 0 => println!("{}", f.display()),
            _ => {
                eprintln!("Fehlt oder leer: {}", f.display());
                ok = false;
            }
        }
    }
    std::process::exit(if ok { 0 } else { 1 });
}
