//! LMS3 (Lumisound Audio) through the Lumisound Audio Engine itself: the
//! decks read LMS3 in place (no unlocked copy) and must sound exactly like
//! the source file they were made from.
//!
//!   cargo test --release -p lumisound-core --test engine_lms3 -- --nocapture --test-threads=1
//!
//! Fixtures are made with ffmpeg (Opus/Ogg, Opus/WebM, AAC/M4A, FLAC, MP3;
//! skipped without ffmpeg). `LUMI_LMS3_DIR=<dir of LMS3 .lms files>` also
//! plays real converted library tracks. Every failure is collected and
//! reported together.

use lumisound_core::engine::{AudioEngine, EngineListener};
use lumisound_media::{decode, lms3, lms3_export};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Events(Mutex<Vec<String>>);
impl EngineListener for Events {
    fn on_track_ended(&self, id: u64) {
        self.0.lock().unwrap().push(format!("end{id}"));
    }
    fn on_track_started(&self, id: u64) {
        self.0.lock().unwrap().push(format!("start{id}"));
    }
    fn on_track_failed(&self, id: u64) {
        self.0.lock().unwrap().push(format!("fail{id}"));
    }
}

const CHUNK: usize = 1024;

fn ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// 14 s of music-like audio: a chord with a moving melody, pink noise and
/// a click track (transients), so decoders really work.
fn make(dir: &Path, name: &str, codec: &[&str], seed: u32) -> Option<PathBuf> {
    let out = dir.join(name);
    let expr = format!(
        "0.18*sin(2*PI*(220+{seed})*t)+0.12*sin(2*PI*277.18*t)+0.1*sin(2*PI*329.63*t)+0.15*sin(2*PI*(440+40*sin(2*PI*0.5*t))*t)+0.25*exp(-40*mod(t\\,0.5))*sin(2*PI*90*t)|0.18*sin(2*PI*(220+{seed})*t+0.3)+0.12*sin(2*PI*277.18*t)+0.1*sin(2*PI*392*t)+0.15*sin(2*PI*(660+30*sin(2*PI*0.7*t))*t)+0.25*exp(-40*mod(t+0.25\\,0.5))*sin(2*PI*90*t)"
    );
    let ok = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("aevalsrc={expr}:s=48000:d=14"),
               "-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.03:d=14:r=48000",
               "-filter_complex", "[0][1]amix=inputs=2:normalize=0[a]", "-map", "[a]", "-metadata", "title=Engine Fixture"])
        .args(codec)
        .arg(&out)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(out)
}

struct Played {
    out: Vec<f32>,
    rate: f64,
    pos_start: f64,
    health: lumisound_core::engine::health::EngineHealth,
}

/// Plays `path` from `start` for `secs` through the engine's offline path,
/// paced so the decode threads keep up (as on a phone).
fn play(path: &Path, start: f64, secs: f64, setup: impl Fn(&AudioEngine)) -> Result<Played, String> {
    let ev = Arc::new(Events(Mutex::new(vec![])));
    let e = AudioEngine::new(ev.clone());
    setup(&e);
    e.load(path.to_string_lossy().into(), start, 0.0).map_err(|err| format!("load: {err:?}"))?;
    std::thread::sleep(Duration::from_millis(300));
    e.play();
    let rate = e.output_rate() as f64;
    let n = (secs * rate) as usize / CHUNK;
    let mut out = Vec::with_capacity(n * CHUNK * 2);
    let mut pos_start = f64::NAN;
    for i in 0..n {
        out.extend(e.render_offline(CHUNK));
        if i == 0 {
            // The engine reports a position once audio has been rendered.
            pos_start = e.position() - CHUNK as f64 / rate;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    Ok(Played { out, rate, pos_start, health: e.health() })
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Best alignment (in stereo frames, b relative to a) within ±`max` frames
/// and the residual level (dB below the signal) at that alignment.
fn aligned_residual_db(a: &[f32], b: &[f32], max: i64) -> (i64, f64) {
    let frames = (a.len().min(b.len()) / 2) as i64 - 2 * max;
    let mut best = (0, f64::MAX);
    for lag in -max..=max {
        let mut err = 0.0f64;
        for f in (max..max + frames).step_by(7) {
            let i = (f * 2) as usize;
            let j = ((f + lag) * 2) as usize;
            err += ((a[i] - b[j]) as f64).powi(2) + ((a[i + 1] - b[j + 1]) as f64).powi(2);
        }
        if err < best.1 {
            best = (lag, err);
        }
    }
    let n = (frames / 7).max(1) as f64 * 2.0;
    (best.0, 20.0 * ((best.1 / n).sqrt() / rms(a).max(1e-9)).max(1e-12).log10())
}

fn clean(h: &lumisound_core::engine::health::EngineHealth) -> Result<(), String> {
    if h.deck_failures > 0 || h.decode_errors > 0 || h.decode_stopped_early > 0 || h.nonfinite_samples > 0 {
        return Err(format!("health: failures {} decode errors {} stopped early {} non-finite {}", h.deck_failures, h.decode_errors, h.decode_stopped_early, h.nonfinite_samples));
    }
    Ok(())
}

/// The checks for one source file and its LMS3 conversion.
fn check(src: &Path, dir: &Path) -> Result<String, String> {
    let lms = dir.join(format!("{}.lms", src.file_name().unwrap().to_string_lossy()));
    let w = lms3::write_verified(src, &lms, &lms3::WriteOpts::default()).map_err(|e| format!("convert: {e}"))?;
    let mut notes = vec![format!("{} {} kbps", w.codec, w.header.bitrate_kbps())];

    // 1. From the top: the engine's output must be the same, sample for sample.
    let a = play(src, 0.0, 5.0, |_| {})?;
    let b = play(&lms, 0.0, 5.0, |_| {})?;
    clean(&b.health)?;
    let d = max_diff(&a.out, &b.out);
    if d > 1e-6 {
        let (lag, res) = aligned_residual_db(&a.out, &b.out, 2400);
        return Err(format!("from start: max diff {d:.2e} (best lag {lag} frames, residual {res:.1} dB)"));
    }
    notes.push("identical from start".into());

    // 2. Seeks: land near the target (one packet / the MP3 reservoir), next
    // to where the source file's own reader lands, and play exactly the
    // music at the position the engine reports (checked against a
    // continuous play-through from the top).
    for start in [3.37, 7.0, 11.5] {
        let a = play(src, start, 1.5, |_| {})?;
        let b = play(&lms, start, 1.5, |_| {})?;
        clean(&b.health)?;
        if (b.pos_start - start).abs() > 0.06 || (b.pos_start - a.pos_start).abs() > 0.025 {
            return Err(format!("seek {start}: engine at {:.3} (source file: {:.3})", b.pos_start, a.pos_start));
        }
        let full = play(&lms, 0.0, start + 1.6, |_| {})?;
        let at = (b.pos_start * full.rate).round() as usize * 2;
        let seg = full.out.get(at..).ok_or("short")?;
        let (lag, res) = aligned_residual_db(seg, &b.out, (0.05 * full.rate) as i64);
        // The decoder starts cold after a seek; its first few ms converge.
        if lag.abs() as f64 > 0.001 * full.rate || res > -40.0 {
            return Err(format!("seek {start}: reported {:.3}s but audio is {lag} frames off (residual {res:.1} dB)", b.pos_start));
        }
    }
    notes.push("seeks ok".into());

    // 3. Everything on (speed, pitch, EQ, reverb, spatial, modulation): finite, unclipped, audible.
    let fx = play(&lms, 4.0, 3.0, |e| {
        e.set_speed_pitch(1.15, 2.0).unwrap();
        e.set_eq(vec![6.0, 5.0, 3.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0], true);
        e.set_effects(0.35, "largeRoom".into(), false, true);
        e.set_spatial(true, 30.0);
        e.set_modulation(0.2, 4.0, 0.2, 0.0, 0.0);
    })?;
    clean(&fx.health)?;
    if fx.out.iter().any(|v| !v.is_finite() || v.abs() > 1.0) || rms(&fx.out) < 1e-3 {
        return Err("effects render bad (non-finite/clipped/silent)".into());
    }
    notes.push("effects ok".into());
    Ok(notes.join(", "))
}

/// Gapless and crossfade between two LMS3 tracks.
fn transitions(a: &Path, b: &Path) -> Result<String, String> {
    let ev = Arc::new(Events(Mutex::new(vec![])));
    let e = AudioEngine::new(ev.clone());
    let dur_a = decode::probe(a).ok_or("probe a")?.duration;
    let id_a = e.load(a.to_string_lossy().into(), dur_a - 2.0, 0.0).map_err(|x| format!("{x:?}"))?;
    let id_b = e.queue_next(b.to_string_lossy().into(), 0.0).map_err(|x| format!("{x:?}"))?;
    std::thread::sleep(Duration::from_millis(400));
    e.play();
    let rate = e.output_rate() as f64;
    let mut out = Vec::new();
    for _ in 0..((4.0 * rate) as usize / CHUNK) {
        out.extend(e.render_offline(CHUNK));
        std::thread::sleep(Duration::from_millis(3));
    }
    let evs = ev.0.lock().unwrap().clone();
    // A gapless hand-over reports the next track started (no separate end).
    let _ = id_a;
    if !evs.contains(&format!("start{id_b}")) || evs.iter().any(|x| x.starts_with("fail")) {
        return Err(format!("gapless events {evs:?}"));
    }
    clean(&e.health())?;
    // No hole at the hand-off: every 10 ms window keeps sound.
    let w = (0.01 * rate) as usize * 2;
    let quiet = out.chunks(w).filter(|c| c.len() == w && rms(c) < 1e-3).count();
    if quiet > 0 {
        return Err(format!("gapless: {quiet} silent 10 ms window(s)"));
    }
    // Crossfade back into A.
    let id_x = e.crossfade_to(a.to_string_lossy().into(), 1.5, 2.0, 0.0, true, 1.0).map_err(|x| format!("{x:?}"))?;
    std::thread::sleep(Duration::from_millis(200));
    for _ in 0..((2.5 * rate) as usize / CHUNK) {
        e.render_offline(CHUNK);
        std::thread::sleep(Duration::from_millis(3));
    }
    if e.current_id() != id_x && !ev.0.lock().unwrap().contains(&format!("start{id_x}")) {
        return Err("crossfade didn't take over".into());
    }
    clean(&e.health())?;
    Ok("gapless + crossfade ok".into())
}

/// Decode throughput (x real time) of a file, whole file, gapless trimmed.
fn speed(p: &Path) -> f64 {
    let t = Instant::now();
    let fp = decode::fingerprint(p).unwrap();
    (fp.frames as f64 / fp.rate as f64) / t.elapsed().as_secs_f64()
}

#[test]
fn lms3_plays_through_the_engine() {
    if !ffmpeg() {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("engine-lms3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cases: &[(&str, &[&str])] = &[
        ("yt.opus", &["-c:a", "libopus", "-b:a", "130k"]),
        ("yt.webm", &["-c:a", "libopus", "-b:a", "256k"]),
        ("t.m4a", &["-c:a", "aac", "-b:a", "128k"]),
        ("t.flac", &["-c:a", "flac"]),
        ("t.mp3", &["-c:a", "libmp3lame", "-b:a", "192k"]),
    ];
    let mut failures = Vec::new();
    let mut made = Vec::new();
    for (i, (name, codec)) in cases.iter().enumerate() {
        let Some(src) = make(&dir, name, codec, i as u32 * 7) else {
            eprintln!("skip {name}: ffmpeg can't encode it here");
            continue;
        };
        match check(&src, &dir) {
            Ok(s) => {
                let lms = dir.join(format!("{name}.lms"));
                eprintln!("ok   {name}: {s}; decode {:.0}x real time (source {:.0}x)", speed(&lms), speed(&src));
                made.push(lms);
            }
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    if made.len() >= 2 {
        match transitions(&made[0], &made[2.min(made.len() - 1)]) {
            Ok(s) => eprintln!("ok   transitions: {s}"),
            Err(e) => failures.push(format!("transitions: {e}")),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}

/// Real converted library tracks: in-place engine playback vs the same
/// track's standard export (what the system player gets).
#[test]
fn lms3_real_tracks_through_the_engine() {
    let Ok(dir) = std::env::var("LUMI_LMS3_DIR") else { return };
    let work = std::env::temp_dir().join(format!("engine-lms3-real-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let mut failures = Vec::new();
    for f in std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| lms3::is_lms3(p)) {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let r = (|| -> Result<String, String> {
            let h = lms3::read_header(&f).ok_or("header")?;
            let start = (h.duration_secs() * 0.4).clamp(0.0, 30.0);
            let export = work.join(format!("x.{}", lms3_export::export_ext(&h)));
            lms3_export::export_to_path(&f, &export).map_err(|e| e.to_string())?;
            let a = play(&export, start, 4.0, |_| {})?;
            let b = play(&f, start, 4.0, |_| {})?;
            clean(&b.health)?;
            if b.health.underrun_events > 0 {
                return Err(format!("{} underrun(s)", b.health.underrun_events));
            }
            // Lands next to where the standard file's reader does (Ogg seeks
            // by page, so up to one packet apart) …
            if (b.pos_start - a.pos_start).abs() > 0.025 {
                return Err(format!("seek: engine at {:.3}, export at {:.3}", b.pos_start, a.pos_start));
            }
            // … and plays exactly the music at the position it reports.
            let full = play(&f, 0.0, start + 1.6, |_| {})?;
            let at = (b.pos_start * full.rate).round() as usize * 2;
            let (lag, res) = aligned_residual_db(full.out.get(at..).ok_or("short")?, &b.out, (0.05 * full.rate) as i64);
            if lag.abs() as f64 > 0.001 * full.rate || res > -40.0 {
                return Err(format!("reported {:.3}s but audio is {lag} frames off (residual {res:.1} dB)", b.pos_start));
            }
            let _ = std::fs::remove_file(&export);
            Ok(format!("{} {} kbps, {:.0}s, seek honest (residual {res:.0} dB), decode {:.0}x", lms3::codec_name(h.codec_type()), h.bitrate_kbps(), h.duration_secs(), speed(&f)))
        })();
        match r {
            Ok(s) => eprintln!("ok   {name}: {s}"),
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    assert!(failures.is_empty(), "{} failure(s):\n{}", failures.len(), failures.join("\n"));
}
