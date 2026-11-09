//! Self-update: check the GitHub release feed, download through a mirror when
//! GitHub is slow, and replace the running binary (rename-then-copy on Windows).
//!
//! HTTP is done with the system `curl` so the tool keeps its zero-dependency
//! footprint; every download falls back through the configured mirrors.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "neko233-com/codegraph233";

fn api_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

fn default_true() -> bool {
    true
}
fn default_interval() -> u64 {
    24
}
fn default_mirrors() -> Vec<String> {
    vec![
        "https://ghfast.top/".into(),
        "https://gh-proxy.com/".into(),
        "https://ghproxy.net/".into(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub auto_update: bool,
    #[serde(default = "default_interval")]
    pub interval_hours: u64,
    /// Explicit mirror prefix; when set it is tried before `mirrors`.
    #[serde(default)]
    pub mirror: String,
    #[serde(default = "default_mirrors")]
    pub mirrors: Vec<String>,
    #[serde(default)]
    pub last_check: i64,
    #[serde(default)]
    pub last_version: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_update: true,
            interval_hours: default_interval(),
            mirror: String::new(),
            mirrors: default_mirrors(),
            last_check: 0,
            last_version: String::new(),
        }
    }
}

pub fn settings_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".codegraph233").join("settings.json")
}

pub fn load_settings() -> Settings {
    let path = settings_path();
    let mut s = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
        .unwrap_or_default();
    if let Ok(m) = std::env::var("CODEGRAPH233_MIRROR") {
        if !m.is_empty() {
            s.mirror = m;
        }
    }
    if let Ok(v) = std::env::var("CODEGRAPH233_AUTO_UPDATE") {
        s.auto_update = !matches!(v.as_str(), "0" | "false" | "no" | "off");
    }
    s
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(&path, format!("{text}\n")).map_err(|e| e.to_string())
}

/// Apply CLI update preferences in memory; validation lives here so both the
/// command and tests exercise the same rules.
fn apply(s: &mut Settings, enable: Option<bool>, interval: Option<u64>, mirror: Option<String>) -> Result<(), String> {
    if let Some(hours) = interval {
        if hours == 0 || hours > 24 * 365 {
            return Err("interval must be between 1 and 8760 hours".into());
        }
        s.interval_hours = hours;
    }
    if let Some(on) = enable {
        s.auto_update = on;
    }
    if let Some(prefix) = mirror {
        s.mirror = prefix;
    }
    Ok(())
}

/// `codegraph233 update --enable|--disable [--interval HOURS] [--mirror URL]`
/// persists the choice used by the background auto-updater.
pub fn apply_options(
    enable: Option<bool>,
    interval: Option<u64>,
    mirror: Option<String>,
) -> Result<Settings, String> {
    let mut s = load_settings();
    apply(&mut s, enable, interval, mirror)?;
    save_settings(&s)?;
    Ok(s)
}

pub fn settings_summary(s: &Settings) -> String {
    let mirror = if s.mirror.is_empty() {
        "auto".to_string()
    } else {
        s.mirror.clone()
    };
    format!(
        "auto_update={} interval_hours={} mirror={mirror}",
        s.auto_update, s.interval_hours
    )
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn version_key(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    let a = version_key(latest);
    let b = version_key(current);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-sS", "-L", "--fail", "--max-time", "90", "-H", "User-Agent: codegraph233"])
        .args(args)
        .output()
        .map_err(|e| format!("curl not available ({e}); install curl or pass --offline"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// Mirrors are prefixes: `https://ghfast.top/` + the original URL.
fn mirrored(url: &str, s: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    if !s.mirror.is_empty() && s.mirror != "none" {
        out.push(format!("{}{}", s.mirror, url));
    }
    for m in &s.mirrors {
        if m.is_empty() {
            continue;
        }
        out.push(format!("{m}{url}"));
    }
    out.push(url.to_string());
    let mut seen = std::collections::HashSet::new();
    out.retain(|u| seen.insert(u.clone()));
    out
}

fn fetch_json(url: &str, s: &Settings) -> Result<Value, String> {
    let mut last = String::new();
    for candidate in mirrored(url, s) {
        match curl(&[&candidate]) {
            Ok(body) => match serde_json::from_slice::<Value>(&body) {
                Ok(v) => return Ok(v),
                Err(e) => last = format!("{candidate}: bad json: {e}"),
            },
            Err(e) => last = format!("{candidate}: {e}"),
        }
    }
    Err(format!("cannot fetch release feed ({last})"))
}

pub struct Release {
    pub version: String,
    pub asset: Option<(String, String, u64)>,
}

fn pick_asset(assets: &Value) -> Option<(String, String, u64)> {
    let list = assets.as_array()?;
    let os = std::env::consts::OS;
    let want: &[&str] = match os {
        "windows" => &["windows"],
        "macos" => &["macos", "darwin", "apple"],
        _ => &["linux"],
    };
    for a in list {
        let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();
        if !want.iter().any(|w| lower.contains(w)) {
            continue;
        }
        if !(lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")) {
            continue;
        }
        let url = a
            .get("browser_download_url")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let size = a.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
        return Some((name.to_string(), url.to_string(), size));
    }
    None
}

pub fn latest_release(s: &Settings) -> Result<Release, String> {
    let v = fetch_json(&api_url(), s)?;
    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "release feed has no tag_name".to_string())?;
    let asset = pick_asset(v.get("assets").unwrap_or(&Value::Null));
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        asset,
    })
}

fn download(url: &str, dest: &Path, s: &Settings) -> Result<u64, String> {
    let mut last = String::new();
    for candidate in mirrored(url, s) {
        if dest.exists() {
            let _ = fs::remove_file(dest);
        }
        match curl(&["-o", &dest.to_string_lossy(), &candidate]) {
            Ok(_) => {
                let size = fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                if size > 0 {
                    if candidate != url {
                        eprintln!("[codegraph233] downloaded via mirror {candidate}");
                    }
                    return Ok(size);
                }
                last = format!("{candidate}: empty file");
            }
            Err(e) => last = format!("{candidate}: {e}"),
        }
    }
    Err(format!("download failed ({last})"))
}

fn extract_binary(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = archive.file_name().and_then(|x| x.to_str()).unwrap_or("");
    if name.to_ascii_lowercase().ends_with(".zip") {
        let file = fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let ename = entry.name().to_string();
            let base = Path::new(&ename)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if !base.eq_ignore_ascii_case("codegraph233") && !base.eq_ignore_ascii_case("codegraph233.exe") {
                continue;
            }
            let out = dir.join("codegraph233.new.exe");
            let mut f = fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
            return Ok(out);
        }
        return Err("zip has no codegraph233 binary".into());
    }
    // .tar.gz / .tgz: use the system tar.
    let status = Command::new("tar")
        .args(["-xzf", &archive.to_string_lossy(), "-C", &dir.to_string_lossy()])
        .status()
        .map_err(|e| format!("tar not available: {e}"))?;
    if !status.success() {
        return Err("tar extraction failed".into());
    }
    for candidate in ["codegraph233", "codegraph233.exe"] {
        let p = dir.join(candidate);
        if p.is_file() {
            return Ok(p);
        }
    }
    // cargo-dist style layout: binary inside a subdirectory.
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if entry.path().is_dir() {
            for candidate in ["codegraph233", "codegraph233.exe"] {
                let p = entry.path().join(candidate);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    Err("archive has no codegraph233 binary".into())
}

fn install_binary(new_exe: &Path) -> Result<String, String> {
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let cur = if cur.is_symlink() {
        fs::canonicalize(&cur).unwrap_or(cur)
    } else {
        cur
    };
    let stamp = crate::store::now_unix();
    let old = cur.with_file_name(format!(
        "{}.old-{stamp}",
        cur.file_name().and_then(|s| s.to_str()).unwrap_or("codegraph233")
    ));
    // Windows refuses to overwrite a running image but allows renaming it.
    fs::rename(&cur, &old).map_err(|e| {
        format!(
            "cannot replace {} ({e}); close running MCP clients and retry",
            cur.display()
        )
    })?;
    if let Err(e) = fs::copy(new_exe, &cur) {
        let _ = fs::rename(&old, &cur);
        return Err(format!("copy failed: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&cur, fs::Permissions::from_mode(0o755));
    }
    let _ = fs::remove_file(&old);
    Ok(cur.display().to_string())
}

/// Remove leftovers from previous self-updates and renamed old builds.
pub fn cleanup_old_binaries() {
    let Ok(cur) = std::env::current_exe() else { return };
    let Some(dir) = cur.parent() else { return };
    let Ok(entries) = fs::read_dir(dir) else { return };
    let cur_name = cur.file_name().and_then(|s| s.to_str()).unwrap_or("");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == cur_name {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let stale = lower.contains(".old-")
            || (lower.starts_with("codegraph233-")
                && lower.ends_with(".exe")
                && lower.chars().any(|c| c.is_ascii_digit()));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

pub fn check_and_update(s: &Settings, force: bool, quiet: bool) -> Result<Option<String>, String> {
    let rel = latest_release(s)?;
    let cur = current_version();
    if !force && !is_newer(&rel.version, cur) {
        if !quiet {
            println!("codegraph233 {cur} is up to date (latest {})", rel.version);
        }
        return Ok(None);
    }
    let (asset, url, size) = rel.asset.clone().ok_or_else(|| {
        format!(
            "release v{} has no downloadable asset for {}; build from source",
            rel.version,
            std::env::consts::OS
        )
    })?;
    if !quiet {
        println!(
            "updating {cur} -> {} ({asset}, {:.1} MB)",
            rel.version,
            size as f64 / 1_048_576.0
        );
    }
    let dir = std::env::temp_dir().join(format!("codegraph233-update-{}", crate::store::now_unix()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join(&asset);
    let got = download(&url, &archive, s)?;
    if size > 0 && got != size {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("size mismatch: expected {size}, got {got}"));
    }
    let new_exe = extract_binary(&archive, &dir)?;
    let installed = install_binary(&new_exe)?;
    let _ = fs::remove_dir_all(&dir);
    if !quiet {
        println!("installed {} at {installed}", rel.version);
        println!("restart MCP clients / the editor to use the new build");
    }
    Ok(Some(rel.version))
}

/// Background check used by `index` and `serve`; never fails the caller.
pub fn auto() {
    if std::env::var("CODEGRAPH233_NO_UPDATE").is_ok() {
        return;
    }
    let mut s = load_settings();
    if !s.auto_update {
        return;
    }
    let now = crate::store::now_unix();
    let interval = s.interval_hours.max(1) as i64 * 3600;
    if s.last_check > 0 && now - s.last_check < interval {
        return;
    }
    s.last_check = now;
    match check_and_update(&s, false, true) {
        Ok(Some(v)) => {
            s.last_version = v.clone();
            eprintln!("[codegraph233] updated to {v}; restart MCP clients to apply");
        }
        Ok(None) => {}
        Err(e) => eprintln!("[codegraph233] update check failed: {e}"),
    }
    let _ = save_settings(&s);
}

pub fn print_status() -> Result<(), String> {
    let s = load_settings();
    println!("current:  {}", current_version());
    println!("settings: {}", settings_path().display());
    println!(
        "auto_update={} interval_hours={} mirror={:?}",
        s.auto_update,
        s.interval_hours,
        if s.mirror.is_empty() {
            "(auto: ghfast.top > gh-proxy.com > ghproxy.net > direct)".to_string()
        } else if s.mirror == "none" {
            "(disabled: direct only)".to_string()
        } else {
            s.mirror.clone()
        }
    );
    if !s.last_version.is_empty() {
        println!("last_update: {}", s.last_version);
    }
    Ok(())
}

/// `codegraph233 update [--check] [--force] [--mirror URL] [--json]`
pub fn run(check: bool, force: bool, mirror: Option<String>, json: bool) -> Result<(), String> {
    let mut s = load_settings();
    if let Some(m) = mirror {
        s.mirror = m;
        save_settings(&s)?;
    }
    let rel = latest_release(&s)?;
    let newer = is_newer(&rel.version, current_version());
    if json {
        let payload = serde_json::json!({
            "current": current_version(),
            "latest": rel.version,
            "up_to_date": !newer,
            "asset": rel.asset.as_ref().map(|a| serde_json::json!({
                "name": a.0, "url": a.1, "size": a.2,
            })),
            "settings": settings_path().display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap_or_default());
        return Ok(());
    }
    if check {
        if newer {
            println!(
                "update available: {} -> {} (run `codegraph233 update`)",
                current_version(),
                rel.version
            );
        } else {
            println!("codegraph233 {} is up to date", current_version());
        }
        return Ok(());
    }
    if let Some(v) = check_and_update(&s, force, false)? {
        s.last_version = v;
        s.last_check = crate::store::now_unix();
        let _ = save_settings(&s);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_is_numeric() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(is_newer("2.0", "1.9.9"));
    }

    #[test]
    fn mirrors_are_prefixed_and_deduplicated() {
        let s = Settings {
            mirror: "https://ghfast.top/".into(),
            mirrors: vec!["https://ghproxy.net/".into(), "https://ghfast.top/".into()],
            ..Default::default()
        };
        let urls = mirrored("https://github.com/x/y.zip", &s);
        assert_eq!(urls[0], "https://ghfast.top/https://github.com/x/y.zip");
        assert_eq!(urls[1], "https://ghproxy.net/https://github.com/x/y.zip");
        assert_eq!(urls.last().unwrap(), "https://github.com/x/y.zip");
        assert_eq!(urls.iter().filter(|u| u.starts_with("https://ghfast.top/")).count(), 1);
    }

    #[test]
    fn disabled_mirror_uses_direct_only() {
        let s = Settings { mirror: "none".into(), mirrors: vec![], ..Default::default() };
        assert_eq!(mirrored("https://a/b", &s), vec!["https://a/b".to_string()]);
    }

    #[test]
    fn picks_platform_zip_asset() {
        let assets = serde_json::json!([
            {"name": "codegraph233-0.4.0-x86_64-unknown-linux-gnu.tar.gz",
             "browser_download_url": "https://e/linux.tar.gz", "size": 10},
            {"name": "codegraph233-0.4.0-x86_64-pc-windows-msvc.zip",
             "browser_download_url": "https://e/win.zip", "size": 20},
            {"name": "codegraph233-0.4.0-aarch64-apple-darwin.tar.gz",
             "browser_download_url": "https://e/mac.tar.gz", "size": 30}
        ]);
        let picked = pick_asset(&assets).expect("asset");
        let want = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "apple",
            _ => "linux",
        };
        assert!(picked.0.contains(want), "picked {:?} on {}", picked.0, std::env::consts::OS);
        assert!(picked.2 > 0);
    }

    #[test]
    fn extracts_binary_from_zip() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("cg233-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("rel.zip");
        {
            let f = std::fs::File::create(&archive).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("codegraph233.exe", opts).unwrap();
            zw.write_all(b"fake-binary").unwrap();
            zw.finish().unwrap();
        }
        let out = extract_binary(&archive, &dir).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"fake-binary");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_options_validate_and_apply() {
        let mut s = Settings::default();
        apply(&mut s, Some(false), Some(6), Some("https://example/".into())).unwrap();
        assert!(!s.auto_update);
        assert_eq!(s.interval_hours, 6);
        assert_eq!(s.mirror, "https://example/");
        assert_eq!(
            settings_summary(&s),
            "auto_update=false interval_hours=6 mirror=https://example/"
        );
        assert!(apply(&mut Settings::default(), None, Some(0), None).is_err());
        assert!(apply(&mut Settings::default(), None, Some(24 * 366), None).is_err());
        assert!(apply(&mut Settings::default(), Some(true), None, None).is_ok());
    }
}
