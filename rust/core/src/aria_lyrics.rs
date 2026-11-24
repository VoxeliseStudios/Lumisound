//! Aria Lyrics, app side: "Generate with Aria" for every app.
//!
//! The bridge runs the engine (vocal separation → finding the sung phrases →
//! on-server speech recognition → aligning to known lyrics) as a job. This
//! drives one job end to end — upload (or point at a cloud file), follow its
//! progress, turn the result into something to preview — so the apps only
//! draw it.
//!
//! Why it lives here: the Swift client polled the job and treated any status
//! other than "pending"/"done" as a failure. The engine reports "running"
//! (with a stage) the moment it starts, so every run "failed" three seconds
//! in with "unknown error" while the bridge carried on transcribing.

use crate::client::{HttpHeader, http_send, http_upload_file};
use crate::error::{CoreError, CoreResult};
use crate::lyrics::{SyncedLine, format_lrc, parse_lrc};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const START: &str = "/user/intelligence/lyrics-transcribe";
const STATUS: &str = "/user/intelligence/lyrics-transcribe/status";
/// Hint text rides in the query string; Cloudflare refuses URLs past ~16 KB.
const HINT_MAX_CHARS: usize = 2500;
const POLL: Duration = Duration::from_millis(1500);
const DEADLINE: Duration = Duration::from_secs(20 * 60);
/// Share of the bar for the upload; the bridge's own progress fills the rest.
const UPLOAD_SHARE: f64 = 0.05;

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AriaLyricsStage {
    Uploading,
    Queued,
    Separating,
    Listening,
    Aligning,
    Finishing,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AriaLyricsProgress {
    pub stage: AriaLyricsStage,
    /// Checklist position: 0 waiting/uploading, 1 separating, 2 listening,
    /// 3 timing, 4 finishing.
    pub step: u32,
    /// Whole run, 0…1.
    pub fraction: f64,
    /// "Separating the vocals".
    pub title: String,
    /// One line under the title.
    pub detail: String,
    /// The words Aria recognized most recently, while she listens.
    pub heard: Option<String>,
    pub elapsed_secs: u64,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AriaLyricLine {
    /// None for untimed lyrics.
    pub time_seconds: Option<f64>,
    /// "1:05" for the preview.
    pub time_label: Option<String>,
    pub text: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AriaLyricsResult {
    pub lines: Vec<AriaLyricLine>,
    /// LRC text when `synced`.
    pub lrc: String,
    /// The words, one per line (always filled when there are lines).
    pub plain: String,
    pub synced: bool,
    pub instrumental: bool,
    /// "high" / "medium" / "low".
    pub confidence: String,
    /// "English", "Japanese", … when the engine detected it.
    pub language: Option<String>,
    /// Share of known-lyrics lines Aria could place on the music (0…1), when
    /// she aligned to known text rather than writing it from scratch.
    pub aligned: Option<f64>,
    pub elapsed_secs: u64,
    /// One-line summary for the result card ("Synced · 38 lines · high confidence").
    pub headline: String,
}

impl AriaLyricsResult {
    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[uniffi::export(with_foreign)]
pub trait AriaLyricsObserver: Send + Sync {
    fn on_progress(&self, progress: AriaLyricsProgress);
}

/// One "Generate with Aria" run. Create it, call `run_upload` (a file on this
/// device) or `run_cloud` (a file in the cloud library), and `cancel` to stop.
#[derive(uniffi::Object)]
pub struct AriaLyricsJob {
    base: String,
    token: String,
    cancelled: AtomicBool,
    job_id: Mutex<Option<String>>,
}

fn stage_of(s: &str) -> AriaLyricsStage {
    match s {
        "separating" => AriaLyricsStage::Separating,
        "listening" => AriaLyricsStage::Listening,
        "aligning" => AriaLyricsStage::Aligning,
        _ => AriaLyricsStage::Queued,
    }
}

fn stage_text(stage: AriaLyricsStage, has_hint: bool) -> (&'static str, &'static str) {
    match stage {
        AriaLyricsStage::Uploading => ("Sending the track to Aria", "Uploading the audio"),
        AriaLyricsStage::Queued => ("Waiting for Aria", "She's finishing another song first"),
        AriaLyricsStage::Separating => ("Separating the vocals", "Lifting the voice out of the mix"),
        AriaLyricsStage::Listening => ("Listening", "Writing down every line she hears"),
        AriaLyricsStage::Aligning if has_hint => ("Timing every line", "Matching the known words to the music"),
        AriaLyricsStage::Aligning => ("Timing every line", "Breaking the words into lines on the beat"),
        AriaLyricsStage::Finishing => ("Finishing up", "Putting the lyrics together"),
    }
}

fn language_name(code: &str) -> String {
    match code {
        "en" => "English", "es" => "Spanish", "fr" => "French", "de" => "German", "it" => "Italian", "pt" => "Portuguese",
        "ja" => "Japanese", "ko" => "Korean", "zh" => "Chinese", "ru" => "Russian", "nl" => "Dutch", "sv" => "Swedish",
        "pl" => "Polish", "tr" => "Turkish", "ar" => "Arabic", "hi" => "Hindi", "id" => "Indonesian", "tl" => "Filipino",
        other => return other.to_uppercase(),
    }
    .to_string()
}

fn time_label(t: f64) -> String {
    let s = t.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The cloud-library path behind a bridge stream URL
/// (".../user/music/stream?path=…"), for `run_cloud`; None for anything else.
#[uniffi::export]
pub fn aria_lyrics_cloud_path(url: String) -> Option<String> {
    let u = url::Url::parse(&url).ok()?;
    if !u.path().ends_with("/user/music/stream") {
        return None;
    }
    u.query_pairs().find(|(k, _)| k == "path").map(|(_, v)| v.into_owned()).filter(|p| !p.is_empty())
}

fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "mp3" => "audio/mpeg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "aiff" | "aif" => "audio/aiff",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    }
}

fn http_err(status: u16, body: &[u8]) -> CoreError {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let detail = v["detail"].as_str().map(str::to_string);
    match status {
        401 => CoreError::Unauthorized,
        429 => CoreError::Http { status, message: detail.unwrap_or_else(|| "Aria is busy with other tracks right now. Try again shortly.".into()) },
        413 => CoreError::Http { status, message: "This file is too big for Aria to listen to (20 MB max).".into() },
        _ => CoreError::Http { status, message: detail.unwrap_or_else(|| format!("Aria couldn't start (HTTP {status}).")) },
    }
}

fn cancelled() -> CoreError {
    CoreError::Network { message: "Cancelled".into(), timed_out: false, cancelled: true }
}

/// Builds the app-facing result from a finished job's status payload.
pub(crate) fn result_from(v: &Value, elapsed_secs: u64) -> AriaLyricsResult {
    let lrc = v["lrc"].as_str().unwrap_or("").trim().to_string();
    let plain_in = v["plain"].as_str().unwrap_or("").trim().to_string();
    let instrumental = v["instrumental"].as_bool().unwrap_or(false);
    let timed: Vec<SyncedLine> = if lrc.is_empty() { vec![] } else { parse_lrc(&lrc) };
    let synced = v["synced"].as_bool().unwrap_or(true) && !timed.is_empty();
    let lines: Vec<AriaLyricLine> = if synced {
        timed.iter().map(|l| AriaLyricLine { time_seconds: Some(l.time), time_label: Some(time_label(l.time)), text: l.text.clone() }).collect()
    } else {
        plain_in.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| AriaLyricLine { time_seconds: None, time_label: None, text: l.to_string() }).collect()
    };
    let plain = lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    let confidence = v["confidence"].as_str().unwrap_or("low").to_string();
    let language = v["language"].as_str().filter(|s| !s.is_empty()).map(language_name);
    let aligned = v["aligned"].as_f64();
    let headline = if instrumental {
        "Instrumental — Aria found no vocals".to_string()
    } else if lines.is_empty() {
        "Aria couldn't make out clear lyrics".to_string()
    } else {
        let kind = if synced { "Synced" } else { "Untimed" };
        let mut h = format!("{kind} · {} line{}", lines.len(), if lines.len() == 1 { "" } else { "s" });
        if let Some(l) = &language {
            h.push_str(&format!(" · {l}"));
        }
        h.push_str(&format!(" · {confidence} confidence"));
        h
    };
    AriaLyricsResult { lrc: if synced { format_lrc(&timed) } else { String::new() }, lines, plain, synced, instrumental, confidence, language, aligned, elapsed_secs, headline }
}

impl AriaLyricsJob {
    fn url(&self, path: &str, query: &[(&str, &str)]) -> String {
        let q: Vec<String> = query.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={}", urlencoding::encode(v))).collect();
        let base = self.base.trim_end_matches('/');
        if q.is_empty() { format!("{base}{path}") } else { format!("{base}{path}?{}", q.join("&")) }
    }

    fn auth(&self) -> Vec<HttpHeader> {
        vec![HttpHeader { name: "Authorization".into(), value: format!("Bearer {}", self.token) }]
    }

    fn report(&self, observer: &Arc<dyn AriaLyricsObserver>, stage: AriaLyricsStage, fraction: f64, heard: Option<String>, has_hint: bool, started: Instant) {
        let (title, detail) = stage_text(stage, has_hint);
        let step = match stage {
            AriaLyricsStage::Uploading | AriaLyricsStage::Queued => 0,
            AriaLyricsStage::Separating => 1,
            AriaLyricsStage::Listening => 2,
            AriaLyricsStage::Aligning => 3,
            AriaLyricsStage::Finishing => 4,
        };
        observer.on_progress(AriaLyricsProgress {
            stage,
            step,
            fraction: fraction.clamp(0.0, 1.0),
            title: title.into(),
            detail: detail.into(),
            heard,
            elapsed_secs: started.elapsed().as_secs(),
        });
    }

    async fn follow(self: Arc<Self>, job_id: String, observer: Arc<dyn AriaLyricsObserver>, has_hint: bool, started: Instant) -> CoreResult<AriaLyricsResult> {
        *self.job_id.lock().unwrap() = Some(job_id.clone());
        let url = self.url(STATUS, &[("job_id", &job_id)]);
        let mut misses = 0u32;
        let mut last_fraction = UPLOAD_SHARE;
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            if started.elapsed() > DEADLINE {
                return Err(CoreError::Network { message: "Aria took too long on this track.".into(), timed_out: true, cancelled: false });
            }
            let ex = match http_send("GET".into(), url.clone(), self.auth(), None, 20.0).await {
                Ok(ex) => ex,
                Err(e) if matches!(e, CoreError::Network { .. }) && misses < 10 => {
                    // A dropped poll is not a failed job: keep following it.
                    misses += 1;
                    tokio::time::sleep(POLL * 2).await;
                    continue;
                }
                Err(e) => return Err(e),
            };
            misses = 0;
            match ex.status {
                200 => {}
                404 => {
                    return Err(CoreError::Http { status: 404, message: "Aria lost track of this job (the server restarted). Try again.".into() });
                }
                s => return Err(http_err(s, &ex.body)),
            }
            let v: Value = serde_json::from_slice(&ex.body).unwrap_or(Value::Null);
            match v["status"].as_str().unwrap_or("") {
                "done" => {
                    self.report(&observer, AriaLyricsStage::Finishing, 1.0, None, has_hint, started);
                    return Ok(result_from(&v, started.elapsed().as_secs()));
                }
                "error" | "failed" => {
                    let msg = v["detail"].as_str().unwrap_or("Aria couldn't transcribe this track.").to_string();
                    return Err(CoreError::Http { status: 500, message: msg });
                }
                "cancelled" => return Err(cancelled()),
                // "pending" / "running" (and anything newer): still working.
                _ => {
                    let stage = stage_of(v["stage"].as_str().unwrap_or(""));
                    let server = v["progress"].as_f64().unwrap_or(0.0);
                    // Never step backwards (a queued job reports 0 again).
                    last_fraction = last_fraction.max(UPLOAD_SHARE + (1.0 - UPLOAD_SHARE) * server);
                    let heard = v["heard"].as_str().map(str::to_string);
                    self.report(&observer, stage, last_fraction, heard, has_hint, started);
                }
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

#[uniffi::export]
impl AriaLyricsJob {
    #[uniffi::constructor]
    pub fn new(base_url: String, token: String) -> Arc<Self> {
        Arc::new(Self { base: base_url, token, cancelled: AtomicBool::new(false), job_id: Mutex::new(None) })
    }

    /// Generate lyrics for a file on this device (uploaded to the bridge).
    /// `hint` is any untimed text already on hand: Aria aligns to it.
    pub async fn run_upload(self: Arc<Self>, file_path: String, title: String, artist: String, hint: Option<String>, observer: Arc<dyn AriaLyricsObserver>) -> CoreResult<AriaLyricsResult> {
        crate::on_rt(async move {
            let started = Instant::now();
            let hint: String = hint.unwrap_or_default().chars().take(HINT_MAX_CHARS).collect();
            let has_hint = hint.lines().filter(|l| !l.trim().is_empty()).count() >= 3;
            self.report(&observer, AriaLyricsStage::Uploading, 0.0, None, has_hint, started);
            let url = self.url(START, &[("title", &title), ("artist", &artist), ("hint_lyrics", &hint)]);
            let mut headers = self.auth();
            headers.push(HttpHeader { name: "Content-Type".into(), value: content_type_for(&file_path).into() });
            let ex = http_upload_file("POST".into(), url, headers, file_path, 120.0).await?;
            if ex.status != 202 {
                return Err(http_err(ex.status, &ex.body));
            }
            let v: Value = serde_json::from_slice(&ex.body).unwrap_or(Value::Null);
            let id = v["job_id"].as_str().ok_or_else(|| CoreError::invalid("The bridge didn't return a job."))?.to_string();
            self.report(&observer, AriaLyricsStage::Queued, UPLOAD_SHARE, None, has_hint, started);
            self.follow(id, observer, has_hint, started).await
        })
        .await
    }

    /// Generate lyrics for a file already in the cloud library (`path` as the
    /// library lists it): nothing is uploaded.
    pub async fn run_cloud(self: Arc<Self>, cloud_path: String, title: String, artist: String, hint: Option<String>, observer: Arc<dyn AriaLyricsObserver>) -> CoreResult<AriaLyricsResult> {
        crate::on_rt(async move {
            let started = Instant::now();
            let hint: String = hint.unwrap_or_default().chars().take(HINT_MAX_CHARS).collect();
            let has_hint = hint.lines().filter(|l| !l.trim().is_empty()).count() >= 3;
            self.report(&observer, AriaLyricsStage::Queued, UPLOAD_SHARE, None, has_hint, started);
            let url = self.url(START, &[("title", &title), ("artist", &artist), ("hint_lyrics", &hint), ("path", &cloud_path)]);
            let ex = http_send("POST".into(), url, self.auth(), None, 30.0).await?;
            if ex.status != 202 {
                return Err(http_err(ex.status, &ex.body));
            }
            let v: Value = serde_json::from_slice(&ex.body).unwrap_or(Value::Null);
            let id = v["job_id"].as_str().ok_or_else(|| CoreError::invalid("The bridge didn't return a job."))?.to_string();
            self.follow(id, observer, has_hint, started).await
        })
        .await
    }

    /// Stops following the job and asks the bridge to stop it too.
    pub async fn cancel(self: Arc<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        let id = self.job_id.lock().unwrap().clone();
        if let Some(id) = id {
            let url = self.url(STATUS, &[("job_id", &id)]);
            let _ = crate::on_rt(async move { http_send("DELETE".into(), url, self.auth(), None, 10.0).await }).await;
        }
    }

    /// "Use these lyrics": stores them for the account (every device sees
    /// them; replaces lyrics this user stored for the track before).
    pub async fn share(self: Arc<Self>, title: String, artist: String, result: AriaLyricsResult) -> CoreResult<()> {
        crate::on_rt(async move {
            let body = serde_json::json!({
                "title": title, "artist": artist, "replace": true,
                "synced_lyrics": if result.synced { Some(result.lrc.clone()) } else { None },
                "plain_lyrics": result.plain,
            });
            let mut headers = self.auth();
            headers.push(HttpHeader { name: "Content-Type".into(), value: "application/json".into() });
            let ex = http_send("POST".into(), self.url("/user/lyrics", &[]), headers, Some(serde_json::to_vec(&body).unwrap_or_default()), 20.0).await?;
            if (200..300).contains(&ex.status) { Ok(()) } else { Err(http_err(ex.status, &ex.body)) }
        })
        .await
    }
}

/// Saves an accepted result next to the track: synced lyrics to
/// `synced_path` (and the untimed file removed, so it can't shadow them),
/// untimed lyrics to `plain_path` only — untrustworthy timing must never land
/// in the synced file, which outranks every lyrics database. Returns whether
/// the saved lyrics are synced.
#[uniffi::export]
pub fn aria_lyrics_save(result: AriaLyricsResult, synced_path: String, plain_path: String) -> CoreResult<bool> {
    if result.is_empty() {
        return Err(CoreError::invalid("There are no lyrics to save."));
    }
    let write = |p: &str, text: &str| -> CoreResult<()> {
        if let Some(dir) = std::path::Path::new(p).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = format!("{p}.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    };
    if result.synced && !result.lrc.is_empty() {
        write(&synced_path, &result.lrc)?;
        let _ = std::fs::remove_file(&plain_path);
        Ok(true)
    } else {
        write(&plain_path, &result.plain)?;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn synced_result_parses_lines_and_headline() {
        let r = result_from(&json!({"status": "done", "lrc": "[00:12.50]First line\n[00:15.00]Second line", "synced": true, "confidence": "high", "language": "en"}), 40);
        assert!(r.synced);
        assert_eq!(r.lines.len(), 2);
        assert_eq!(r.lines[0].time_seconds, Some(12.5));
        assert_eq!(r.plain, "First line\nSecond line");
        assert_eq!(r.headline, "Synced · 2 lines · English · high confidence");
    }

    #[test]
    fn untimed_and_instrumental_results() {
        let r = result_from(&json!({"status": "done", "lrc": "", "plain": "a\n\nb\n", "synced": false, "confidence": "low"}), 1);
        assert!(!r.synced);
        assert_eq!(r.lines.iter().map(|l| l.time_seconds).collect::<Vec<_>>(), vec![None, None]);
        assert!(r.headline.starts_with("Untimed · 2 lines"));
        let i = result_from(&json!({"status": "done", "lrc": "", "instrumental": true}), 1);
        assert!(i.instrumental && i.lines.is_empty());
    }

    #[test]
    fn save_picks_the_right_file() {
        let dir = std::env::temp_dir().join(format!("aria-save-{}", std::process::id()));
        let (s, p) = (dir.join("x.lrc").to_string_lossy().into_owned(), dir.join("x.txt").to_string_lossy().into_owned());
        let synced = result_from(&json!({"lrc": "[00:01.00]Hi", "synced": true}), 0);
        let plain = result_from(&json!({"plain": "Hi", "synced": false}), 0);
        assert!(!aria_lyrics_save(plain, s.clone(), p.clone()).unwrap());
        assert!(std::path::Path::new(&p).exists());
        assert!(aria_lyrics_save(synced, s.clone(), p.clone()).unwrap());
        assert!(std::path::Path::new(&s).exists() && !std::path::Path::new(&p).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cloud_path_from_stream_url() {
        assert_eq!(aria_lyrics_cloud_path("https://b.example/user/music/stream?path=L%26L%20Song.opus.lms&ticket=x".into()).as_deref(), Some("L&L Song.opus.lms"));
        assert_eq!(aria_lyrics_cloud_path("file:///var/a.mp3".into()), None);
        assert_eq!(time_label(65.4), "1:05");
    }

    #[test]
    fn query_values_are_escaped() {
        let j = AriaLyricsJob::new("https://b.example/".into(), "t".into());
        assert_eq!(j.url(START, &[("title", "R&B #1"), ("artist", "")]), "https://b.example/user/intelligence/lyrics-transcribe?title=R%26B%20%231");
    }
}
