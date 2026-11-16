//! Time-stretching (pitch-preserving speed) for interleaved stereo: a phase
//! vocoder with identity phase locking, stereo-coherent phases and
//! transient phase resets. Runs on the deck decode thread, never in the
//! audio callback.
//!
//! Why not WSOLA (the previous stretcher): splicing time-domain segments
//! only works when one alignment fits the whole signal. On chords and
//! polyphonic music no single offset does, so every splice broke phase and
//! the result warbled (measured THD+N ≈ -2 dB on a three-note chord at
//! 1.25x). Working per frequency bin keeps every partial continuous.
//!
//! * Identity phase locking (Laroche & Dolson): bins near a spectral peak
//!   keep their phase relation to the peak, so each partial stays one
//!   coherent sinusoid instead of a smear of independent bins.
//! * Stereo coherence: synthesis phases come from the mid (L+R) spectrum and
//!   the same rotation is applied to both channels, preserving the
//!   inter-channel phase (stereo image) instead of letting it drift.
//! * Transients: a frame whose spectrum suddenly gains energy across many
//!   bins (a drum hit) resets synthesis phase to the analysis phase, so
//!   attacks stay sharp instead of being smeared ("phasiness"). Only the
//!   bins that jumped (6 dB+) are reset: resetting the whole frame put the
//!   notes still ringing under the hit out of phase with the 7 overlapping
//!   frames before it, which cancelled them for ~85 ms — the "stutter dips"
//!   heard at every pitch (sustained bass fell 4-8 dB on each drum hit;
//!   ~320 dips a minute on cloud tracks). Trade-off measured: drum attacks
//!   at 1.25x speed are a little softer (pre-echo -13.8 dB vs -25 before).
//! * Attack gating (pre-echo): onsets are found in the input itself (a jump
//!   in high-frequency energy). An analysis frame whose second half would
//!   reach an onset is faded out just before it, so the hit can't leak into
//!   frames that are synthesised earlier in time — the pre-echo a long
//!   window otherwise smears ahead of every drum hit when stretching. The
//!   overlap-add is normalised by the window weight each output sample
//!   actually received, so the gated frames leave no level dip; the frames
//!   that do contain the hit start with it near their centre, where the
//!   bin-wise phase reset keeps it sharp. 1.25x: pre-echo -12 → see
//!   tests/engine_quality.rs; sustained notes under the hits keep level.
//! * At ratio 1 every frame is a phase reset: the overlap-add reconstructs
//!   the input exactly, so a stretch that returns to 1 (beat-matched
//!   crossfades) never jumps.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::PI;
use std::sync::Arc;

/// Frame size (≈85 ms at 48 kHz) and synthesis hop (87.5% overlap).
/// Measured (tests/engine_quality.rs) against the old 2048 / 75%: chords
/// at 1.25x -51.6 → -76.2 dB THD+N, chords pitched +3 st -39.7 → -74.6 dB,
/// and less pre-echo on drum hits (-17.9 → -25.2 dB), since the finer hop
/// lets transient phase resets land closer to the attack.
const N: usize = 4096;
const HS: usize = N / 8;
const BINS: usize = N / 2 + 1;
/// Hann² overlap-add at 87.5% overlap sums to 3.0.
const OLA_GAIN: f32 = 3.0;
/// Fraction of rising bins that marks a transient.
const TRANSIENT_FRACTION: f32 = 0.38;
/// Onset detector block (frames) and its thresholds: high-frequency energy
/// 8x the recent average, above an absolute floor, at most one per 60 ms.
const OD_BLOCK: usize = 128;
const OD_HISTORY: usize = 8;
const OD_JUMP: f32 = 8.0;
const OD_FLOOR: f32 = 1e-4;
const OD_SPACING: u64 = 2880;
/// Fade (frames) ending at a gated onset.
const GATE_FADE: usize = 64;
/// A bin "jumped" when its magnitude at least doubled (+6 dB): what was
/// there before is then at most a quarter of the power, so resetting its
/// phase can't audibly cancel it.
const BIN_JUMP: f32 = 2.0;

pub struct Wsola {
    /// Input frames per output frame (> 1 = faster).
    ratio: f64,
    input: Vec<[f32; 2]>,
    /// Fractional analysis position (frames into `input`).
    pos: f64,
    fwd: Arc<dyn Fft<f32>>,
    inv: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Overlap-add accumulator (N frames), output read from the front.
    acc: Vec<[f32; 2]>,
    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    prev_mag: Vec<f32>,
    /// Start (in `input`) of the previous analysis frame; the phase advance
    /// must use the hop actually taken (whole frames), not the fractional
    /// target, or every non-integer ratio accumulates phase error.
    prev_start: Option<u64>,
    /// Input frames dropped from the front so far (absolute positions).
    dropped: u64,
    started: bool,
    used: bool,
    /// Output frames still to drop: the stretched silence that primes the
    /// first frame (so a stream starts at full level, not on a window ramp).
    skip: u64,
    /// Output frames the real input calls for (Σ input / ratio) and the
    /// frames emitted, so `finish` ends the stream at exactly that length.
    expected: f64,
    emitted: u64,
    buf: [Vec<Complex32>; 2],
    mid: Vec<Complex32>,
    scratch: Vec<Complex32>,
    // Per-frame work buffers, reused (no allocation on the decode thread).
    mags: Vec<f32>,
    phase: Vec<f32>,
    peaks: Vec<usize>,
    new_synth: Vec<f32>,
    /// Bins that jumped this frame (transient frames only).
    jumped: Vec<bool>,
    /// Overlap-add weight each output sample received (gated) and would
    /// have received (ungated): output is scaled by their ratio.
    acc_w: Vec<f32>,
    acc_full: Vec<f32>,
    gate: Vec<f32>,
    /// Onset detector: recent block energies, the block being summed, the
    /// previous sample (for the first difference), detected onsets
    /// (absolute input frames).
    od_hist: std::collections::VecDeque<f32>,
    od_acc: f32,
    od_n: usize,
    od_prev: f32,
    od_pos: u64,
    od_last: u64,
    onsets: std::collections::VecDeque<u64>,
    /// Input frames owed by transient holds (frames walked at ratio 1 while
    /// a hit sat in the first half of the window), repaid over later hops.
    debt: f64,
}

fn wrap(x: f32) -> f32 {
    let y = (x + PI).rem_euclid(2.0 * PI) - PI;
    if y <= -PI { y + 2.0 * PI } else { y }
}

impl Wsola {
    pub fn new(ratio: f64) -> Self {
        let mut planner = FftPlanner::new();
        let fwd = planner.plan_fft_forward(N);
        let inv = planner.plan_fft_inverse(N);
        let scratch_len = fwd.get_inplace_scratch_len().max(inv.get_inplace_scratch_len());
        let window = (0..N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N as f32).cos()).collect();
        Self {
            ratio: ratio.clamp(0.25, 4.0),
            input: Vec::new(),
            pos: 0.0,
            fwd,
            inv,
            window,
            acc: vec![[0.0; 2]; N],
            prev_phase: vec![0.0; BINS],
            synth_phase: vec![0.0; BINS],
            prev_mag: vec![0.0; BINS],
            prev_start: None,
            dropped: 0,
            started: false,
            used: false,
            skip: 0,
            expected: 0.0,
            emitted: 0,
            buf: [vec![Complex32::new(0.0, 0.0); N], vec![Complex32::new(0.0, 0.0); N]],
            mid: vec![Complex32::new(0.0, 0.0); N],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            mags: vec![0.0; BINS],
            phase: vec![0.0; BINS],
            peaks: Vec::with_capacity(BINS / 2),
            new_synth: vec![0.0; BINS],
            jumped: vec![false; BINS],
            acc_w: vec![0.0; N],
            acc_full: vec![0.0; N],
            gate: vec![1.0; N],
            od_hist: std::collections::VecDeque::with_capacity(OD_HISTORY + 1),
            od_acc: 0.0,
            od_n: 0,
            od_prev: 0.0,
            od_pos: 0,
            od_last: 0,
            onsets: std::collections::VecDeque::new(),
            debt: 0.0,
        }
    }

    pub fn is_identity(&self) -> bool {
        (self.ratio - 1.0).abs() < 1e-6
    }

    /// Change the ratio mid-stream (beat-matched crossfades ramp it).
    pub fn set_ratio(&mut self, ratio: f64) {
        self.ratio = ratio.clamp(0.25, 4.0);
    }

    /// At ratio 1 with nothing ever processed the input passes straight through.
    pub fn can_bypass(&self) -> bool {
        self.is_identity() && !self.used
    }

    /// Kept for the deck's API. Once used, the vocoder keeps running: at
    /// ratio 1 it reconstructs the input exactly (phase reset every frame),
    /// so there is no seam to manage and nothing to drain.
    pub fn drain_identity(&mut self, _out: &mut Vec<f32>) {}

    /// Feed interleaved stereo, append stretched interleaved stereo to `out`.
    pub fn process(&mut self, interleaved: &[f32], out: &mut Vec<f32>) {
        if self.can_bypass() {
            out.extend_from_slice(interleaved);
            return;
        }
        if !self.used {
            // Prime with N - HS frames of silence: the first real sample then
            // sits where the overlap-add is already complete. The silence's
            // own stretched output is dropped below.
            self.used = true;
            // The silence is walked at ratio 1 (exact reconstruction), so the
            // first real sample comes out at full level, exactly `N - HS`
            // frames in; stretching starts from there.
            self.input.resize(N - HS, [0.0; 2]);
            self.skip = (N - HS) as u64;
        }
        self.expected += (interleaved.len() / 2) as f64 / self.ratio;
        let at = self.dropped + self.input.len() as u64;
        self.input.extend(interleaved.chunks_exact(2).map(|c| [c[0], c[1]]));
        self.detect_onsets(at);
        self.run(out);
    }

    /// End of stream: flush the tail still inside the analysis window, then
    /// cut the output to exactly the length the input calls for.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if !self.used {
            return;
        }
        let start = out.len();
        let before = self.emitted;
        self.input.extend(std::iter::repeat_n([0.0f32; 2], N + HS));
        self.run(out);
        let allowed = (self.expected.round() as u64).saturating_sub(before) as usize;
        out.truncate(start + (allowed * 2).min(out.len() - start));
        self.emitted = before + ((out.len() - start) / 2) as u64;
    }

    /// Scan input appended from absolute frame `from` for onsets.
    fn detect_onsets(&mut self, from: u64) {
        // Positions are absolute input frames, priming silence included.
        if self.od_pos == 0 && self.od_n == 0 && self.od_hist.is_empty() {
            self.od_pos = from;
        }
        let start = (from - self.dropped) as usize;
        for i in start..self.input.len() {
            let m = 0.5 * (self.input[i][0] + self.input[i][1]);
            let d = m - self.od_prev;
            self.od_prev = m;
            self.od_acc += d * d;
            self.od_n += 1;
            if self.od_n < OD_BLOCK {
                continue;
            }
            let e = self.od_acc / OD_BLOCK as f32;
            let block_start = self.od_pos;
            self.od_pos += OD_BLOCK as u64;
            self.od_acc = 0.0;
            self.od_n = 0;
            let avg = if self.od_hist.is_empty() { f32::MAX } else { self.od_hist.iter().sum::<f32>() / self.od_hist.len() as f32 };
            if e > OD_FLOOR && e > avg * OD_JUMP && block_start >= self.od_last + OD_SPACING && self.od_hist.len() == OD_HISTORY {
                // The attack may have begun late in the previous block:
                // date it a block early (an early cut is renormalised away;
                // a late one lets the attack leak).
                self.onsets.push_back(block_start.saturating_sub(OD_BLOCK as u64));
                self.od_last = block_start;
            }
            self.od_hist.push_back(e);
            if self.od_hist.len() > OD_HISTORY {
                self.od_hist.pop_front();
            }
        }
    }

    fn run(&mut self, out: &mut Vec<f32>) {
        while (self.pos as usize) + N <= self.input.len() {
            let start = self.pos as usize;
            let abs = self.dropped + start as u64;
            let hop = self.prev_start.map_or(HS as u64, |p| abs - p).max(1) as usize;
            let priming = abs < (N - HS) as u64;
            // Transient hold: while a fresh hit is in the first half of the
            // window, walk the input at ratio 1 so every frame places the
            // attack at the same output time (stretching across it moves it
            // earlier frame by frame: pre-echo). The time is repaid on the
            // following hops (≤ 25 % per hop), so tempo and length hold.
            let wanted = self.ratio;
            let stretching = !priming && (wanted - 1.0).abs() > 1e-6;
            let hold = stretching && self.debt.abs() < N as f64 && self.onsets.iter().any(|o| *o >= abs && *o < abs + (N / 2) as u64);
            let step = if priming {
                HS as f64
            } else if hold {
                self.debt += HS as f64 * (wanted - 1.0);
                HS as f64
            } else {
                let base = HS as f64 * wanted;
                let extra = self.debt.clamp(-0.25 * base, 0.25 * base);
                self.debt -= extra;
                base + extra
            };
            let ratio = if priming { 1.0 } else { wanted };
            self.frame(start, hop, (ratio - 1.0).abs() < 1e-6);
            self.prev_start = Some(abs);
            // Emit the finished front hop, shift the accumulator.
            for (i, f) in self.acc[..HS].iter().enumerate() {
                if self.skip > 0 {
                    self.skip -= 1;
                    continue;
                }
                // Make up for weight that gated frames didn't contribute.
                let (w, full) = (self.acc_w[i], self.acc_full[i]);
                let g = if w < full - 1e-4 { full / w.max(full * 0.3) } else { 1.0 };
                out.push(f[0] / OLA_GAIN * g);
                out.push(f[1] / OLA_GAIN * g);
                self.emitted += 1;
            }
            self.acc.copy_within(HS.., 0);
            for f in &mut self.acc[N - HS..] {
                *f = [0.0; 2];
            }
            self.acc_w.copy_within(HS.., 0);
            self.acc_full.copy_within(HS.., 0);
            self.acc_w[N - HS..].fill(0.0);
            self.acc_full[N - HS..].fill(0.0);
            self.pos += step;
            // Drop consumed input to bound memory.
            let drop = self.pos as usize;
            if drop > 16_384 {
                self.input.drain(..drop);
                self.pos -= drop as f64;
                self.dropped += drop as u64;
            }
        }
    }

    fn frame(&mut self, at: usize, hop: usize, exact: bool) {
        // Attack gate: an onset in this frame's second half is left out of it
        // (faded just before), so it can't be synthesised ahead of time.
        let abs = self.dropped + at as u64;
        while self.onsets.front().is_some_and(|o| *o < abs) {
            self.onsets.pop_front();
        }
        let cut = if exact {
            None
        } else {
            self.onsets.iter().find(|o| **o >= abs + (N / 2) as u64 && **o < abs + N as u64).map(|o| (o - abs) as usize)
        };
        match cut {
            Some(c) => {
                let fade_from = c.saturating_sub(GATE_FADE);
                for i in 0..N {
                    self.gate[i] = if i < fade_from { 1.0 } else if i >= c { 0.0 } else { 0.5 + 0.5 * (PI * (i - fade_from) as f32 / GATE_FADE as f32).cos() };
                }
            }
            None => self.gate.fill(1.0),
        }
        for i in 0..N {
            let w2 = self.window[i] * self.window[i];
            self.acc_full[i] += w2;
            self.acc_w[i] += w2 * self.gate[i];
        }
        // Analysis: windowed FFT of both channels and their mid.
        for ch in 0..2 {
            for i in 0..N {
                self.buf[ch][i] = Complex32::new(self.input[at + i][ch] * self.window[i] * self.gate[i], 0.0);
            }
            self.fwd.process_with_scratch(&mut self.buf[ch], &mut self.scratch);
        }
        for k in 0..BINS {
            self.mid[k] = self.buf[0][k] + self.buf[1][k];
        }
        // Transient: many bins jumping up in level at once.
        let mut rising = 0usize;
        let mut considered = 0usize;
        let mut mags = std::mem::take(&mut self.mags);
        for k in 0..BINS {
            mags[k] = self.mid[k].norm();
        }
        let floor = mags.iter().cloned().fold(0.0f32, f32::max) * 1e-3;
        for k in 4..BINS {
            if mags[k] > floor {
                considered += 1;
                if mags[k] > self.prev_mag[k] * 1.41 + 1e-9 {
                    rising += 1;
                }
            }
        }
        let transient = considered > 16 && rising as f32 / considered as f32 > TRANSIENT_FRACTION;
        let reset = !self.started || exact;
        if transient {
            for k in 0..BINS {
                self.jumped[k] = mags[k] > floor && mags[k] > self.prev_mag[k] * BIN_JUMP + 1e-9;
            }
        }

        let mut phase = std::mem::take(&mut self.phase);
        for k in 0..BINS {
            phase[k] = self.mid[k].arg();
        }
        if reset {
            self.synth_phase.copy_from_slice(&phase);
        } else {
            let ha = hop as f32;
            let hs = HS as f32;
            // Peaks (local maxima over ±2 bins) get true-frequency phase advance.
            let mut peaks = std::mem::take(&mut self.peaks);
            peaks.clear();
            for k in 2..BINS - 2 {
                let m = mags[k];
                if m > floor && m >= mags[k - 1] && m >= mags[k - 2] && m > mags[k + 1] && m > mags[k + 2] {
                    peaks.push(k);
                }
            }
            let mut new_synth = std::mem::take(&mut self.new_synth);
            for &p in &peaks {
                if transient && self.jumped[p] {
                    // New energy (the hit itself): take its own phase, sharp attack.
                    new_synth[p] = phase[p];
                    continue;
                }
                let omega = 2.0 * PI * p as f32 / N as f32;
                let dev = wrap(phase[p] - self.prev_phase[p] - omega * ha);
                let inst = omega + dev / ha;
                new_synth[p] = self.synth_phase[p] + inst * hs;
            }
            // Identity locking: every bin follows its nearest peak's rotation.
            if peaks.is_empty() {
                for k in 0..BINS {
                    let omega = 2.0 * PI * k as f32 / N as f32;
                    new_synth[k] = self.synth_phase[k] + omega * hs;
                }
            } else {
                let mut pi = 0;
                for k in 0..BINS {
                    while pi + 1 < peaks.len() && (peaks[pi + 1] as isize - k as isize).abs() <= (peaks[pi] as isize - k as isize).abs() {
                        pi += 1;
                    }
                    let p = peaks[pi];
                    if k != p {
                        new_synth[k] = new_synth[p] + (phase[k] - phase[p]);
                    }
                }
            }
            std::mem::swap(&mut self.synth_phase, &mut new_synth);
            self.new_synth = new_synth;
            self.peaks = peaks;
        }
        self.prev_phase.copy_from_slice(&phase);
        std::mem::swap(&mut self.prev_mag, &mut mags);
        self.mags = mags;
        self.started = true;

        // Same rotation (synthesis - analysis phase of the mid) on both
        // channels keeps their phase difference: the stereo image.
        // (`mid` is done with: it holds the per-bin rotation, computed once.)
        for k in 0..BINS {
            self.mid[k] = Complex32::from_polar(1.0, self.synth_phase[k] - phase[k]);
        }
        self.phase = phase;
        for ch in 0..2 {
            for k in 0..BINS {
                let v = self.buf[ch][k] * self.mid[k];
                self.buf[ch][k] = v;
                if k > 0 && k < N / 2 {
                    self.buf[ch][N - k] = v.conj();
                }
            }
            self.inv.process_with_scratch(&mut self.buf[ch], &mut self.scratch);
            let scale = 1.0 / N as f32;
            for i in 0..N {
                self.acc[i][ch] += self.buf[ch][i].re * scale * self.window[i];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stretches_length_and_keeps_pitch() {
        let sr = 48_000usize;
        let tone: Vec<f32> = (0..sr * 2).flat_map(|i| {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5;
            [v, v]
        }).collect();
        for ratio in [0.5, 1.5] {
            let mut w = Wsola::new(ratio);
            let mut out = Vec::new();
            for c in tone.chunks(4096) {
                w.process(c, &mut out);
            }
            let frames = out.len() / 2;
            let expect = (sr as f64 * 2.0 / ratio) as usize;
            assert!((frames as f64 - expect as f64).abs() / (expect as f64) < 0.05, "ratio {ratio}: {frames} vs {expect}");
            // Zero crossings per second ≈ 2·440 regardless of speed.
            let mid = &out[out.len() / 4..out.len() / 4 * 3];
            let zc = mid.chunks_exact(2).collect::<Vec<_>>().windows(2).filter(|p| (p[0][0] >= 0.0) != (p[1][0] >= 0.0)).count();
            let secs = mid.len() as f64 / 2.0 / sr as f64;
            let hz = zc as f64 / secs / 2.0;
            assert!((hz - 440.0).abs() < 15.0, "ratio {ratio}: {hz} Hz");
        }
    }

    #[test]
    fn ratio_one_after_use_reconstructs_exactly() {
        let sr = 48_000usize;
        let sig: Vec<f32> = (0..sr).flat_map(|i| {
            let t = i as f32 / sr as f32;
            let v = (2.0 * PI * 300.0 * t).sin() * 0.3 + (2.0 * PI * 1234.0 * t).sin() * 0.2;
            [v, -v * 0.5]
        }).collect();
        let mut w = Wsola::new(1.2);
        let mut out = Vec::new();
        w.process(&sig[..8192], &mut out);
        w.set_ratio(1.0);
        let before = out.len();
        w.process(&sig[8192..], &mut out);
        // After a frame of settling, output equals the input delayed by the
        // vocoder's latency (N - HS) relative to where the analysis was.
        let tail = &out[before + N * 2..];
        let mut best = f32::MAX;
        for lag in 0..(N * 4) {
            let off = lag * 2;
            if off + 4000 > sig.len() { break; }
            let err: f32 = tail.iter().take(4000).zip(&sig[off..off + 4000]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            best = best.min(err);
        }
        assert!(best < 1e-3, "ratio-1 reconstruction error {best}");
    }

    #[test]
    fn exact_length_and_full_level_from_the_start() {
        let sr = 48_000usize;
        let input: Vec<f32> = (0..sr).flat_map(|i| {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin() * 0.5;
            [v, v]
        }).collect();
        for ratio in [1.25, 0.8, 2.0] {
            let mut w = Wsola::new(ratio);
            let mut out = Vec::new();
            for c in input.chunks(1024) {
                w.process(c, &mut out);
            }
            w.finish(&mut out);
            let frames = out.len() / 2;
            let want = (sr as f64 / ratio).round() as usize;
            assert!(frames.abs_diff(want) <= 1, "ratio {ratio}: {frames} frames, want {want}");
            let rms = |a: usize, b: usize| (out[a * 2..b * 2].iter().step_by(2).map(|v| v * v).sum::<f32>() / (b - a) as f32).sqrt();
            let head = rms(0, 480);
            let body = rms(frames / 2, frames / 2 + 4800);
            assert!((head / body - 1.0).abs() < 0.1, "ratio {ratio}: first 10 ms at {:.2} of full level", head / body);
        }
    }
}

#[cfg(test)]
mod pre_echo_probe {
    use super::*;
    /// Per-hit pre-echo straight on the stretcher (engine_quality's fixture).
    #[test]
    #[ignore]
    fn per_hit() {
        let sr = 48_000f64;
        let sig: Vec<f32> = (0..(sr * 8.0) as usize).flat_map(|i| {
            let t = i as f64 / sr;
            let k = (t / 0.4).floor();
            let dt = t - k * 0.4 - 0.2;
            let pad = 0.03 * (std::f64::consts::TAU * 220.0 * t).sin();
            let v = if dt < 0.0 { pad } else {
                let n = ((t * 48_000.0).round() * 12.9898).sin() * 43758.5453;
                pad + 0.6 * (-dt / 0.015).exp() * (n - n.floor() - 0.5) * 2.0
            };
            [v as f32, v as f32]
        }).collect();
        for ratio in [1.0, 1.25, 0.8] {
            let mut w = Wsola::new(ratio);
            let mut out = Vec::new();
            if ratio == 1.0 { out = sig.clone(); } else {
            for c in sig.chunks(2048) { w.process(c, &mut out); }
            }
            let x: Vec<f64> = out.iter().step_by(2).map(|v| *v as f64).collect();
            let win = (sr * 0.025) as usize / 5;
            let env: Vec<f64> = x.chunks(win).map(|c| c.iter().map(|v| v * v).sum()).collect();
            let mut i = 10;
            let mut vals = vec![];
            while i + 10 < env.len() {
                if env[i] > 10.0 * env[i - 1] && env[i] > 1e-3 {
                    let b: f64 = env[i - 5..i].iter().sum();
                    let a: f64 = env[i..i + 5].iter().sum();
                    vals.push(format!("{:.1}@{:.2}s", 10.0 * (b / a).log10(), (i * win) as f64 / sr));
                    i += 20;
                } else { i += 1; }
            }
            eprintln!("ratio {ratio}: onsets detected {} | {}", w.onsets.len(), vals.join(" "));
        }
    }
}
