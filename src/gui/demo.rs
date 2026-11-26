//! Demo entry point for `examples/export_screenshots.rs` (feature
//! `screenshots`, non-Windows). It wraps the real [`View`] so that the
//! screenshots show exactly the GUI a Windows user sees. The demo view is
//! marked "Beispieldaten" so sample data is never mistaken for live data.

use super::view::View;
use crate::ui_model::Bridge;
use eframe::egui;
use std::sync::Arc;

/// Pages of the overview window.
pub use super::view::Page as DemoPage;

/// The overview GUI wired to a preloaded [`Bridge`] (no tray behind it).
pub struct DemoView {
    view: View,
}

impl DemoView {
    pub fn new(cc: &eframe::CreationContext<'_>, bridge: Arc<Bridge>) -> DemoView {
        let mut view = View::new(cc, bridge, |_| {});
        view.set_demo(true);
        DemoView { view }
    }

    /// Show another page.
    pub fn set_page(&mut self, page: DemoPage) {
        self.view.set_page(page);
    }
}

impl eframe::App for DemoView {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.view.ui(ui, frame);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.view.clear_color(visuals)
    }
}
