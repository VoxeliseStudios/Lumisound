//! Lumi clips: clip maker, clip export, lyric video audio, AI DJ set export
//! and song identification read library tracks directly — LMS3 (Lumisound
//! Audio) in place, locked or plain files too — through the same decoder
//! as the engine. No unlocked or decoded copy of a track is ever written.
//!
//! * [`LumiRender`]: a timeline of track segments (sample-exact start,
//!   gapless trimmed, equal-power fades, crossfades between consecutive
//!   tracks) and raw PCM (Aria's speech), rendered on demand as 48 kHz
//!   interleaved stereo. The app pulls chunks straight into Apple's AAC
//!   encoder (`AVAssetWriter`): only the finished export touches disk.
//! * [`lumi_clip_lossless`]: an AAC LMS3 track's clip cut as M4A with no
//!   re-encode at all (packets + an exact edit list).
//! * [`lumi_identify_clip`]: the first seconds as an 11.025 kHz mono WAV —
//!   Chromaprint's own working format — for the bridge's fingerprinting.
//! * [`lumi_envelope`]: per-video-frame loudness for the lyric video bars.

use lumisound_media::decode::{self, PcmStream};
use rubato::{FftFixedIn, Resampler};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct LumiSegment {
    pub path: String,
    pub start_secs: f64,
    /// 0 = to the end of the track.
    pub duration_secs: f64,
    pub fade_in_secs: f64,
    pub fade_out_secs: f64,
    pub gain_db: f32,
}

/// Where a segment's audio comes from.
enum Input {
    Track { path: String, start: f64 },
    Pcm { samples: Vec<f32>, rate: u32 },
}

struct Seg {
    input: Input,
    /// Output frames this segment lasts.
    frames: u64,
    /// Output frame where it starts.
    at: u64,
    fade_in: u64,
    fade_out: u64,
    gain: f32,
    is_track: bool,
}

/// A segment being played: decoded, resampled to the output rate.
struct Live {
    seg: usize,
    stream: Option<PcmStream>,
    pcm: Option<(Vec<f32>, usize)>,
    rs: Option<FftFixedIn<f32>>,
    src_rate: u32,
    /// Native-rate stereo waiting for the resampler (planar).
    pending: [Vec<f32>; 2],
    /// Output-rate stereo ready (interleaved).
    ready: VecDeque<f32>,
    /// Output frames this segment has emitted.
    done: u64,
    ended: bool,
}

const CHUNK: usize = 1024;

impl Live {
    fn open(seg: usize, s: &Seg, out_rate: u32) -> Option<Live> {
        let (stream, pcm, src_rate) = match &s.input {
            Input::Track { path, start } => {
                let st = PcmStream::open(Path::new(path), *start)?;
                let r = st.rate;
                (Some(st), None, r)
            }
            Input::Pcm { samples, rate } => (None, Some((samples.clone(), 0usize)), *rate),
        };
        let rs = (src_rate != out_rate).then(|| FftFixedIn::<f32>::new(src_rate as usize, out_rate as usize, CHUNK, 2, 2).ok()).flatten();
        Some(Live { seg, stream, pcm, rs, src_rate, pending: [Vec::new(), Vec::new()], ready: VecDeque::new(), done: 0, ended: false })
    }

    /// Source block of native-rate interleaved stereo.
    fn source_block(&mut self) -> Option<Vec<f32>> {
        if let Some(s) = self.stream.as_mut() {
            return s.next_block();
        }
        let (buf, pos) = self.pcm.as_mut()?;
        if *pos >= buf.len() {
            return None;
        }
        let n = (buf.len() - *pos).min(CHUNK * 2);
        let b = buf[*pos..*pos + n].to_vec();
        *pos += n;
        Some(b)
    }

    /// Make at least `want` output frames ready (fewer at the end).
    fn fill(&mut self, want: usize) {
        while self.ready.len() / 2 < want && !self.ended {
            match self.source_block() {
                Some(b) => {
                    if self.rs.is_none() {
                        self.ready.extend(b);
                        continue;
                    }
                    let (frames, _) = b.as_chunks::<2>();
                    for [l, r] in frames {
                        self.pending[0].push(*l);
                        self.pending[1].push(*r);
                    }
                    self.resample(false);
                }
                None => {
                    self.resample(true);
                    self.ended = true;
                }
            }
        }
    }

    fn resample(&mut self, last: bool) {
        let Some(rs) = self.rs.as_mut() else { return };
        loop {
            let need = rs.input_frames_next();
            let have = self.pending[0].len();
            let out = if have >= need {
                let inp = [&self.pending[0][..need], &self.pending[1][..need]];
                let r = rs.process(&inp, None);
                self.pending[0].drain(..need);
                self.pending[1].drain(..need);
                r
            } else if last && have > 0 {
                let inp = [&self.pending[0][..], &self.pending[1][..]];
                let r = rs.process_partial(Some(&inp), None);
                self.pending[0].clear();
                self.pending[1].clear();
                r
            } else {
                break;
            };
            let Ok(o) = out else { break };
            for i in 0..o[0].len() {
                self.ready.push_back(o[0][i]);
                self.ready.push_back(o[1][i]);
            }
        }
        let _ = self.src_rate;
    }
}

struct State {
    rate: u32,
    crossfade: f64,
    segs: Vec<Seg>,
    live: Vec<Live>,
    next_seg: usize,
    cursor: u64,
    total: u64,
}

impl State {
    fn layout(&mut self) {
        let xf = (self.crossfade * self.rate as f64) as u64;
        let mut at = 0u64;
        for i in 0..self.segs.len() {
            if i > 0 {
                let (pa, pb) = (&self.segs[i - 1], &self.segs[i]);
                let overlap = if pa.is_track && pb.is_track { xf.min(pa.frames / 2).min(pb.frames / 2) } else { 0 };
                at = (self.segs[i - 1].at + self.segs[i - 1].frames).saturating_sub(overlap);
                if overlap > 0 {
                    self.segs[i - 1].fade_out = self.segs[i - 1].fade_out.max(overlap);
                    self.segs[i].fade_in = self.segs[i].fade_in.max(overlap);
                }
            }
            self.segs[i].at = at;
        }
        self.total = self.segs.last().map(|s| s.at + s.frames).unwrap_or(0);
    }
}

/// Equal-power fade gain for frame `k` of a segment lasting `n` frames.
fn fade(k: u64, n: u64, fin: u64, fout: u64) -> f32 {
    let mut g = 1.0f64;
    if fin > 0 && k < fin {
        g *= (std::f64::consts::FRAC_PI_2 * k as f64 / fin as f64).sin();
    }
    if fout > 0 && k + fout > n {
        let left = n.saturating_sub(k) as f64;
        g *= (std::f64::consts::FRAC_PI_2 * left / fout as f64).sin();
    }
    g as f32
}

#[derive(uniffi::Object)]
pub struct LumiRender {
    st: Mutex<State>,
}

#[uniffi::export]
impl LumiRender {
    /// `sample_rate` of the output (48 000 for exports); `crossfade_secs`
    /// overlapped between consecutive track segments (0 = butt-joined).
    #[uniffi::constructor]
    pub fn new(sample_rate: u32, crossfade_secs: f64) -> Arc<Self> {
        Arc::new(LumiRender { st: Mutex::new(State { rate: sample_rate.max(8000), crossfade: crossfade_secs.max(0.0), segs: Vec::new(), live: Vec::new(), next_seg: 0, cursor: 0, total: 0 }) })
    }

    /// Add a slice of a library track (any format the engine plays,
    /// LMS3 read in place). False when it can't be decoded or is empty.
    pub fn add_track(&self, seg: LumiSegment) -> bool {
        let p = Path::new(&seg.path);
        let Some(info) = decode::probe(p) else { return false };
        let start = seg.start_secs.clamp(0.0, info.duration.max(0.0));
        let avail = (info.duration - start).max(0.0);
        let dur = if seg.duration_secs > 0.0 { seg.duration_secs.min(avail) } else { avail };
        if dur <= 0.01 || PcmStream::open(p, 0.0).is_none() {
            return false;
        }
        let mut st = self.st.lock().unwrap();
        let rate = st.rate as f64;
        st.segs.push(Seg {
            input: Input::Track { path: seg.path, start },
            frames: (dur * rate).round() as u64,
            at: 0,
            fade_in: (seg.fade_in_secs.max(0.0) * rate) as u64,
            fade_out: (seg.fade_out_secs.max(0.0) * rate) as u64,
            gain: 10f32.powf(seg.gain_db / 20.0),
            is_track: true,
        });
        st.layout();
        true
    }

    /// Add raw PCM (interleaved, `channels` 1 or 2) — e.g. Aria's speech.
    pub fn add_pcm(&self, samples: Vec<f32>, sample_rate: u32, channels: u32) {
        if samples.is_empty() || sample_rate == 0 {
            return;
        }
        let stereo: Vec<f32> = if channels <= 1 { samples.iter().flat_map(|v| [*v, *v]).collect() } else { samples.chunks(channels as usize).flat_map(|f| [f[0], f[1]]).collect() };
        let mut st = self.st.lock().unwrap();
        let frames = ((stereo.len() / 2) as f64 * st.rate as f64 / sample_rate as f64).round() as u64;
        st.segs.push(Seg { input: Input::Pcm { samples: stereo, rate: sample_rate }, frames, at: 0, fade_in: 0, fade_out: 0, gain: 1.0, is_track: false });
        st.layout();
    }

    pub fn duration_secs(&self) -> f64 {
        let st = self.st.lock().unwrap();
        st.total as f64 / st.rate as f64
    }

    pub fn sample_rate(&self) -> u32 {
        self.st.lock().unwrap().rate
    }

    pub fn progress(&self) -> f64 {
        let st = self.st.lock().unwrap();
        if st.total == 0 { 1.0 } else { (st.cursor as f64 / st.total as f64).min(1.0) }
    }

    /// Up to `max_frames` of interleaved stereo; empty when finished.
    pub fn next_chunk(&self, max_frames: u32) -> Vec<f32> {
        let mut st = self.st.lock().unwrap();
        let rate = st.rate;
        let n = (max_frames.max(1) as u64).min(st.total.saturating_sub(st.cursor)) as usize;
        if n == 0 {
            return Vec::new();
        }
        let (from, to) = (st.cursor, st.cursor + n as u64);
        // Open segments that start within this chunk.
        while st.next_seg < st.segs.len() && st.segs[st.next_seg].at < to {
            let i = st.next_seg;
            if let Some(l) = Live::open(i, &st.segs[i], rate) {
                st.live.push(l);
            }
            st.next_seg += 1;
        }
        let mut out = vec![0f32; n * 2];
        let State { segs, live, .. } = &mut *st;
        for l in live.iter_mut() {
            let s = &segs[l.seg];
            let seg_end = s.at + s.frames;
            let (a, b) = (from.max(s.at), to.min(seg_end));
            if a >= b {
                continue;
            }
            let want = (b - a) as usize;
            l.fill(want);
            for k in 0..want {
                let (Some(x), Some(y)) = (l.ready.pop_front(), l.ready.pop_front()) else { break };
                let g = fade(l.done, s.frames, s.fade_in, s.fade_out) * s.gain;
                let o = ((a - from) as usize + k) * 2;
                out[o] += x * g;
                out[o + 1] += y * g;
                l.done += 1;
            }
        }
        // Finished (or a source that ran out early: the rest is silence).
        live.retain(|l| l.done < segs[l.seg].frames && !(l.ended && l.ready.is_empty()));
        st.cursor = to;
        for v in &mut out {
            *v = v.clamp(-1.0, 1.0);
        }
        out
    }
}

/// Clip [start, end] of an AAC LMS3 track into `out` (M4A) with no
/// re-encode. False when the track isn't AAC LMS3 (render it instead).
#[uniffi::export]
pub fn lumi_clip_lossless(path: String, start_secs: f64, end_secs: f64, out_path: String) -> bool {
    lumisound_media::lms3_export::export_aac_clip(Path::new(&path), start_secs, end_secs, Path::new(&out_path)).is_some()
}

/// The first `max_secs` (from `start_secs`) as an 11.025 kHz mono 16-bit
/// WAV, for fingerprint identification.
#[uniffi::export]
pub fn lumi_identify_clip(path: String, start_secs: f64, max_secs: f64) -> Option<Vec<u8>> {
    let rate = 11_025u32;
    let pcm = decode::mono_pcm(Path::new(&path), rate, Some(start_secs.max(0.0)), Some(max_secs.max(1.0)))?;
    let data = pcm.len() as u32 * 2;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        b.extend_from_slice(&s.to_le_bytes());
    }
    Some(b)
}

/// Loudness per video frame (`fps`) over [start, start + duration],
/// normalised to the clip's own peak (0–1).
#[uniffi::export]
pub fn lumi_envelope(path: String, start_secs: f64, duration_secs: f64, fps: u32) -> Vec<f32> {
    let rate = 11_025u32;
    let Some(pcm) = decode::mono_pcm(Path::new(&path), rate, Some(start_secs.max(0.0)), Some(duration_secs.max(0.1))) else { return Vec::new() };
    let per = (rate / fps.max(1)).max(1) as usize;
    let env: Vec<f32> = pcm.chunks(per).map(|c| (c.iter().map(|v| (*v as f32 / 32768.0).powi(2)).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let peak = env.iter().cloned().fold(0.0, f32::max);
    if peak <= 0.0 { env } else { env.into_iter().map(|v| (v / peak).min(1.0)).collect() }
}

/// Seconds of music in a library track (LMS3 header, else the container).
#[uniffi::export]
pub fn lumi_duration(path: String) -> f64 {
    decode::probe(Path::new(&path)).map(|p| p.duration).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(path: &Path, hz: f64, secs: f64, rate: u32) {
        let n = (secs * rate as f64) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n * 4).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(n * 4).to_le_bytes());
        for i in 0..n {
            let v = ((std::f64::consts::TAU * hz * i as f64 / rate as f64).sin() * 0.5 * 32767.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn render_timeline_with_crossfade_and_pcm() {
        let d = std::env::temp_dir().join(format!("lumiclip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a.wav"), d.join("b.wav"));
        tone(&a, 440.0, 6.0, 44_100);
        tone(&b, 660.0, 6.0, 48_000);
        let r = LumiRender::new(48_000, 1.0);
        let seg = |p: &Path, start: f64| LumiSegment { path: p.to_string_lossy().into(), start_secs: start, duration_secs: 3.0, fade_in_secs: 0.0, fade_out_secs: 0.0, gain_db: 0.0 };
        assert!(r.add_track(seg(&a, 1.0)));
        assert!(r.add_track(seg(&b, 2.0)));
        r.add_pcm(vec![0.25; 24_000], 24_000, 1);
        assert!(!r.add_track(seg(&d.join("missing.wav"), 0.0)));
        // 3 + 3 - 1 (crossfade) + 1 s of speech.
        assert!((r.duration_secs() - 6.0).abs() < 0.01, "{}", r.duration_secs());
        let mut out = Vec::new();
        loop {
            let c = r.next_chunk(4096);
            if c.is_empty() {
                break;
            }
            out.extend(c);
        }
        assert_eq!(out.len() / 2, 6 * 48_000);
        let rms = |a: f64, b: f64| {
            let s = &out[(a * 48_000.0) as usize * 2..(b * 48_000.0) as usize * 2];
            (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
        };
        // Steady tone level (0.5 peak → 0.354 rms), the crossfade keeps power
        // (equal-power on uncorrelated tones), speech at 0.25.
        assert!((rms(0.5, 1.5) - 0.354).abs() < 0.02, "{}", rms(0.5, 1.5));
        assert!((rms(2.2, 2.8) - 0.354).abs() < 0.06, "{}", rms(2.2, 2.8));
        assert!((rms(5.2, 5.8) - 0.25).abs() < 0.01, "{}", rms(5.2, 5.8));
        assert!(out.iter().all(|v| v.is_finite()));
        // Identify clip and envelope read the same file.
        let wav = lumi_identify_clip(a.to_string_lossy().into(), 0.0, 2.0).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert!(wav.len() > 44 + 11_025 * 2 * 2 - 100);
        let env = lumi_envelope(a.to_string_lossy().into(), 1.0, 2.0, 30);
        assert!(env.len() >= 59 && env.iter().all(|v| (0.0..=1.0).contains(v)));
        let _ = std::fs::remove_dir_all(d);
    }
}
