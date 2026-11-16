//! On-device decoding with symphonia (MP3, AAC/ALAC in MP4, FLAC, WAV/AIFF/
//! CAF, Ogg Vorbis). Produces mono PCM at an analysis rate, like the
//! bridge's `ffmpeg -ac 1 -ar <rate> -f s16le`.

use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub struct Probe {
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

impl symphonia::core::io::MediaSource for crate::lock::MaskedFile {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

/// A symphonia source for `path` and its format hint: a plain file as is,
/// or a locked `.lms` file unmasked on the fly (decoded in place: no
/// unlocked copy of the whole track), hinted by its inner extension.
pub fn media_source(path: &Path) -> std::io::Result<(MediaSourceStream, Hint)> {
    let mut hint = Hint::new();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // LMS3: the file itself is the container (its reader unmasks packets).
    if crate::lms3::is_lms3(path) {
        hint.with_extension("lms");
        let file = std::fs::File::open(path)?;
        return Ok((MediaSourceStream::new(Box::new(file), Default::default()), hint));
    }
    if let Some(m) = crate::lock::MaskedFile::open(path) {
        if let Some(e) = crate::lock::inner_ext(name) {
            hint.with_extension(&e);
        }
        return Ok((MediaSourceStream::new(Box::new(m), Default::default()), hint));
    }
    // A legacy plain `.lms` (renamed, never masked): hint the inner format
    // ("Song.mp3.lms" -> mp3), not "lms".
    if let Some(e) = crate::lock::inner_ext(name).or_else(|| path.extension().and_then(|e| e.to_str()).map(str::to_string)) {
        hint.with_extension(&e);
    }
    if let Some(fd) = crate::fdfile::FdFile::open(path) {
        return Ok((MediaSourceStream::new(Box::new(fd?), Default::default()), hint));
    }
    let file = std::fs::File::open(path)?;
    Ok((MediaSourceStream::new(Box::new(file), Default::default()), hint))
}

/// symphonia's probe with every enabled format plus LMS3.
pub fn format_probe() -> &'static symphonia::core::probe::Probe {
    static P: std::sync::LazyLock<symphonia::core::probe::Probe> = std::sync::LazyLock::new(|| {
        let mut p = symphonia::core::probe::Probe::default();
        symphonia::default::register_enabled_formats(&mut p);
        p.register_all::<crate::lms3::Lms3Reader>();
        p
    });
    &P
}

fn open(path: &Path) -> Option<(Box<dyn symphonia::core::formats::FormatReader>, u32)> {
    let (mss, hint) = media_source(path).ok()?;
    let probed = format_probe().format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default()).ok()?;
    let fmt = probed.format;
    let track = audio_track(&*fmt)?;
    let id = track.id;
    Some((fmt, id))
}

/// The audio track: the container's default one when it's audio, else the
/// first track that has a sample rate (an .m4a/.mp4 with a video stream
/// first lists the video as default).
pub fn audio_track(fmt: &dyn symphonia::core::formats::FormatReader) -> Option<&symphonia::core::formats::Track> {
    let is_audio = |t: &&symphonia::core::formats::Track| t.codec_params.sample_rate.is_some() && t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL;
    fmt.default_track().filter(is_audio).or_else(|| fmt.tracks().iter().find(is_audio))
}

/// Seeks to `start` seconds of music (past any encoder priming) and
/// returns the trimmer for the frames decoded from there.
pub fn start_at(fmt: &mut dyn symphonia::core::formats::FormatReader, id: u32, rate: u32, gapless: Option<crate::gapless::Gapless>, start: Option<f64>) -> (crate::gapless::Trim, f64) {
    let mut from = 0u64;
    if let Some(s) = start.filter(|s| *s > 0.0) {
        let src = gapless.map_or(s, |g| g.source_frame(s, rate) as f64 / rate as f64);
        if let Ok(seeked) = fmt.seek(SeekMode::Accurate, SeekTo::Time { time: Time::new(src.trunc() as u64, src.fract()), track_id: Some(id) }) {
            let tb = fmt.tracks().iter().find(|t| t.id == id).and_then(|t| t.codec_params.time_base);
            from = match tb {
                Some(tb) => {
                    let t = tb.calc_time(seeked.actual_ts);
                    ((t.seconds as f64 + t.frac) * rate as f64).round() as u64
                }
                None => seeked.actual_ts,
            };
        }
    }
    let at = gapless.map_or(from as f64 / rate.max(1) as f64, |g| g.music_secs(from, rate));
    (gapless.map(|g| g.trim_from(from)).unwrap_or_default(), at)
}

/// Duration / rate / channels without decoding.
pub fn probe(path: &Path) -> Option<Probe> {
    let (fmt, id) = open(path)?;
    let t = fmt.tracks().iter().find(|t| t.id == id)?;
    let p = &t.codec_params;
    let sr = p.sample_rate?;
    let dur = match (p.n_frames, p.time_base) {
        (Some(n), Some(tb)) => {
            let t = tb.calc_time(n);
            t.seconds as f64 + t.frac
        }
        (Some(n), None) => n as f64 / sr as f64,
        _ => 0.0,
    };
    let dur = crate::gapless::for_track(path, p.codec).and_then(|g| g.duration(sr)).unwrap_or(dur);
    Some(Probe { duration: dur, sample_rate: sr, channels: p.channels.map(|c| c.count() as u16).unwrap_or(2) })
}

/// Mono i16 PCM at `rate` Hz from `start` seconds for at most `max_len`
/// seconds (whole file when None). Linear-interpolating resampler.
pub fn mono_pcm(path: &Path, rate: u32, start: Option<f64>, max_len: Option<f64>) -> Option<Vec<i16>> {
    let (mut fmt, id) = open(path)?;
    let params = fmt.tracks().iter().find(|t| t.id == id)?.codec_params.clone();
    let mut dec = crate::opus::codecs().make(&params, &DecoderOptions::default()).ok()?;
    // The decoder's rate: HE-AAC plays at twice its core rate.
    let src_rate = dec.codec_params().sample_rate.or(params.sample_rate)? as f64;
    let (mut trim, _) = start_at(&mut *fmt, id, src_rate as u32, crate::gapless::for_track(path, params.codec), start);
    let want = max_len.map(|l| (l * rate as f64) as usize);
    let step = src_rate / rate as f64;
    let mut out: Vec<i16> = Vec::with_capacity(want.unwrap_or(rate as usize * 60));
    let mut mono: Vec<f32> = Vec::new();
    let mut pos = 0.0f64; // fractional read position into `mono`
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        if want.is_some_and(|w| out.len() >= w) {
            break;
        }
        let packet = match fmt.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(_)) | Err(Error::ResetRequired) => break,
            Err(_) => break,
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * ch {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        let keep = trim.keep(b.samples().len() / ch);
        for frame in b.samples()[keep.start * ch..keep.end * ch].chunks(ch) {
            mono.push(frame.iter().sum::<f32>() / ch as f32);
        }
        // Resample everything we can from `mono`, keep the remainder.
        while pos + 1.0 < mono.len() as f64 {
            let i = pos as usize;
            let f = (pos - i as f64) as f32;
            let v = mono[i] * (1.0 - f) + mono[i + 1] * f;
            out.push((v.clamp(-1.0, 1.0) * 32767.0) as i16);
            pos += step;
            if want.is_some_and(|w| out.len() >= w) {
                break;
            }
        }
        let used = (pos as usize).min(mono.len());
        mono.drain(..used);
        pos -= used as f64;
    }
    if let Some(w) = want {
        out.truncate(w);
    }
    (!out.is_empty()).then_some(out)
}

/// Mono i16 PCM at `rate` Hz for the whole file, handed to `f` in blocks of
/// `block` samples (the last one shorter), decoded once from the top: no
/// seeking, so block boundaries are sample-exact. `f` returns false to stop.
pub fn mono_pcm_blocks(path: &Path, rate: u32, block: usize, mut f: impl FnMut(Vec<i16>) -> bool) -> bool {
    let Some((mut fmt, id)) = open(path) else { return false };
    let Some(params) = fmt.tracks().iter().find(|t| t.id == id).map(|t| t.codec_params.clone()) else { return false };
    let Ok(mut dec) = crate::opus::codecs().make(&params, &DecoderOptions::default()) else { return false };
    let Some(src_rate) = dec.codec_params().sample_rate.or(params.sample_rate).map(|r| r as f64) else { return false };
    let mut trim = crate::gapless::for_track(path, params.codec).map(|g| g.trim_from(0)).unwrap_or_default();
    let step = src_rate / rate as f64;
    let block = block.max(1);
    let mut out: Vec<i16> = Vec::with_capacity(block);
    let mut mono: Vec<f32> = Vec::new();
    let mut pos = 0.0f64;
    let mut buf: Option<SampleBuffer<f32>> = None;
    let mut any = false;
    loop {
        let packet = match fmt.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * ch {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        let keep = trim.keep(b.samples().len() / ch);
        for frame in b.samples()[keep.start * ch..keep.end * ch].chunks(ch) {
            mono.push(frame.iter().sum::<f32>() / ch as f32);
        }
        while pos + 1.0 < mono.len() as f64 {
            let i = pos as usize;
            let fr = (pos - i as f64) as f32;
            let v = mono[i] * (1.0 - fr) + mono[i + 1] * fr;
            out.push((v.clamp(-1.0, 1.0) * 32767.0) as i16);
            pos += step;
            if out.len() >= block {
                any = true;
                if !f(std::mem::replace(&mut out, Vec::with_capacity(block))) {
                    return true;
                }
            }
        }
        let used = (pos as usize).min(mono.len());
        mono.drain(..used);
        pos -= used as f64;
    }
    if !out.is_empty() {
        any = true;
        f(out);
    }
    any
}

/// Native-rate float PCM, de-interleaved per channel (max 2 used), from
/// `start` for up to `max_len` seconds. Returns (rate, channels).
pub fn planar(path: &Path, start: Option<f64>, max_len: Option<f64>) -> Option<(u32, Vec<Vec<f32>>)> {
    let (mut fmt, id) = open(path)?;
    let params = fmt.tracks().iter().find(|t| t.id == id)?.codec_params.clone();
    let mut dec = crate::opus::codecs().make(&params, &DecoderOptions::default()).ok()?;
    let rate = dec.codec_params().sample_rate.or(params.sample_rate)?;
    let (mut trim, _) = start_at(&mut *fmt, id, rate, crate::gapless::for_track(path, params.codec), start);
    let want = max_len.map(|l| (l * rate as f64) as usize);
    let mut chans: Vec<Vec<f32>> = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        if want.is_some_and(|w| chans.first().is_some_and(|c| c.len() >= w)) {
            break;
        }
        let packet = match fmt.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        if chans.is_empty() {
            chans = vec![Vec::new(); ch.min(2)];
        }
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * ch {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        let keep = trim.keep(b.samples().len() / ch);
        for f in b.samples()[keep.start * ch..keep.end * ch].chunks(ch) {
            for (c, v) in chans.iter_mut().zip(f) {
                c.push(*v);
            }
        }
    }
    if let Some(w) = want {
        chans.iter_mut().for_each(|c| c.truncate(w));
    }
    (!chans.is_empty() && !chans[0].is_empty()).then_some((rate, chans))
}

/// Whole-file decode in the engine's pipeline (gapless trimmed), handing
/// interleaved native-rate float blocks to `f(rate, channels, samples)`.
/// `f` returns false to stop. False when nothing could be decoded.
pub fn for_each_block(path: &Path, mut f: impl FnMut(u32, usize, &[f32]) -> bool) -> bool {
    let Some((mut fmt, id)) = open(path) else { return false };
    let Some(params) = fmt.tracks().iter().find(|t| t.id == id).map(|t| t.codec_params.clone()) else { return false };
    let Ok(mut dec) = crate::opus::codecs().make(&params, &DecoderOptions::default()) else { return false };
    let Some(rate) = dec.codec_params().sample_rate.or(params.sample_rate) else { return false };
    let mut trim = crate::gapless::for_track(path, params.codec).map(|g| g.trim_from(0)).unwrap_or_default();
    let mut buf: Option<SampleBuffer<f32>> = None;
    let mut any = false;
    while let Ok(packet) = fmt.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let decoded = match dec.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * ch {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        let keep = trim.keep(b.samples().len() / ch);
        if keep.is_empty() {
            continue;
        }
        any = true;
        if !f(rate, ch, &b.samples()[keep.start * ch..keep.end * ch]) {
            return true;
        }
    }
    any
}

/// What a file sounds like, exactly: (frames, rate, channels, CRC32 of the
/// decoded float samples). Two files with equal fingerprints decode to the
/// same audio sample for sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    pub frames: u64,
    pub rate: u32,
    pub channels: u16,
    pub crc: u32,
}

pub fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let mut crc = crc32fast::Hasher::new();
    let (mut frames, mut spec) = (0u64, (0u32, 0u16));
    let mut bytes: Vec<u8> = Vec::new();
    let ok = for_each_block(path, |rate, ch, s| {
        spec = (rate, ch as u16);
        frames += (s.len() / ch) as u64;
        bytes.clear();
        bytes.extend(s.iter().flat_map(|v| v.to_bits().to_le_bytes()));
        crc.update(&bytes);
        true
    });
    ok.then(|| Fingerprint { frames, rate: spec.0, channels: spec.1, crc: crc.finalize() })
}
