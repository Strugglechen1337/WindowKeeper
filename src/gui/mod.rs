//! The overview window (egui/eframe, OpenGL).
//!
//! `theme` and `view` are platform independent egui code. They are built on
//! Windows (the real app) and, for the documentation screenshots, on other
//! systems with the `screenshots` feature. Everything that touches Win32
//! (opening the window from the tray, focus, ShellExecute, local time) lives
//! in `win` and is `cfg(windows)` only.

mod icons;
mod theme;
mod view;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::open_or_focus;
#[cfg(windows)]
pub(crate) use win::{format_local, open_url};

#[cfg(not(windows))]
mod other;
#[cfg(not(windows))]
pub(crate) use other::{format_local, open_url};

#[cfg(all(not(windows), feature = "screenshots"))]
pub mod demo;
