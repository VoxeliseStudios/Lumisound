//! LMS3 (Lumisound Audio) end to end, over every format the app imports:
//! source → LMS3 must decode sample-identically (the conversion is lossless
//! by construction: packets are copied), survive a header/meta rewrite,
//! seek, and export to a standard file that ffmpeg reads without errors and
//! that decodes to the same length. Skipped when ffmpeg isn't installed.
//!
//!   cargo test -p lumisound-media --test lms3 -- --nocapture
//!
//! `LUMI_LMS3_REAL=<dir>` additionally converts every audio/`.lms` file in
//! that directory (copies of real library tracks) and checks the same.

use lumisound_media::{decode, lms3, lms3_export, lock};
use std::path::{Path, PathBuf};
use std::process::Command;

const SECS: f64 = 3.0;

/// (file name, ffmpeg codec args, exported length must be exact)
const CASES: &[(&str, &[&str], bool)] = &[
    ("t.mp3", &["-c:a", "libmp3lame", "-b:a", "192k"], false),
    ("t.m4a", &["-c:a", "aac", "-b:a", "192k"], true),
    ("he.m4a", &["-c:a", "aac", "-profile:a", "aac_he", "-b:a", "48k"], false),
    ("alac.m4a", &["-c:a", "alac"], true),
    ("t.mp4", &["-c:a", "aac"], true),
    ("t.mov", &["-c:a", "aac"], true),
    ("t.aac", &["-c:a", "aac"], false),
    ("t.flac", &["-c:a", "flac"], true),
    ("s16.wav", &["-c:a", "pcm_s16le"], true),
    ("s24.wav", &["-c:a", "pcm_s24le"], true),
    ("f32.wav", &["-c:a", "pcm_f32le"], true),
    ("adpcm.wav", &["-c:a", "adpcm_ima_wav"], false),
    ("alaw.wav", &["-c:a", "pcm_alaw"], true),
    ("t.aiff", &["-c:a", "pcm_s16be"], true),
    ("pcm.caf", &["-c:a", "pcm_s16le"], true),
    ("alac.caf", &["-c:a", "alac"], true),
    ("t.ogg", &["-c:a", "libvorbis"], true),
    ("t.opus", &["-c:a", "libopus", "-b:a", "128k"], true),
    ("t.webm", &["-c:a", "libopus"], false),
    ("vorbis.webm", &["-c:a", "libvorbis"], false),
    ("t.mka", &["-c:a", "libopus"], false),
    ("t.mp2", &["-c:a", "mp2"], false),
];

fn ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn make(dir: &Path, name: &str, codec: &[&str]) -> Option<PathBuf> {
    let out = dir.join(name);
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y",
               "-f", "lavfi", "-i", &format!("sine=frequency=440:sample_rate=44100:duration={SECS}"),
               "-f", "lavfi", "-i", &format!("sine=frequency=1000:sample_rate=44100:duration={SECS}"),
               "-filter_complex", "[0][1]amerge=inputs=2,volume=0.5[a]", "-map", "[a]", "-vn",
               "-metadata", "title=Two Tones", "-metadata", "artist=Lumisound Test"])
        .args(codec)
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(out)
}

/// ffmpeg decodes `p` with no errors; returns its duration (s).
fn ffmpeg_reads(p: &Path) -> Result<f64, String> {
    let o = Command::new("ffmpeg").args(["-hide_banner", "-v", "error", "-i"]).arg(p).args(["-f", "null", "-"]).output().map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
    if !o.status.success() || !err.is_empty() {
        return Err(format!("ffmpeg: {err}"));
    }
    let o = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"]).arg(p).output().map_err(|e| e.to_string())?;
    String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().map_err(|e| format!("ffprobe duration: {e}"))
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lms3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Convert + verify + export one file. Errors are collected, not panicked,
/// so one run reports every failing format.
fn check(src: &Path, dir: &Path, exact_export: bool) -> Result<String, String> {
    let name = src.file_name().unwrap().to_string_lossy().to_string();
    let inner = lock::inner_ext(&name).unwrap_or_else(|| src.extension().unwrap().to_string_lossy().to_lowercase());
    let dst = dir.join(format!("{}.{inner}.lms", lock::display_stem(&name)));
    let w = lms3::write_verified(src, &dst, &lms3::WriteOpts::default()).map_err(|e| format!("write: {e}"))?;
    if !lock::is_locked(&dst) || !lms3::is_lms3(&dst) {
        return Err("not recognised as LMS3".into());
    }
    let fp_src = decode::fingerprint(src).ok_or("source decode")?;
    let fp = decode::fingerprint(&dst).ok_or("lms3 decode")?;
    if fp != fp_src {
        return Err(format!("decode differs: {fp:?} vs {fp_src:?}"));
    }
    let probe = decode::probe(&dst).ok_or("probe")?;
    let want = fp.frames as f64 / fp.rate as f64;
    if (probe.duration - want).abs() > 0.03 {
        return Err(format!("probe duration {} vs decoded {want}", probe.duration));
    }
    // Tags carried over and readable without a temp copy.
    let info = lumisound_media::tags::read_info(&dst).ok_or("read_info")?;
    let src_title = lumisound_media::tags::read_info(src).and_then(|i| i.title);
    if info.title != src_title {
        return Err(format!("title {:?} vs source {src_title:?}", info.title));
    }
    // Seek: a mid-file start decodes the same audio as the source does.
    // (ADPCM WAV sources can't seek in the existing decoder either: skipped.)
    if let Some(a) = decode::planar(src, Some(1.5), Some(0.5)) {
    let b = decode::planar(&dst, Some(1.5), Some(0.5)).ok_or("planar lms3")?;
    if a.0 != b.0 || a.1.len() != b.1.len() || (a.1[0].len() as i64 - b.1[0].len() as i64).abs() > 2 {
        return Err(format!("seek: {} vs {} frames", a.1[0].len(), b.1[0].len()));
    }
    }
    // Flags in place, then meta rewrite: audio untouched.
    lms3::set_flags(&dst, lms3::FLAG_ENGINE_EXCLUSIVE).map_err(|e| e.to_string())?;
    let mut m = lms3::read_meta(&dst).ok_or("meta")?;
    m.quality = Some("test".into());
    lms3::set_meta(&dst, &m).map_err(|e| e.to_string())?;
    if lms3::read_header(&dst).map(|h| h.flags) != Some(lms3::FLAG_ENGINE_EXCLUSIVE) || !lms3::check_integrity(&dst) || decode::fingerprint(&dst) != Some(fp) {
        return Err("flags/meta rewrite changed the audio".into());
    }
    // Export to a standard file.
    let ext = lms3_export::export_ext(&lms3::read_header(&dst).unwrap());
    let out = dir.join(format!("export-{name}.{ext}"));
    lock::unlock_file(&dst, &out).map_err(|e| format!("export: {e}"))?;
    let ff = ffmpeg_reads(&out)?;
    let fe = decode::fingerprint(&out).ok_or("exported file doesn't decode")?;
    let exp_secs = fe.frames as f64 / fe.rate as f64;
    if exact_export && fe.frames != fp.frames {
        return Err(format!("export length {} frames vs {}", fe.frames, fp.frames));
    }
    if (exp_secs - want).abs() > 0.06 || (ff - want).abs() > 0.08 {
        return Err(format!("export length engine {exp_secs:.3}s / ffmpeg {ff:.3}s vs {want:.3}s"));
    }
    let info2 = lumisound_media::tags::read_info(&out);
    let tagged = info2.as_ref().and_then(|i| i.title.clone()).is_some();
    Ok(format!("{} → {} ({} B, {} kbps) → .{ext}{}", w.header.packet_count, w.codec, w.bytes, w.header.bitrate_kbps(), if tagged { " tagged" } else { "" }))
}

#[test]
fn lms3_every_format() {
    if !ffmpeg() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let dir = tmpdir("fmt");
    let mut failures = Vec::new();
    for (name, codec, exact) in CASES {
        let Some(src) = make(&dir, name, codec) else {
            eprintln!("{name}: ffmpeg can't make it here, skipped");
            continue;
        };
        match check(&src, &dir, *exact) {
            Ok(s) => eprintln!("ok   {name}: {s}"),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
        // The same source locked the old way (LMSLOCK2) converts too.
        let locked = dir.join(format!("old-{name}.lms"));
        let locked = locked.with_file_name(format!("old{}.{}.lms", name.split('.').next().unwrap(), name.rsplit('.').next().unwrap()));
        lock::lock_file_flags(&src, &locked, lock::FLAG_LYRICS_SYNCED).unwrap();
        let up = dir.join(format!("up-{}", locked.file_name().unwrap().to_string_lossy()));
        match lms3::write_verified(&locked, &up, &lms3::WriteOpts::default()) {
            Ok(w) if w.header.flags == lock::FLAG_LYRICS_SYNCED => {}
            Ok(w) => failures.push(format!("{name} (locked): flags {} not carried", w.header.flags)),
            Err(e) => failures.push(format!("{name} (locked): {e}")),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn lms3_video_is_left_alone() {
    if !ffmpeg() {
        return;
    }
    let dir = tmpdir("video");
    let src = dir.join("clip.mp4");
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=64x64:rate=10:duration=1", "-f", "lavfi", "-i", "sine=duration=1", "-c:v", "mpeg4", "-c:a", "aac", "-shortest"])
        .arg(&src)
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        let r = lms3::write(&src, &dir.join("clip.mp4.lms"), &lms3::WriteOpts::default());
        assert_eq!(r.err(), Some(lms3::WriteError::NotAudioOnly));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lms3_real_library() {
    let Ok(dir) = std::env::var("LUMI_LMS3_REAL") else { return };
    let work = tmpdir("real");
    let mut failures = Vec::new();
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let audio = lock::inner_ext(&name).is_some() || lumisound_media::AUDIO_EXTS.contains(&p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str());
        if !audio {
            continue;
        }
        n += 1;
        let t = std::time::Instant::now();
        match check_real(&p, &work) {
            Ok(s) => eprintln!("ok   {name}: {s} in {:.2}s", t.elapsed().as_secs_f64()),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    assert!(failures.is_empty(), "{}/{n} failed:\n{}", failures.len(), failures.join("\n"));
}

fn check_real(src: &Path, dir: &Path) -> Result<String, String> {
    let name = src.file_name().unwrap().to_string_lossy().to_string();
    let dst = dir.join(&name);
    let before = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let w = lms3::write_verified(src, &dst, &lms3::WriteOpts::default()).map_err(|e| format!("write: {e}"))?;
    let ext = lms3_export::export_ext(&w.header);
    let out = dir.join(format!("export.{ext}"));
    lock::unlock_file(&dst, &out).map_err(|e| format!("export: {e}"))?;
    let ff = ffmpeg_reads(&out)?;
    let want = w.header.duration_secs();
    if (ff - want).abs() > 0.1 {
        return Err(format!("export {ff:.3}s vs {want:.3}s"));
    }
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&dst);
    Ok(format!("{} {} kbps, {} → {} B", w.codec, w.header.bitrate_kbps(), before, w.bytes))
}
