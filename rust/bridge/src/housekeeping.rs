//! Storage housekeeping and the cloud trash.
//!
//! - Size caps for the caches that used to grow forever: Rich Presence art
//!   (`rp-art-yt`, `rp-artwork`), embedded-artwork thumbnails
//!   (`artwork-cache`), the LMSLOCK1 wrap cache, and the Demucs vocals cache.
//!   Least recently used files go first. Runs hourly.
//! - `storage_report`: sizes of every cache, the LMS3 backup and the trash
//!   (admin card), and `delete_lms3_backup`.
//! - Trash: list deleted cloud tracks and restore one (file, metadata row
//!   from `.meta.json`, locked artwork from `.art.jpg`).

use crate::state::AppState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// (cache folder under YTDLP_CACHE_DIR or an absolute path, byte cap).
fn caps(st: &AppState) -> Vec<(String, PathBuf, u64)> {
    let c = Path::new(&st.cfg.ytdlp_cache_dir);
    let mb = |n: u64| n * 1024 * 1024;
    vec![
        ("Rich Presence art (YouTube)".into(), c.join("rp-art-yt"), mb(150)),
        ("Rich Presence art (uploads)".into(), c.join("rp-artwork"), mb(100)),
        ("Artwork thumbnails".into(), c.join("artwork-cache"), mb(400)),
        ("Old-app stream copies".into(), std::env::temp_dir().join("lms3-legacy-cache"), mb(2048)),
        ("Lyrics vocals (Demucs)".into(), crate::lyricsgen::vocals_cache_dir(st), mb(1536)),
    ]
}

pub fn spawn(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let caps = caps(&st);
            let freed = tokio::task::spawn_blocking(move || caps.iter().map(|(n, d, cap)| (n.clone(), trim_to(d, *cap))).collect::<Vec<_>>()).await.unwrap_or_default();
            for (name, (files, bytes)) in freed {
                if files > 0 {
                    tracing::info!("housekeeping: {name}: removed {files} file(s), {} MB", bytes / 1024 / 1024);
                }
            }
        }
    });
}

/// Every regular file under `dir` with (size, last use).
fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            walk(&e.path(), out);
        } else if m.is_file() {
            let used = m.accessed().ok().max(m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((e.path(), m.len(), used));
        }
    }
}

pub fn dir_size(dir: &Path) -> (u64, u64) {
    let mut v = Vec::new();
    walk(dir, &mut v);
    (v.len() as u64, v.iter().map(|f| f.1).sum())
}

/// Delete least recently used files until `dir` holds at most `cap` bytes.
/// Returns (files, bytes) removed.
pub fn trim_to(dir: &Path, cap: u64) -> (u64, u64) {
    let mut files = Vec::new();
    walk(dir, &mut files);
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= cap {
        return (0, 0);
    }
    files.sort_by_key(|f| f.2);
    let (mut n, mut freed) = (0, 0);
    for (p, len, _) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
            freed += len;
            n += 1;
        }
    }
    (n, freed)
}

/// Sizes for the admin storage card (walks the folders; call off the runtime).
pub fn storage_report(st: &AppState) -> Value {
    let mut caches = Vec::new();
    for (name, dir, cap) in caps(st) {
        let (files, bytes) = dir_size(&dir);
        caches.push(json!({"name": name, "files": files, "bytes": bytes, "cap_bytes": cap}));
    }
    let mut backups = Vec::new();
    let mut trash = Vec::new();
    for root in st.user_music_roots() {
        let b = root.join(crate::lms3_cloud::BACKUP_DIR);
        if b.is_dir() {
            let (files, bytes) = dir_size(&b);
            backups.push(json!({"root": root.display().to_string(), "files": files, "bytes": bytes}));
        }
        let t = root.join(".trash");
        if t.is_dir() {
            let (files, bytes) = dir_size(&t);
            trash.push(json!({"root": root.display().to_string(), "files": files, "bytes": bytes}));
        }
    }
    json!({"caches": caches, "lms3_backup": backups, "trash": trash})
}

/// Erase `<root>/.lms3-backup` on every music root (admin, confirmed).
pub fn delete_lms3_backup(st: &AppState) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    for root in st.user_music_roots() {
        let b = root.join(crate::lms3_cloud::BACKUP_DIR);
        if b.is_dir() {
            let (f, s) = dir_size(&b);
            if std::fs::remove_dir_all(&b).is_ok() {
                files += f;
                bytes += s;
            }
        }
    }
    tracing::warn!("housekeeping: deleted the LMS3 backup ({files} files, {} MB) at the admin's request", bytes / 1024 / 1024);
    (files, bytes)
}

// ---- trash ------------------------------------------------------------------------------------

fn is_sidecar(name: &str) -> bool {
    name.ends_with(".meta.json") || name.ends_with(".art.jpg")
}

fn valid_day(day: &str) -> bool {
    day.len() == 8 && day.chars().all(|c| c.is_ascii_digit())
}

/// Every trashed track, newest day first: user, day, path, size, whether its
/// metadata row was kept, the title from it.
pub fn trash_list(st: &AppState, limit: usize) -> Vec<Value> {
    let mut out = Vec::new();
    for root in st.user_music_roots() {
        let Ok(users) = std::fs::read_dir(root.join(".trash")) else { continue };
        for u in users.flatten() {
            let user = u.file_name().to_string_lossy().into_owned();
            let Ok(days) = std::fs::read_dir(u.path()) else { continue };
            for d in days.flatten() {
                let day = d.file_name().to_string_lossy().into_owned();
                if !valid_day(&day) {
                    continue;
                }
                let mut files = Vec::new();
                walk(&d.path(), &mut files);
                for (p, len, _) in files {
                    let Ok(rel) = p.strip_prefix(d.path()) else { continue };
                    let rel = rel.to_string_lossy().into_owned();
                    if is_sidecar(&rel) {
                        continue;
                    }
                    let meta: Option<Value> = std::fs::read(sidecar(&p, "meta.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
                    out.push(json!({
                        "user": user, "day": day, "path": rel, "bytes": len,
                        "has_meta": meta.is_some(),
                        "title": meta.as_ref().and_then(|m| m.get("title")).cloned().unwrap_or(Value::Null),
                        "artist": meta.as_ref().and_then(|m| m.get("artist")).cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
    }
    out.sort_by(|a, b| b["day"].as_str().cmp(&a["day"].as_str()).then(a["path"].as_str().cmp(&b["path"].as_str())));
    out.truncate(limit);
    out
}

fn sidecar(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(format!(".{suffix}"));
    PathBuf::from(s)
}

/// Put one trashed track back: the file to its old place in the user's
/// library (refused if something now lives there), its metadata row, and
/// its locked artwork.
pub async fn trash_restore(st: &AppState, user: &str, day: &str, rel: &str) -> Result<String, String> {
    if user.is_empty() || user.contains('/') || user.contains("..") || !valid_day(day) || is_sidecar(rel) {
        return Err("bad request".into());
    }
    let dir = st.user_music_dir(user).ok_or("no music folder for that user")?;
    let root = dir.parent().ok_or("bad music root")?.to_path_buf();
    let trash_day = root.join(".trash").join(user).join(day);
    let src = crate::storage::safe_join(&trash_day, rel).ok_or("bad path")?;
    let dest = crate::storage::safe_join(&dir, rel).ok_or("bad path")?;
    if !src.is_file() {
        return Err("not in the trash".into());
    }
    if dest.exists() {
        return Err("a file with that name is back in the library already".into());
    }
    if let Some(p) = dest.parent() {
        tokio::fs::create_dir_all(p).await.map_err(|e| e.to_string())?;
    }
    tokio::fs::rename(&src, &dest).await.map_err(|e| format!("move failed: {e}"))?;
    let mut restored_row = false;
    if let Ok(bytes) = tokio::fs::read(sidecar(&src, "meta.json")).await {
        if let Ok(meta) = serde_json::from_slice::<Value>(&bytes) {
            let id = meta.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            let r = sqlx::query(
                "INSERT INTO ios_user_music_metadata SELECT * FROM jsonb_populate_record(NULL::ios_user_music_metadata, $1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(sqlx::types::Json(&meta))
            .execute(&st.db)
            .await;
            match r {
                Ok(_) => restored_row = true,
                Err(e) => tracing::warn!("trash restore: metadata row for {rel} not restored: {e}"),
            }
            let art = sidecar(&src, "art.jpg");
            if !id.is_empty() && art.is_file() {
                let safe: String = id.chars().filter(|c| c.is_alphanumeric()).collect();
                let to = dir.join(".artwork").join(format!("{safe}.jpg"));
                let _ = tokio::fs::create_dir_all(to.parent().unwrap()).await;
                let _ = tokio::fs::rename(&art, &to).await;
            }
            let _ = tokio::fs::remove_file(sidecar(&src, "meta.json")).await;
        }
    }
    // Leave no empty folders behind in the trash day.
    let mut parent = src.parent().map(Path::to_path_buf);
    while let Some(d) = parent {
        if d == trash_day.parent().unwrap_or(&trash_day) || std::fs::remove_dir(&d).is_err() {
            break;
        }
        parent = d.parent().map(Path::to_path_buf);
    }
    tracing::info!("trash restore: {user} {day} {rel} (metadata row: {restored_row})");
    st.push_live(user, json!({"type": "sync_changed", "reason": "cloud_restore"}));
    Ok(if restored_row { "restored".into() } else { "restored (file only; the app re-reads its tags)".into() })
}
