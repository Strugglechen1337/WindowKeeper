//! Autostart via HKCU\...\Run (the only registry use of this app).

use std::path::Path;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "WindowKeeper";

/// Registry command line for the given EXE (always quoted).
pub fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// True if the stored command points to `exe` (quotes/case/slashes ignored).
pub fn points_to(stored: &str, exe: &Path) -> bool {
    fn norm(s: &str) -> String {
        s.trim().trim_matches('"').replace('/', "\\").to_lowercase()
    }
    norm(stored) == norm(&exe.display().to_string())
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
        let mut key = HKEY::default();
        let key_path = wide(RUN_KEY);
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path.as_ptr()),
                0,
                access,
                &mut key,
            )
        };
        (rc == ERROR_SUCCESS).then_some(key)
    }

    /// Currently stored autostart command, if any.
    pub fn read() -> Option<String> {
        let key = open(KEY_READ)?;
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        let rc = unsafe {
            RegQueryValueExW(
                key,
                w!("WindowKeeper"),
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            )
        };
        unsafe {
            let _ = RegCloseKey(key);
        }
        if rc != ERROR_SUCCESS {
            return None;
        }
        let len = (size as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        Some(s.trim_end_matches('\0').to_string())
    }

    pub fn is_enabled() -> bool {
        read().is_some()
    }

    /// Enable (write current EXE path) or disable (remove value) autostart.
    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        let key = open(KEY_WRITE).ok_or("Registry-Schlüssel konnte nicht geöffnet werden")?;
        let rc = if enabled {
            let data: Vec<u16> = wide(&command_for(exe));
            let bytes =
                unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
            unsafe { RegSetValueExW(key, w!("WindowKeeper"), 0, REG_SZ, Some(bytes)) }
        } else {
            let r = unsafe { RegDeleteValueW(key, w!("WindowKeeper")) };
            // A missing value is fine when disabling.
            if r.0 == 2 {
                ERROR_SUCCESS
            } else {
                r
            }
        };
        unsafe {
            let _ = RegCloseKey(key);
        }
        if rc == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Registry-Fehler {}", rc.0))
        }
    }

    /// If autostart is on but points elsewhere (EXE moved), rewrite it.
    pub fn refresh_path(exe: &Path) -> bool {
        match read() {
            Some(stored) if !points_to(&stored, exe) => set(true, exe).is_ok(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_comparison() {
        let p = Path::new(r"C:\Program Files\WK\WindowKeeper.exe");
        assert_eq!(command_for(p), r#""C:\Program Files\WK\WindowKeeper.exe""#);
        assert!(points_to(&command_for(p), p));
        assert!(points_to(r"c:\program files\wk\windowkeeper.exe", p));
        assert!(!points_to(r#""D:\WindowKeeper.exe""#, p));
    }
}
