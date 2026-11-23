//! yt-dlp wrapper. The extractor itself stays the upstream yt-dlp CLI (it
//! changes too often to re-implement); this module decides the arguments,
//! runs it as a child process under concurrency/memory/pacing limits and
//! parses its JSON and stderr.

use crate::cache::TtlCache;
use crate::config::Config;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::{Semaphore, SemaphorePermit};

const MEMORY_FLOOR_MB: f64 = 900.0;
const COOKIE_CACHE_TTL: Duration = Duration::from_secs(300);
const MIN_ACCOUNT_AGE_DAYS_FOR_GLOBAL_COOKIES: i64 = 15;
const RATE_LIMIT_MARKERS: &[&str] = &[
    // yt-dlp prints "you’re" with a typographic apostrophe; match the tail.
    "not a bot",
    "http error 429",
    "too many requests",
    "this content isn't available, try again later",
];
const PERMANENT_FAILURE_MARKERS: &[&str] = &[
    "video unavailable",
    "video is unavailable",
    "private video",
    "this video is private",
    "removed by the uploader",
    "has been terminated",
    "no longer available",
    "video has been removed",
    "this video is no longer available",
    "does not exist",
    "unavailable in your country",
    // Same answer on every client without Premium cookies.
    "only available to music premium",
];
pub const ARIA2_ARGS: &[&str] = &[
    "--downloader",
    "aria2c",
    "--downloader-args",
    "aria2c:-x16 -s16 -j16 -k1M --file-allocation=none --console-log-level=warn --summary-interval=0 --retry-wait=1 --max-tries=3",
];
pub const STREAM_CLIENT_ARGS: &[&str] = &["--extractor-args", "youtube:player_client=web,default"];
pub const MUXED_ITAGS: &[&str] = &["17", "18", "22", "36", "43", "59"];

pub fn available_memory_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = s.lines().find(|l| l.starts_with("MemAvailable:"))?;
    line.split_whitespace().nth(1)?.parse::<f64>().ok().map(|kb| kb / 1024.0)
}

/// Semaphore that also holds back slots above `safe` while the host is low
/// on memory (each yt-dlp + ffmpeg pair can use a few hundred MB).
pub struct AdaptiveLimiter {
    sem: Semaphore,
    safe: usize,
    in_use: AtomicUsize,
}

pub struct Slot<'a> {
    _permit: SemaphorePermit<'a>,
    in_use: &'a AtomicUsize,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.in_use.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AdaptiveLimiter {
    pub fn new(max: usize, safe: usize) -> Self {
        let max = max.max(1);
        Self { sem: Semaphore::new(max), safe: safe.clamp(1, max), in_use: AtomicUsize::new(0) }
    }

    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    pub async fn slot(&self) -> Slot<'_> {
        let permit = self.sem.acquire().await.expect("limiter closed");
        while self.in_use.load(Ordering::SeqCst) >= self.safe {
            match available_memory_mb() {
                Some(mb) if mb < MEMORY_FLOOR_MB => {
                    tracing::info!(
                        "concurrency held back: {mb:.0}MB available (floor {MEMORY_FLOOR_MB:.0}MB), {} in use",
                        self.in_use.load(Ordering::SeqCst)
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                _ => break,
            }
        }
        self.in_use.fetch_add(1, Ordering::SeqCst);
        Slot { _permit: permit, in_use: &self.in_use }
    }
}

/// Spaces out yt-dlp launches; widens the gap 4x after YouTube rate-limits
/// us and relaxes it again after five quiet minutes.
pub struct LaunchPacer {
    base: f64,
    max: f64,
    inner: Mutex<PacerState>,
}

struct PacerState {
    next_at: Instant,
    interval: f64,
    penalised_at: Option<Instant>,
}

impl LaunchPacer {
    pub fn new(base: f64, max: f64) -> Self {
        Self { base, max, inner: Mutex::new(PacerState { next_at: Instant::now(), interval: base, penalised_at: None }) }
    }

    pub async fn wait(&self, floor: f64) -> Duration {
        let delay = {
            let mut s = self.inner.lock().unwrap();
            let now = Instant::now();
            if let Some(p) = s.penalised_at {
                if now.duration_since(p).as_secs_f64() > 300.0 {
                    s.interval = (s.interval / 2.0).max(self.base);
                    s.penalised_at = if s.interval <= self.base { None } else { Some(now) };
                }
            }
            let interval = s.interval.max(floor);
            let delay = s.next_at.saturating_duration_since(now);
            s.next_at = now + delay + Duration::from_secs_f64(interval);
            delay
        };
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        delay
    }

    pub fn penalise(&self, reason: &str) {
        let mut s = self.inner.lock().unwrap();
        s.interval = (s.interval.max(self.base) * 4.0).min(self.max);
        s.penalised_at = Some(Instant::now());
        tracing::warn!("yt-dlp launch spacing raised to {:.1}s after {reason}", s.interval);
    }

    pub fn interval(&self) -> f64 {
        self.inner.lock().unwrap().interval
    }
}

pub struct Output {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum RunError {
    Timeout,
    Spawn(std::io::Error),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Timeout => write!(f, "timed out"),
            RunError::Spawn(e) => write!(f, "spawn failed: {e}"),
        }
    }
}

pub struct Ytdlp {
    pub cfg: Config,
    pub db: PgPool,
    pub limiter: AdaptiveLimiter,
    pub transcode: AdaptiveLimiter,
    pub pacer: LaunchPacer,
    network_args: Vec<String>,
    user_cookies: TtlCache<String, Option<String>>,
    account_age: TtlCache<String, Option<i64>>,
    cookie_pref: TtlCache<String, String>,
    written_cookie_hash: dashmap::DashMap<String, [u8; 32]>,
    global_copy: Mutex<Option<(std::time::SystemTime, u64)>>,
    /// YouTube rate-limited the session that a cookie owner (user id, or
    /// "global") downloads with: hold that owner's downloads until then.
    rate_hold: dashmap::DashMap<String, Instant>,
    /// Per cookie jar ("global", "pool:<name>", "user:<id>"): last success,
    /// bot-wall history and a bench after YouTube's "not a bot" wall.
    pub jar_health: dashmap::DashMap<String, JarHealth>,
    pool_copies: dashmap::DashMap<String, (std::time::SystemTime, u64)>,
}

/// How a cookie jar is doing (admin card, rotation order, alerts).
#[derive(Clone, Debug, Default)]
pub struct JarHealth {
    pub last_ok: Option<chrono::DateTime<chrono::Utc>>,
    pub last_botwall: Option<chrono::DateTime<chrono::Utc>>,
    /// Bot walls since the last success.
    pub botwalls: u32,
    pub benched_until: Option<Instant>,
}

/// One cookie jar to try: who owns it and the yt-dlp arguments for it.
#[derive(Clone, Debug, PartialEq)]
pub struct CookieChoice {
    /// "global", "pool:<name>", "user:<id>" or "none".
    pub owner: String,
    pub args: Vec<String>,
}

/// How long a jar sits out after a bot wall before it is tried first again.
const BOTWALL_BENCH: Duration = Duration::from_secs(20 * 60);

/// YouTube's "Sign in to confirm you're not a bot" wall.
pub fn is_bot_wall(stderr: &[u8]) -> bool {
    let t = String::from_utf8_lossy(stderr).to_lowercase();
    t.contains("not a bot") || t.contains("sign in to confirm")
}

impl Ytdlp {
    pub fn network(&self) -> &[String] {
        &self.network_args
    }

    pub fn new(cfg: &Config, db: PgPool) -> Self {
        let mut network_args: Vec<String> = ["-4", "--socket-timeout", "10"].iter().map(|s| s.to_string()).collect();
        if let Some(pot) = &cfg.ytdlp_pot_provider_url {
            network_args.push("--extractor-args".into());
            network_args.push(format!("youtubepot-bgutilhttp:base_url={pot}"));
        }
        Self {
            limiter: AdaptiveLimiter::new(cfg.ytdlp_max, cfg.ytdlp_safe),
            transcode: AdaptiveLimiter::new(cfg.transcode_max, cfg.transcode_safe),
            pacer: LaunchPacer::new(cfg.launch_spacing, cfg.launch_spacing_max),
            network_args,
            user_cookies: TtlCache::new(COOKIE_CACHE_TTL, 10_000),
            account_age: TtlCache::new(COOKIE_CACHE_TTL, 10_000),
            cookie_pref: TtlCache::new(COOKIE_CACHE_TTL, 10_000),
            written_cookie_hash: dashmap::DashMap::new(),
            global_copy: Mutex::new(None),
            rate_hold: dashmap::DashMap::new(),
            jar_health: dashmap::DashMap::new(),
            pool_copies: dashmap::DashMap::new(),
            cfg: cfg.clone(),
            db,
        }
    }

    pub fn network_args(&self) -> &[String] {
        &self.network_args
    }

    pub fn cache_dir(&self) -> &str {
        &self.cfg.ytdlp_cache_dir
    }

    // ---- cookies ---------------------------------------------------------------------------

    fn global_available(&self) -> bool {
        std::fs::metadata(&self.cfg.ytdlp_cookies_file).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
    }

    /// yt-dlp rewrites its cookie jar on exit, and the shared jar is
    /// bind-mounted read-only, so yt-dlp gets a writable copy that is
    /// refreshed whenever the source changes.
    fn writable_global_cookies_path(&self) -> Option<String> {
        let src = Path::new(&self.cfg.ytdlp_cookies_file);
        let meta = std::fs::metadata(src).ok()?;
        if meta.len() == 0 {
            return None;
        }
        let cache_dir = Path::new(&self.cfg.ytdlp_cache_dir);
        let copy = cache_dir.join("global-cookies.txt");
        let stamp = cache_dir.join("global-cookies.source");
        let sig = (meta.modified().ok()?, meta.len());
        {
            let g = self.global_copy.lock().unwrap();
            if *g == Some(sig) && copy.exists() {
                return Some(copy.to_string_lossy().into_owned());
            }
        }
        let bytes = std::fs::read(src).ok()?;
        let hash = hex::encode(Sha256::digest(&bytes));
        let stale = std::fs::read_to_string(&stamp).map(|s| s.trim() != hash).unwrap_or(true) || !copy.exists();
        if stale {
            let r = std::fs::create_dir_all(cache_dir)
                .and_then(|_| std::fs::write(&copy, &bytes))
                .and_then(|_| std::fs::write(&stamp, &hash));
            match r {
                Ok(_) => tracing::info!("refreshed writable cookie copy from source ({} bytes)", bytes.len()),
                Err(e) => {
                    tracing::error!("failed to refresh writable global cookies copy: {e}");
                    return if copy.exists() { Some(self.cfg.ytdlp_cookies_file.clone()) } else { None };
                }
            }
        }
        *self.global_copy.lock().unwrap() = Some(sig);
        Some(copy.to_string_lossy().into_owned())
    }

    pub fn invalidate_user_cookies(&self, user_id: &str) {
        // New jar: its predecessor's bot-wall bench doesn't apply.
        self.jar_health.remove(&format!("user:{user_id}"));
        self.user_cookies.remove(&user_id.to_string());
        self.cookie_pref.remove(&user_id.to_string());
        self.written_cookie_hash.remove(user_id);
    }

    async fn user_cookies_text(&self, user_id: &str) -> Option<String> {
        let key = user_id.to_string();
        if let Some(v) = self.user_cookies.get(&key) {
            return v;
        }
        let text: Option<String> = sqlx::query_scalar("SELECT ytdlp_cookies FROM ios_user_settings WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&self.db)
            .await
            .ok()
            .flatten()
            .flatten()
            .filter(|t: &String| !t.is_empty());
        self.user_cookies.set(key, text.clone());
        text
    }

    async fn user_cookies_file(&self, user_id: Option<&str>) -> Option<String> {
        let uid = user_id?;
        let text = self.user_cookies_text(uid).await?;
        let dir = PathBuf::from(&self.cfg.ytdlp_user_cookies_dir);
        let path = dir.join(format!("{uid}.txt"));
        let hash: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        let unchanged = self.written_cookie_hash.get(uid).is_some_and(|h| *h == hash) && path.exists();
        if !unchanged {
            let _ = tokio::fs::create_dir_all(&dir).await;
            if tokio::fs::write(&path, text.as_bytes()).await.is_ok() {
                self.written_cookie_hash.insert(uid.to_string(), hash);
            }
        }
        Some(path.to_string_lossy().into_owned())
    }

    async fn account_age_days(&self, user_id: &str) -> Option<i64> {
        let key = user_id.to_string();
        if let Some(v) = self.account_age.get(&key) {
            return v;
        }
        let created: Option<chrono::NaiveDateTime> =
            sqlx::query_scalar("SELECT created_at FROM ios_users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten()
                .flatten();
        let age = created.map(|c| (chrono::Utc::now().naive_utc() - c).num_days());
        self.account_age.set(key, age);
        age
    }

    pub async fn cookie_fallback_preference(&self, user_id: &str) -> String {
        let key = user_id.to_string();
        if let Some(v) = self.cookie_pref.get(&key) {
            return v;
        }
        let pref: Option<String> =
            sqlx::query_scalar("SELECT cookie_fallback_preference FROM ios_user_settings WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&self.db)
                .await
                .ok()
                .flatten()
                .flatten();
        let pref = match pref.as_deref() {
            Some(p @ ("auto" | "own_only" | "global_only")) => p.to_string(),
            _ => "auto".to_string(),
        };
        self.cookie_pref.set(key, pref.clone());
        pref
    }

    async fn may_use_global_cookies(&self, user_id: Option<&str>) -> bool {
        let Some(uid) = user_id else { return true };
        match self.account_age_days(uid).await {
            Some(d) if d >= MIN_ACCOUNT_AGE_DAYS_FOR_GLOBAL_COOKIES => {}
            _ => return false,
        }
        self.cookie_fallback_preference(uid).await != "own_only"
    }

    /// Extra shared jars (`YTDLP_COOKIES_POOL_DIR/*.txt`), as writable copies
    /// (yt-dlp rewrites its jar on exit), refreshed when the source changes.
    fn pool_jars(&self) -> Vec<(String, String)> {
        let Some(dir) = self.cfg.ytdlp_cookies_pool_dir.as_deref() else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let copies = Path::new(&self.cfg.ytdlp_cache_dir).join("pool-cookies");
        let mut out = Vec::new();
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "txt")).collect();
        entries.sort();
        for src in entries {
            let Some(name) = src.file_stem().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let Ok(meta) = std::fs::metadata(&src) else { continue };
            if meta.len() == 0 {
                continue;
            }
            let copy = copies.join(format!("{name}.txt"));
            let sig = (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len());
            let fresh = self.pool_copies.get(&name).is_some_and(|s| *s == sig) && copy.exists();
            if !fresh {
                let r = std::fs::create_dir_all(&copies).and_then(|_| std::fs::copy(&src, &copy).map(|_| ()));
                if let Err(e) = r {
                    tracing::warn!("cookie pool: could not copy {name}: {e}");
                    continue;
                }
                self.pool_copies.insert(name.clone(), sig);
            }
            out.push((format!("pool:{name}"), copy.to_string_lossy().into_owned()));
        }
        out
    }

    fn benched(&self, owner: &str) -> bool {
        self.jar_health.get(owner).and_then(|h| h.benched_until).is_some_and(|t| t > Instant::now())
    }

    /// Every jar this request may use, best first: the user's own jar, the
    /// shared one and the pool (by the user's preference and account age),
    /// with jars benched after a bot wall moved to the back. Never empty: a
    /// request with no usable jar gets one cookieless choice.
    pub async fn cookie_candidates(&self, user_id: Option<&str>, force_global: bool) -> Vec<CookieChoice> {
        // skip=authcheck too: with cookies + skip=webpage, yt-dlp 2026.10+
        // refuses every playlist ("Playlists that require authentication may
        // not extract correctly without a successful webpage download"), so
        // playlist imports and library downloads stopped on 2026-11-09.
        let base = vec!["--extractor-args".to_string(), "youtubetab:skip=webpage,authcheck".to_string()];
        let eligible = self.may_use_global_cookies(user_id).await;
        let shared = |out: &mut Vec<(String, String)>| {
            if self.global_available() {
                if let Some(p) = self.writable_global_cookies_path() {
                    out.push(("global".into(), p));
                }
            }
            out.extend(self.pool_jars());
        };
        let mut jars: Vec<(String, String)> = Vec::new();
        let pref = match user_id {
            Some(u) => self.cookie_fallback_preference(u).await,
            None => "auto".to_string(),
        };
        if (force_global || pref == "global_only") && eligible {
            shared(&mut jars);
        } else {
            if let (Some(u), Some(own)) = (user_id, self.user_cookies_file(user_id).await) {
                jars.push((format!("user:{u}"), own));
            }
            if eligible {
                shared(&mut jars);
            }
        }
        let (mut ready, rested): (Vec<_>, Vec<_>) = jars.into_iter().partition(|(o, _)| !self.benched(o));
        ready.extend(rested);
        if ready.is_empty() {
            return vec![CookieChoice { owner: "none".into(), args: base }];
        }
        ready
            .into_iter()
            .map(|(owner, path)| {
                let mut args = base.clone();
                args.extend(["--cookies".to_string(), path]);
                CookieChoice { owner, args }
            })
            .collect()
    }

    /// Cookie arguments for a request on behalf of `user_id`: the best jar
    /// from `cookie_candidates`.
    pub async fn cookie_args(&self, user_id: Option<&str>, force_global: bool) -> Vec<String> {
        self.cookie_candidates(user_id, force_global).await.swap_remove(0).args
    }

    /// A yt-dlp run with this jar got through.
    pub fn note_jar_ok(&self, owner: &str) {
        if owner == "none" {
            return;
        }
        let mut h = self.jar_health.entry(owner.to_string()).or_default();
        h.last_ok = Some(chrono::Utc::now());
        h.botwalls = 0;
        h.benched_until = None;
    }

    /// YouTube showed this jar the bot wall: bench it so the next requests
    /// start with another jar, and flag the shared jar stale so the host's
    /// refresher (`lumisound-refresh-cookies`) installs a fresh export.
    pub fn note_jar_botwall(&self, owner: &str) {
        if owner == "none" {
            return;
        }
        {
            let mut h = self.jar_health.entry(owner.to_string()).or_default();
            h.last_botwall = Some(chrono::Utc::now());
            h.botwalls = h.botwalls.saturating_add(1);
            h.benched_until = Some(Instant::now() + BOTWALL_BENCH);
        }
        tracing::warn!("cookie jar {owner} hit YouTube's bot wall; benched for {} min", BOTWALL_BENCH.as_secs() / 60);
        if owner == "global" {
            self.write_stale_sentinel("bot wall");
        }
    }

    fn write_stale_sentinel(&self, why: &str) {
        let p = Path::new(&self.cfg.ytdlp_cache_dir).join(".cookies_stale");
        let _ = std::fs::create_dir_all(Path::new(&self.cfg.ytdlp_cache_dir));
        if let Err(e) = std::fs::write(&p, format!("{} {why}", chrono::Utc::now().to_rfc3339())) {
            tracing::warn!("failed to write cookies-stale sentinel: {e}");
        }
    }

    /// Bot wall → bench the jar; a clean exit → mark it healthy.
    pub fn note_jar_result(&self, owner: &str, ok: bool, stderr: &[u8]) {
        if ok {
            self.note_jar_ok(owner);
        } else if is_bot_wall(stderr) {
            self.note_jar_botwall(owner);
        }
    }

    /// Admin view of every jar the bridge has used or could use.
    pub fn jar_report(&self) -> Vec<Value> {
        let mut owners: Vec<String> = Vec::new();
        if self.global_available() {
            owners.push("global".into());
        }
        owners.extend(self.pool_jars().into_iter().map(|(o, _)| o));
        for e in self.jar_health.iter() {
            if !owners.contains(e.key()) {
                owners.push(e.key().clone());
            }
        }
        let now = Instant::now();
        owners
            .into_iter()
            .map(|o| {
                let h = self.jar_health.get(&o).map(|h| h.clone()).unwrap_or_default();
                json!({
                    "owner": if o.starts_with("user:") { "user" } else { o.as_str() },
                    "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                    "last_botwall": h.last_botwall.map(|t| t.to_rfc3339()),
                    "botwalls_since_ok": h.botwalls,
                    "benched_secs": h.benched_until.map(|t| t.saturating_duration_since(now).as_secs()).unwrap_or(0),
                })
            })
            .collect()
    }

    /// Modification time of the shared jar and whether the refresher's
    /// stale sentinel is set.
    pub fn global_jar_status(&self) -> Value {
        let m = std::fs::metadata(&self.cfg.ytdlp_cookies_file).ok();
        let age_h = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() / 3600);
        let sentinel = std::fs::read_to_string(Path::new(&self.cfg.ytdlp_cache_dir).join(".cookies_stale")).ok();
        json!({
            "present": m.is_some_and(|m| m.len() > 0),
            "age_hours": age_h,
            "stale_flag": sentinel.map(|s| s.trim().to_string()),
            "pool_size": self.pool_jars().len(),
        })
    }

    pub async fn listing_args(&self, target: &str, source: &str, user_id: Option<&str>) -> Vec<String> {
        if source == "soundcloud" {
            return vec![target.into(), "--dump-json".into(), "--no-playlist".into()];
        }
        let mut a: Vec<String> = vec![
            target.into(),
            "--dump-json".into(),
            "--flat-playlist".into(),
            "--no-playlist".into(),
            "--cache-dir".into(),
            self.cfg.ytdlp_cache_dir.clone(),
        ];
        a.extend(self.cookie_args(user_id, false).await);
        a
    }

    // ---- process control -------------------------------------------------------------------

    /// Spawn with retries when the kernel refuses a new process (EAGAIN
    /// under thread/process pressure), widening the launch pacer each time.
    pub async fn spawn(&self, program: &str, args: &[String]) -> Result<tokio::process::Child, std::io::Error> {
        let delays = [0.5, 2.0, 5.0];
        let mut last = None;
        for (i, delay) in delays.iter().map(Some).chain(std::iter::once(None)).enumerate() {
            match Command::new(program)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
            {
                Ok(c) => return Ok(c),
                Err(e) if e.raw_os_error() == Some(11) => {
                    let Some(d) = delay else {
                        last = Some(e);
                        break;
                    };
                    if program == "yt-dlp" {
                        self.pacer.penalise(&format!("spawn refused with EAGAIN (attempt {}/4)", i + 1));
                    }
                    tracing::warn!("spawn of {program} refused with EAGAIN; retrying in {d:.1}s");
                    tokio::time::sleep(Duration::from_secs_f64(*d)).await;
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        tracing::error!("spawn of {program} still refused with EAGAIN after 4 attempts; giving up");
        Err(last.unwrap_or_else(|| std::io::Error::other("spawn failed")))
    }

    /// Run a command to completion, killing it on timeout.
    pub async fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<Output, RunError> {
        let child = self.spawn(program, args).await.map_err(RunError::Spawn)?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(o)) => Ok(Output { status: o.status.code(), stdout: o.stdout, stderr: o.stderr }),
            Ok(Err(e)) => Err(RunError::Spawn(e)),
            // Dropping the future drops the child, and kill_on_drop reaps it.
            Err(_) => Err(RunError::Timeout),
        }
    }

    /// `_run_ytdlp`: yt-dlp with the network args, under the limiter; one JSON
    /// object per stdout line.
    pub async fn run_json(&self, args: &[String], timeout: Duration) -> Result<Vec<Value>, RunError> {
        let mut full: Vec<String> = self.network_args.clone();
        full.extend_from_slice(args);
        tracing::info!("Running: yt-dlp {}", redact(&full));
        let out = {
            let _slot = self.limiter.slot().await;
            self.run("yt-dlp", &full, timeout).await?
        };
        if !out.stderr.is_empty() {
            self.flag_cookies_stale_if_needed(&out.stderr);
        }
        Ok(out
            .stdout
            .split(|b| *b == b'\n')
            .filter_map(|l| {
                let l = l.trim_ascii();
                if l.is_empty() { None } else { serde_json::from_slice(l).ok() }
            })
            .collect())
    }

    // ---- stderr classification -------------------------------------------------------------

    pub fn flag_cookies_stale_if_needed(&self, stderr: &[u8]) {
        let t = String::from_utf8_lossy(stderr).to_lowercase();
        if !t.contains("no longer valid") || !t.contains("cookie") {
            return;
        }
        self.write_stale_sentinel("cookies no longer valid");
    }

    /// Seconds left on a rate-limit hold for this cookie owner.
    pub fn rate_hold_left(&self, user: Option<&str>) -> Option<u64> {
        let key = user.unwrap_or("global");
        let until = *self.rate_hold.get(key)?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            self.rate_hold.remove(key);
            return None;
        }
        Some(left.as_secs().max(1))
    }

    /// After a rate-limit answer: hold this owner's downloads (15 min for
    /// YouTube's "session rate-limited for up to an hour", 2 min for a plain
    /// 429) instead of burning through a playlist that fails the same way.
    pub fn hold_if_rate_limited(&self, user: Option<&str>, stderr: &[u8]) {
        let t = String::from_utf8_lossy(stderr).to_lowercase();
        let secs = if t.contains("rate-limited") || t.contains("rate limited") {
            900
        } else if t.contains("http error 429") || t.contains("too many requests") {
            120
        } else {
            return;
        };
        let key = user.unwrap_or("global").to_string();
        let until = Instant::now() + Duration::from_secs(secs);
        let longer = self.rate_hold.get(&key).is_none_or(|u| *u < until);
        if longer {
            self.rate_hold.insert(key.clone(), until);
            tracing::warn!("YouTube rate-limited downloads for {key}; holding them for {} min", secs / 60);
        }
    }

    pub fn penalise_if_rate_limited(&self, stderr: &[u8]) -> bool {
        let t = String::from_utf8_lossy(stderr).to_lowercase();
        for m in RATE_LIMIT_MARKERS {
            if t.contains(m) {
                self.pacer.penalise(&format!("yt-dlp reported: {m:?}"));
                return true;
            }
        }
        false
    }

    pub fn auth_failure_reason(&self, stderr: &[u8]) -> Option<String> {
        self.flag_cookies_stale_if_needed(stderr);
        let t = String::from_utf8_lossy(stderr).to_lowercase();
        let r = if t.contains("music premium") {
            "This video is only available to YouTube Music Premium members. Upload cookies from a Premium account in Settings to download it."
        } else if t.contains("sign in to confirm") || t.contains("not a bot") {
            "YouTube is requiring sign-in to verify this isn't a bot. Upload your YouTube cookies in Settings to fix this."
        } else if t.contains("age-restricted") || (t.contains("age") && t.contains("restrict")) {
            "This video is age-restricted and YouTube refused it on every client. Upload YouTube cookies from a logged-in, age-verified account in Settings to play it."
        } else if t.contains("private video") {
            "This video is private."
        } else if t.contains("sign in") && t.contains("login") {
            "This video requires you to be signed in. Upload your YouTube cookies in Settings."
        } else if t.contains("requested format is not available") || t.contains("no video formats found") {
            "YouTube didn't expose a compatible audio stream for this track right now. This is usually temporary — try again in a bit."
        } else {
            return None;
        };
        Some(r.to_string())
    }

    // ---- stream URL resolution -------------------------------------------------------------

    /// Direct media URL for `target_url`. One yt-dlp run with the preferred
    /// format and its fallbacks chained (`a/b/c`), instead of one run per
    /// format. A bot wall moves on to the next cookie jar (own → shared →
    /// pool) and finally to `client_fallback` (the age-gate client, which
    /// YouTube walls less); other misses get one plain retry.
    /// `client` overrides the player client of the first attempt (a CDN 403
    /// on the URL the web client handed out retries through another).
    pub async fn raw_url(&self, target_url: &str, format_flag: &str, user_id: Option<&str>) -> Result<(Option<String>, Option<String>), RunError> {
        self.raw_url_via(target_url, format_flag, user_id, None).await
    }

    pub async fn raw_url_via(&self, target_url: &str, format_flag: &str, user_id: Option<&str>, client: Option<&str>) -> Result<(Option<String>, Option<String>), RunError> {
        let mut chain: Vec<&str> = Vec::new();
        for f in format_flag.split('/').chain(["bestaudio", "best"]) {
            if !chain.contains(&f) {
                chain.push(f);
            }
        }
        let selector = chain.join("/");
        let jars = self.cookie_candidates(user_id, false).await;
        let mut jar = 0usize;
        let mut client: Option<String> = client.map(str::to_string);
        let mut plain_retries = 1;
        let start = Instant::now();
        let mut last_stderr = Vec::new();
        for attempt in 1..=(jars.len() + 3) {
            let choice = &jars[jar.min(jars.len() - 1)];
            let mut args: Vec<String> = self.network_args.clone();
            match &client {
                Some(c) => args.extend(["--extractor-args".to_string(), format!("youtube:player_client={c}")]),
                None => args.extend(STREAM_CLIENT_ARGS.iter().map(|s| s.to_string())),
            }
            args.extend(["-f".into(), selector.clone(), "--get-url".into(), "--no-playlist".into()]);
            args.extend(choice.args.iter().cloned());
            args.push(target_url.into());
            tracing::info!("Running (raw, jar {}): yt-dlp {}", choice.owner, redact(&args));
            let out = {
                let _slot = self.limiter.slot().await;
                match self.run("yt-dlp", &args, Duration::from_secs(15)).await {
                    Err(RunError::Timeout) => {
                        tracing::warn!("raw_url: yt-dlp --get-url timed out after {:.1}s for {target_url}", start.elapsed().as_secs_f64());
                        return Err(RunError::Timeout);
                    }
                    r => r?,
                }
            };
            if let Some(url) = String::from_utf8_lossy(&out.stdout).lines().map(str::trim).find(|l| l.starts_with("http")) {
                self.note_jar_ok(&choice.owner);
                tracing::info!("raw_url: resolved in {:.2}s (attempt {attempt}, jar {}) for {target_url}", start.elapsed().as_secs_f64(), choice.owner);
                return Ok((Some(url.to_string()), None));
            }
            last_stderr = out.stderr;
            if is_bot_wall(&last_stderr) {
                self.note_jar_botwall(&choice.owner);
                self.penalise_if_rate_limited(&last_stderr);
                if jar + 1 < jars.len() {
                    jar += 1;
                    continue;
                }
                let fb = self.cfg.ytdlp_fallback_client.clone();
                if client.as_deref() != Some(fb.as_str()) {
                    tracing::info!("raw_url: every cookie jar walled; trying the {fb} client for {target_url}");
                    client = Some(fb);
                    continue;
                }
                break;
            }
            if is_permanent_failure(&last_stderr) || self.auth_failure_reason(&last_stderr).is_some() || plain_retries == 0 {
                break;
            }
            plain_retries -= 1;
        }
        tracing::error!(
            "raw_url: no stream URL for {target_url} in {:.2}s — stderr: {}",
            start.elapsed().as_secs_f64(),
            tail(&String::from_utf8_lossy(&last_stderr), 500)
        );
        let reason = self.auth_failure_reason(&last_stderr).or_else(|| is_permanent_failure(&last_stderr).then(|| generic_failure_summary(&last_stderr)));
        Ok((None, reason))
    }
}

/// When a googlevideo URL stops working (`expire=` query, Unix seconds).
pub fn url_expiry(url: &str) -> Option<i64> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix("expire=")).and_then(|v| v.parse().ok())
}

/// YouTube is throttling, not refusing this video: its message ("Video
/// unavailable. This content isn't available, try again later. The current
/// session has been rate-limited…") must not read as a removed video.
pub fn is_rate_limited(stderr: &[u8]) -> bool {
    let t = String::from_utf8_lossy(stderr).to_lowercase();
    t.contains("rate-limited") || t.contains("rate limited") || t.contains("try again later") || t.contains("http error 429") || t.contains("too many requests")
}

pub fn is_permanent_failure(stderr: &[u8]) -> bool {
    let t = String::from_utf8_lossy(stderr).to_lowercase();
    if t.contains("sign in to confirm") || t.contains("not a bot") || is_rate_limited(stderr) {
        return false;
    }
    PERMANENT_FAILURE_MARKERS.iter().any(|m| t.contains(m))
}

/// YouTube refused an age-restricted video because the cookie account isn't
/// age-verified. The `web*`/`mweb` clients then hand out streams that 403,
/// but the configured fallback client (`tv`) still serves them, so the next
/// attempt should go straight to it.
pub fn needs_age_fallback(stderr: &[u8]) -> bool {
    let t = String::from_utf8_lossy(stderr).to_lowercase();
    t.contains("age-restricted") || t.contains("age-verification") || t.contains("confirm your age")
}

/// YouTube says the video itself is gone or blocked ("Video unavailable",
/// "This video is not available") — not a sign-in, bot or Premium problem.
pub fn is_unavailable(stderr: &[u8]) -> bool {
    let t = String::from_utf8_lossy(stderr).to_lowercase();
    (t.contains("video unavailable") || t.contains("this video is not available")) && !t.contains("premium") && !t.contains("sign in") && !is_rate_limited(stderr)
}

pub fn generic_failure_summary(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        return "Could not download track".into();
    }
    let errors: Vec<&str> = text.lines().map(str::trim).filter(|l| l.starts_with("ERROR:")).collect();
    let summary = errors.last().copied().unwrap_or_else(|| text.lines().last().unwrap_or("").trim());
    let summary = summary.strip_prefix("ERROR:").map(str::trim).unwrap_or(summary);
    if summary.is_empty() { "Could not download track".into() } else { summary.chars().take(300).collect() }
}

pub fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

/// Command line for logs, with cookie paths left in (they are not secret)
/// but nothing else changed.
fn redact(args: &[String]) -> String {
    args.join(" ")
}

pub fn format_flag(format: &str) -> &'static str {
    match format {
        "lumi" => "251/bestaudio[acodec=opus]/bestaudio/best",
        // Premium accounts streaming through the engine: Premium Opus first.
        "premium" => "774/251/bestaudio[acodec=opus]/bestaudio/best",
        "mp3" | "flac" | "wav" | "best" => "bestaudio/best",
        "opus" => "bestaudio[ext=webm]/bestaudio/best",
        _ => "bestaudio[ext=m4a]/bestaudio/best",
    }
}

/// yt-dlp args + expected extension for a download format, for an account
/// whose cookies are YouTube Premium (`premium`): Premium Opus (774, ~256
/// kbps) first wherever the format asks for the best audio.
///
/// "lumi" (Lumisound Audio, the app's default since LMS3) takes the best
/// stream as it is (`--audio-format best` copies, never re-encodes): Opus
/// lands in `.opus`, AAC in `.m4a`, and the app stores the packets in LMS3.
pub fn download_format_args_for(format: &str, premium: bool) -> (Vec<&'static str>, &'static str) {
    match (format, premium) {
        ("lumi", true) => (vec!["-f", "774/251/bestaudio[acodec=opus]/bestaudio/best", "-x", "--audio-format", "best"], "opus"),
        ("lumi", false) => (vec!["-f", "251/bestaudio[acodec=opus]/bestaudio/best", "-x", "--audio-format", "best"], "opus"),
        ("opus", true) => (vec!["-f", "774/bestaudio[ext=webm]/bestaudio", "-x", "--audio-format", "opus", "--audio-quality", "0"], "opus"),
        ("best", true) => (vec!["-f", "774/bestaudio/best", "-x"], "m4a"),
        (f, true) if matches!(f, "mp3" | "flac" | "wav") => {
            let (mut a, e) = download_format_args(f);
            a[1] = "774/bestaudio";
            (a, e)
        }
        (f, _) => download_format_args(f),
    }
}

/// yt-dlp args + expected extension for a download format.
pub fn download_format_args(format: &str) -> (Vec<&'static str>, &'static str) {
    match format {
        "lumi" => download_format_args_for("lumi", false),
        "mp3" => (vec!["-f", "bestaudio", "-x", "--audio-format", "mp3", "--audio-quality", "0"], "mp3"),
        "flac" => (vec!["-f", "bestaudio", "-x", "--audio-format", "flac", "--audio-quality", "0"], "flac"),
        "opus" => (vec!["-f", "bestaudio[ext=webm]/bestaudio", "-x", "--audio-format", "opus", "--audio-quality", "0"], "opus"),
        "wav" => (vec!["-f", "bestaudio", "-x", "--audio-format", "wav", "--audio-quality", "0"], "wav"),
        "best" => (vec!["-f", "bestaudio/best", "-x"], "m4a"),
        _ => (vec!["-f", "bestaudio[ext=m4a]/bestaudio/best"], "m4a"),
    }
}

pub fn source_from_url(url: &str) -> &'static str {
    if url.contains("soundcloud.com") {
        "soundcloud"
    } else if url.contains("bandcamp.com") {
        "bandcamp"
    } else {
        "youtube"
    }
}

// ---- track parsing ---------------------------------------------------------------------------

const TOPIC_SUFFIX: &str = " - Topic";

pub fn strip_topic_suffix(name: &str) -> String {
    let n = name.trim();
    n.strip_suffix(TOPIC_SUFFIX).map(|s| s.trim().to_string()).unwrap_or_else(|| n.to_string())
}

fn gs<'a>(e: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn pick_youtube_thumbnail(e: &Map<String, Value>, track_id: &str) -> String {
    let thumbs: Vec<&Map<String, Value>> =
        e.get("thumbnails").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_object).collect()).unwrap_or_default();
    let best = thumbs
        .iter()
        .filter(|t| gs(t, "url").is_some() && truthy(t.get("width")))
        .max_by(|a, b| {
            let key = |t: &Map<String, Value>| {
                (t.get("width").and_then(Value::as_f64).unwrap_or(0.0), t.get("preference").and_then(Value::as_f64).unwrap_or(0.0))
            };
            key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(b) = best {
        return gs(b, "url").unwrap_or_default().to_string();
    }
    if let Some(t) = gs(e, "thumbnail") {
        return t.to_string();
    }
    if let Some(last) = thumbs.last() {
        if let Some(u) = gs(last, "url") {
            return u.to_string();
        }
    }
    if !track_id.is_empty() {
        return format!("https://i.ytimg.com/vi/{track_id}/hqdefault.jpg");
    }
    String::new()
}

/// Split "Artist - Title" (also en/em dash): the first dash with spaces on
/// both sides that is not inside brackets. A bare hyphen ("Chase-Master") or
/// a dash in "(Dragon Ball - 2026)" is part of the title, not a separator.
fn split_artist_title(title: &str) -> Option<(String, String)> {
    let t = title.trim();
    let mut depth = 0i32;
    for (i, ch) in t.char_indices() {
        match ch {
            '(' | '[' | '{' | '【' | '「' => depth += 1,
            ')' | ']' | '}' | '】' | '」' => depth = (depth - 1).max(0),
            '-' | '–' | '—' if depth == 0 && i > 0 && t[..i].ends_with(' ') && t[i + ch.len_utf8()..].starts_with(' ') => {
                let a = t[..i].trim();
                let b = t[i + ch.len_utf8()..].trim();
                if !a.is_empty() && !b.is_empty() {
                    return Some((a.to_string(), b.to_string()));
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod split_tests {
    use super::split_artist_title;

    #[test]
    fn splits_only_real_separators() {
        assert_eq!(split_artist_title("Anamanaguchi - Miku"), Some(("Anamanaguchi".into(), "Miku".into())));
        assert_eq!(split_artist_title("Zero the Chase-Master (Eggman Robot Zero)"), None);
        assert_eq!(split_artist_title("What if AI made a Gogeta Song? (Dragon Ball - 2026)"), None);
        assert_eq!(split_artist_title("CUCO – Lover Is a Day"), Some(("CUCO".into(), "Lover Is a Day".into())));
    }
}

/// `_parse_track`: a yt-dlp (or Data API shaped) entry -> client track JSON.
pub fn parse_track(entry: &Value, source: &str) -> Value {
    let empty = Map::new();
    let e = entry.as_object().unwrap_or(&empty);
    let track_id = gs(e, "id").or_else(|| gs(e, "webpage_url_basename")).unwrap_or("").to_string();
    let title = gs(e, "track").or_else(|| gs(e, "title")).or_else(|| gs(e, "fulltitle")).unwrap_or("Unknown Title").to_string();
    let raw_channel = gs(e, "channel").or_else(|| gs(e, "uploader")).unwrap_or("");
    let is_topic = raw_channel.trim().ends_with(TOPIC_SUFFIX);
    let mut artist = gs(e, "artist")
        .or_else(|| gs(e, "uploader"))
        .or_else(|| gs(e, "uploader_id"))
        .or_else(|| gs(e, "channel"))
        .or_else(|| gs(e, "creator"))
        .unwrap_or("Unknown Artist")
        .to_string();
    if is_topic {
        artist = strip_topic_suffix(&artist);
    } else if gs(e, "artist").is_none() {
        if let Some((a, t)) = split_artist_title(&title) {
            if a.chars().count() <= 60 && t.chars().count() >= 2 {
                artist = a;
            }
        }
    }
    let duration = match e.get("duration") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) as i64,
        Some(Value::String(s)) => s.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0),
        _ => 0,
    };
    let thumbnail = if source == "youtube" {
        pick_youtube_thumbnail(e, &track_id)
    } else {
        gs(e, "thumbnail")
            .map(str::to_string)
            .or_else(|| {
                e.get("thumbnails")
                    .and_then(Value::as_array)
                    .and_then(|a| a.last())
                    .and_then(|t| t.get("url"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default()
    };
    let youtube_url = gs(e, "webpage_url")
        .or_else(|| gs(e, "original_url"))
        .map(str::to_string)
        .unwrap_or_else(|| if source == "youtube" { format!("https://youtube.com/watch?v={track_id}") } else { String::new() });
    json!({
        "id": track_id,
        "title": title,
        "artist": artist,
        "duration_seconds": duration,
        "thumbnail_url": thumbnail,
        "source": source,
        "youtube_url": youtube_url,
        "is_topic_channel": is_topic,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn unavailable_is_not_premium_or_sign_in() {
        assert!(super::is_unavailable(b"ERROR: [youtube] ifDJ_aDgIXQ: Video unavailable. This video is not available"));
        assert!(!super::is_unavailable(b"ERROR: This video is only available to YouTube Music Premium members"));
        assert!(!super::is_unavailable(b"ERROR: Sign in to confirm you're not a bot. Video unavailable"));
        let limited = b"ERROR: [youtube] TTSId7Uwtxs: Video unavailable. This content isn't available, try again later. The current session has been rate-limited by YouTube for up to an hour.";
        assert!(!super::is_unavailable(limited));
        assert!(!super::is_permanent_failure(limited));
        assert!(super::is_rate_limited(limited));
        assert!(super::is_permanent_failure(b"ERROR: [youtube] x: Video unavailable. This video has been removed by the uploader"));
    }

    #[test]
    fn age_and_premium_detection() {
        let age = b"WARNING: [youtube] abc: This video is age-restricted and YouTube is requiring account age-verification; some formats may be missing\nERROR: unable to download video data: HTTP Error 403: Forbidden";
        assert!(super::needs_age_fallback(age));
        assert!(!super::is_permanent_failure(age));
        let premium = b"ERROR: [youtube] 83PR0rOtEB0: This video is only available to Music Premium members";
        assert!(super::is_permanent_failure(premium));
        assert!(!super::needs_age_fallback(premium));
        assert!(!super::needs_age_fallback(b"ERROR: unable to download video data: HTTP Error 403: Forbidden"));
    }

    use super::*;

    #[test]
    fn splits_artist_title() {
        let t = parse_track(&json!({"id": "x", "title": "Daft Punk - One More Time", "channel": "DaftPunkVEVO"}), "youtube");
        assert_eq!(t["artist"], "Daft Punk");
        let t = parse_track(&json!({"id": "x", "title": "Song", "channel": "Band - Topic"}), "youtube");
        assert_eq!(t["artist"], "Band");
        assert_eq!(t["is_topic_channel"], true);
        assert_eq!(t["thumbnail_url"], "https://i.ytimg.com/vi/x/hqdefault.jpg");
    }

    #[test]
    fn chain_dedups() {
        assert_eq!(format_flag("m4a"), "bestaudio[ext=m4a]/bestaudio/best");
    }
}
