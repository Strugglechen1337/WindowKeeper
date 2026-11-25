//! WindowKeeper: remembers window placements and restores them.
//!
//! Pure logic lives in platform independent modules (tested on any OS);
//! Win32 specific code is gated behind `cfg(windows)`.

pub mod autostart;
pub mod config;
pub mod filter;
pub mod geometry;
pub mod icon;
pub mod layout;
pub mod logger;
pub mod matching;
pub mod paths;
pub mod state;
pub mod ui_model;
pub mod updater;
pub mod version;

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod gui;
#[cfg(windows)]
pub mod winapi;

/// Application name used for registry values, mutex names and messages.
pub const APP_NAME: &str = "WindowKeeper";
/// Current version from Cargo.toml.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
