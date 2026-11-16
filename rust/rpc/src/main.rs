//! Lumisound Discord Rich Presence daemon. Polls the bridge for the account's
//! playback state and mirrors it into Discord through the local IPC socket
//! (named pipe on Windows). `lumisound-discord-rpc login` performs the
//! one-time sign-in and stores a long-lived Rich Presence token.

mod ipc;
mod live;
mod login;

use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const DEFAULT_BRIDGE_URL: &str = "https://lumisound-bridge.xenusanimations.studio";
const STALE_AFTER_SECS: f64 = 120.0;
const MIN_SEND_INTERVAL_SECS: f64 = 4.5;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// While live pushes flow, a slow poll still refreshes `updated_at` (so a
/// long track isn't judged stale) and catches anything a push missed.
const LIVE_POLL_SECS: u64 = 30;
const RELEASES_API: &str = "https://api.github.com/repos/VoxeliseStudios/Lumisound/releases?per_page=50";

pub fn log(msg: &str) {
    println!("{}  {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
}

/// Same lookup order as the Python daemon so existing installs keep working.
pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUMISOUND_RPC_CONFIG") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
    let legacy = home.join(".config").join("lumisound-discord-rpc").join("config.json");
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let win = PathBuf::from(appdata).join("lumisound-discord-rpc").join("config.json");
            return if legacy.exists() && !win.exists() { legacy } else { win };
        }
        return legacy;
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("lumisound-discord-rpc").join("config.json");
    }
    legacy
}

pub fn write_config(path: &PathBuf, config: &Value) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config).unwrap_or_default() + "\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

struct Bridge {
    base: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl Bridge {
    fn get(&self, path: &str) -> Result<Value, String> {
        let mut rq = self.http.get(format!("{}{path}", self.base)).header("User-Agent", format!("lumisound-discord-rpc/{VERSION}"));
        if let Some(t) = &self.token {
            rq = rq.bearer_auth(t);
        }
        let r = rq.send().map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        if status == 401 {
            return Err("401: the Rich Presence token was revoked or expired — run `lumisound-discord-rpc login`".into());
        }
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        r.json().map_err(|e| e.to_string())
    }
}

fn sampled_at(state: &Value) -> Option<f64> {
    let raw = state["updated_at"].as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw).ok().map(|d| d.timestamp_millis() as f64 / 1000.0)
}

fn now() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

/// YouTube video id from an `i.ytimg.com` / `img.youtube.com` thumbnail URL.
fn youtube_thumb_id(url: &str) -> Option<&str> {
    let rest = ["https://i.ytimg.com/vi/", "https://img.youtube.com/vi/", "https://i.ytimg.com/vi_webp/"].iter().find_map(|p| url.strip_prefix(p))?;
    let id = rest.split('/').next()?;
    (id.len() == 11 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')).then_some(id)
}

/// The art URL Discord should show. Discord draws presence art square and
/// crops anything else, so a YouTube thumbnail is swapped for the bridge's
/// shaped copy (`/api/rp-art/yt/{id}.jpg`): the full 16:9 frame over a
/// blurred fill instead of the middle third of it. Bridge-hosted art
/// (`/api/rp-artwork/...`) is already shaped when it is uploaded.
pub fn presence_art(art: &str, bridge_base: &str) -> String {
    match youtube_thumb_id(art) {
        Some(id) if !bridge_base.is_empty() => format!("{bridge_base}/api/rp-art/yt/{id}.jpg"),
        _ => art.to_string(),
    }
}

/// Third line of the card (Discord shows the large image's text there). It
/// used to repeat the title; now it says how the track is being played, e.g.
/// "Rust engine · LMS OPUS · 48 kHz · 124 BPM".
fn info_line(state: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    match state["engine"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(e) => parts.push(e.to_string()),
        None => parts.push("Lumisound".into()),
    }
    if let Some(bpm) = state["bpm"].as_f64().filter(|b| b.is_finite() && *b > 0.0) {
        let tag = format!("{} BPM", bpm.round() as i64);
        if !parts.iter().any(|p| p.contains("BPM")) {
            parts.push(tag);
        }
    }
    parts.join(" · ")
}

/// What the member list shows after "Listening to" (Discord's
/// `status_display_type`): the app name, the artist, or the song title.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StatusDisplay {
    App,
    Artist,
    Title,
}

impl StatusDisplay {
    pub fn from_config(v: Option<&str>) -> Self {
        match v.map(str::to_ascii_lowercase).as_deref() {
            Some("app") | Some("name") | Some("lumisound") => StatusDisplay::App,
            Some("artist") | Some("state") => StatusDisplay::Artist,
            _ => StatusDisplay::Title,
        }
    }
}

pub struct CardOpts<'a> {
    pub large_image: Option<&'a str>,
    pub small_image: Option<&'a str>,
    pub show_buttons: bool,
    pub show_when_paused: bool,
    pub status_display: StatusDisplay,
    /// Newer activity fields (`status_display_type`, `details_url`). Turned
    /// off for the session if this Discord client rejects them.
    pub rich_fields: bool,
}

/// Fields older Discord clients reject; dropped when they do.
const RICH_FIELDS: [&str; 2] = ["status_display_type", "details_url"];

fn without_rich_fields(mut a: Value) -> Value {
    if let Some(o) = a.as_object_mut() {
        for k in RICH_FIELDS {
            o.remove(k);
        }
    }
    a
}

/// Discord activity for a playback state, or None to clear it.
pub fn build_activity(state: &Value, bridge_base: &str, o: &CardOpts) -> Option<Value> {
    let (large_image, small_image, show_buttons, show_when_paused) = (o.large_image, o.small_image, o.show_buttons, o.show_when_paused);
    let title = state["title"].as_str().map(str::trim).filter(|t| !t.is_empty())?;
    let playing = state["is_playing"].as_bool().unwrap_or(false);
    if !playing && !show_when_paused {
        return None;
    }
    let sampled = sampled_at(state);
    if sampled.is_some_and(|s| now() - s > STALE_AFTER_SECS) {
        return None;
    }
    // Discord wants 2..=128 characters per text field.
    let cut = |s: &str| {
        let t: String = s.chars().take(128).collect();
        if t.chars().count() < 2 { format!("{t} ") } else { t }
    };
    let mut a = json!({"type": 2, "details": cut(title)});
    if let Some(artist) = state["artist"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        a["state"] = json!(cut(&format!("by {artist}")));
    }
    if playing {
        let pos = state["position_seconds"].as_f64().unwrap_or(0.0).max(0.0);
        let start = sampled.unwrap_or_else(now) - pos;
        let mut ts = json!({"start": (start * 1000.0).round() as i64});
        // With an end as well, Discord draws the progress bar and both times
        // for a Listening activity, matching the app's own scrubber.
        if let Some(dur) = state["duration_seconds"].as_f64().filter(|d| d.is_finite() && *d > pos && *d > 1.0) {
            ts["end"] = json!(((start + dur) * 1000.0).round() as i64);
        }
        a["timestamps"] = ts;
    }
    let art = state["artwork_url"].as_str().filter(|s| s.starts_with("https://") || s.starts_with("http://")).map(|s| presence_art(s, bridge_base));
    let mut assets = serde_json::Map::new();
    if let Some(img) = art.as_deref().or(large_image) {
        assets.insert("large_image".into(), json!(img));
        assets.insert("large_text".into(), json!(cut(&info_line(state))));
    }
    if let (Some(small), Some(_)) = (small_image, art.as_deref()) {
        assets.insert("small_image".into(), json!(small));
        assets.insert("small_text".into(), json!(if playing { "Playing on Lumisound" } else { "Paused" }));
    }
    if !assets.is_empty() {
        a["assets"] = Value::Object(assets);
    }
    let track_url = state["track_url"].as_str().filter(|s| s.starts_with("https://") && s.len() <= 256);
    if show_buttons
        && let Some(url) = track_url {
            a["buttons"] = json!([{"label": "Listen", "url": url}]);
        }
    if o.rich_fields {
        match o.status_display {
            StatusDisplay::App => {}
            StatusDisplay::Artist if a.get("state").is_some() => a["status_display_type"] = json!(1),
            _ => a["status_display_type"] = json!(2),
        }
        // The song title links to the track, like Spotify's card.
        if let Some(url) = track_url {
            a["details_url"] = json!(url);
        }
    }
    Some(a)
}

/// What decides whether an activity must be re-sent right away: any text or
/// art change, play/pause, or a timeline jump (seek) of more than 1.5 s.
fn needs_send(prev: Option<&Value>, next: Option<&Value>) -> bool {
    match (prev, next) {
        (None, None) => false,
        (Some(p), Some(n)) => {
            let strip = |v: &Value| {
                let mut v = v.clone();
                v.as_object_mut().map(|o| o.remove("timestamps"));
                v
            };
            if strip(p) != strip(n) {
                return true;
            }
            let ms = |v: &Value, k: &str| v["timestamps"][k].as_i64();
            let jump = |k: &str| match (ms(p, k), ms(n, k)) {
                (Some(a), Some(b)) => (a - b).abs() > 1500,
                (a, b) => a.is_some() != b.is_some(),
            };
            jump("start") || jump("end")
        }
        _ => true,
    }
}

/// Newest `rpc/v*` release on GitHub, if newer than this build.
fn newer_release() -> Option<String> {
    let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).build().ok()?;
    let v: Value = http.get(RELEASES_API).header("User-Agent", format!("lumisound-discord-rpc/{VERSION}")).send().ok()?.json().ok()?;
    let tag = v.as_array()?.iter().filter_map(|r| r["tag_name"].as_str()).find(|t| t.starts_with("rpc/v"))?;
    let parse = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    (parse(tag.trim_start_matches("rpc/v")) > parse(VERSION)).then(|| tag.to_string())
}

fn spawn_update_check() {
    let _ = std::thread::Builder::new().name("lumisound-update-check".into()).spawn(|| loop {
        if let Some(tag) = newer_release() {
            log(&format!("Update available: {} (you have v{VERSION}) — https://github.com/VoxeliseStudios/Lumisound/releases/tag/{tag}", tag.trim_start_matches("rpc/")));
        }
        std::thread::sleep(Duration::from_secs(24 * 3600));
    });
}

/// Keep the newer of two playback states (by `updated_at`).
fn newer(current: Option<Value>, incoming: Value) -> Option<Value> {
    match (current.as_ref().and_then(sampled_at), sampled_at(&incoming)) {
        (Some(a), Some(b)) if b + 0.001 < a => current,
        _ => Some(incoming),
    }
}

fn run() -> Result<(), String> {
    let path = default_config_path();
    let raw = std::fs::read_to_string(&path).map_err(|_| format!("No config at {} — run `lumisound-discord-rpc login` first.", path.display()))?;
    let config: Value = serde_json::from_str(&raw).map_err(|e| format!("config.json is not valid JSON: {e}"))?;
    let bridge = Bridge {
        base: config["bridge_url"].as_str().unwrap_or(DEFAULT_BRIDGE_URL).trim_end_matches('/').to_string(),
        token: config["access_token"].as_str().map(str::to_string),
        http: reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?,
    };
    let Some(token) = bridge.token.clone() else {
        return Err("config.json has no access_token — run `lumisound-discord-rpc login`.".into());
    };
    if config["check_updates"].as_bool() != Some(false) {
        spawn_update_check();
    }
    let mut rpc = loop {
        match bridge.get("/user/discord-rpc-config") {
            Ok(v) => break v,
            Err(e) => {
                log(&format!("bridge not ready ({e}); retrying in 10s"));
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    };
    let pick = |k: &str, rpc: &Value| config[k].as_str().or(rpc[k].as_str()).map(str::to_string);
    let mut client_id = pick("discord_client_id", &rpc).ok_or("no Discord client id (set discord_client_id)")?;
    let poll = Duration::from_secs_f64(config["poll_interval_seconds"].as_f64().unwrap_or(1.0).max(0.5));
    let status_display = StatusDisplay::from_config(config["status_display"].as_str());
    let mut rich_fields = config["discord_new_fields"].as_bool().unwrap_or(true);
    let mut conn = ipc::Ipc::new(&client_id);
    let (mut last_activity, mut last_sent) = (None::<Value>, 0.0f64);
    let mut last_rpc_check = Instant::now();
    // Live pushes (instant); polling covers gaps and older bridges.
    let (tx, rx) = std::sync::mpsc::channel();
    let live_up = if config["live_updates"].as_bool() != Some(false) { live::spawn(&bridge.base, &token, tx) } else { std::sync::Arc::new(Default::default()) };
    let mut state: Option<Value> = None;
    let mut next_poll = Instant::now();
    // Discord isn't always running: retry its socket with backoff instead of
    // scanning every candidate path each second.
    let (mut ipc_backoff, mut next_ipc_try, mut waiting_logged) = (Duration::from_secs(1), Instant::now(), false);
    log(&format!("Lumisound Rich Presence v{VERSION} running (bridge {})", bridge.base));
    loop {
        match rx.recv_timeout(poll) {
            Ok(live::Event::State(s)) => state = newer(state.take(), s),
            Ok(live::Event::Unauthorized) | Err(_) => {}
        }
        while let Ok(ev) = rx.try_recv() {
            if let live::Event::State(s) = ev {
                state = newer(state.take(), s);
            }
        }
        if last_rpc_check.elapsed() > Duration::from_secs(300) {
            if let Ok(n) = bridge.get("/user/discord-rpc-config") {
                if let Some(id) = pick("discord_client_id", &n).filter(|id| *id != client_id) {
                    log("Discord client id changed; reconnecting");
                    client_id = id;
                    conn.close();
                    conn = ipc::Ipc::new(&client_id);
                }
                rpc = n;
            }
            last_rpc_check = Instant::now();
        }
        if rpc["enabled"].as_bool() == Some(false) {
            if conn.connected() {
                let _ = conn.set_activity(None);
                conn.close();
            }
            continue;
        }
        if Instant::now() >= next_poll {
            let live = live_up.load(std::sync::atomic::Ordering::Relaxed);
            match bridge.get("/user/playback-state") {
                Ok(s) => {
                    state = newer(state.take(), s);
                    next_poll = Instant::now() + if live { Duration::from_secs(LIVE_POLL_SECS) } else { poll };
                }
                Err(e) => {
                    log(&format!("bridge request failed ({e}); retrying"));
                    next_poll = Instant::now() + poll.max(Duration::from_secs(5));
                }
            }
        }
        if !conn.connected() {
            if Instant::now() < next_ipc_try {
                continue;
            }
            if conn.connect().is_err() {
                if !waiting_logged {
                    log("Waiting for Discord to start");
                    waiting_logged = true;
                }
                next_ipc_try = Instant::now() + ipc_backoff;
                ipc_backoff = (ipc_backoff * 2).min(Duration::from_secs(30));
                continue;
            }
            (ipc_backoff, waiting_logged) = (Duration::from_secs(1), false);
            // A fresh connection has no activity yet.
            last_activity = None;
        }
        let Some(st) = state.as_ref() else { continue };
        let (large, small) = (pick("large_image", &rpc), pick("small_image", &rpc));
        let opts = CardOpts {
            large_image: large.as_deref(),
            small_image: small.as_deref(),
            show_buttons: config["show_buttons"].as_bool().or(rpc["show_buttons"].as_bool()).unwrap_or(true),
            show_when_paused: config["show_when_paused"].as_bool().unwrap_or(false),
            status_display,
            rich_fields,
        };
        let activity = build_activity(st, &bridge.base, &opts);
        let t = now();
        // Changes go out at once (Discord allows 5 updates per 20 s, and a
        // track change or seek is rare enough); otherwise refresh every few
        // seconds so a restarted Discord client picks the card back up.
        let changed = needs_send(last_activity.as_ref(), activity.as_ref());
        if changed || (activity.is_some() && t - last_sent >= MIN_SEND_INTERVAL_SECS * 3.0) {
            let mut r = conn.set_activity(activity.clone());
            if let (Err(ipc::SetError::Rejected(m)), true) = (&r, rich_fields) {
                log(&format!("Discord refused the newer card fields ({m}); sending the classic card"));
                rich_fields = false;
                r = conn.set_activity(activity.clone().map(without_rich_fields));
            }
            match r {
                Ok(()) => {
                    last_sent = t;
                    last_activity = activity;
                }
                Err(ipc::SetError::Rejected(m)) => {
                    log(&format!("Discord rejected the activity ({m})"));
                    last_sent = t;
                    last_activity = activity;
                }
                Err(e) => {
                    log(&format!("Discord IPC write failed ({e}); reconnecting"));
                    conn.close();
                }
            }
        }
    }
}

fn main() {
    // rustls (live socket) needs a process-wide crypto provider; reqwest
    // already links ring, so use that one.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("login") => login::main(&args[1..]),
        Some("--version") | Some("-V") => {
            println!("lumisound-discord-rpc {VERSION}");
            Ok(())
        }
        Some("--help") | Some("-h") => {
            println!(
                "lumisound-discord-rpc            run the Rich Presence daemon\n\
                 lumisound-discord-rpc login [--bridge-url URL] [--token TOKEN] [--identifier EMAIL_OR_USERNAME]\n\
                 lumisound-discord-rpc --version\n\n\
                 config.json options: status_display (\"title\" | \"artist\" | \"app\"), show_when_paused, show_buttons,\n\
                 live_updates (default true), check_updates (default true), discord_new_fields (default true)"
            );
            Ok(())
        }
        _ => run(),
    };
    if let Err(e) = r {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts<'a>(large: Option<&'a str>, small: Option<&'a str>, rich: bool) -> CardOpts<'a> {
        CardOpts { large_image: large, small_image: small, show_buttons: true, show_when_paused: false, status_display: StatusDisplay::Title, rich_fields: rich }
    }

    #[test]
    fn activity_shape() {
        let st = json!({"title": "Song", "artist": "Band", "is_playing": true, "position_seconds": 10.0, "duration_seconds": 200.0,
            "artwork_url": "https://a/b.jpg", "track_url": "https://x", "engine": "Rust engine · LMS OPUS · 48 kHz", "bpm": 123.6});
        let a = build_activity(&st, "https://br", &opts(Some("lumisound"), Some("playpause-small"), false)).unwrap();
        assert_eq!(a["state"], "by Band");
        assert_eq!(a["assets"]["large_image"], "https://a/b.jpg");
        // The third line is engine info, never the title again.
        assert_eq!(a["assets"]["large_text"], "Rust engine · LMS OPUS · 48 kHz · 124 BPM");
        assert_ne!(a["assets"]["large_text"], a["details"]);
        assert_eq!(a["assets"]["small_text"], "Playing on Lumisound");
        assert_eq!(a["buttons"][0]["label"], "Listen");
        let ts = &a["timestamps"];
        assert_eq!(ts["end"].as_i64().unwrap() - ts["start"].as_i64().unwrap(), 200_000);
        assert!(build_activity(&json!({"title": "S", "is_playing": false}), "", &opts(None, None, false)).is_none());
        let stale = json!({"title": "S", "is_playing": true, "updated_at": "2020-01-01T00:00:00+00:00"});
        assert!(build_activity(&stale, "", &opts(None, None, false)).is_none());
        // No engine reported (older app): still not the title.
        let old = json!({"title": "Song", "is_playing": true, "artwork_url": "https://a/b.jpg"});
        assert_eq!(build_activity(&old, "", &opts(None, None, false)).unwrap()["assets"]["large_text"], "Lumisound");
    }

    #[test]
    fn youtube_art_goes_through_bridge() {
        assert_eq!(presence_art("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg", "https://br"), "https://br/api/rp-art/yt/dQw4w9WgXcQ.jpg");
        assert_eq!(presence_art("https://br/api/rp-artwork/ab.jpg", "https://br"), "https://br/api/rp-artwork/ab.jpg");
        assert_eq!(presence_art("https://i.ytimg.com/vi/bad/hq.jpg", "https://br"), "https://i.ytimg.com/vi/bad/hq.jpg");
    }

    #[test]
    fn resend_rules() {
        let a = json!({"details": "S", "timestamps": {"start": 1000, "end": 5000}});
        let jitter = json!({"details": "S", "timestamps": {"start": 1400, "end": 5400}});
        let seek = json!({"details": "S", "timestamps": {"start": 30000, "end": 34000}});
        assert!(!needs_send(Some(&a), Some(&jitter)));
        assert!(needs_send(Some(&a), Some(&seek)));
        assert!(needs_send(Some(&a), None));
        assert!(needs_send(Some(&a), Some(&json!({"details": "T", "timestamps": {"start": 1000, "end": 5000}}))));
    }

    #[test]
    fn rich_fields_and_fallback() {
        let st = json!({"title": "Song", "artist": "Band", "is_playing": true, "track_url": "https://music.youtube.com/watch?v=x"});
        let a = build_activity(&st, "", &opts(None, None, true)).unwrap();
        assert_eq!(a["status_display_type"], 2);
        assert_eq!(a["details_url"], "https://music.youtube.com/watch?v=x");
        let mut o = opts(None, None, true);
        o.status_display = StatusDisplay::Artist;
        assert_eq!(build_activity(&st, "", &o).unwrap()["status_display_type"], 1);
        o.status_display = StatusDisplay::App;
        assert!(build_activity(&st, "", &o).unwrap().get("status_display_type").is_none());
        let classic = without_rich_fields(a);
        assert!(classic.get("status_display_type").is_none() && classic.get("details_url").is_none());
        assert_eq!(classic["details"], "Song");
        assert_eq!(StatusDisplay::from_config(Some("Artist")), StatusDisplay::Artist);
        assert_eq!(StatusDisplay::from_config(None), StatusDisplay::Title);
    }

    #[test]
    fn keeps_the_newest_state() {
        let old = json!({"title": "A", "updated_at": "2026-11-16T08:00:00.000+00:00"});
        let new = json!({"title": "B", "updated_at": "2026-11-16T08:00:05.000+00:00"});
        assert_eq!(newer(Some(new.clone()), old.clone()).unwrap()["title"], "B");
        assert_eq!(newer(Some(old), new).unwrap()["title"], "B");
        assert_eq!(newer(None, json!({"title": "C"})).unwrap()["title"], "C");
    }
}
