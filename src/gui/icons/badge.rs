//! Linux stand-in for the shell icons: small generated badges per program
//! (recognizable colors and a simple pictogram). Only used for the
//! documentation screenshots; they are not the programs' real icons.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;

const SIZE: usize = 64;

pub struct Icons {
    textures: HashMap<String, TextureHandle>,
}

#[derive(Clone, Copy)]
enum Glyph {
    Disc,
    Chevrons,
    Folder,
    Envelope,
    Plain,
}

impl Icons {
    pub fn new(_ctx: &egui::Context) -> Icons {
        Icons {
            textures: HashMap::new(),
        }
    }

    pub fn get(&mut self, ctx: &egui::Context, exe_path: &str) -> Option<&TextureHandle> {
        let key = exe_name(exe_path);
        Some(self.textures.entry(key.clone()).or_insert_with(|| {
            ctx.load_texture(format!("badge:{key}"), render(&key), TextureOptions::LINEAR)
        }))
    }
}

fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

fn style(name: &str) -> ([f32; 3], [f32; 3], Glyph) {
    match name {
        "firefox.exe" => ([255.0, 149.0, 0.0], [160.0, 40.0, 190.0], Glyph::Disc),
        "code.exe" => ([38.0, 150.0, 235.0], [0.0, 90.0, 170.0], Glyph::Chevrons),
        "explorer.exe" => ([255.0, 210.0, 70.0], [240.0, 160.0, 20.0], Glyph::Folder),
        "outlook.exe" => ([40.0, 140.0, 230.0], [0.0, 80.0, 170.0], Glyph::Envelope),
        _ => ([120.0, 130.0, 150.0], [70.0, 80.0, 100.0], Glyph::Plain),
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn coverage(dist: f32) -> f32 {
    (0.5 - dist).clamp(0.0, 1.0)
}

/// Signed distance to a rounded box centered at the origin.
fn rbox(px: f32, py: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = px.abs() - hx + r;
    let qy = py.abs() - hy + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Distance to a line segment (a thick stroke has radius `w`).
fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (px - a.0 - t * dx).hypot(py - a.1 - t * dy)
}

fn render(name: &str) -> ColorImage {
    let (top, bottom, glyph) = style(name);
    let mut img = ColorImage::filled([SIZE, SIZE], egui::Color32::TRANSPARENT);
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let base = coverage(rbox(px, py, c - 2.0, c - 2.0, 14.0));
            if base <= 0.0 {
                continue;
            }
            let t = y as f32 / SIZE as f32;
            let mut rgb = [
                lerp(top[0], bottom[0], t),
                lerp(top[1], bottom[1], t),
                lerp(top[2], bottom[2], t),
            ];
            // (shape coverage, shape color)
            let (cov, col): (f32, [f32; 3]) = match glyph {
                Glyph::Disc => {
                    let d = px.hypot(py) - 17.0;
                    let ring = coverage(d.abs() - 4.5);
                    let core = coverage(px.hypot(py) - 8.0) * 0.9;
                    (ring.max(core), [255.0, 236.0, 200.0])
                }
                Glyph::Chevrons => {
                    let w = 3.6;
                    let l = coverage(seg(px, py, (-6.0, -14.0), (-20.0, 0.0)) - w)
                        .max(coverage(seg(px, py, (-20.0, 0.0), (-6.0, 14.0)) - w));
                    let r = coverage(seg(px, py, (6.0, -14.0), (20.0, 0.0)) - w)
                        .max(coverage(seg(px, py, (20.0, 0.0), (6.0, 14.0)) - w));
                    (l.max(r), [255.0, 255.0, 255.0])
                }
                Glyph::Folder => {
                    let body = coverage(rbox(px, py + 3.0, 19.0, 12.0, 3.0));
                    let tab = coverage(rbox(px + 9.0, py + 16.0, 8.0, 4.0, 2.0));
                    let front = coverage(rbox(px, py + 7.0, 19.0, 9.5, 3.0));
                    rgb = [
                        lerp(rgb[0], 255.0, body.max(tab) * 0.75),
                        lerp(rgb[1], 232.0, body.max(tab) * 0.75),
                        lerp(rgb[2], 150.0, body.max(tab) * 0.75),
                    ];
                    (front, [255.0, 190.0, 40.0])
                }
                Glyph::Envelope => {
                    let env = coverage(rbox(px, py, 19.0, 13.0, 3.0));
                    let flap = coverage(
                        seg(px, py, (-16.0, -9.0), (0.0, 3.0)).min(seg(
                            px,
                            py,
                            (16.0, -9.0),
                            (0.0, 3.0),
                        )) - 1.8,
                    );
                    rgb = [
                        lerp(rgb[0], 255.0, env),
                        lerp(rgb[1], 255.0, env),
                        lerp(rgb[2], 255.0, env),
                    ];
                    (flap, [30.0, 110.0, 200.0])
                }
                Glyph::Plain => (coverage(rbox(px, py, 15.0, 11.0, 3.0)) * 0.9, [255.0; 3]),
            };
            for i in 0..3 {
                rgb[i] = lerp(rgb[i], col[i], cov);
            }
            img.pixels[y * SIZE + x] = egui::Color32::from_rgba_unmultiplied(
                rgb[0] as u8,
                rgb[1] as u8,
                rgb[2] as u8,
                (base * 255.0) as u8,
            );
        }
    }
    img
}
