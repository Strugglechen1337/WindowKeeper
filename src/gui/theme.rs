//! Windows 11 look: palette, fonts and egui style.

use eframe::egui::{
    self, epaint::Shadow, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Stroke, TextStyle, Theme, Visuals,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Font family for headings / emphasis (Segoe UI Semibold when available).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Font family of the icon font (Segoe Fluent Icons / MDL2 Assets).
pub fn icons_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub nav_bg: Color32,
    pub bg: Color32,
    pub card: Color32,
    pub card_stroke: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub hover: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub danger: Color32,
    pub ok: Color32,
    pub warn: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            dark,
            nav_bg: rgb(0x1C, 0x1C, 0x1C),
            bg: rgb(0x20, 0x20, 0x20),
            card: rgb(0x2C, 0x2C, 0x2C),
            card_stroke: rgb(0x3A, 0x3A, 0x3A),
            text: rgb(0xF3, 0xF3, 0xF3),
            weak: rgb(0xA6, 0xA6, 0xA6),
            accent: rgb(0x60, 0xCD, 0xFF),
            on_accent: rgb(0x00, 0x00, 0x00),
            hover: rgb(0x32, 0x32, 0x32),
            control: rgb(0x38, 0x38, 0x38),
            control_hover: rgb(0x40, 0x40, 0x40),
            danger: rgb(0xFF, 0x99, 0xA4),
            ok: rgb(0x6C, 0xCB, 0x5F),
            warn: rgb(0xFC, 0xE1, 0x00),
        }
    } else {
        Palette {
            dark,
            nav_bg: rgb(0xEE, 0xEE, 0xEE),
            bg: rgb(0xF3, 0xF3, 0xF3),
            card: rgb(0xFF, 0xFF, 0xFF),
            card_stroke: rgb(0xE5, 0xE5, 0xE5),
            text: rgb(0x1A, 0x1A, 0x1A),
            weak: rgb(0x60, 0x60, 0x60),
            accent: rgb(0x00, 0x5F, 0xB8),
            on_accent: rgb(0xFF, 0xFF, 0xFF),
            hover: rgb(0xE6, 0xE6, 0xE6),
            control: rgb(0xFB, 0xFB, 0xFB),
            control_hover: rgb(0xF3, 0xF3, 0xF3),
            danger: rgb(0xC4, 0x2B, 0x1C),
            ok: rgb(0x0F, 0x7B, 0x0F),
            warn: rgb(0x9D, 0x5D, 0x00),
        }
    }
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let r4 = CornerRadius::same(4);
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.card_stroke);
    v.window_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.menu_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = p.card;
    v.faint_bg_color = p.hover;
    v.text_edit_bg_color = Some(p.control);
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.danger;
    v.warn_fg_color = p.warn;
    v.selection.bg_fill = p.accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.card_stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r4;
    for (wv, fill) in [
        (&mut w.inactive, p.control),
        (&mut w.hovered, p.control_hover),
        (&mut w.active, p.control_hover),
        (&mut w.open, p.control_hover),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0, p.card_stroke);
        wv.fg_stroke = Stroke::new(1.0, p.text);
        wv.corner_radius = r4;
        wv.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, p.weak.gamma_multiply(0.5));
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    v
}

/// Install visuals for both themes; egui then follows the OS setting.
pub fn apply(ctx: &egui::Context) {
    for (theme, dark) in [(Theme::Dark, true), (Theme::Light, false)] {
        let p = palette(dark);
        ctx.set_visuals_of(theme, visuals(&p));
        ctx.style_mut_of(theme, |s| {
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.button_padding = egui::vec2(14.0, 6.0);
            s.spacing.interact_size.y = 32.0;
            s.spacing.icon_width = 20.0;
            s.spacing.scroll.bar_width = 8.0;
            s.spacing.scroll.floating = true;
            s.text_styles = text_styles();
        });
    }
}

fn text_styles() -> BTreeMap<TextStyle, FontId> {
    use FontFamily::{Monospace, Proportional};
    BTreeMap::from([
        (TextStyle::Heading, FontId::new(28.0, semibold())),
        (TextStyle::Body, FontId::new(14.5, Proportional)),
        (TextStyle::Button, FontId::new(14.5, Proportional)),
        (TextStyle::Small, FontId::new(12.5, Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, Monospace)),
    ])
}

fn fonts_dir() -> PathBuf {
    let win = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(win).join("Fonts")
}

fn read_font(file: &str) -> Option<Vec<u8>> {
    std::fs::read(fonts_dir().join(file)).ok()
}

/// Load Segoe UI (+ Semibold, Symbol and the icon font) from the system.
/// Returns true if an icon font is available.
pub fn install_fonts(ctx: &egui::Context) -> bool {
    let mut defs = FontDefinitions::default();
    let add = |defs: &mut FontDefinitions, key: &str, file: &str| -> bool {
        match read_font(file) {
            Some(bytes) => {
                defs.font_data
                    .insert(key.into(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            None => false,
        }
    };

    let regular = add(&mut defs, "segoe", "segoeui.ttf");
    let semi = add(&mut defs, "segoe-semibold", "seguisb.ttf");
    let symbols = add(&mut defs, "segoe-symbol", "seguisym.ttf");
    let icons = add(&mut defs, "segoe-icons", "SegoeIcons.ttf")
        || add(&mut defs, "segoe-icons", "segmdl2.ttf");

    let proportional = defs.families.entry(FontFamily::Proportional).or_default();
    if symbols {
        proportional.insert(0, "segoe-symbol".into());
    }
    if regular {
        proportional.insert(0, "segoe".into());
    }
    let base = proportional.clone();

    let mut semibold_chain = base.clone();
    if semi {
        semibold_chain.insert(0, "segoe-semibold".into());
    }
    defs.families.insert(semibold(), semibold_chain);
    defs.families.insert(
        icons_family(),
        if icons {
            vec!["segoe-icons".into()]
        } else {
            base
        },
    );
    ctx.set_fonts(defs);
    icons
}
