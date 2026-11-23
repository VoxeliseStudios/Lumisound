//! Cookie health and download recovery.
//!
//! - Watches the shared YouTube cookie jars: when the bridge has had no
//!   success with the shared jar for a while it runs one cheap probe, and
//!   when YouTube walls it (bot check) it tells the operator once per 6 h
//!   (bug-report webhook + `cookies_botwall` event; the stale sentinel for
//!   the host refresher is written by `Ytdlp::note_jar_botwall`).
//! - Keeps the downloads that failed only on a bot wall or rate limit
//!   (`ios_download_retry`), and once that cause has cleared marks them
//!   ready and tells the account's apps over `/ws/live` (`downloads_retry`),
//!   which re-queue them. GET /api/download/retry lists them for apps that
//!   were offline; POST /api/download/retry/ack clears them.

use crate::state::AppState;
use serde_json::{Value, json};
use std::time::Duration;

/// Probe the shared jar when nothing has used it successfully for this long.
const IDLE_PROBE_AFTER: chrono::Duration = chrono::Duration::minutes(45);
/// A video that is always up (YouTube's own "Me at the zoo").
const PROBE_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

pub fn spawn(st: &AppState) {
    let s = st.clone();
    tokio::spawn(async move { cookie_watch_loop(s).await });
    let s = st.clone();
    tokio::spawn(async move { retry_loop(s).await });
}

/// "bot_wall" / "rate_limit" for a download failure worth retrying later.
pub fn retry_reason(stderr: &[u8]) -> Option<&'static str> {
    if crate::ytdlp::is_bot_wall(stderr) {
        Some("bot_wall")
    } else if crate::ytdlp::is_rate_limited(stderr) {
        Some("rate_limit")
    } else {
        None
    }
}

/// Remember a download that failed for `reason` (see `retry_reason`).
#[allow(clippy::too_many_arguments)]
pub async fn remember_failed(st: &AppState, user: &str, source: &str, id: &str, title: &str, artist: Option<&str>, format: &str, reason: &str) {
    let r = sqlx::query(
        "INSERT INTO ios_download_retry (user_id, source, source_id, title, artist, format, reason) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (user_id, source, source_id) DO UPDATE SET reason = EXCLUDED.reason, \
           failures = ios_download_retry.failures + 1, failed_at = NOW() AT TIME ZONE 'UTC', ready_at = NULL",
    )
    .bind(user)
    .bind(source)
    .bind(crate::db::truncate(id, 280))
    .bind(crate::db::truncate(title, 500))
    .bind(artist.map(|a| crate::db::truncate(a, 300)))
    .bind(crate::db::truncate(format, 20))
    .bind(reason)
    .execute(&st.db)
    .await;
    if let Err(e) = r {
        tracing::warn!("download retry: could not remember {source}:{id}: {e}");
    }
}

/// The track downloaded (now or on a retry): nothing left to retry.
pub async fn forget(st: &AppState, user: &str, source: &str, id: &str) {
    let _ = sqlx::query("DELETE FROM ios_download_retry WHERE user_id = $1 AND source = $2 AND source_id = $3")
        .bind(user)
        .bind(source)
        .bind(id)
        .execute(&st.db)
        .await;
}

async fn cookie_watch_loop(st: AppState) {
    let mut last_alert: Option<std::time::Instant> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(10 * 60));
    tick.tick().await;
    loop {
        tick.tick().await;
        let health = st.ytdlp.jar_health.get("global").map(|h| h.clone()).unwrap_or_default();
        let now = chrono::Utc::now();
        let idle = health.last_ok.is_none_or(|t| now - t > IDLE_PROBE_AFTER);
        let walled = health.botwalls > 0;
        let benched = health.benched_until.is_some_and(|t| t > std::time::Instant::now());
        // Idle, or walled but off the bench: find out whether the jar works
        // now, so health (and the retry queue) doesn't wait for a user.
        if idle || (walled && !benched) {
            probe_global(&st).await;
        }
        let health = st.ytdlp.jar_health.get("global").map(|h| h.clone()).unwrap_or_default();
        if health.botwalls >= 2 && last_alert.is_none_or(|t| t.elapsed() > Duration::from_secs(6 * 3600)) {
            last_alert = Some(std::time::Instant::now());
            alert_operator(&st, health.botwalls).await;
        }
    }
}

/// One `--simulate` resolve with the shared jar; records ok / bot wall.
async fn probe_global(st: &AppState) {
    let jars = st.ytdlp.cookie_candidates(None, true).await;
    let Some(global) = jars.into_iter().find(|j| j.owner == "global") else { return };
    let mut args: Vec<String> = st.ytdlp.network_args().to_vec();
    args.extend(crate::ytdlp::STREAM_CLIENT_ARGS.iter().map(|s| s.to_string()));
    args.extend(["--simulate".into(), "-f".into(), "bestaudio/best".into(), "--no-playlist".into()]);
    args.extend(global.args);
    args.push(PROBE_URL.into());
    let out = {
        let _slot = st.ytdlp.limiter.slot().await;
        st.ytdlp.run("yt-dlp", &args, Duration::from_secs(30)).await
    };
    match out {
        Ok(o) => {
            let ok = o.status == Some(0);
            st.ytdlp.note_jar_result("global", ok, &o.stderr);
            tracing::info!("cookie probe (shared jar): {}", if ok { "ok" } else if crate::ytdlp::is_bot_wall(&o.stderr) { "bot wall" } else { "failed" });
        }
        Err(e) => tracing::warn!("cookie probe could not run: {e}"),
    }
}

async fn alert_operator(st: &AppState, walls: u32) {
    let msg = format!(
        "Lumisound: YouTube is showing the shared cookies its bot check ({walls} times since the last success). \
         Export fresh cookies.txt to ~/Downloads on the server; the refresher installs it."
    );
    tracing::warn!("{msg}");
    crate::db::log_event(&st.db, "streaming", "cookies_botwall", None, "error", &msg, Some(json!({"jars": st.ytdlp.jar_report()}))).await;
    if let Some(url) = st.cfg.bug_report_webhook_url.as_deref() {
        let _ = st.http.post(url).json(&json!({"content": msg})).send().await;
    }
}

/// Marks retry rows ready once their cause has cleared and tells the apps.
async fn retry_loop(st: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(120));
    loop {
        tick.tick().await;
        if let Err(e) = retry_tick(&st).await {
            tracing::warn!("download retry tick failed: {e}");
        }
    }
}

type Row = (String, String, String, Option<String>, Option<String>, Option<String>, String, chrono::NaiveDateTime, Option<chrono::NaiveDateTime>);

async fn retry_tick(st: &AppState) -> anyhow::Result<()> {
    // Older than 14 days: the user has moved on.
    sqlx::query("DELETE FROM ios_download_retry WHERE failed_at < NOW() AT TIME ZONE 'UTC' - INTERVAL '14 days'").execute(&st.db).await?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT r.user_id, r.source, r.source_id, r.title, r.artist, r.format, r.reason, r.failed_at, s.ytdlp_cookies_updated_at \
         FROM ios_download_retry r LEFT JOIN ios_user_settings s ON s.user_id = r.user_id \
         WHERE r.ready_at IS NULL ORDER BY r.failed_at LIMIT 2000",
    )
    .fetch_all(&st.db)
    .await?;
    if rows.is_empty() {
        return Ok(());
    }
    let global_ok = st.ytdlp.jar_health.get("global").and_then(|h| h.last_ok);
    let mut ready: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
    for (user, source, id, title, artist, format, reason, failed_at, cookies_at) in rows {
        let failed = failed_at.and_utc();
        let cleared = match reason.as_str() {
            "rate_limit" => st.ytdlp.rate_hold_left(Some(&user)).is_none() && chrono::Utc::now() - failed > chrono::Duration::minutes(15),
            _ => {
                let own_ok = st.ytdlp.jar_health.get(&format!("user:{user}")).and_then(|h| h.last_ok);
                // New cookies uploaded since, or a jar this account uses worked since.
                let uploaded = cookies_at.map(|t| t.and_utc());
                [global_ok, own_ok, uploaded].into_iter().flatten().any(|t| t > failed)
            }
        };
        if cleared {
            ready.entry(user).or_default().push(json!({"source": source, "id": id, "title": title, "artist": artist, "format": format, "reason": reason}));
        }
    }
    for (user, tracks) in ready {
        let ids: Vec<String> = tracks.iter().filter_map(|t| t["id"].as_str().map(str::to_string)).collect();
        sqlx::query("UPDATE ios_download_retry SET ready_at = NOW() AT TIME ZONE 'UTC' WHERE user_id = $1 AND source_id = ANY($2)")
            .bind(&user)
            .bind(&ids)
            .execute(&st.db)
            .await?;
        tracing::info!("download retry: {} track(s) ready again for {user}", tracks.len());
        st.push_live(&user, json!({"type": "downloads_retry", "tracks": tracks}));
    }
    Ok(())
}
