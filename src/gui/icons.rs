//! Application icons of the remembered windows.
//!
//! Icons are extracted with `SHGetFileInfoW` on a background thread and cached
//! per EXE path. While an icon is pending (or unavailable) the UI draws a
//! placeholder.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::mpsc::{channel, Receiver, Sender};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

struct Rgba {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

enum Slot {
    Pending,
    Ready(TextureHandle),
    Missing,
}

/// Per-window icon cache (lives as long as the GUI window).
pub struct Icons {
    tx: Sender<String>,
    rx: Receiver<(String, Option<Rgba>)>,
    slots: HashMap<String, Slot>,
}

impl Icons {
    pub fn new(ctx: &egui::Context) -> Icons {
        let (req_tx, req_rx) = channel::<String>();
        let (res_tx, res_rx) = channel::<(String, Option<Rgba>)>();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Shell icon extraction expects an initialized COM apartment.
            let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
            while let Ok(path) = req_rx.recv() {
                let icon = std::panic::catch_unwind(|| extract(&path)).ok().flatten();
                if res_tx.send((path, icon)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            if com {
                unsafe { CoUninitialize() };
            }
        });
        Icons {
            tx: req_tx,
            rx: res_rx,
            slots: HashMap::new(),
        }
    }

    /// Texture of the EXE's icon, if it is available (yet). Requests the
    /// extraction on first use.
    pub fn get(&mut self, ctx: &egui::Context, exe_path: &str) -> Option<&TextureHandle> {
        while let Ok((path, icon)) = self.rx.try_recv() {
            let slot = match icon {
                Some(i) => Slot::Ready(ctx.load_texture(
                    format!("exe-icon:{path}"),
                    ColorImage::from_rgba_unmultiplied([i.w, i.h], &i.px),
                    TextureOptions::LINEAR,
                )),
                None => Slot::Missing,
            };
            self.slots.insert(path, slot);
        }
        if !self.slots.contains_key(exe_path) {
            self.slots.insert(exe_path.to_string(), Slot::Pending);
            let _ = self.tx.send(exe_path.to_string());
        }
        match self.slots.get(exe_path) {
            Some(Slot::Ready(t)) => Some(t),
            _ => None,
        }
    }
}

fn extract(path: &str) -> Option<Rgba> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    let r = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if r == 0 || info.hIcon.is_invalid() {
        return None;
    }
    let img = icon_pixels(info.hIcon);
    unsafe {
        let _ = DestroyIcon(info.hIcon);
    }
    img
}

/// Read the pixels of a bitmap as top-down 32 bit BGRA.
unsafe fn bitmap_bgra(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    bmp: HGDIOBJ,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = GetDIBits(
        hdc,
        windows::Win32::Graphics::Gdi::HBITMAP(bmp.0),
        0,
        h as u32,
        Some(buf.as_mut_ptr() as *mut c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    (lines == h).then_some(buf)
}

fn icon_pixels(hicon: HICON) -> Option<Rgba> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(hicon, &mut ii).ok()?;
        let result = (|| {
            if ii.hbmColor.is_invalid() {
                return None; // monochrome icons are not worth the trouble
            }
            let mut bm = BITMAP::default();
            let got = GetObjectW(
                HGDIOBJ(ii.hbmColor.0),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut BITMAP as *mut c_void),
            );
            if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 || bm.bmWidth > 256 {
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let hdc = CreateCompatibleDC(None);
            if hdc.is_invalid() {
                return None;
            }
            let color = bitmap_bgra(hdc, HGDIOBJ(ii.hbmColor.0), w, h);
            let mask = if ii.hbmMask.is_invalid() {
                None
            } else {
                bitmap_bgra(hdc, HGDIOBJ(ii.hbmMask.0), w, h)
            };
            let _ = DeleteDC(hdc);
            let mut px = color?;
            let has_alpha = px.chunks(4).any(|p| p[3] != 0);
            for (i, p) in px.chunks_mut(4).enumerate() {
                p.swap(0, 2); // BGRA -> RGBA
                if !has_alpha {
                    // Classic icon: transparency comes from the AND mask.
                    let transparent = mask.as_ref().is_some_and(|m| m[i * 4] != 0);
                    p[3] = if transparent { 0 } else { 255 };
                }
            }
            Some(Rgba {
                w: w as usize,
                h: h as usize,
                px,
            })
        })();
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        result
    }
}
