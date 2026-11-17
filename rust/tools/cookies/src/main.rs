//! Host-only YouTube cookie refresh for the Lumisound bridge (the Rust port of
//! ios-bridge/refresh_youtube_cookies.sh).
//!
//! It runs on the host, not in the bridge container: browser cookie databases
//! and their keyrings stay on the host. Two ways to get a fresh jar, tried in
//! this order:
//!
//! 1. Browser mode (`YTDLP_BROWSER` set): yt-dlp reads the local browser
//!    profile and writes a Netscape jar.
//! 2. Inbox mode (no `YTDLP_BROWSER`): adopt the newest `cookies*.txt` in
//!    `YTDLP_COOKIE_INBOX` (default ~/Downloads) that is newer than the
//!    installed jar — for hosts whose browsers can't be read (no keyring), where
//!    exporting a jar by hand is the only way.
//!
//! Both modes end in the same checks (Netscape file, YouTube session cookies,
//! and a live sign-in probe: the account's Watch Later list only resolves for a
//! signed-in session) before the jar is installed, so a malformed, anonymous or
//! rotated export never replaces a working one. Cookie values are never
//! printed.
//!
//! The jar is written INTO the existing file (same inode), never renamed over
//! it: Docker bind-mounts a single file by inode, so a rename would leave the
//! running bridge on the old, expired jar.
//!
//! Exit codes: 0 = installed or nothing to do, 1 = refused (kept the current
//! jar), 2 = configuration error.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::SystemTime;

const DEFAULT_REFRESH_URL: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
const DEFAULT_PROBE_URL: &str = "https://www.youtube.com/playlist?list=WL";
const SESSION_COOKIES: [&str; 6] = ["SID", "HSID", "SSID", "APISID", "SAPISID", "LOGIN_INFO"];

struct Config {
    cookie_file: PathBuf,
    cache_dir: PathBuf,
    refresh_url: String,
    probe_url: String,
    browser: String,
    ytdlp: PathBuf,
    inbox: PathBuf,
    extra_dests: Vec<PathBuf>,
    skip_live_check: bool,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn home() -> PathBuf {
    env("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path: OsString = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn config() -> Result<Config, String> {
    let h = home();
    let ytdlp = env("YTDLP_BIN").map(PathBuf::from).or_else(|| find_in_path("yt-dlp"));
    let ytdlp = match ytdlp {
        Some(p) if is_executable(&p) => p,
        _ => return Err("yt-dlp is required on the host (set YTDLP_BIN if it is not on PATH).".into()),
    };
    Ok(Config {
        cookie_file: env("YTDLP_COOKIES_FILE").map(PathBuf::from).unwrap_or_else(|| h.join("Projects/Lumisound/ios-bridge/cookies.txt")),
        cache_dir: env("YTDLP_CACHE_DIR").map(PathBuf::from).unwrap_or_else(|| h.join("lumisound/cache/yt-dlp")),
        refresh_url: env("YTDLP_COOKIE_REFRESH_URL").unwrap_or_else(|| DEFAULT_REFRESH_URL.into()),
        probe_url: env("YTDLP_COOKIE_PROBE_URL").unwrap_or_else(|| DEFAULT_PROBE_URL.into()),
        browser: env("YTDLP_BROWSER").unwrap_or_default(),
        ytdlp,
        inbox: env("YTDLP_COOKIE_INBOX").map(PathBuf::from).unwrap_or_else(|| h.join("Downloads")),
        extra_dests: env("YTDLP_EXTRA_COOKIE_DESTS")
            .map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default(),
        skip_live_check: env("YTDLP_COOKIE_SKIP_LIVE_CHECK").as_deref() == Some("1"),
    })
}

fn say(msg: &str) {
    eprintln!("{msg}");
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Write `src`'s bytes into `dest` without replacing its inode (see the
/// module docs). chmod first so the bytes are never briefly world-readable.
fn install_in_place(src: &Path, dest: &Path) -> std::io::Result<()> {
    let bytes = fs::read(src)?;
    if dest.exists() {
        let _ = fs::set_permissions(dest, fs::Permissions::from_mode(0o600));
    }
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(dest)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    fs::set_permissions(dest, fs::Permissions::from_mode(0o600))
}

/// Bring every extra destination in line with `src`. Only files that already
/// exist are touched: a stale entry must never scatter session cookies to new
/// paths.
fn sync_extra(cfg: &Config, src: &Path) {
    let Ok(want) = fs::read(src) else { return };
    for dest in &cfg.extra_dests {
        if !dest.is_file() {
            say(&format!("Skipping extra cookie destination (not an existing file): {}", dest.display()));
            continue;
        }
        if fs::read(dest).is_ok_and(|have| have == want) {
            continue;
        }
        match install_in_place(src, dest) {
            Ok(()) => say(&format!("Also refreshed: {}", dest.display())),
            Err(e) => say(&format!("Couldn't refresh {}: {e}", dest.display())),
        }
    }
}

/// Runs yt-dlp with all output discarded (it could include cookie values).
fn ytdlp_ok(cfg: &Config, args: &[&str]) -> bool {
    Command::new(&cfg.ytdlp)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Does this jar still sign in? (Watch Later only exists for a signed-in
/// session; `--ignore-no-formats-error` because the probe is about the
/// account, not about resolving media.)
fn authenticates(cfg: &Config, jar: &Path) -> bool {
    let jar = jar.to_string_lossy();
    ytdlp_ok(
        cfg,
        &["--cookies", &jar, "--flat-playlist", "--playlist-items", "1", "--skip-download", "--ignore-no-formats-error", "--print", "%(id)s", &cfg.probe_url],
    )
}

/// Structural checks only; values are never read out.
fn has_youtube_cookies(text: &str) -> bool {
    text.lines().any(|l| {
        l.split_whitespace().any(|tok| {
            let t = tok.to_ascii_lowercase();
            let t = t.strip_prefix("#httponly_").unwrap_or(&t);
            let t = t.strip_prefix('.').unwrap_or(t);
            t == "youtube.com" || t == "www.youtube.com"
        })
    })
}

fn has_session_cookie(text: &str) -> bool {
    text.lines().any(|l| {
        l.split_whitespace().skip(1).any(|tok| {
            let t = tok.to_ascii_uppercase();
            let t = t.strip_prefix("__SECURE-").unwrap_or(&t);
            SESSION_COOKIES.contains(&t)
        })
    })
}

fn is_netscape(path: &Path) -> bool {
    let mut head = [0u8; 64];
    let n = File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains("Netscape HTTP Cookie File")
}

/// Newest `cookies*.txt` in the inbox.
fn newest_export(inbox: &Path) -> Option<PathBuf> {
    fs::read_dir(inbox)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("cookies") && n.ends_with(".txt"))
        })
        .filter_map(|p| Some((mtime(&p)?, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run() -> Result<u8, String> {
    let cfg = match config() {
        Ok(c) => c,
        Err(e) => {
            say(&e);
            return Ok(2);
        }
    };
    unsafe { libc::umask(0o077) };
    let cookie_dir = cfg.cookie_file.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&cookie_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&cfg.cache_dir).map_err(|e| e.to_string())?;
    let sentinel = cfg.cache_dir.join(".cookies_stale");

    // A timer run and a sentinel-triggered run must not overlap. The lock
    // file holds no cookie data.
    let lock = OpenOptions::new().create(true).write(true).truncate(false).open(cfg.cache_dir.join(".cookies_refresh.lock")).map_err(|e| e.to_string())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(0);
    }

    let staging_dir = cookie_dir.join(format!(".cookies-refresh.{}", std::process::id()));
    if fs::create_dir(&staging_dir).is_err() {
        say("Another cookie refresh is already staging a file.");
        return Ok(0);
    }
    let _cleanup = Staging(staging_dir.clone());
    let staged = staging_dir.join("cookies.txt");

    if !cfg.browser.is_empty() {
        let out = staged.to_string_lossy();
        if !ytdlp_ok(&cfg, &["--cookies-from-browser", &cfg.browser, "--cookies", &out, "--skip-download", "--no-playlist", "--no-warnings", &cfg.refresh_url]) {
            say("Browser cookie refresh failed; keeping the existing cookie file.");
            return Ok(1);
        }
        if fs::metadata(&staged).map(|m| m.len()).unwrap_or(0) == 0 {
            say("Browser cookie refresh produced no cookie file; keeping the existing file.");
            return Ok(1);
        }
    } else {
        let Some(newest) = newest_export(&cfg.inbox) else {
            say(&format!("No cookies*.txt in {} and no YTDLP_BROWSER set; nothing to do.", cfg.inbox.display()));
            return Ok(0);
        };
        let installed = mtime(&cfg.cookie_file);
        if cfg.cookie_file.exists() && installed.is_some_and(|i| mtime(&newest).is_none_or(|n| n <= i)) {
            // Already adopted. The sentinel is written on a symptom (a
            // sign-in wall, which YouTube also shows for plain rate
            // limiting), so check the installed jar: clear a stale sentinel,
            // keep a truthful one.
            if sentinel.exists() && !cfg.skip_live_check {
                if authenticates(&cfg, &cfg.cookie_file) {
                    let _ = fs::remove_file(&sentinel);
                    say("Installed cookies still authenticate; cleared a stale sentinel.");
                } else {
                    say(&format!(
                        "Installed cookies no longer authenticate and no newer export is available in {} — export a fresh cookies.txt from a signed-in browser.",
                        cfg.inbox.display()
                    ));
                    // No extra-dest sync: never spread a jar that just failed.
                    return Ok(0);
                }
            }
            sync_extra(&cfg, &cfg.cookie_file);
            return Ok(0);
        }
        fs::copy(&newest, &staged).map_err(|e| e.to_string())?;
        if fs::metadata(&staged).map(|m| m.len()).unwrap_or(0) == 0 {
            say(&format!("Cookie export in {} is empty; keeping the existing file.", cfg.inbox.display()));
            return Ok(1);
        }
        if !is_netscape(&staged) {
            say("Cookie export is not a Netscape cookie file; keeping the existing file.");
            return Ok(1);
        }
    }

    let text = String::from_utf8_lossy(&fs::read(&staged).map_err(|e| e.to_string())?).into_owned();
    if !has_youtube_cookies(&text) {
        say("Refusing to install a cookie file without YouTube cookies.");
        return Ok(1);
    }
    if !has_session_cookie(&text) {
        say("Refusing to install a cookie file without a YouTube session cookie.");
        return Ok(1);
    }
    drop(text);
    // Expiry dates say nothing (YouTube rotates sessions server-side while
    // the records keep far-future expiries), so ask YouTube. A network
    // outage looks the same as dead cookies; both keep the current jar.
    if !cfg.skip_live_check && !authenticates(&cfg, &staged) {
        say("Cookie export did not authenticate against YouTube (rotated, or network unreachable); keeping the existing file.");
        return Ok(1);
    }

    install_in_place(&staged, &cfg.cookie_file).map_err(|e| format!("Couldn't write {}: {e}", cfg.cookie_file.display()))?;
    sync_extra(&cfg, &staged);
    // Stamp the install time so inbox mode stops matching this export.
    if let Ok(f) = OpenOptions::new().write(true).open(&cfg.cookie_file) {
        let _ = f.set_modified(SystemTime::now());
    }
    let _ = fs::remove_file(&sentinel);
    say("YouTube cookies refreshed successfully.");
    Ok(0)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            say(&format!("Cookie refresh failed: {e}"));
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAR: &str = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1821996693\tSAPISID\tvalue\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-3PSID\tv\n";

    #[test]
    fn structural_checks() {
        assert!(has_youtube_cookies(JAR));
        assert!(has_session_cookie(JAR));
        let anon = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tVISITOR_INFO1_LIVE\tv\n";
        assert!(has_youtube_cookies(anon));
        assert!(!has_session_cookie(anon));
        let other = "# Netscape HTTP Cookie File\n.example.com\tTRUE\t/\tTRUE\t0\tSID\tv\n";
        assert!(!has_youtube_cookies(other));
        // "SID" as the domain column doesn't count as a cookie name.
        assert!(!has_session_cookie("SID\n"));
    }

    #[test]
    fn installs_in_place_and_picks_newest_export() {
        let dir = std::env::temp_dir().join(format!("lumi-cookies-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("cookies.txt");
        fs::write(&dest, "old").unwrap();
        use std::os::unix::fs::MetadataExt;
        let ino = fs::metadata(&dest).unwrap().ino();
        let src = dir.join("cookies (2).txt");
        fs::write(&src, JAR).unwrap();
        install_in_place(&src, &dest).unwrap();
        let m = fs::metadata(&dest).unwrap();
        assert_eq!(m.ino(), ino, "same inode (bind mounts keep seeing it)");
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&dest).unwrap(), JAR);
        assert!(is_netscape(&dest));
        let newer = dir.join("cookies-new.txt");
        fs::write(&newer, "x").unwrap();
        let f = OpenOptions::new().write(true).open(&newer).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(60)).unwrap();
        assert_eq!(newest_export(&dir), Some(newer));
        let _ = fs::remove_dir_all(&dir);
    }
}
