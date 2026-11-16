//! Cloud library → LMS3, and Premium quality upgrades of cloud tracks.
//!
//! Migration: every stored `.lms` file that isn't LMS3 yet (LMSLOCK1/2
//! wraps, gzip-stored or not) is converted beside itself, proven to decode
//! sample-identically to the original, then swapped in under the same name
//! (so database rows, playlists and the apps' references don't change).
//! The original is moved — never deleted — to `<root>/.lms3-backup/<user>/
//! <relative path>` on the same drive, until the operator removes that
//! folder. One file at a time, behind the transcode limiter, paced.
//!
//! Upgrades: for accounts whose YouTube cookies are Premium (see
//! `premium.rs`), cloud tracks that came from YouTube below Premium quality
//! are re-fetched as format 774 (~256 kbps Opus) and replaced in place via
//! `lms3::replace_audio` (same recording check, tags kept), the old file
//! backed up the same way.
//!
//! `LMS3_CLOUD=off` disables both; status at GET /admin/api/lms3.

use crate::state::AppState;
use crate::storage;
use lumisound_media::{lms3, lock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

pub const BACKUP_DIR: &str = ".lms3-backup";

#[derive(Default)]
struct Progress {
    phase: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    scanned: u64,
    converted: u64,
    already: u64,
    skipped: u64,
    failed: u64,
    bytes_before: u64,
    bytes_after: u64,
    upgraded: u64,
    upgrade_failed: u64,
    last_errors: Vec<String>,
    current: Option<String>,
}

static PROGRESS: LazyLock<Mutex<Progress>> = LazyLock::new(|| Mutex::new(Progress::default()));

fn note_error(msg: String) {
    tracing::warn!("lms3 cloud: {msg}");
    let mut p = PROGRESS.lock().unwrap();
    p.last_errors.push(msg);
    let n = p.last_errors.len();
    if n > 30 {
        p.last_errors.drain(..n - 30);
    }
}

pub fn status() -> Value {
    let p = PROGRESS.lock().unwrap();
    json!({
        "phase": p.phase, "started_at": p.started_at, "finished_at": p.finished_at, "current": p.current,
        "scanned": p.scanned, "converted": p.converted, "already_lms3": p.already, "skipped": p.skipped, "failed": p.failed,
        "bytes_before": p.bytes_before, "bytes_after": p.bytes_after,
        "premium_upgraded": p.upgraded, "premium_upgrade_failed": p.upgrade_failed,
        "last_errors": p.last_errors,
    })
}

pub fn spawn(st: &AppState) {
    if std::env::var("LMS3_CLOUD").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        tracing::info!("lms3 cloud: disabled (LMS3_CLOUD=off)");
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        // Let startup work (schema, caches) settle first.
        tokio::time::sleep(Duration::from_secs(90)).await;
        loop {
            migrate_all(&st).await;
            upgrade_all(&st).await;
            tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
        }
    });
}

async fn user_ids(st: &AppState) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM ios_users ORDER BY created_at").fetch_all(&st.db).await.unwrap_or_default()
}

/// Where the original of `path` (inside `user_dir`) is kept.
fn backup_path(user_dir: &Path, path: &Path, sub: &str) -> Option<PathBuf> {
    let root = user_dir.parent()?;
    let user = user_dir.file_name()?;
    let rel = path.strip_prefix(user_dir).ok()?;
    let mut b = root.join(BACKUP_DIR).join(sub).join(user).join(rel);
    let mut n = 1;
    while b.exists() {
        b = root.join(BACKUP_DIR).join(sub).join(user).join(format!("{}.{n}", rel.to_string_lossy()));
        n += 1;
    }
    Some(b)
}

fn move_to_backup(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d)?;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)
        }
    }
}

enum Outcome {
    Converted { before: u64, after: u64, kbps: u32 },
    Already,
    Skipped(String),
}

/// Convert one stored file in place (blocking). The original ends up in
/// the backup folder only once the LMS3 file is proven and in place.
fn convert_one(user_dir: &Path, path: &Path) -> Result<Outcome, String> {
    if lms3::is_lms3(path) {
        return Ok(Outcome::Already);
    }
    let before = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let dir = path.parent().ok_or("no parent")?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("bad name")?.to_string();
    // A gzip-stored upload is unpacked beside it first (same drive).
    let mut unpacked: Option<PathBuf> = None;
    let src: PathBuf = if storage::is_gzip(path) {
        let inner = lock::inner_ext(&name).unwrap_or_else(|| "bin".into());
        let tmp = dir.join(format!(".{}.gunzip.{inner}.lms", uuid::Uuid::new_v4().simple()));
        let r = (|| -> std::io::Result<()> {
            let mut d = flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(&mut d, &mut out)?;
            out.sync_all()
        })();
        if let Err(e) = r {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("gunzip: {e}"));
        }
        unpacked = Some(tmp.clone());
        tmp
    } else {
        path.to_path_buf()
    };
    let cleanup = |u: &Option<PathBuf>| {
        if let Some(t) = u {
            let _ = std::fs::remove_file(t);
        }
    };
    if lms3::is_lms3(&src) {
        // A gzip of an LMS3 file: store it unpacked.
        let backup = backup_path(user_dir, path, "originals").ok_or("backup path")?;
        move_to_backup(path, &backup).map_err(|e| format!("backup: {e}"))?;
        std::fs::rename(&src, path).map_err(|e| format!("rename: {e}"))?;
        let after = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return Ok(Outcome::Converted { before, after, kbps: lms3::read_header(path).map(|h| h.bitrate_kbps()).unwrap_or(0) });
    }
    let staged = dir.join(format!(".{}.lms3-cloud", uuid::Uuid::new_v4().simple()));
    let w = match lms3::write_verified(&src, &staged, &lms3::WriteOpts::default()) {
        Ok(w) => w,
        Err(lms3::WriteError::Io(e)) => {
            cleanup(&unpacked);
            return Err(format!("write: {e}"));
        }
        Err(e) => {
            cleanup(&unpacked);
            return Ok(Outcome::Skipped(e.to_string()));
        }
    };
    cleanup(&unpacked);
    let backup = match backup_path(user_dir, path, "originals") {
        Some(b) => b,
        None => {
            let _ = std::fs::remove_file(&staged);
            return Err("backup path".into());
        }
    };
    if let Err(e) = move_to_backup(path, &backup) {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("backup: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged, path) {
        // Put the original back: nothing may go missing.
        let _ = std::fs::rename(&backup, path).or_else(|_| std::fs::copy(&backup, path).map(|_| ()));
        let _ = std::fs::remove_file(&staged);
        return Err(format!("swap: {e}"));
    }
    Ok(Outcome::Converted { before, after: w.bytes, kbps: w.header.bitrate_kbps() })
}

fn is_cloud_track(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    !name.starts_with('.') && lock::inner_ext(name).is_some()
}

async fn update_row(st: &AppState, user: &str, user_dir: &Path, path: &Path, bytes: u64, kbps: u32) {
    let Ok(rel) = path.strip_prefix(user_dir) else { return };
    let rel = rel.to_string_lossy().to_string();
    let _ = sqlx::query("UPDATE ios_user_music_metadata SET file_size_bytes = $1, bitrate = CASE WHEN $2 > 0 THEN $2 ELSE bitrate END WHERE user_id = $3 AND (filename = $4 OR relative_path = $4)")
        .bind(bytes as i64)
        .bind(kbps as i32)
        .bind(user)
        .bind(&rel)
        .execute(&st.db)
        .await;
}

pub async fn migrate_all(st: &AppState) {
    {
        let mut p = PROGRESS.lock().unwrap();
        *p = Progress { phase: "migrating".into(), started_at: Some(chrono::Utc::now().to_rfc3339()), ..Default::default() };
    }
    for user in user_ids(st).await {
        let Some(dir) = st.user_music_dir(&user).filter(|d| d.is_dir()) else { continue };
        let d = dir.clone();
        let files: Vec<PathBuf> = tokio::task::spawn_blocking(move || storage::walk_files(&d).into_iter().filter(|p| is_cloud_track(p)).collect()).await.unwrap_or_default();
        for path in files {
            PROGRESS.lock().unwrap().scanned += 1;
            // Deleted (moved to the trash) since the folder was listed.
            if !path.exists() {
                continue;
            }
            if lms3::is_lms3(&path) {
                PROGRESS.lock().unwrap().already += 1;
                continue;
            }
            PROGRESS.lock().unwrap().current = Some(path.to_string_lossy().into_owned());
            let r = {
                let _slot = st.ytdlp.transcode.slot().await;
                let (d, p) = (dir.clone(), path.clone());
                tokio::task::spawn_blocking(move || convert_one(&d, &p)).await.unwrap_or_else(|e| Err(e.to_string()))
            };
            match r {
                Ok(Outcome::Converted { before, after, kbps }) => {
                    {
                        let mut p = PROGRESS.lock().unwrap();
                        p.converted += 1;
                        p.bytes_before += before;
                        p.bytes_after += after;
                    }
                    update_row(st, &user, &dir, &path, after, kbps).await;
                }
                Ok(Outcome::Already) => PROGRESS.lock().unwrap().already += 1,
                Ok(Outcome::Skipped(why)) => {
                    PROGRESS.lock().unwrap().skipped += 1;
                    tracing::info!("lms3 cloud: kept {} as it is ({why})", path.display());
                }
                Err(e) => {
                    PROGRESS.lock().unwrap().failed += 1;
                    note_error(format!("{}: {e}", path.display()));
                }
            }
            // Leave room for interactive work (streams, downloads).
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    let mut p = PROGRESS.lock().unwrap();
    p.phase = "migrated".into();
    p.current = None;
    p.finished_at = Some(chrono::Utc::now().to_rfc3339());
    tracing::info!("lms3 cloud: migration pass done — converted {}, already {}, skipped {}, failed {}", p.converted, p.already, p.skipped, p.failed);
}

/// Premium-quality pass over the cloud libraries of Premium accounts.
pub async fn upgrade_all(st: &AppState) {
    let users: Vec<String> = sqlx::query_scalar("SELECT user_id FROM ios_ytdlp_premium WHERE premium").fetch_all(&st.db).await.unwrap_or_default();
    if users.is_empty() {
        return;
    }
    PROGRESS.lock().unwrap().phase = "upgrading".into();
    for user in users {
        // Re-confirm (cookies may have lapsed) before spending downloads.
        let s = crate::premium::status(st, &user, false).await;
        if !s.premium {
            continue;
        }
        let Some(dir) = st.user_music_dir(&user).filter(|d| d.is_dir()) else { continue };
        let d = dir.clone();
        let candidates: Vec<(PathBuf, String)> = tokio::task::spawn_blocking(move || {
            storage::walk_files(&d)
                .into_iter()
                .filter(|p| is_cloud_track(p) && lms3::is_lms3(p))
                .filter_map(|p| {
                    let m = lms3::read_meta(&p)?;
                    let (kbps, _) = lms3::kbps_and_duration(&p);
                    if lms3::is_premium(m.quality.as_deref(), kbps) {
                        return None;
                    }
                    Some((p, m.youtube_id()?))
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        // A bounded batch per pass; the rest follows next pass.
        for (path, vid) in candidates.into_iter().take(150) {
            PROGRESS.lock().unwrap().current = Some(format!("upgrade {}", path.display()));
            match upgrade_one(st, &user, &dir, &path, &vid, s.client.as_deref()).await {
                Ok(detail) => {
                    PROGRESS.lock().unwrap().upgraded += 1;
                    tracing::info!("lms3 cloud: upgraded {} ({detail})", path.display());
                }
                Err(e) => {
                    PROGRESS.lock().unwrap().upgrade_failed += 1;
                    note_error(format!("upgrade {}: {e}", path.display()));
                }
            }
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    }
    let mut p = PROGRESS.lock().unwrap();
    p.phase = "idle".into();
    p.current = None;
}

async fn upgrade_one(st: &AppState, user: &str, user_dir: &Path, path: &Path, vid: &str, client: Option<&str>) -> Result<String, String> {
    let tmp = std::env::temp_dir().join(format!("lms3up-{}", uuid::Uuid::new_v4().simple()));
    tokio::fs::create_dir_all(&tmp).await.map_err(|e| e.to_string())?;
    let mut a: Vec<String> = st.ytdlp.network().to_vec();
    if let Some(c) = client {
        a.extend(["--extractor-args".into(), format!("youtube:player_client={c}")]);
    }
    a.extend(st.ytdlp.cookie_args(Some(user), false).await);
    a.extend(["--no-playlist", "-f", "774", "-x", "--audio-format", "best", "-o"].map(String::from));
    a.push(tmp.join("a.%(ext)s").to_string_lossy().into_owned());
    a.push(format!("https://music.youtube.com/watch?v={vid}"));
    st.ytdlp.pacer.wait(3.0).await;
    let out = {
        let _slot = st.ytdlp.limiter.slot().await;
        st.ytdlp.run("yt-dlp", &a, Duration::from_secs(240)).await
    };
    let file = match out {
        Ok(o) if o.status == Some(0) => std::fs::read_dir(&tmp).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.is_file())),
        Ok(o) => {
            st.ytdlp.penalise_if_rate_limited(&o.stderr);
            None
        }
        Err(_) => None,
    };
    let Some(file) = file else {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err("no Premium stream for this track".into());
    };
    let (ud, p, f) = (user_dir.to_path_buf(), path.to_path_buf(), file.clone());
    let r = {
        let _slot = st.ytdlp.transcode.slot().await;
        tokio::task::spawn_blocking(move || -> Result<(lms3::Replaced, u64), String> {
            let backup = backup_path(&ud, &p, "upgrades").ok_or("backup path")?;
            if let Some(d) = backup.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&p, &backup).map_err(|e| format!("backup: {e}"))?;
            match lms3::replace_audio(&p, &f, "yt:774", &|_| {}) {
                Ok(r) => Ok((r, std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0))),
                Err((_, _, why)) => {
                    let _ = std::fs::remove_file(&backup);
                    Err(why)
                }
            }
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let (r, bytes) = r?;
    update_row(st, user, user_dir, path, bytes, r.new_kbps).await;
    Ok(r.detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_paths_mirror_the_library_and_never_overwrite() {
        let root = std::env::temp_dir().join(format!("lms3bk-{}", uuid::Uuid::new_v4().simple()));
        let user = root.join("u1");
        std::fs::create_dir_all(user.join("Album")).unwrap();
        let track = user.join("Album/Song.opus.lms");
        let b = backup_path(&user, &track, "originals").unwrap();
        assert_eq!(b, root.join(".lms3-backup/originals/u1/Album/Song.opus.lms"));
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::write(&b, b"x").unwrap();
        let b2 = backup_path(&user, &track, "originals").unwrap();
        assert_ne!(b, b2);
        assert!(is_cloud_track(&track) && !is_cloud_track(&user.join(".x.lms3-cloud")));
        let _ = std::fs::remove_dir_all(root);
    }
}
