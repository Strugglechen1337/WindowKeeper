//! Portable path handling: everything lives next to the executable.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// `data/` folder next to the given executable.
pub fn data_dir_for(exe: &Path) -> PathBuf {
    exe.parent().unwrap_or_else(|| Path::new(".")).join("data")
}

/// Resolve and create the data folder. If the EXE folder is not writable the
/// system temp folder is used as a last resort (never %APPDATA%). The bool is
/// true when the fallback was used.
pub fn ensure_data_dir(exe: &Path) -> (PathBuf, bool) {
    let primary = data_dir_for(exe);
    if dir_writable(&primary) {
        return (primary, false);
    }
    let fallback = std::env::temp_dir().join("WindowKeeper-data");
    let _ = std::fs::create_dir_all(&fallback);
    (fallback, true)
}

fn dir_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-test");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Write a file atomically (temp file + rename).
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    if std::fs::rename(&tmp, path).is_err() {
        // Windows: rename onto an existing file can fail transiently.
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_next_to_exe() {
        assert_eq!(
            data_dir_for(Path::new("/opt/x/WindowKeeper.exe")),
            Path::new("/opt/x/data")
        );
    }

    #[test]
    fn ensure_creates_dir_and_atomic_write_works() {
        let base = std::env::temp_dir().join(format!("wk-paths-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let (d, fb) = ensure_data_dir(&base.join("WindowKeeper.exe"));
        assert!(!fb && d.is_dir());
        let f = d.join("a.json");
        write_atomic(&f, b"1").unwrap();
        write_atomic(&f, b"22").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"22");
        std::fs::remove_dir_all(&base).ok();
    }
}
