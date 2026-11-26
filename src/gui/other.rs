//! Non-Windows stand-ins, only used for the documentation screenshots.

use crate::ui_model::format_datetime_de;

/// Fixed offset used for the demo: Europe/Berlin in winter (UTC+1).
const DEMO_UTC_OFFSET_SECS: i64 = 3600;

/// The demo never opens a browser.
pub(crate) fn open_url(_url: &str) {}

/// German date/time with a fixed UTC+1 offset (demo only).
pub(crate) fn format_local(unix: u64) -> String {
    format_datetime_de(unix, DEMO_UTC_OFFSET_SECS)
}
