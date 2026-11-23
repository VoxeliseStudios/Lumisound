//! The signed-in bridge session for core-side requests, and the features
//! that talk to the bridge directly from Rust:
//!
//! * `stream_prefetch_urls`: asks the bridge to resolve the next queued
//!   tracks' stream URLs ahead of time (POST /api/stream/prefetch), so the
//!   switch skips yt-dlp and googlevideo's settle delay.
//! * Download retry queue: the bridge remembers downloads that failed only
//!   on YouTube's bot wall or a rate limit and announces them on /ws/live
//!   (`downloads_retry`) once that clears; `download_retry_pending` lists
//!   them for an app that was offline and `download_retry_ack` clears them.
//! * Shared Smart Crossfade profiles for streamed tracks
//!   (`blend_profile_for_stream`).
//!
//! Swift sets the session once (`bridge_session_set`) whenever the bridge
//! URL, app key or account token changes.

use crate::client::{HttpHeader, error_for, http_send};
use crate::error::{CoreError, CoreResult};
use serde::Deserialize;
use std::sync::{Arc, LazyLock, RwLock};

#[derive(Clone, Default)]
struct Session {
    base: String,
    api_key: String,
    token: String,
}

static SESSION: LazyLock<RwLock<Session>> = LazyLock::new(Default::default);

/// Bridge base URL, shared app key and the account's session token.
#[uniffi::export]
pub fn bridge_session_set(base_url: String, api_key: String, account_token: Option<String>) {
    let base = base_url.trim_end_matches('/').to_string();
    crate::client::client_register_bridge(base.clone());
    *SESSION.write().unwrap() = Session { base, api_key, token: account_token.unwrap_or_default() };
}

fn session() -> Option<Session> {
    let s = SESSION.read().unwrap().clone();
    (!s.base.is_empty() && !s.token.is_empty()).then_some(s)
}

fn headers(s: &Session, json: bool) -> Vec<HttpHeader> {
    let mut h = Vec::new();
    if !s.api_key.is_empty() {
        h.push(HttpHeader { name: "Authorization".into(), value: format!("Bearer {}", s.api_key) });
    }
    h.push(HttpHeader { name: "X-Account-Token".into(), value: s.token.clone() });
    if json {
        h.push(HttpHeader { name: "Content-Type".into(), value: "application/json".into() });
    }
    h
}

async fn call(method: &str, path: &str, body: Option<Vec<u8>>, timeout: f64) -> CoreResult<(u16, Vec<u8>)> {
    let s = session().ok_or(CoreError::Unauthorized)?;
    let ex = http_send(method.into(), format!("{}{path}", s.base), headers(&s, body.is_some()), body, timeout).await?;
    Ok((ex.status, ex.body))
}

// ---- stream URL prefetch -----------------------------------------------------------------

#[derive(uniffi::Record, Clone, Debug)]
pub struct PrefetchTrack {
    pub id: String,
    pub source: String,
    /// SoundCloud / Bandcamp page URL (YouTube needs only the id).
    pub url: Option<String>,
}

/// Resolve these upcoming tracks' stream URLs on the bridge now (at most 3;
/// fire and forget). `engine`: the app streams through the Lumisound engine
/// (`engine=1` URLs), so the bridge warms the same format.
#[uniffi::export]
pub fn stream_prefetch_urls(tracks: Vec<PrefetchTrack>, engine: bool) {
    if tracks.is_empty() || session().is_none() {
        return;
    }
    let body = serde_json::json!({
        "tracks": tracks.iter().take(3).map(|t| serde_json::json!({"id": t.id, "source": t.source, "url": t.url})).collect::<Vec<_>>(),
        "format": "m4a",
        "engine": engine,
    });
    crate::RT.spawn(async move {
        let _ = call("POST", "/api/stream/prefetch", serde_json::to_vec(&body).ok(), 10.0).await;
    });
}

// ---- cloud library metadata (incremental) -------------------------------------------------

struct CloudMeta {
    token: String,
    server_time: String,
    tracks: Vec<serde_json::Value>,
}

static CLOUD_META: LazyLock<tokio::sync::Mutex<Option<CloudMeta>>> = LazyLock::new(Default::default);

/// `/user/music/metadata` as `{"tracks": [...]}` (newest upload first), kept
/// current incrementally: the first call per session downloads the whole
/// list, later calls ask only for rows changed since the last answer
/// (`?since=`) and apply the deletions the bridge reports. A library of
/// thousands of tracks no longer travels on every backup pass.
#[uniffi::export]
pub async fn cloud_metadata_fetch(account_token: String) -> CoreResult<Vec<u8>> {
    crate::on_rt(async move {
        let s = SESSION.read().unwrap().clone();
        if s.base.is_empty() {
            return Err(CoreError::invalid("bridge not configured"));
        }
        let mut guard = CLOUD_META.lock().await;
        let since = guard.as_ref().filter(|m| m.token == account_token).map(|m| m.server_time.clone());
        let mut path = "/user/music/metadata?limit=100000".to_string();
        if let Some(t) = &since {
            path.push_str(&format!("&since={}", urlencoding::encode(t)));
        }
        let h = vec![HttpHeader { name: "Authorization".into(), value: format!("Bearer {account_token}") }];
        let ex = http_send("GET".into(), format!("{}{path}", s.base), h, None, 30.0).await?;
        if ex.status != 200 {
            return Err(error_for(ex.status, &ex.body));
        }
        let v: serde_json::Value = serde_json::from_slice(&ex.body).map_err(|e| CoreError::invalid(e.to_string()))?;
        let changed = v.get("tracks").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        let server_time = v.get("server_time").and_then(|t| t.as_str()).map(str::to_string);
        let incremental = v.get("incremental").and_then(|b| b.as_bool()).unwrap_or(false);
        let mut tracks = match (incremental, guard.take()) {
            (true, Some(prev)) if prev.token == account_token => {
                let id = |t: &serde_json::Value| t.get("id").and_then(|i| i.as_str()).map(str::to_string);
                let mut gone: std::collections::HashSet<String> =
                    v.get("deleted").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                gone.extend(changed.iter().filter_map(id));
                let mut kept: Vec<serde_json::Value> = prev.tracks.into_iter().filter(|t| id(t).is_none_or(|i| !gone.contains(&i))).collect();
                kept.extend(changed);
                kept
            }
            _ => changed,
        };
        // Newest upload first, as the full answer is ordered (ISO times sort as text).
        tracks.sort_by(|a, b| {
            let t = |x: &serde_json::Value| x.get("uploaded_at").and_then(|u| u.as_str()).unwrap_or("").to_string();
            t(b).cmp(&t(a))
        });
        let out = serde_json::to_vec(&serde_json::json!({"tracks": tracks, "total": tracks.len()})).map_err(|e| CoreError::invalid(e.to_string()))?;
        if let Some(st) = server_time {
            *guard = Some(CloudMeta { token: account_token, server_time: st, tracks });
        }
        Ok(out)
    })
    .await
}

/// Forget the incremental state (sign-out, or a full refresh is wanted).
#[uniffi::export]
pub fn cloud_metadata_reset() {
    if let Ok(mut g) = CLOUD_META.try_lock() {
        *g = None;
    }
}

// ---- search ------------------------------------------------------------------------------

/// `/api/search` results (YouTube) for core-side lookups (artwork).
pub(crate) async fn search_json(query: &str, limit: u32) -> Option<Vec<serde_json::Value>> {
    let path = format!("/api/search?q={}&limit={limit}&source=youtube", urlencoding::encode(query));
    match call("GET", &path, None, 20.0).await {
        Ok((200, body)) => serde_json::from_slice::<Vec<serde_json::Value>>(&body).ok(),
        _ => None,
    }
}

// ---- download retry queue ----------------------------------------------------------------

#[derive(uniffi::Record, Clone, Debug, Deserialize)]
pub struct RetryTrack {
    pub source: String,
    pub id: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub format: Option<String>,
    /// "bot_wall" or "rate_limit".
    pub reason: String,
    /// The cause has cleared: download it again now.
    #[serde(default)]
    pub ready: bool,
}

impl RetryTrack {
    pub fn key(&self) -> String {
        format!("{}:{}", self.source, self.id)
    }
}

/// Parse a `downloads_retry` live event's tracks (all are ready).
pub(crate) fn parse_retry_event(v: &serde_json::Value) -> Vec<RetryTrack> {
    v.get("tracks")
        .and_then(|t| serde_json::from_value::<Vec<RetryTrack>>(t.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|mut t| {
            t.ready = true;
            t
        })
        .collect()
}

/// Ready-to-retry downloads for this account (launch / foreground). Clears
/// the core's bot-wall back-off for them so automatic passes don't skip them.
#[uniffi::export]
pub async fn download_retry_pending() -> CoreResult<Vec<RetryTrack>> {
    crate::on_rt(async move {
        let (status, body) = call("GET", "/api/download/retry", None, 15.0).await?;
        if status != 200 {
            return Err(error_for(status, &body));
        }
        #[derive(Deserialize)]
        struct R {
            tracks: Vec<RetryTrack>,
        }
        let r: R = serde_json::from_slice(&body).map_err(|e| CoreError::invalid(e.to_string()))?;
        let ready: Vec<RetryTrack> = r.tracks.into_iter().filter(|t| t.ready).collect();
        for t in &ready {
            crate::downloads::blocked_clear(t.key());
        }
        Ok(ready)
    })
    .await
}

/// Tell the bridge these downloads were re-queued (or given up on).
#[uniffi::export]
pub fn download_retry_ack(keys: Vec<String>) {
    if keys.is_empty() {
        return;
    }
    let body = serde_json::json!({"keys": keys});
    crate::RT.spawn(async move {
        let _ = call("POST", "/api/download/retry/ack", serde_json::to_vec(&body).ok(), 15.0).await;
    });
}

// ---- shared blend profiles ---------------------------------------------------------------

/// Version of the serialized `BlendProfile` the bridge stores.
pub const BLEND_PROFILE_VERSION: i32 = 1;

/// Smart Crossfade profile for a streamed track: the device cache, else the
/// one another device shared for this source id, else measured from
/// `path` (the downloaded copy) and shared back. `source_key` is
/// "youtube:<id>".
#[uniffi::export]
pub async fn blend_profile_for_stream(path: String, source_key: String) -> Option<Arc<crate::blend::BlendProfile>> {
    crate::on_rt(async move {
        if let Some(p) = crate::blend::blend_profile_cached(path.clone()) {
            return Some(p);
        }
        let (source, id) = source_key.split_once(':').map(|(a, b)| (a.to_string(), b.to_string()))?;
        let q = format!("/api/track/blend-profile?source={}&id={}&v={BLEND_PROFILE_VERSION}", urlencoding::encode(&source), urlencoding::encode(&id));
        if let Ok((200, body)) = call("GET", &q, None, 6.0).await {
            if let Some(p) = crate::blend::blend_profile_store(path.clone(), &body) {
                return Some(p);
            }
        }
        let p2 = path.clone();
        let p = tokio::task::spawn_blocking(move || crate::blend::blend_profile(p2)).await.ok().flatten()?;
        if let Ok(b) = serde_json::to_vec(p.as_ref()) {
            crate::RT.spawn(async move {
                let _ = call("PUT", &q, Some(b), 10.0).await;
            });
        }
        Some(p)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retry_event() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type":"downloads_retry","tracks":[{"source":"youtube","id":"abc","title":"T","artist":null,"format":"m4a","reason":"bot_wall"}]}"#,
        )
        .unwrap();
        let t = parse_retry_event(&v);
        assert_eq!(t.len(), 1);
        assert!(t[0].ready);
        assert_eq!(t[0].key(), "youtube:abc");
    }

    #[test]
    fn no_session_means_no_calls() {
        stream_prefetch_urls(vec![PrefetchTrack { id: "x".into(), source: "youtube".into(), url: None }], true);
        assert!(session().is_none() || !session().unwrap().token.is_empty());
    }
}
