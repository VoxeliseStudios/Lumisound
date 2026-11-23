//! Where a song's artwork comes from, in order, and the network part of
//! getting it — ported from the Swift `ArtworkService.loadArtwork` chain.
//!
//! `artwork_resolve_local` tries the track's own sources:
//! 1. its remote thumbnail URL (streamed tracks),
//! 2. the cover embedded in the file (any container; locked files in place),
//! 3. the `LUMISOUND_THUMBNAIL` URL the bridge tagged it with,
//! 4. YouTube's `hqdefault.jpg` for a `youtube:` source id.
//!
//! `artwork_search` is the last resort by title + artist: a YouTube search
//! through the bridge (only a result whose title overlaps the song's), then
//! the iTunes Search API.
//!
//! Both return the picture already processed (`artwork_process_full`: bars
//! trimmed, short side capped, JPEG), ready to cache. Swift keeps only the
//! Apple-only sources (Media Library items, a video's first frame) and the
//! UIImage caches.
//!
//! `artwork_no_art_*`: a 5-minute negative cache, so a track with no art
//! anywhere doesn't hit four services on every scroll.

use crate::client::HttpHeader;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(uniffi::Record, Clone, Debug)]
pub struct ArtworkQuery {
    /// Streamed tracks: their thumbnail URL.
    pub remote_url: Option<String>,
    /// Headers for `remote_url` (bridge-hosted art needs auth).
    pub headers: Vec<HttpHeader>,
    /// Local file (read for embedded art and the thumbnail tag).
    pub path: Option<String>,
    /// Where locked files may be unlocked for tag reads.
    pub unlock_dir: String,
    /// "youtube:<id>" etc.
    pub source_track_id: Option<String>,
    /// Short-side cap of the processed picture (px).
    pub max_short: u32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ArtworkFound {
    /// Processed JPEG.
    pub data: Vec<u8>,
    /// remote / embedded / thumbnail_tag / youtube_hq / youtube_search / itunes
    pub source: String,
}

async fn fetch_image(url: &str, headers: &[HttpHeader]) -> Option<Vec<u8>> {
    let ex = crate::client::http_send("GET".into(), url.to_string(), headers.to_vec(), None, 20.0).await.ok()?;
    (ex.status == 200 && ex.body.len() > 100).then_some(ex.body)
}

fn processed(bytes: Vec<u8>, max_short: u32, source: &str) -> Option<ArtworkFound> {
    let data = lumisound_media::artwork::process_full(&bytes, max_short.max(16))?;
    Some(ArtworkFound { data, source: source.into() })
}

/// The track's own artwork sources (see module doc), first hit wins.
#[uniffi::export]
pub async fn artwork_resolve_local(q: ArtworkQuery) -> Option<ArtworkFound> {
    crate::on_rt(async move {
        if let Some(u) = q.remote_url.as_deref().filter(|u| u.starts_with("http")) {
            if let Some(b) = fetch_image(u, &q.headers).await {
                if let Some(f) = processed(b, q.max_short, "remote") {
                    return Some(f);
                }
            }
        }
        if let Some(path) = q.path.clone() {
            let (p, d) = (path.clone(), q.unlock_dir.clone());
            let embedded = tokio::task::spawn_blocking(move || crate::library::artwork_embedded(p, d)).await.ok().flatten();
            if let Some(f) = embedded.and_then(|b| processed(b, q.max_short, "embedded")) {
                return Some(f);
            }
            let p = path.clone();
            let thumb = tokio::task::spawn_blocking(move || crate::downloads::tags_read_lumisound(p).thumbnail_url).await.ok().flatten();
            if let Some(t) = thumb.filter(|t| t.starts_with("http")) {
                if let Some(f) = fetch_image(&t, &[]).await.and_then(|b| processed(b, q.max_short, "thumbnail_tag")) {
                    return Some(f);
                }
            }
        }
        if let Some(vid) = q.source_track_id.as_deref().and_then(|s| s.strip_prefix("youtube:")).filter(|v| !v.is_empty()) {
            let u = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", urlencoding::encode(vid));
            if let Some(f) = fetch_image(&u, &[]).await.and_then(|b| processed(b, q.max_short, "youtube_hq")) {
                return Some(f);
            }
        }
        None
    })
    .await
}

/// A search result's title is about the same song: one normalized title
/// contains the other. A plain YouTube search often ranks covers, reactions
/// or same-titled songs first.
pub fn titles_overlap(wanted: &str, got: &str) -> bool {
    let (a, b) = (crate::duplicates::dup_normalize(wanted.to_string()), crate::duplicates::dup_normalize(got.to_string()));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// Last resorts by title + artist: YouTube search via the bridge (needs the
/// core's bridge session), then iTunes Search.
#[uniffi::export]
pub async fn artwork_search(title: String, artist: String, max_short: u32) -> Option<ArtworkFound> {
    crate::on_rt(async move {
        let query = [title.trim(), artist.trim()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
        if query.is_empty() {
            return None;
        }
        if let Some(results) = crate::bridge::search_json(&query, 5).await {
            let hit = results.iter().find(|r| r.get("title").and_then(|t| t.as_str()).is_some_and(|t| titles_overlap(&title, t)));
            let thumb = hit.and_then(|r| r.get("thumbnail_url").or_else(|| r.get("thumbnail")).and_then(|t| t.as_str())).filter(|t| t.starts_with("http"));
            if let Some(t) = thumb {
                if let Some(f) = fetch_image(t, &[]).await.and_then(|b| processed(b, max_short, "youtube_search")) {
                    return Some(f);
                }
            }
        }
        let url = format!("https://itunes.apple.com/search?term={}&entity=song&limit=1&country=US", urlencoding::encode(&query));
        let ex = crate::client::http_send("GET".into(), url, vec![], None, 15.0).await.ok()?;
        if ex.status != 200 {
            return None;
        }
        let v: serde_json::Value = serde_json::from_slice(&ex.body).ok()?;
        let first = v.get("results")?.as_array()?.first()?;
        let art = first
            .get("artworkUrl600")
            .and_then(|a| a.as_str())
            .map(str::to_string)
            .or_else(|| first.get("artworkUrl100").and_then(|a| a.as_str()).map(|a| a.replace("100x100bb", "600x600bb")))?;
        fetch_image(&art, &[]).await.and_then(|b| processed(b, max_short, "itunes"))
    })
    .await
}

static NO_ART: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);
const NO_ART_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// Still inside the no-artwork cooldown for `key`.
#[uniffi::export]
pub fn artwork_no_art_check(key: String) -> bool {
    let mut m = NO_ART.lock().unwrap();
    match m.get(&key) {
        Some(t) if t.elapsed() < NO_ART_COOLDOWN => true,
        Some(_) => {
            m.remove(&key);
            false
        }
        None => false,
    }
}

/// Every source came up empty for `key`: skip it for 5 minutes.
#[uniffi::export]
pub fn artwork_no_art_mark(key: String) {
    let mut m = NO_ART.lock().unwrap();
    if m.len() > 20_000 {
        m.retain(|_, t| t.elapsed() < NO_ART_COOLDOWN);
    }
    m.insert(key, Instant::now());
}

#[uniffi::export]
pub fn artwork_no_art_clear() {
    NO_ART.lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_guard() {
        assert!(titles_overlap("Blinding Lights", "The Weeknd - Blinding Lights (Official Video)"));
        assert!(!titles_overlap("Blinding Lights", "Levitating"));
        assert!(!titles_overlap("", "x"));
    }

    #[test]
    fn negative_cache() {
        artwork_no_art_mark("k1".into());
        assert!(artwork_no_art_check("k1".into()));
        assert!(!artwork_no_art_check("k2".into()));
        artwork_no_art_clear();
        assert!(!artwork_no_art_check("k1".into()));
    }
}
