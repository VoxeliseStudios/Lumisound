//! YouTube Premium detection for an account's cookies.
//!
//! Premium accounts get YouTube Music's ~256 kbps Opus (format 774) on top
//! of the ~130 kbps everyone gets. Whether a cookie file is Premium is found
//! by asking yt-dlp for the formats of a YouTube Music track with those
//! cookies, per client (which client serves 774 has moved around between
//! yt-dlp releases, so the first that does is remembered and used for that
//! account's downloads). Results are kept per user + cookie-file hash in
//! `ios_ytdlp_premium` so a restart doesn't re-probe, and re-checked after
//! a day or when the cookies change.

use crate::state::AppState;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// YouTube Music songs ("Topic"/Art Track uploads always carry 774 for
/// Premium). Several, in case one is pulled.
const PROBE_TRACKS: &[&str] = &["4D7u5KF7SP8", "JhulBGMA7G4", "Rgrt_8mXrK8"];
/// Clients tried in order; the first that lists a Premium format wins.
const CLIENTS: &[&str] = &["web_music", "web_creator", "tv", "web,default"];
const RECHECK_SECS: i64 = 24 * 3600;
pub const PREMIUM_FORMATS: &[&str] = &["774", "141"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub premium: bool,
    /// Best audio bitrate the cookies were offered (kbps).
    pub best_kbps: u32,
    /// yt-dlp player_client that serves the Premium formats.
    pub client: Option<String>,
    pub checked_at: Option<String>,
    pub has_cookies: bool,
}

impl Status {
    pub fn json(&self) -> Value {
        json!({"premium": self.premium, "best_kbps": self.best_kbps, "client": self.client, "checked_at": self.checked_at, "has_cookies": self.has_cookies,
               "max_kbps": if self.premium { 256 } else { 130 }})
    }
}

/// Hash of the cookie file yt-dlp would use for `user` (None = no cookies).
async fn cookie_fingerprint(st: &AppState, user: &str) -> Option<(String, String)> {
    let args = st.ytdlp.cookie_args(Some(user), false).await;
    let i = args.iter().position(|a| a == "--cookies")?;
    let path = args.get(i + 1)?.clone();
    let bytes = tokio::fs::read(&path).await.ok()?;
    // Only the cookie lines matter (yt-dlp rewrites comments/order).
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty() && l[0] != b'#').collect();
    lines.sort();
    let mut h = Sha256::new();
    for l in lines {
        h.update(l);
    }
    Some((format!("{:x}", h.finalize()), path))
}

/// Premium formats/bitrate in one yt-dlp `-J` dump.
fn best_audio(info: &Value) -> (bool, u32) {
    let fmts = info["formats"].as_array().cloned().unwrap_or_default();
    let mut premium = false;
    let mut best = 0f64;
    for f in &fmts {
        let id = f["format_id"].as_str().unwrap_or("");
        let audio_only = f["vcodec"].as_str() == Some("none") || f["video_ext"].as_str() == Some("none");
        if !audio_only {
            continue;
        }
        if PREMIUM_FORMATS.contains(&id) || id.starts_with("774") {
            premium = true;
        }
        if let Some(a) = f["abr"].as_f64().or_else(|| f["tbr"].as_f64()) {
            best = best.max(a);
        }
    }
    (premium || best >= 200.0, best.round() as u32)
}

async fn probe(st: &AppState, user: &str) -> Status {
    let cookies = st.ytdlp.cookie_args(Some(user), false).await;
    let mut status = Status { has_cookies: cookies.iter().any(|a| a == "--cookies"), ..Default::default() };
    if !status.has_cookies {
        return status;
    }
    'clients: for client in CLIENTS {
        for vid in PROBE_TRACKS {
            let mut a: Vec<String> = st.ytdlp.network().to_vec();
            a.extend(["--extractor-args".into(), format!("youtube:player_client={client}"), "-J".into(), "--no-playlist".into(), "--skip-download".into()]);
            a.extend(cookies.iter().cloned());
            a.push(format!("https://music.youtube.com/watch?v={vid}"));
            let out = {
                let _slot = st.ytdlp.limiter.slot().await;
                st.ytdlp.run("yt-dlp", &a, Duration::from_secs(45)).await
            };
            let Ok(out) = out else { continue };
            let Ok(info) = serde_json::from_slice::<Value>(&out.stdout) else { continue };
            let (premium, kbps) = best_audio(&info);
            status.best_kbps = status.best_kbps.max(kbps);
            if premium {
                status.premium = true;
                status.client = Some((*client).to_string());
                break 'clients;
            }
            // This client answered for this track: try the next client.
            continue 'clients;
        }
    }
    status
}

/// Cached status; probes (and stores) when there is none for the current
/// cookies or it is a day old. `force` re-probes.
pub async fn status(st: &AppState, user: &str, force: bool) -> Status {
    let Some((hash, _)) = cookie_fingerprint(st, user).await else {
        return Status::default();
    };
    if !force {
        type Row = (bool, i32, Option<String>, Option<String>, Option<i64>);
        let row: Option<Row> = sqlx::query_as(
            "SELECT premium, best_kbps, client, lumi_iso(checked_at), EXTRACT(EPOCH FROM (NOW() - checked_at))::BIGINT \
             FROM ios_ytdlp_premium WHERE user_id = $1 AND cookie_hash = $2",
        )
        .bind(user)
        .bind(&hash)
        .fetch_optional(&st.db)
        .await
        .unwrap_or(None);
        if let Some((premium, kbps, client, at, _)) = row.filter(|r| r.4.unwrap_or(i64::MAX) < RECHECK_SECS) {
            return Status { premium, best_kbps: kbps.max(0) as u32, client, checked_at: at, has_cookies: true };
        }
    }
    let mut s = probe(st, user).await;
    let r = sqlx::query(
        "INSERT INTO ios_ytdlp_premium (user_id, cookie_hash, premium, best_kbps, client, checked_at) VALUES ($1, $2, $3, $4, $5, NOW()) \
         ON CONFLICT (user_id) DO UPDATE SET cookie_hash = EXCLUDED.cookie_hash, premium = EXCLUDED.premium, best_kbps = EXCLUDED.best_kbps, \
           client = EXCLUDED.client, checked_at = NOW() RETURNING lumi_iso(checked_at)",
    )
    .bind(user)
    .bind(&hash)
    .bind(s.premium)
    .bind(s.best_kbps as i32)
    .bind(&s.client)
    .fetch_one(&st.db)
    .await;
    match r {
        Ok(row) => s.checked_at = sqlx::Row::try_get(&row, 0).ok(),
        Err(e) => tracing::warn!("premium status not stored for {user}: {e}"),
    }
    tracing::info!("YouTube Premium check for {user}: premium={} best={} kbps client={:?}", s.premium, s.best_kbps, s.client);
    s
}

/// Stored status only (no probing): what downloads use, so a download never
/// waits on a Premium check.
pub async fn cached(st: &AppState, user: Option<&str>) -> Option<Status> {
    let user = user?;
    let row: Option<(bool, i32, Option<String>)> =
        sqlx::query_as("SELECT premium, best_kbps, client FROM ios_ytdlp_premium WHERE user_id = $1").bind(user).fetch_optional(&st.db).await.unwrap_or(None);
    let (premium, kbps, client) = row?;
    Some(Status { premium, best_kbps: kbps.max(0) as u32, client, checked_at: None, has_cookies: true })
}

/// Forget the stored status (cookies changed or removed) and re-probe in
/// the background.
pub fn refresh_later(st: &AppState, user: &str) {
    let (st, user) = (st.clone(), user.to_string());
    tokio::spawn(async move {
        let _ = sqlx::query("DELETE FROM ios_ytdlp_premium WHERE user_id = $1").bind(&user).execute(&st.db).await;
        // Give the cookie file a moment to be rewritten from the database.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = status(&st, &user, true).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn premium_from_formats() {
        let free = json!({"formats": [{"format_id": "140", "vcodec": "none", "abr": 129.5}, {"format_id": "251", "vcodec": "none", "abr": 135.0}, {"format_id": "18", "vcodec": "avc1", "tbr": 500.0}]});
        assert_eq!(best_audio(&free), (false, 135));
        let prem = json!({"formats": [{"format_id": "251", "vcodec": "none", "abr": 135.0}, {"format_id": "774", "vcodec": "none", "abr": 256.4}]});
        assert_eq!(best_audio(&prem), (true, 256));
    }
}
