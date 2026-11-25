//! Tiny size-capped file logger (`data/windowkeeper.log`).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 512 * 1024;

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

/// Format unix seconds as `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days algorithm (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Open the log file, rotating it to `.1` when it grew too large.
pub fn init(path: &Path) {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    if let Ok(f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = LOG.set(Mutex::new(f));
    }
}

pub fn write(level: &str, msg: &str) {
    if let Some(m) = LOG.get() {
        if let Ok(mut f) = m.lock() {
            let _ = writeln!(f, "{} UTC [{level}] {msg}", format_utc(now_secs()));
        }
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logger::write("INFO", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logger::write("WARN", &format!($($arg)*)) };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logger::write("ERROR", &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(1_709_210_096), "2024-02-29 12:34:56");
    }
}
