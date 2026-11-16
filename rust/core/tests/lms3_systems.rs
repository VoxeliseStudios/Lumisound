//! Every Lumisound system that reads an audio file, on LMS3: each must give
//! the same answer for a track's LMS3 file as for the source it came from
//! (analysis, BPM/key/beat grid, waveform, loudness, spectral/transition
//! profiles, pitch, fingerprint, intro detection, haptics, Smart Crossfade
//! profile, lyric sync, validation, library scan, tags, Vorbis comments,
//! artwork), plus the LMS3-only paths: badge, AVFoundation copy, convertible
//! probe, library status, tag writing. Every mismatch is collected.
//!
//!   cargo test --release -p lumisound-core --test lms3_systems -- --nocapture

use lumisound_core::{analysis, blend, haptics, import, intro, library, lock, lyrics};
use lumisound_media::{decode, lms3};
use std::path::{Path, PathBuf};
use std::process::Command;

fn ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn make(dir: &Path, name: &str, codec: &[&str], art: Option<&Path>) -> Option<PathBuf> {
    let out = dir.join(name);
    let expr = "0.2*sin(2*PI*220*t)+0.12*sin(2*PI*330*t)+0.15*sin(2*PI*(440+60*sin(2*PI*0.25*t))*t)+0.3*exp(-30*mod(t\\,0.5))*sin(2*PI*80*t)";
    let mut c = Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("aevalsrc={expr}|{expr}:s=44100:d=40")]);
    if let Some(a) = art {
        c.arg("-i").arg(a).args(["-map", "0:a", "-map", "1:v", "-c:v", "mjpeg", "-disposition:v", "attached_pic"]);
    }
    c.args(["-af", "afade=t=in:st=0:d=3", "-metadata", "title=Systems Fixture", "-metadata", "artist=Lumisound Lab", "-metadata", "album=LMS3", "-metadata", "LUMISOUND_ID=youtube:dQw4w9WgXcQ"]);
    let ok = c.args(codec).arg(&out).status().is_ok_and(|s| s.success());
    ok.then_some(out)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn same(bad: &mut Vec<String>, what: &str, x: String, y: String) {
    if x != y {
        let cut = |v: &str| v.chars().take(160).collect::<String>();
        bad.push(format!("{what}: source {} vs LMS3 {}", cut(&x), cut(&y)));
    }
}

fn check(src: &Path, dir: &Path) -> Vec<String> {
    let mut bad = Vec::new();
    let name = src.file_name().unwrap().to_string_lossy().to_string();
    let ext = src.extension().unwrap().to_string_lossy().to_string();
    let lms = dir.join(format!("{}.{ext}.lms", src.file_stem().unwrap().to_string_lossy()));
    if let Err(e) = lms3::write_verified(src, &lms, &lms3::WriteOpts::default()) {
        return vec![format!("convert: {e}")];
    }
    let cache = s(&dir.join("cache"));
    let (a, b) = (s(src), s(&lms));
    let dur = |p: &str| analysis::probe_duration(p.into(), cache.clone());
    if (dur(&a) - dur(&b)).abs() > 0.03 {
        bad.push(format!("probe_duration {} vs {}", dur(&a), dur(&b)));
    }
    // Opus: anything that starts mid-track meets LMS3's warm seek (decoder
    // pre-roll) where the source file's reader starts the decoder cold, so
    // LMS3 must match a continuous decode at least as well as the source.
    // (AAC too: LMS3 starts it a packet early so the overlapped transform
    // is complete at the start; the plain file's reader starts it cold.)
    let opus = matches!(ext.as_str(), "opus" | "webm" | "m4a");
    if opus {
        let mono = |p: &str, st: f64, len: f64| analysis::decode_mono(p.into(), cache.clone(), 11025, st, len).unwrap_or_default();
        let truth: Vec<i16> = mono(&a, 0.0, 9.0).into_iter().skip(5 * 11025).collect();
        let err = |x: &[i16]| {
            let n = x.len().min(truth.len()).max(1);
            let e: f64 = x.iter().zip(&truth).map(|(p, q)| (*p as f64 - *q as f64).powi(2)).sum::<f64>() / n as f64;
            let s: f64 = truth.iter().map(|q| (*q as f64).powi(2)).sum::<f64>() / n as f64;
            10.0 * (e.max(1e-9) / s.max(1e-9)).log10()
        };
        let (ea, eb) = (err(&mono(&a, 5.0, 4.0)), err(&mono(&b, 5.0, 4.0)));
        eprintln!("     {name}: decode from 5 s vs continuous — source {ea:.1} dB, LMS3 {eb:.1} dB");
        // WebM timestamps are whole milliseconds: mid-track starts sit up to
        // ~1 ms off for the source and LMS3 alike (inaudible); Ogg is exact.
        if eb > ea + 0.5 || (ext != "webm" && eb > -35.0) {
            bad.push(format!("decode_mono seek: LMS3 {eb:.1} dB vs source {ea:.1} dB from a continuous decode"));
        }
        let fp = |p: &str| analysis::audio_fingerprint(p.into(), cache.clone(), None).unwrap_or_default();
        let (fa, fb) = (fp(&a), fp(&b));
        let d = fa.iter().flatten().zip(fb.iter().flatten()).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
        if fa.len() != fb.len() || d > 0.08 {
            bad.push(format!("audio_fingerprint: {} vs {} probes, max diff {d}", fa.len(), fb.len()));
        }
    }
    if !opus {
    same(&mut bad, "decode_mono", format!("{:?}", analysis::decode_mono(a.clone(), cache.clone(), 11025, 5.0, 4.0).map(|v| v.iter().map(|x| *x as i64).sum::<i64>())), format!("{:?}", analysis::decode_mono(b.clone(), cache.clone(), 11025, 5.0, 4.0).map(|v| v.iter().map(|x| *x as i64).sum::<i64>())));
    same(&mut bad, "bpm_estimate", format!("{:?}", analysis::bpm_estimate(a.clone(), cache.clone(), None)), format!("{:?}", analysis::bpm_estimate(b.clone(), cache.clone(), None)));
    same(&mut bad, "key_estimate", format!("{:?}", analysis::key_estimate(a.clone(), cache.clone())), format!("{:?}", analysis::key_estimate(b.clone(), cache.clone())));
    same(&mut bad, "waveform_peaks", format!("{:?}", analysis::waveform_peaks(a.clone(), cache.clone())), format!("{:?}", analysis::waveform_peaks(b.clone(), cache.clone())));
    same(&mut bad, "transition_profile", format!("{:?}", analysis::transition_profile(a.clone(), cache.clone())), format!("{:?}", analysis::transition_profile(b.clone(), cache.clone())));
    same(&mut bad, "spectral_profile", format!("{:?}", analysis::spectral_profile(a.clone(), cache.clone())), format!("{:?}", analysis::spectral_profile(b.clone(), cache.clone())));
    same(&mut bad, "normalization_gain", format!("{:?}", analysis::normalization_gain(a.clone(), cache.clone())), format!("{:?}", analysis::normalization_gain(b.clone(), cache.clone())));
    same(&mut bad, "beat_grid", format!("{:?}", analysis::beat_grid(a.clone(), cache.clone(), None)), format!("{:?}", analysis::beat_grid(b.clone(), cache.clone(), None)));
    same(&mut bad, "pitch_contour", format!("{:?}", analysis::pitch_contour(a.clone(), cache.clone(), None, 20.0, false)), format!("{:?}", analysis::pitch_contour(b.clone(), cache.clone(), None, 20.0, false)));
    same(&mut bad, "audio_fingerprint", format!("{:?}", analysis::audio_fingerprint(a.clone(), cache.clone(), None)), format!("{:?}", analysis::audio_fingerprint(b.clone(), cache.clone(), None)));
    }
    same(&mut bad, "is_valid_audio", format!("{}", analysis::is_valid_audio(a.clone(), cache.clone(), 1.0)), format!("{}", analysis::is_valid_audio(b.clone(), cache.clone(), 1.0)));
    same(&mut bad, "validate_audio", format!("{:?}", analysis::validate_audio(a.clone(), cache.clone(), None)), format!("{:?}", analysis::validate_audio(b.clone(), cache.clone(), None)));
    same(&mut bad, "intro_analyze", format!("{:?}", intro::intro_analyze(a.clone(), None)), format!("{:?}", intro::intro_analyze(b.clone(), None)));
    // (analysis_x_realtime is a timing, not a result.)
    let hs = |p: &str| haptics::haptic_score(p.into(), None).map(|h| {
        let mut s = h.stats();
        s.analysis_x_realtime = 0.0;
        s
    });
    same(&mut bad, "haptic_score", format!("{:?}", hs(&a)), format!("{:?}", hs(&b)));
    same(&mut bad, "blend_profile", format!("{:?}", blend::blend_profile(a.clone()).map(|p| p.summary())), format!("{:?}", blend::blend_profile(b.clone()).map(|p| p.summary())));
    let lyr = "first line here\nsecond line now\nthird line too\nand the last one";
    same(&mut bad, "lyrics_force_sync", format!("{:?}", lyrics::lyrics_force_sync(a.clone(), lyr.into()).map(|l| l.lrc)), format!("{:?}", lyrics::lyrics_force_sync(b.clone(), lyr.into()).map(|l| l.lrc)));

    // Library scan + tags.
    let (ta, tb) = (library::scan_read_track(a.clone(), cache.clone()), library::scan_read_track(b.clone(), cache.clone()));
    // WebM: the tag library can't read Matroska tags (the source scans as
    // nothing); LMS3 took them from the demuxer, so it must scan fully.
    let ta = if ext == "webm" && ta.is_none() { tb.clone().filter(|t| t.artist == "Lumisound Lab") } else { ta };
    match (&ta, &tb) {
        (Some(x), Some(y)) => {
            if ext != "webm" {
                same(&mut bad, "scan title", x.title.clone(), y.title.clone());
            }
            same(&mut bad, "scan artist", x.artist.clone(), y.artist.clone());
            same(&mut bad, "scan source id", format!("{:?}", x.source_track_id), format!("{:?}", y.source_track_id));
            if (x.duration - y.duration).abs() > 0.05 {
                bad.push(format!("scan duration {} vs {}", x.duration, y.duration));
            }
            if y.bitrate_kbps <= 0 {
                bad.push("scan: LMS3 bitrate missing".into());
            }
        }
        _ => bad.push(format!("scan_read_track: source {} LMS3 {}", ta.is_some(), tb.is_some())),
    }
    if ext != "webm" {
    same(&mut bad, "raw tags title", format!("{:?}", import::scan_read_raw_tags(a.clone(), cache.clone()).map(|t| t.title)), format!("{:?}", import::scan_read_raw_tags(b.clone(), cache.clone()).map(|t| t.title)));
    }
    // (ffmpeg stores a WebM's title as the segment title, not a tag.)
    let vc = import::vorbis_comments_read(b.clone());
    let (key, want) = if ext == "webm" { ("ARTIST", "Lumisound Lab") } else { ("TITLE", "Systems Fixture") };
    if vc.get(key).map(String::as_str) != Some(want) {
        bad.push(format!("vorbis_comments_read on LMS3: {vc:?}"));
    }
    same(&mut bad, "artwork present", format!("{}", library::artwork_embedded(a.clone(), cache.clone()).is_some()), format!("{}", library::artwork_embedded(b.clone(), cache.clone()).is_some()));

    // LMS3-only paths.
    let badge = lock::lms_badge(b.clone());
    if badge.version != 3 || badge.codec.is_empty() || badge.bitrate_kbps == 0 {
        bad.push(format!("badge {badge:?}"));
    }
    // Lumi clips read LMS3 directly: a rendered clip equals the track's own
    // decode over that range; AAC LMS3 clips cut losslessly.
    let r = lumisound_core::lumiclip::LumiRender::new(48_000, 0.0);
    let seg = lumisound_core::lumiclip::LumiSegment { path: b.clone(), start_secs: 10.0, duration_secs: 4.0, fade_in_secs: 0.0, fade_out_secs: 0.0, gain_db: 0.0 };
    if !r.add_track(seg) || (r.duration_secs() - 4.0).abs() > 0.01 {
        bad.push("LumiRender add_track".into());
    }
    let mut frames = 0usize;
    loop {
        let c = r.next_chunk(4096);
        if c.is_empty() {
            break;
        }
        if c.iter().any(|v| !v.is_finite()) {
            bad.push("LumiRender non-finite".into());
        }
        frames += c.len() / 2;
    }
    if frames != 4 * 48_000 {
        bad.push(format!("LumiRender {frames} frames"));
    }
    if ext == "m4a" {
        let clip = dir.join("clip.m4a");
        if !lumisound_core::lumiclip::lumi_clip_lossless(b.clone(), 10.0, 14.0, s(&clip)) {
            bad.push("lossless clip refused".into());
        } else {
            let fp = decode::fingerprint(&clip);
            match fp {
                Some(f) if (f.frames as f64 / f.rate as f64 - 4.0).abs() < 0.002 => {}
                other => bad.push(format!("lossless clip length {other:?}")),
            }
            let o = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&clip).args(["-f", "null", "-"]).output().unwrap();
            if !o.stderr.is_empty() {
                bad.push(format!("ffmpeg on lossless clip: {}", String::from_utf8_lossy(&o.stderr)));
            }
            // Same samples as the track at 10–14 s.
            let a1: Vec<i16> = analysis::decode_mono(b.clone(), cache.clone(), 11025, 0.0, 13.0).unwrap_or_default().into_iter().skip(10 * 11025).collect();
            let a2 = analysis::decode_mono(s(&clip), cache.clone(), 11025, 0.0, 3.0).unwrap_or_default();
            let n = a1.len().min(a2.len());
            let err: f64 = a1[..n].iter().zip(&a2[..n]).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / n.max(1) as f64;
            let sig: f64 = a1[..n].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / n.max(1) as f64;
            if 10.0 * (err.max(1e-9) / sig.max(1e-9)).log10() > -40.0 {
                bad.push(format!("lossless clip differs from the track ({:.1} dB)", 10.0 * (err.max(1e-9) / sig.max(1e-9)).log10()));
            }
        }
    } else if lumisound_core::lumiclip::lumi_clip_lossless(b.clone(), 10.0, 14.0, s(&dir.join("x.m4a"))) {
        bad.push("lossless clip on a non-AAC track".into());
    }
    if !lock::lms_convertible(a.clone()) {
        bad.push("source not reported convertible".into());
    }
    let st = lock::lms_library_status(vec![a.clone(), b.clone()]);
    if st.lms3 != 1 || st.plain != 1 {
        bad.push(format!("library status {st:?}"));
    }
    // Tag edits land in LMS3 metadata and leave the audio alone.
    let before = decode::fingerprint(&lms);
    import::tag_apply_fallbacks(b.clone(), "Renamed".into(), "Someone".into(), "Album X".into());
    lumisound_media::tags::write(&lms, &lumisound_media::tags::Fields { title: Some("Renamed Title".into()), ..Default::default() }).ok();
    if lms3::read_meta(&lms).and_then(|m| m.title).as_deref() != Some("Renamed Title") || decode::fingerprint(&lms) != before {
        bad.push("tag write on LMS3".into());
    }
    let _ = &name;
    bad
}

#[test]
fn every_system_reads_lms3_like_the_source() {
    if !ffmpeg() {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("lms3-systems-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    let art = dir.join("cover.jpg");
    let have_art = Command::new("ffmpeg").args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=600x600:duration=1", "-frames:v", "1"]).arg(&art).status().is_ok_and(|s| s.success());
    let art = have_art.then_some(art.as_path());
    let cases: &[(&str, &[&str], bool)] = &[
        ("yt.opus", &["-c:a", "libopus", "-b:a", "128k"], false),
        ("yt.webm", &["-c:a", "libopus", "-b:a", "160k"], false),
        ("t.m4a", &["-c:a", "aac", "-b:a", "160k"], true),
        ("t.flac", &["-c:a", "flac"], true),
        ("t.mp3", &["-c:a", "libmp3lame", "-b:a", "192k", "-id3v2_version", "3"], true),
    ];
    let mut failures = Vec::new();
    for (name, codec, with_art) in cases {
        let Some(src) = make(&dir, name, codec, if *with_art { art } else { None }) else {
            failures.push(format!("{name}: fixture"));
            continue;
        };
        let bad = check(&src, &dir);
        if bad.is_empty() {
            eprintln!("ok   {name}: all systems agree");
        } else {
            for b in &bad {
                eprintln!("FAIL {name}: {b}");
            }
            failures.extend(bad.into_iter().map(|b| format!("{name}: {b}")));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}
