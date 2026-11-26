//! Self-updater based on GitHub Releases with mandatory SHA256 verification.

use crate::version::{is_newer, parse_tag};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "Strugglechen1337/WindowKeeper";
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
const DAY_SECS: u64 = 86_400;

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// An available update with everything needed to install it.
#[derive(Debug, Clone)]
pub struct Update {
    pub version: String,
    pub exe: Asset,
    pub sha: Asset,
}

/// True if an automatic startup check is due (at most once per day).
pub fn check_due(last_check: u64, now: u64) -> bool {
    last_check > now || now - last_check >= DAY_SECS
}

/// Only download from GitHub over HTTPS.
pub fn is_trusted_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.to_lowercase();
    host == "github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com")
}

/// Asset base name for the architecture this binary was built for. x64 keeps
/// the historical name `WindowKeeper.exe` so existing installs keep updating.
pub fn asset_stem() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "windowkeeper-arm64"
    } else {
        "windowkeeper"
    }
}

/// Pick the EXE asset and the checksum asset of a release for this build's
/// architecture.
pub fn select_assets(rel: &Release) -> (Option<Asset>, Option<Asset>) {
    select_assets_for(rel, asset_stem())
}

/// Like [`select_assets`], for an explicit asset stem (`windowkeeper` or
/// `windowkeeper-arm64`). Never falls back to another architecture's file.
pub fn select_assets_for(rel: &Release, stem: &str) -> (Option<Asset>, Option<Asset>) {
    let find = |pred: &dyn Fn(&str) -> bool| {
        rel.assets
            .iter()
            .find(|a| pred(&a.name.to_lowercase()))
            .cloned()
    };
    let exe_name = format!("{stem}.exe");
    let sha_name = format!("{stem}.exe.sha256");
    let is_arm = |n: &str| n.contains("arm64");
    let want_arm = stem.contains("arm64");
    // Fallback: any file of the right architecture (arm64 builds carry "arm64"
    // in the name, x64 builds do not).
    let right_arch = |n: &str| is_arm(n) == want_arm;
    let exe = find(&|n| n == exe_name).or_else(|| find(&|n| n.ends_with(".exe") && right_arch(n)));
    let sha = find(&|n| n == sha_name)
        .or_else(|| find(&|n| n == "sha256sums" || n == "sha256sums.txt"))
        .or_else(|| find(&|n| n.ends_with(".sha256") && right_arch(n)));
    (exe, sha)
}

/// Extract the SHA256 (lowercase hex) for `file_name` from a checksum file.
/// Accepts `<hash>`, `<hash>  name` and `<hash> *name` lines.
pub fn parse_sha256(text: &str, file_name: &str) -> Option<String> {
    let valid = |h: &str| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit());
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for l in &lines {
        let mut it = l.split_whitespace();
        let (Some(h), name) = (it.next(), it.next()) else {
            continue;
        };
        if !valid(h) {
            continue;
        }
        match name {
            Some(n) if n.trim_start_matches('*').eq_ignore_ascii_case(file_name) => {
                return Some(h.to_lowercase())
            }
            None if lines.len() == 1 => return Some(h.to_lowercase()),
            _ => {}
        }
    }
    None
}

/// Decide whether the release is an update for `current` and has all assets.
pub fn evaluate(rel: &Release, current: &str) -> Result<Option<Update>, String> {
    evaluate_for(rel, current, asset_stem())
}

/// [`evaluate`] for an explicit asset stem.
pub fn evaluate_for(rel: &Release, current: &str, stem: &str) -> Result<Option<Update>, String> {
    if rel.draft || rel.prerelease || parse_tag(&rel.tag_name).is_none() {
        return Ok(None);
    }
    if !is_newer(current, &rel.tag_name) {
        return Ok(None);
    }
    let (exe, sha) = select_assets_for(rel, stem);
    let exe = exe.ok_or("Das Release enthält keine EXE-Datei.")?;
    let sha = sha.ok_or("Das Release enthält keine SHA256-Prüfsumme – Update abgebrochen.")?;
    Ok(Some(Update {
        version: rel.tag_name.trim_start_matches(['v', 'V']).to_string(),
        exe,
        sha,
    }))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(&format!("WindowKeeper/{}", crate::APP_VERSION))
        .build()
}

/// Query GitHub for the latest release and compare it with `current`.
pub fn check(current: &str) -> Result<Option<Update>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = agent()
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Release-Abfrage fehlgeschlagen: {e}"))?;
    let rel: Release = resp
        .into_json()
        .map_err(|e| format!("Ungültige Antwort von GitHub: {e}"))?;
    evaluate(&rel, current)
}

fn fetch_text(url: &str) -> Result<String, String> {
    if !is_trusted_url(url) {
        return Err("Nicht vertrauenswürdige Download-URL.".into());
    }
    let mut s = String::new();
    agent()
        .get(url)
        .call()
        .map_err(|e| format!("Download fehlgeschlagen: {e}"))?
        .into_reader()
        .take(64 * 1024)
        .read_to_string(&mut s)
        .map_err(|e| format!("Lesefehler: {e}"))?;
    Ok(s)
}

/// Download the EXE to `dest` and verify its SHA256. Fails closed: on any
/// problem `dest` is removed.
pub fn download_verified(update: &Update, dest: &Path) -> Result<(), String> {
    let expected = parse_sha256(
        &fetch_text(&update.sha.browser_download_url)?,
        &update.exe.name,
    )
    .ok_or("Prüfsumme nicht lesbar – Update abgebrochen.")?;
    if !is_trusted_url(&update.exe.browser_download_url) {
        return Err("Nicht vertrauenswürdige Download-URL.".into());
    }
    let result = (|| {
        let resp = agent()
            .get(&update.exe.browser_download_url)
            .call()
            .map_err(|e| format!("Download fehlgeschlagen: {e}"))?;
        let mut reader = resp.into_reader().take(MAX_DOWNLOAD);
        let mut file = std::fs::File::create(dest).map_err(|e| format!("Schreibfehler: {e}"))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("Lesefehler: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            std::io::Write::write_all(&mut file, &buf[..n])
                .map_err(|e| format!("Schreibfehler: {e}"))?;
            total += n as u64;
        }
        drop(file);
        if total == 0 {
            return Err("Leere Datei heruntergeladen.".to_string());
        }
        let actual = hex(&hasher.finalize());
        if actual != expected {
            return Err("SHA256-Prüfsumme stimmt nicht überein – Update abgebrochen.".to_string());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA256 of a file as lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Path used for the downloaded, not yet installed EXE.
pub fn new_path(exe: &Path) -> PathBuf {
    sibling_with_suffix(exe, ".new")
}

/// Path the running EXE is renamed to during the swap.
pub fn old_path(exe: &Path) -> PathBuf {
    sibling_with_suffix(exe, ".old")
}

fn sibling_with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Swap the (possibly running) `current` EXE for `new_file`.
///
/// Windows allows renaming a running EXE, so the running file is moved to
/// `*.old` and the new one takes its place. On failure the old state is
/// restored.
pub fn swap_in(current: &Path, new_file: &Path) -> Result<PathBuf, String> {
    let old = old_path(current);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(current, &old)
        .map_err(|e| format!("Aktuelle EXE konnte nicht verschoben werden: {e}"))?;
    if let Err(e) = std::fs::rename(new_file, current) {
        let _ = std::fs::rename(&old, current);
        return Err(format!("Neue EXE konnte nicht installiert werden: {e}"));
    }
    Ok(old)
}

/// Remove a leftover `*.old` file from a previous update. Returns true if
/// nothing is left over.
pub fn cleanup_old(current: &Path) -> bool {
    let old = old_path(current);
    !old.exists() || std::fs::remove_file(&old).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(n: &str) -> Asset {
        Asset {
            name: n.into(),
            browser_download_url: format!("https://github.com/x/y/releases/download/v1/{n}"),
        }
    }

    fn release(tag: &str, names: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            assets: names.iter().map(|n| asset(n)).collect(),
        }
    }

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn daily_check_logic() {
        assert!(check_due(0, 100_000));
        assert!(!check_due(100_000, 100_000 + 3600));
        assert!(check_due(100_000, 100_000 + 86_400));
        assert!(check_due(500, 100)); // clock went backwards
    }

    #[test]
    fn url_trust() {
        assert!(is_trusted_url(
            "https://github.com/a/b/releases/download/v1/x.exe"
        ));
        assert!(is_trusted_url("https://objects.githubusercontent.com/x"));
        assert!(!is_trusted_url("http://github.com/x"));
        assert!(!is_trusted_url("https://github.com.evil.com/x"));
        assert!(!is_trusted_url("https://evilgithub.com/x"));
    }

    #[test]
    fn sha_parsing() {
        assert_eq!(parse_sha256(H, "WindowKeeper.exe").as_deref(), Some(H));
        let upper = format!("{}  WindowKeeper.exe\n", H.to_uppercase());
        assert_eq!(parse_sha256(&upper, "windowkeeper.exe").as_deref(), Some(H));
        let multi = format!("{H} *other.zip\n{} *WindowKeeper.exe\n", "a".repeat(64));
        assert_eq!(
            parse_sha256(&multi, "WindowKeeper.exe"),
            Some("a".repeat(64))
        );
        assert_eq!(parse_sha256(&multi, "missing.exe"), None);
        assert_eq!(parse_sha256("not a hash", "x"), None);
        assert_eq!(parse_sha256(&format!("{H}\n{H}\n"), "x"), None);
    }

    #[test]
    fn selects_asset_per_architecture() {
        let r = release(
            "v0.2.0",
            &[
                "WindowKeeper-arm64.exe",
                "WindowKeeper-arm64.exe.sha256",
                "WindowKeeper.exe",
                "WindowKeeper.exe.sha256",
            ],
        );
        let (e, s) = select_assets_for(&r, "windowkeeper");
        assert_eq!(e.unwrap().name, "WindowKeeper.exe");
        assert_eq!(s.unwrap().name, "WindowKeeper.exe.sha256");
        let (e, s) = select_assets_for(&r, "windowkeeper-arm64");
        assert_eq!(e.unwrap().name, "WindowKeeper-arm64.exe");
        assert_eq!(s.unwrap().name, "WindowKeeper-arm64.exe.sha256");
        // Old release without arm64 files: arm64 gets nothing, never the x64 EXE.
        let old = release("v0.1.1", &["WindowKeeper.exe", "WindowKeeper.exe.sha256"]);
        assert!(select_assets_for(&old, "windowkeeper-arm64").0.is_none());
        // x64 never falls back to the arm64 file.
        let arm_only = release("v0.2.0", &["WindowKeeper-arm64.exe"]);
        assert!(select_assets_for(&arm_only, "windowkeeper").0.is_none());
    }

    #[test]
    fn evaluate_requires_hash_asset() {
        let r = release("v0.2.0", &["WindowKeeper.exe"]);
        assert!(evaluate_for(&r, "0.1.0", "windowkeeper").is_err());
        let r = release("v0.2.0", &["WindowKeeper.exe", "WindowKeeper.exe.sha256"]);
        let u = evaluate_for(&r, "0.1.0", "windowkeeper").unwrap().unwrap();
        assert_eq!(u.version, "0.2.0");
        assert_eq!(u.sha.name, "WindowKeeper.exe.sha256");
        assert!(evaluate_for(&r, "0.2.0", "windowkeeper").unwrap().is_none());
        let mut d = r.clone();
        d.prerelease = true;
        assert!(evaluate_for(&d, "0.1.0", "windowkeeper").unwrap().is_none());
        let sums = release("0.3.0", &["WindowKeeper.exe", "SHA256SUMS"]);
        assert!(evaluate_for(&sums, "0.1.0", "windowkeeper")
            .unwrap()
            .is_some());
    }

    #[test]
    fn swap_and_cleanup() {
        let dir = std::env::temp_dir().join(format!("wk-upd-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("WindowKeeper.exe");
        std::fs::write(&exe, b"old").unwrap();
        let new = new_path(&exe);
        std::fs::write(&new, b"new").unwrap();
        let old = swap_in(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(&old).unwrap(), b"old");
        assert!(cleanup_old(&exe));
        assert!(!old.exists());
        // Failure restores the original.
        assert!(swap_in(&exe, &dir.join("nope")).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(
            sha256_file(&exe).unwrap(),
            "11507a0e2f5e69d5dfa40a62a1bd7b6ee57e6bcd85c67c9b8431b36fff21c437"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
