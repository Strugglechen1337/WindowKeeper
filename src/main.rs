#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    windowkeeper::app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("WindowKeeper läuft nur unter Windows.");
    std::process::exit(1);
}
