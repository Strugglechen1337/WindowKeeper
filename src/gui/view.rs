//! The egui application: navigation, pages and small Windows 11 style widgets.

use super::icons::Icons;
use super::theme::{self, palette, Palette};
use crate::config::{SettingsEdit, DEBOUNCE_MS, SCAN_INTERVAL_SECS, WATCH_MINUTES_MAX};
use crate::icon;
use crate::layout::Entry;
use crate::matching::Identity;
use crate::ui_model::{
    exe_file_name, matches_filter, monitor_name, position_text, state_label, status_sentence,
    Bridge, Snapshot, UiCommand, UpdateStatus, NOTICE_SECS,
};
use crate::{APP_NAME, APP_VERSION};
use eframe::egui::{
    self, vec2, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui,
};
use std::sync::Arc;

const REPO_URL: &str = "https://github.com/Strugglechen1337/WindowKeeper";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Windows,
    Settings,
    Info,
}

impl Page {
    const ALL: [Page; 4] = [Page::Overview, Page::Windows, Page::Settings, Page::Info];

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Übersicht",
            Page::Windows => "Fenster",
            Page::Settings => "Einstellungen",
            Page::Info => "Info",
        }
    }

    /// Segoe Fluent Icons / MDL2 Assets code point.
    fn glyph(self) -> char {
        match self {
            Page::Overview => '\u{E80F}',
            Page::Windows => '\u{E7C4}',
            Page::Settings => '\u{E713}',
            Page::Info => '\u{E946}',
        }
    }
}

pub struct View {
    bridge: Arc<Bridge>,
    set_phase: fn(bool),
    snap: Snapshot,
    snap_version: u64,
    page: Page,
    search: String,
    confirm_delete: Option<Identity>,
    /// Settings as edited in the UI / last sent / last seen from the tray.
    edit: SettingsEdit,
    last_sent: SettingsEdit,
    seen_settings: SettingsEdit,
    icons: Icons,
    has_icon_font: bool,
    app_icon: TextureHandle,
    notice_seen: u64,
    notice: Option<(String, f64)>,
    first_frame: bool,
}

impl View {
    pub fn new(cc: &eframe::CreationContext<'_>, bridge: Arc<Bridge>, set_phase: fn(bool)) -> View {
        let ctx = &cc.egui_ctx;
        let has_icon_font = theme::install_fonts(ctx);
        theme::apply(ctx);
        let repaint_ctx = ctx.clone();
        bridge.set_repaint(Some(Arc::new(move || repaint_ctx.request_repaint())));
        let app_icon = ctx.load_texture(
            "app-icon",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon::SIZE as usize, icon::SIZE as usize],
                &icon::rgba(),
            ),
            TextureOptions::LINEAR,
        );
        let snap = bridge.snapshot();
        let snap_version = bridge.version();
        let settings = snap.settings;
        set_phase(true);
        View {
            bridge,
            set_phase,
            snap,
            snap_version,
            page: Page::Overview,
            search: String::new(),
            confirm_delete: None,
            edit: settings,
            last_sent: settings,
            seen_settings: settings,
            icons: Icons::new(ctx),
            has_icon_font,
            app_icon,
            notice_seen: 0,
            notice: None,
            first_frame: true,
        }
    }

    fn send(&self, cmd: UiCommand) {
        self.bridge.send(cmd);
    }

    /// Pull a fresh snapshot and keep the settings editor in sync with
    /// changes made elsewhere (tray menu) without fighting active edits.
    fn sync(&mut self, ctx: &egui::Context) {
        let v = self.bridge.version();
        if v != self.snap_version {
            self.snap = self.bridge.snapshot();
            self.snap_version = v;
            if let Some(n) = &self.snap.notice {
                if n.seq != self.notice_seen {
                    self.notice_seen = n.seq;
                    let until = ctx.input(|i| i.time) + NOTICE_SECS;
                    self.notice = Some((n.text.clone(), until));
                }
            }
        }
        let busy = busy(ctx);
        if !busy && self.snap.settings != self.seen_settings {
            self.edit = self.snap.settings;
            self.last_sent = self.snap.settings;
            self.seen_settings = self.snap.settings;
        }
    }

    /// Persist edited settings once the user let go of the control.
    fn commit_settings(&mut self, ctx: &egui::Context) {
        if self.edit != self.last_sent && !busy(ctx) {
            self.last_sent = self.edit;
            self.send(UiCommand::SetSettings(self.edit));
        }
    }
}

fn busy(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input()
}

impl eframe::App for View {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.first_frame {
            self.first_frame = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.sync(&ctx);
        let p = palette(ui.visuals().dark_mode);

        egui::Panel::left("nav")
            .exact_size(224.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.nav_bg)
                    .inner_margin(Margin::symmetric(10, 14)),
            )
            .show(ui, |ui| self.nav(ui, &p));

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(28, 22)),
            )
            .show(ui, |ui| match self.page {
                Page::Overview => self.page_overview(ui, &p),
                Page::Windows => self.page_windows(ui, &p),
                Page::Settings => self.page_settings(ui, &p),
                Page::Info => self.page_info(ui, &p),
            });

        self.toast(&ctx, &p);
        self.commit_settings(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.bridge.set_repaint(None);
        (self.set_phase)(false);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }
}

// ----------------------------------------------------------- navigation ----

impl View {
    fn nav(&mut self, ui: &mut Ui, p: &Palette) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.add(egui::Image::new((self.app_icon.id(), vec2(26.0, 26.0))));
            ui.label(semibold(APP_NAME, 17.0));
        });
        ui.add_space(14.0);
        for page in Page::ALL {
            let glyph = self.has_icon_font.then(|| page.glyph());
            if nav_item(ui, p, page.label(), glyph, self.page == page) {
                self.page = page;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!("Version {APP_VERSION}"))
                        .small()
                        .color(p.weak),
                );
            });
        });
    }
}

fn nav_item(ui: &mut Ui, p: &Palette, text: &str, glyph: Option<char>, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let painter = ui.painter();
    let fill = if selected {
        p.card
    } else if resp.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, 6, fill);
    if selected {
        let pill = Rect::from_min_size(
            Pos2::new(rect.left() + 3.0, rect.center().y - 8.0),
            vec2(3.0, 16.0),
        );
        painter.rect_filled(pill, 2, p.accent);
    }
    let mut x = rect.left() + 16.0;
    if let Some(g) = glyph {
        let color = if selected { p.accent } else { p.text };
        painter.text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            g,
            FontId::new(16.0, theme::icons_family()),
            color,
        );
        x += 30.0;
    }
    let font = if selected {
        FontId::new(14.5, theme::semibold())
    } else {
        FontId::proportional(14.5)
    };
    painter.text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        p.text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

// ------------------------------------------------------- small widgets ----

fn semibold(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).font(FontId::new(size, theme::semibold()))
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).heading());
    ui.add_space(8.0);
}

fn card<R>(ui: &mut Ui, p: &Palette, fill: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, p.card_stroke))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        Button::new(RichText::new(text).color(p.on_accent))
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .min_size(vec2(0.0, 34.0)),
    )
}

fn secondary_button(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(enabled, Button::new(text).min_size(vec2(0.0, 34.0)))
}

/// Windows 11 style on/off switch.
fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(42.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let radius = rect.height() / 2.0;
    let (fill, stroke, knob) = if *on {
        (p.accent, Stroke::NONE, p.on_accent)
    } else {
        (Color32::TRANSPARENT, Stroke::new(1.0, p.weak), p.weak)
    };
    let alpha = if ui.is_enabled() { 1.0 } else { 0.4 };
    let painter = ui.painter();
    painter.rect(
        rect.shrink(0.5),
        radius,
        fill.gamma_multiply(alpha),
        stroke,
        StrokeKind::Inside,
    );
    let knob_r = if resp.hovered() { 7.0 } else { 6.0 };
    let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    painter.circle_filled(
        Pos2::new(x, rect.center().y),
        knob_r,
        knob.gamma_multiply(alpha),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One settings line: title + description on the left, control on the right.
fn setting_row(ui: &mut Ui, p: &Palette, name: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let left = (ui.available_width() - 200.0).max(160.0);
        ui.vertical(|ui| {
            ui.set_max_width(left);
            ui.label(semibold(name, 15.0));
            ui.add(egui::Label::new(RichText::new(desc).small().color(p.weak)).wrap());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

fn separator(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

// ------------------------------------------------------------ overview ----

impl View {
    fn page_overview(&mut self, ui: &mut Ui, p: &Palette) {
        title(ui, "Übersicht");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let s = &self.snap;
                card(ui, p, p.card, |ui| {
                    ui.horizontal(|ui| {
                        dot(ui, if s.watching { p.warn } else { p.ok });
                        ui.vertical(|ui| {
                            ui.label(semibold("WindowKeeper läuft", 18.0));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(status_sentence(
                                        s.watching,
                                        s.settings.auto_save,
                                    ))
                                    .color(p.weak),
                                )
                                .wrap(),
                            );
                        });
                    });
                });
                ui.add_space(4.0);

                let on_off = |b: bool| if b { "An" } else { "Aus" };
                let tiles = [
                    ("Gemerkte Fenster", s.entries.len().to_string(), p.text),
                    ("Ignoriert", s.ignored_count().to_string(), p.text),
                    (
                        "Automatisch speichern",
                        on_off(s.settings.auto_save).into(),
                        if s.settings.auto_save { p.ok } else { p.weak },
                    ),
                    (
                        "Mit Windows starten",
                        on_off(s.autostart).into(),
                        if s.autostart { p.ok } else { p.weak },
                    ),
                ];
                let per_row = if ui.available_width() >= 600.0 { 4 } else { 2 };
                for chunk in tiles.chunks(per_row) {
                    ui.columns(per_row, |cols| {
                        for (col, (label, value, color)) in cols.iter_mut().zip(chunk) {
                            card(col, p, p.card, |ui| {
                                ui.label(RichText::new(*label).small().color(p.weak));
                                ui.label(semibold(value.clone(), 24.0).color(*color));
                            });
                        }
                    });
                }
                ui.add_space(4.0);

                card(ui, p, p.card, |ui| {
                    egui::Grid::new("times")
                        .num_columns(2)
                        .spacing([32.0, 10.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("Zuletzt gespeichert").color(p.weak));
                            ui.label(super::format_local(s.last_save_at));
                            ui.end_row();
                            ui.label(RichText::new("Zuletzt wiederhergestellt").color(p.weak));
                            ui.label(super::format_local(s.last_restore_at));
                            ui.end_row();
                        });
                });
                ui.add_space(8.0);

                let restorable = s.restorable_count();
                let empty = s.entries.is_empty();
                ui.horizontal(|ui| {
                    if primary_button(ui, p, "Layout speichern", true).clicked() {
                        self.bridge.send(UiCommand::SaveLayout);
                    }
                    let r = secondary_button(ui, "Layout wiederherstellen", restorable > 0);
                    let r = if empty {
                        r.on_disabled_hover_text("Es ist noch kein Layout gespeichert.")
                    } else {
                        r.on_disabled_hover_text("Alle Einträge sind ignoriert.")
                    };
                    if r.clicked() {
                        self.bridge.send(UiCommand::RestoreAll);
                    }
                });
            });
    }
}

// ------------------------------------------------------------- windows ----

impl View {
    fn page_windows(&mut self, ui: &mut Ui, p: &Palette) {
        title(ui, "Fenster");
        ui.label(
            RichText::new(format!(
                "{} gemerkt  ·  {} ignoriert",
                self.snap.entries.len(),
                self.snap.ignored_count()
            ))
            .color(p.weak),
        );
        ui.add_space(4.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Fenster suchen …")
                .margin(Margin::symmetric(10, 7))
                .desired_width(ui.available_width()),
        );
        ui.add_space(6.0);

        let entries = std::mem::take(&mut self.snap.entries);
        let shown: Vec<&Entry> = entries
            .iter()
            .filter(|e| matches_filter(e, &self.search))
            .collect();
        if shown.is_empty() {
            card(ui, p, p.card, |ui| {
                ui.add_space(14.0);
                ui.vertical_centered(|ui| {
                    if entries.is_empty() {
                        ui.label(semibold("Noch keine Fenster gemerkt", 17.0));
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Sobald Programme geöffnet sind, merkt sich WindowKeeper ihre \
                                     Position automatisch. Über „Layout speichern“ in der \
                                     Übersicht geht es auch sofort.",
                                )
                                .color(p.weak),
                            )
                            .wrap(),
                        );
                    } else {
                        ui.label(semibold("Keine Treffer", 17.0));
                        ui.label(
                            RichText::new(format!("Nichts gefunden für „{}“.", self.search.trim()))
                                .color(p.weak),
                        );
                    }
                });
                ui.add_space(14.0);
            });
        } else {
            let ctx = ui.ctx().clone();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for e in shown {
                        self.entry_row(ui, &ctx, p, e);
                        ui.add_space(2.0);
                    }
                });
        }
        self.snap.entries = entries;
    }

    fn entry_row(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette, e: &Entry) {
        let ignored = e.ignored;
        let name = exe_file_name(&e.identity.exe_path);
        let fill = if ignored {
            p.card.gamma_multiply(0.55)
        } else {
            p.card
        };
        let text_color = if ignored { p.weak } else { p.text };
        card(ui, p, fill, |ui| {
            ui.horizontal(|ui| {
                let icon_resp = match self.icons.get(ctx, &e.identity.exe_path) {
                    Some(tex) => {
                        let tint = if ignored {
                            Color32::WHITE.gamma_multiply(0.45)
                        } else {
                            Color32::WHITE
                        };
                        ui.add(egui::Image::new((tex.id(), vec2(36.0, 36.0))).tint(tint))
                    }
                    None => placeholder_icon(ui, p, ignored),
                };
                icon_resp.on_hover_text(&e.identity.exe_path);

                let left = (ui.available_width() - 190.0).max(120.0);
                ui.vertical(|ui| {
                    ui.set_max_width(left);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(semibold(name, 15.5).color(text_color)).truncate())
                            .on_hover_text(&e.identity.exe_path);
                        if ignored {
                            pill(ui, p, "Ignoriert");
                        }
                    });
                    let t = e.identity.title.trim();
                    let t = if t.is_empty() { "(ohne Titel)" } else { t };
                    ui.add(egui::Label::new(RichText::new(t).color(p.weak)).truncate());
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut on = ignored;
                    if toggle(ui, p, &mut on).changed() {
                        self.bridge
                            .send(UiCommand::SetIgnored(e.identity.clone(), on));
                    }
                    ui.label(RichText::new("Ignorieren").color(text_color));
                });
            });

            ui.label(
                RichText::new(format!(
                    "{}  ·  {}  ·  {}",
                    position_text(&e.placement),
                    monitor_name(&e.placement),
                    state_label(e.placement.state)
                ))
                .small()
                .color(p.weak),
            );

            ui.horizontal(|ui| {
                let r = ui.button("Jetzt wiederherstellen");
                if r.on_hover_text(
                    "Setzt nur dieses Fenster zurück (auch wenn es ignoriert wird). \
                     Das Programm wird nicht gestartet.",
                )
                .clicked()
                {
                    self.bridge
                        .send(UiCommand::RestoreEntry(e.identity.clone()));
                }
                if self.confirm_delete.as_ref() == Some(&e.identity) {
                    ui.label(RichText::new("Wirklich entfernen?").color(p.danger));
                    if ui
                        .add(
                            Button::new(RichText::new("Löschen").color(p.danger))
                                .stroke(Stroke::new(1.0, p.danger)),
                        )
                        .clicked()
                    {
                        self.confirm_delete = None;
                        self.bridge.send(UiCommand::DeleteEntry(e.identity.clone()));
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.confirm_delete = None;
                    }
                } else if ui
                    .button("Löschen")
                    .on_hover_text(
                        "Entfernt den Eintrag. Ist das Fenster geöffnet, wird es beim nächsten \
                         Speichern erneut gemerkt – zum dauerhaften Ausschließen „Ignorieren“ \
                         verwenden.",
                    )
                    .clicked()
                {
                    self.confirm_delete = Some(e.identity.clone());
                }
            });
        });
    }
}

fn pill(ui: &mut Ui, p: &Palette, text: &str) {
    Frame::new()
        .fill(p.control)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(p.weak));
        });
}

fn placeholder_icon(ui: &mut Ui, p: &Palette, muted: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
    let a = if muted { 0.5 } else { 1.0 };
    let painter = ui.painter();
    let body = rect.shrink2(vec2(4.0, 6.0));
    painter.rect(
        body,
        4,
        p.control.gamma_multiply(a),
        Stroke::new(1.0, p.weak.gamma_multiply(a)),
        StrokeKind::Inside,
    );
    let bar = Rect::from_min_size(body.min, vec2(body.width(), 6.0));
    painter.rect_filled(
        bar,
        CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        },
        p.weak.gamma_multiply(a),
    );
    resp
}

// ------------------------------------------------------------ settings ----

impl View {
    fn page_settings(&mut self, ui: &mut Ui, p: &Palette) {
        title(ui, "Einstellungen");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card(ui, p, p.card, |ui| {
                    setting_row(
                        ui,
                        p,
                        "Mit Windows starten",
                        "WindowKeeper startet automatisch bei der Anmeldung.",
                        |ui| {
                            let mut on = self.snap.autostart;
                            if toggle(ui, p, &mut on).changed() {
                                self.bridge.send(UiCommand::SetAutostart(on));
                            }
                        },
                    );
                    separator(ui);
                    setting_row(
                        ui,
                        p,
                        "Automatisch speichern",
                        "Fensterpositionen werden gespeichert, sobald sie sich ändern.",
                        |ui| {
                            toggle(ui, p, &mut self.edit.auto_save);
                        },
                    );
                    separator(ui);
                    setting_row(
                        ui,
                        p,
                        "Beim Start nach Updates suchen",
                        "Höchstens einmal pro Tag. Installiert wird nur nach Rückfrage.",
                        |ui| {
                            toggle(ui, p, &mut self.edit.check_updates_on_start);
                        },
                    );
                });
                ui.add_space(4.0);
                card(ui, p, p.card, |ui| {
                    setting_row(
                        ui,
                        p,
                        "Verzögerung vor dem Speichern",
                        "Wartezeit nach einer Änderung, bevor automatisch gespeichert wird.",
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.edit.debounce_ms)
                                    .range(DEBOUNCE_MS)
                                    .speed(50)
                                    .suffix(" ms"),
                            );
                        },
                    );
                    separator(ui);
                    setting_row(
                        ui,
                        p,
                        "Prüfintervall im Hintergrund",
                        "Zusätzlicher, vorsichtiger Scan als Sicherheitsnetz.",
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.edit.scan_interval_secs)
                                    .range(SCAN_INTERVAL_SECS)
                                    .speed(1)
                                    .suffix(" s"),
                            );
                        },
                    );
                    separator(ui);
                    setting_row(
                        ui,
                        p,
                        "Beobachtung später Fenster",
                        "So lange nach dem Start werden spät erscheinende Fenster platziert \
                         (0 = aus). Gilt ab dem nächsten Start.",
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.edit.restore_watch_minutes)
                                    .range(0..=WATCH_MINUTES_MAX)
                                    .speed(1)
                                    .suffix(" Min."),
                            );
                        },
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Änderungen werden sofort in data/config.toml gespeichert.")
                        .small()
                        .color(p.weak),
                );
            });
    }
}

// ---------------------------------------------------------------- info ----

impl View {
    fn page_info(&mut self, ui: &mut Ui, p: &Palette) {
        title(ui, "Info");
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card(ui, p, p.card, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::new((self.app_icon.id(), vec2(56.0, 56.0))));
                        ui.vertical(|ui| {
                            ui.label(semibold(APP_NAME, 22.0));
                            ui.label(RichText::new(format!("Version {APP_VERSION}")).color(p.weak));
                        });
                    });
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            "Portable Tray-App, die Position, Größe und Monitor Ihrer Fenster \
                             merkt und sie nach dem Start oder Login wieder dorthin setzt.",
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("GitHub").color(p.weak));
                        if ui.link(REPO_URL).clicked() {
                            super::open_url(REPO_URL);
                        }
                    });
                    ui.label(
                        RichText::new("Lizenz: MIT – © 2026 Strugglechen1337")
                            .small()
                            .color(p.weak),
                    );
                });
                ui.add_space(4.0);
                card(ui, p, p.card, |ui| {
                    ui.label(semibold("Updates", 17.0));
                    ui.add_space(2.0);
                    let (color, text) = update_look(p, &self.snap.update);
                    ui.horizontal(|ui| {
                        dot(ui, color);
                        ui.add(egui::Label::new(text).wrap());
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Zuletzt geprüft").color(p.weak));
                        ui.label(super::format_local(self.snap.last_update_check));
                    });
                    ui.add_space(6.0);
                    let checking = self.snap.update == UpdateStatus::Checking;
                    if primary_button(ui, p, "Nach Updates suchen", !checking).clicked() {
                        self.bridge.send(UiCommand::CheckUpdates);
                    }
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Updates kommen von GitHub, werden per SHA256 geprüft und nur nach \
                             Ihrer Bestätigung installiert.",
                        )
                        .small()
                        .color(p.weak),
                    );
                });
            });
    }
}

fn update_look(p: &Palette, s: &UpdateStatus) -> (Color32, String) {
    let color = match s {
        UpdateStatus::Unknown => p.weak,
        UpdateStatus::Checking => p.accent,
        UpdateStatus::UpToDate => p.ok,
        UpdateStatus::Available(_) => p.warn,
        UpdateStatus::Failed(_) => p.danger,
    };
    (color, s.label())
}

// --------------------------------------------------------------- toast ----

impl View {
    fn toast(&mut self, ctx: &egui::Context, p: &Palette) {
        let now = ctx.input(|i| i.time);
        let Some((text, until)) = self.notice.clone() else {
            return;
        };
        if now >= until {
            self.notice = None;
            return;
        }
        ctx.request_repaint_after_secs((until - now) as f32 + 0.05);
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, vec2(112.0, -24.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(p.card)
                    .stroke(Stroke::new(1.0, p.accent))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(18, 11))
                    .show(ui, |ui| {
                        ui.label(text);
                    });
            });
    }
}
