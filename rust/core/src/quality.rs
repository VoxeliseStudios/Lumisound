//! Quality upgrades: library tracks that came from YouTube below Premium
//! quality are re-fetched once the account's YouTube cookies are Premium
//! (YouTube Music's ~256 kbps Opus, format 774) and swapped in place as
//! LMS3 — same path, same tags/artwork/flags/vault tag, proven to decode
//! before anything is replaced.

use crate::lock::{is_premium, track_read_tag};
use lumisound_media::lms3;
use std::path::Path;

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct QualityCandidate {
    pub path: String,
    /// Bare 11-character YouTube video id.
    pub youtube_id: String,
    pub bitrate_kbps: u32,
    pub duration: f64,
    pub quality: Option<String>,
}

/// YouTube id of a library file: LMS3 metadata, then the vault xattr, then
/// the embedded LUMISOUND_ID tag.
pub fn youtube_id_of_file(path: &str) -> Option<String> {
    let p = Path::new(path);
    if let Some(id) = lms3::read_meta(p).and_then(|m| m.youtube_id()) {
        return Some(id);
    }
    if let Some(id) = track_read_tag(path.to_string()).and_then(|v| lms3::youtube_id_of(&v.track_id).or_else(|| lms3::youtube_id_of(&v.source_url))) {
        return Some(id);
    }
    lumisound_media::tags::read_lumisound(p).0.and_then(|s| lms3::youtube_id_of(&s))
}

/// The tracks among `paths` worth upgrading: YouTube-sourced and not
/// already Premium quality. Reads headers/tags only (call off the main
/// thread). Missing files are skipped.
#[uniffi::export]
pub fn quality_upgrade_candidates(paths: Vec<String>) -> Vec<QualityCandidate> {
    paths
        .into_iter()
        .filter_map(|path| {
            let p = Path::new(&path);
            if !p.is_file() {
                return None;
            }
            let (kbps, duration, quality) = match lms3::read_header(p) {
                Some(h) => (h.bitrate_kbps(), h.duration_secs(), lms3::read_meta(p).and_then(|m| m.quality)),
                None => {
                    let i = lumisound_media::tags::read_info(p)?;
                    (i.bitrate_kbps, i.duration, None)
                }
            };
            if is_premium(quality.as_deref(), kbps) {
                return None;
            }
            let youtube_id = youtube_id_of_file(&path)?;
            Some(QualityCandidate { path, youtube_id, bitrate_kbps: kbps, duration, quality })
        })
        .collect()
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct QualityReplaceOutcome {
    pub replaced: bool,
    pub old_kbps: u32,
    pub new_kbps: u32,
    pub detail: String,
}

/// Replace the audio of library file `path` with `new_audio_path` (a fresh
/// download of the same track), as LMS3, keeping everything else about the
/// track. Only replaces when the new audio is genuinely better (at least
/// 24 kbps more, or Premium where the old wasn't) and is the same recording
/// (duration within 2 s / 1.5 %). `new_audio_path` is always removed.
#[uniffi::export]
pub fn quality_replace_audio(path: String, new_audio_path: String, quality: String) -> QualityReplaceOutcome {
    let r = replace(&path, &new_audio_path, &quality);
    let _ = std::fs::remove_file(&new_audio_path);
    r
}

fn replace(path: &str, new_audio: &str, quality: &str) -> QualityReplaceOutcome {
    let vault = track_read_tag(path.to_string());
    let carry = |staged: &Path| {
        if let Some(v) = &vault {
            crate::lock::track_tag(staged.to_string_lossy().into_owned(), v.track_id.clone(), v.source_url.clone());
        }
    };
    match lms3::replace_audio(Path::new(path), Path::new(new_audio), quality, &carry) {
        Ok(r) => QualityReplaceOutcome { replaced: true, old_kbps: r.old_kbps, new_kbps: r.new_kbps, detail: r.detail },
        Err((old_kbps, new_kbps, detail)) => QualityReplaceOutcome { replaced: false, old_kbps, new_kbps, detail },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ff(out: &Path, args: &[&str]) -> bool {
        std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:sample_rate=48000:duration=4"])
            .args(args)
            .arg(out)
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn upgrade_replaces_only_with_better_same_recording() {
        let d = std::env::temp_dir().join(format!("qual-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let low = d.join("low.opus");
        let high = d.join("high.opus");
        let other = d.join("other.opus");
        if !ff(&low, &["-c:a", "libopus", "-b:a", "48k", "-metadata", "title=Song", "-metadata", "LUMISOUND_ID=youtube:dQw4w9WgXcQ", "-metadata", "LUMISOUND_QUALITY=yt:251"])
            || !ff(&high, &["-c:a", "libopus", "-b:a", "256k"])
            || !std::process::Command::new("ffmpeg").args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=duration=9", "-c:a", "libopus", "-b:a", "256k"]).arg(&other).status().is_ok_and(|s| s.success())
        {
            eprintln!("ffmpeg/libopus missing; skipped");
            return;
        }
        let track = d.join("Song.opus.lms");
        let o = crate::lock::lms_convert(low.to_string_lossy().into(), track.to_string_lossy().into(), d.join("tmp").to_string_lossy().into(), None);
        assert!(o.locked_path.is_some(), "{}", o.detail);
        let t = track.to_string_lossy().to_string();
        let c = quality_upgrade_candidates(vec![t.clone()]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].youtube_id, "dQw4w9WgXcQ");
        assert_eq!(c[0].quality.as_deref(), Some("yt:251"), "bridge quality tag carried into LMS3");
        // A different recording is refused.
        let tmp_other = d.join("o2.opus");
        std::fs::copy(&other, &tmp_other).unwrap();
        let r = quality_replace_audio(t.clone(), tmp_other.to_string_lossy().into(), "yt:774".into());
        assert!(!r.replaced, "{}", r.detail);
        // The Premium copy goes in; tags survive.
        let r = quality_replace_audio(t.clone(), high.to_string_lossy().into(), "yt:774".into());
        assert!(r.replaced, "{}", r.detail);
        assert!(!high.exists());
        let info = crate::lock::lms_info(t.clone());
        assert!(info.premium && info.version == 3 && info.quality.as_deref() == Some("yt:774"));
        assert_eq!(lms3::read_meta(&track).unwrap().title.as_deref(), Some("Song"));
        assert!(quality_upgrade_candidates(vec![t]).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }
}
