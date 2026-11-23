//! Played-stream cache: a streamed track's audio, kept by its source id
//! ("youtube:abc") instead of by URL, with a size cap and least-recently-used
//! eviction.
//!
//! Before: the app downloaded each stream into `tmp/stream_<hash(url)>`, the
//! URL carried the rotating stream ticket (so the same track got a new name
//! every half hour), and anything older than an hour was deleted. Replaying a
//! track, going back one, or a repeat-all queue downloaded it again, and the
//! next track couldn't be prepared ahead for gapless / Smart Crossfade.
//!
//! Now `stream_cache_fetch` downloads once (concurrent callers for the same
//! track share one download), names the file by the source id with the real
//! container's extension (from Content-Type), and the engine plays it like a
//! local file. Files live under the app's Caches directory (the system may
//! purge it; that only costs a re-download).

use crate::client::HttpHeader;
use crate::error::{CoreError, CoreResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

struct Config {
    dir: Option<PathBuf>,
    cap: u64,
}

static CFG: LazyLock<Mutex<Config>> = LazyLock::new(|| Mutex::new(Config { dir: None, cap: 1024 * 1024 * 1024 }));
/// One download per source id at a time.
static INFLIGHT: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

const EXTS: [&str; 7] = ["m4a", "webm", "opus", "mp3", "flac", "wav", "ogg"];

/// Every name a cached file can have: `<stem>.<ext>`, or `<stem>.<ext>.lms`
/// for a Lumisound-locked track (the engine unmasks those in place).
fn candidates(d: &Path, s: &str) -> impl Iterator<Item = PathBuf> {
    let d = d.to_path_buf();
    let s = s.to_string();
    EXTS.into_iter().flat_map(move |e| [d.join(format!("{s}.{e}")), d.join(format!("{s}.{e}.lms"))])
}

/// A locked / LMS3 file (by its first bytes).
fn is_locked(p: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).is_ok() && (&head == b"LMSAUDIO" || head.starts_with(b"LMSLOCK"))
}

/// Where cached streams live and how much they may use (call at launch).
#[uniffi::export]
pub fn stream_cache_configure(dir: String, cap_bytes: u64) {
    let dir = PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    let mut c = CFG.lock().unwrap();
    c.dir = Some(dir);
    c.cap = cap_bytes.max(64 * 1024 * 1024);
}

fn dir() -> Option<PathBuf> {
    CFG.lock().unwrap().dir.clone()
}

fn stem(source_key: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(source_key.as_bytes())[..12])
}

fn touch(p: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Cache key for a stream URL: "<source>:<id>" for the bridge's
/// `/api/stream/proxy`, else the URL without its credentials (`ticket`,
/// `token`), so the same track keeps one cache entry.
#[uniffi::export]
pub fn stream_source_key(url: String) -> String {
    let Ok(u) = url::Url::parse(&url) else { return url };
    let q: HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if u.path().ends_with("/api/stream/proxy") {
        if let Some(id) = q.get("id").filter(|i| !i.is_empty()) {
            return format!("{}:{id}", q.get("source").map(String::as_str).unwrap_or("youtube"));
        }
    }
    let mut clean = u.clone();
    let kept: Vec<(String, String)> = u.query_pairs().filter(|(k, _)| k != "ticket" && k != "token").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    clean.set_query(None);
    if !kept.is_empty() {
        clean.query_pairs_mut().extend_pairs(kept);
    }
    clean.to_string()
}

/// The cached file for `source_key`, marked used. None on a miss.
#[uniffi::export]
pub fn stream_cache_lookup(source_key: String) -> Option<String> {
    let d = dir()?;
    let s = stem(&source_key);
    candidates(&d, &s).find(|p| p.metadata().is_ok_and(|m| m.len() > 0)).map(|p| {
        touch(&p);
        p.to_string_lossy().into_owned()
    })
}

/// Drop a cached file (it failed to decode, or the track changed).
#[uniffi::export]
pub fn stream_cache_remove(source_key: String) {
    let Some(d) = dir() else { return };
    let s = stem(&source_key);
    for p in candidates(&d, &s) {
        let _ = std::fs::remove_file(p);
    }
}

/// Bytes used / file count (Settings → Storage).
#[uniffi::export]
pub fn stream_cache_usage() -> Vec<u64> {
    let files = list();
    vec![files.iter().map(|f| f.1).sum(), files.len() as u64]
}

#[uniffi::export]
pub fn stream_cache_clear() {
    for (p, _, _) in list() {
        let _ = std::fs::remove_file(p);
    }
}

fn list() -> Vec<(PathBuf, u64, SystemTime)> {
    let Some(d) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            (m.is_file() && !name.starts_with('.')).then(|| (e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}

/// Least recently used files go until the cache is under its cap; `keep`
/// (the file just stored) always stays.
fn evict(keep: &Path) {
    let cap = CFG.lock().unwrap().cap;
    let mut files = list();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= cap {
        return;
    }
    files.sort_by_key(|f| f.2);
    for (p, len, _) in files {
        if total <= cap {
            break;
        }
        if p != keep && std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
    // Interrupted downloads (".part") older than an hour.
    if let Some(d) = dir() {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a > Duration::from_secs(3600));
                if e.file_name().to_string_lossy().ends_with(".part") && old {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// The container a response carries, from its Content-Type (the bridge may
/// answer m4a for an opus request), else from the URL's `format=` query.
pub fn ext_for(content_type: Option<&str>, url: &str) -> &'static str {
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    let by_ct = if ct.contains("webm") {
        Some("webm")
    } else if ct.contains("ogg") || ct.contains("opus") {
        Some("opus")
    } else if ct.contains("mpeg") {
        Some("mp3")
    } else if ct.contains("flac") {
        Some("flac")
    } else if ct.contains("wav") {
        Some("wav")
    } else if ct.contains("mp4") || ct.contains("m4a") || ct.contains("aac") {
        Some("m4a")
    } else {
        None
    };
    by_ct.unwrap_or_else(|| {
        let q = url::Url::parse(url).ok().and_then(|u| u.query_pairs().find(|(k, _)| k == "format").map(|(_, v)| v.to_ascii_lowercase()));
        match q.as_deref() {
            Some("opus") => "opus",
            Some("mp3") => "mp3",
            Some("flac") => "flac",
            Some("wav") => "wav",
            Some("webm") => "webm",
            _ => "m4a",
        }
    })
}

/// Result of `stream_cache_fetch`.
#[derive(uniffi::Record, Clone, Debug)]
pub struct StreamFile {
    pub path: String,
    /// Served from the cache (no network).
    pub cached: bool,
    pub bytes: u64,
    pub status: u16,
    pub seconds: f64,
    /// The bridge's `X-Loudness-Gain-Db` for a cloud track (fresh downloads only).
    pub loudness_gain_db: Option<f32>,
}

/// The track's audio as a local file: from the cache, else downloaded from
/// `url` (with `headers`) and stored under `source_key`. A non-2xx answer is
/// an `Http` error carrying the status (404/403/401 = don't retry).
#[uniffi::export]
pub async fn stream_cache_fetch(source_key: String, url: String, headers: Vec<HttpHeader>, timeout_secs: f64) -> CoreResult<StreamFile> {
    crate::on_rt(async move { fetch(source_key, url, headers, timeout_secs).await }).await
}

pub(crate) async fn fetch(source_key: String, url: String, headers: Vec<HttpHeader>, timeout_secs: f64) -> CoreResult<StreamFile> {
    let started = std::time::Instant::now();
    if let Some(p) = stream_cache_lookup(source_key.clone()) {
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        return Ok(StreamFile { path: p, cached: true, bytes, status: 200, seconds: 0.0, loudness_gain_db: None });
    }
    let d = dir().ok_or_else(|| CoreError::invalid("stream cache not configured"))?;
    let lock = INFLIGHT.lock().unwrap().entry(source_key.clone()).or_default().clone();
    let _g = lock.lock().await;
    // Another caller may have finished it while this one waited.
    if let Some(p) = stream_cache_lookup(source_key.clone()) {
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        INFLIGHT.lock().unwrap().remove(&source_key);
        return Ok(StreamFile { path: p, cached: true, bytes, status: 200, seconds: started.elapsed().as_secs_f64(), loudness_gain_db: None });
    }
    let part = d.join(format!(".{}.{}.part", stem(&source_key), uuid::Uuid::new_v4().simple()));
    let r = crate::client::http_download_to(&url, &headers, &part.to_string_lossy(), timeout_secs).await;
    let out = match r {
        Ok(dl) if (200..300).contains(&dl.status) && dl.bytes > 2048 => {
            let ct = dl.headers.iter().find(|h| h.name.eq_ignore_ascii_case("content-type")).map(|h| h.value.as_str());
            let ext = ext_for(ct, &url);
            // Locked tracks keep a `.lms` name, which is how the engine
            // knows to unmask them (nothing is ever stored unlocked).
            let suffix = if is_locked(&part) { ".lms" } else { "" };
            let dest = d.join(format!("{}.{ext}{suffix}", stem(&source_key)));
            match std::fs::rename(&part, &dest) {
                Ok(()) => {
                    evict(&dest);
                    let loudness_gain_db = dl.headers.iter().find(|h| h.name.eq_ignore_ascii_case("x-loudness-gain-db")).and_then(|h| h.value.trim().parse::<f32>().ok());
                    Ok(StreamFile { path: dest.to_string_lossy().into_owned(), cached: false, bytes: dl.bytes, status: dl.status, seconds: started.elapsed().as_secs_f64(), loudness_gain_db })
                }
                Err(e) => Err(CoreError::from(e)),
            }
        }
        Ok(dl) => {
            let body = std::fs::read(&part).unwrap_or_default();
            Err(crate::client::error_for(dl.status, &body))
        }
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&part);
    INFLIGHT.lock().unwrap().remove(&source_key);
    out
}

/// Download the queued tracks' audio ahead of time (next 1-2 tracks), so
/// they start instantly and can be prepared for gapless / Smart Crossfade.
/// Fire and forget; failures are ignored (playback will try again).
#[uniffi::export]
pub fn stream_cache_prefetch(items: Vec<StreamPrefetch>) {
    for it in items.into_iter().take(3) {
        if stream_cache_lookup(it.source_key.clone()).is_some() {
            continue;
        }
        crate::RT.spawn(async move {
            let _ = fetch(it.source_key, it.url, it.headers, 180.0).await;
        });
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct StreamPrefetch {
    pub source_key: String,
    pub url: String,
    pub headers: Vec<HttpHeader>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_ignore_credentials() {
        assert_eq!(stream_source_key("https://b.x/api/stream/proxy?id=abc&source=youtube&format=m4a&ticket=T1".into()), "youtube:abc");
        assert_eq!(stream_source_key("https://b.x/api/stream/proxy?id=abc&format=m4a".into()), "youtube:abc");
        assert_eq!(
            stream_source_key("https://b.x/user/music/stream?path=A%20B.mp3&ticket=T2".into()),
            stream_source_key("https://b.x/user/music/stream?path=A%20B.mp3&ticket=T9".into())
        );
    }

    #[test]
    fn extension_from_content_type_then_query() {
        assert_eq!(ext_for(Some("audio/webm"), "https://b/api/stream/proxy?format=m4a"), "webm");
        assert_eq!(ext_for(Some("audio/mp4"), "https://b/x?format=opus"), "m4a");
        assert_eq!(ext_for(None, "https://b/x?id=1&format=flac"), "flac");
        assert_eq!(ext_for(Some("application/octet-stream"), "https://b/x"), "m4a");
    }

    #[test]
    fn fetch_caches_by_source_key_and_evicts() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = std::env::temp_dir().join(format!("lms-sc-{}", uuid::Uuid::new_v4().simple()));
        stream_cache_configure(dir.to_string_lossy().into_owned(), 0);
        rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let h2 = hits.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                loop {
                    let (mut s, _) = l.accept().await.unwrap();
                    h2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut buf = [0u8; 2048];
                    let _ = s.read(&mut buf).await;
                    let body = vec![1u8; 40_000];
                    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: audio/webm\r\n\r\n", body.len()).as_bytes()).await;
                    let _ = s.write_all(&body).await;
                }
            });
            let url = format!("http://127.0.0.1:{port}/api/stream/proxy?ticket=a&format=m4a");
            let (a, b) = tokio::join!(fetch("youtube:x".into(), url.clone(), vec![], 10.0), fetch("youtube:x".into(), url.replace("ticket=a", "ticket=b"), vec![], 10.0));
            let (a, b) = (a.unwrap(), b.unwrap());
            assert_eq!(a.path, b.path);
            assert!(a.path.ends_with(".webm"));
            assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1, "concurrent fetches share one download");
            let c = fetch("youtube:x".into(), url, vec![], 10.0).await.unwrap();
            assert!(c.cached);
            assert_eq!(stream_cache_lookup("youtube:x".into()), Some(a.path.clone()));
            stream_cache_remove("youtube:x".into());
            assert!(stream_cache_lookup("youtube:x".into()).is_none());
        });
        let _ = std::fs::remove_dir_all(dir);
    }
}
