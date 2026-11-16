//! LMS3, the Lumisound Audio file (`.lms`, magic `LMSAUDIO`).
//!
//! A native container for the Lumisound engine. A track is demuxed once and
//! its coded packets are stored exactly as received (no re-encode, so no
//! generation loss: the bitrate is whatever the best source had), with
//! everything the engine and app need beside them:
//!
//! ```text
//! header (160 B) | codec extra data | payload (masked packets) | packet index | metadata
//! ```
//!
//! * header: codec parameters (symphonia `CodecParameters`), gapless range,
//!   flags, section offsets, a CRC of the plain payload and a CRC of the
//!   header itself.
//! * payload: packets back to back, XOR-masked with the lock key
//!   (payload-relative offsets), so the bytes are not audio to anything but
//!   Lumisound.
//! * index: one varint record per packet (size, timestamp delta, duration,
//!   optional trim): exact sample-accurate seeking without scanning.
//! * metadata: tags, artwork, source id, and the quality the track was
//!   fetched at (used to decide quality upgrades).
//!
//! The engine reads it through [`Lms3Reader`] (a symphonia `FormatReader`);
//! anything that needs a standard file (AVFoundation, ffmpeg, the share
//! sheet) gets one from [`crate::lms3_export`].

use crate::gapless::Gapless;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use symphonia::core::audio::{Channels, Layout};
use symphonia::core::codecs::{CODEC_TYPE_NULL, CodecParameters, CodecType};
use symphonia::core::errors::{Error, Result as SymResult, decode_error, seek_error};
use symphonia::core::formats::{Cue, FormatOptions, FormatReader, Packet, SeekMode, SeekTo, SeekedTo, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{Metadata, MetadataLog, MetadataOptions};
use symphonia::core::probe::{Descriptor, Instantiate, QueryDescriptor};
use symphonia::core::sample::SampleFormat;
use symphonia::core::units::TimeBase;

pub const MAGIC: &[u8; 8] = b"LMSAUDIO";
pub const VERSION: u16 = 3;
pub const HEADER_LEN: usize = 160;
/// Header flags (same bits as the LMSLOCK2 header).
pub const FLAG_ENGINE_EXCLUSIVE: u32 = crate::lock::FLAG_ENGINE_EXCLUSIVE;
pub const FLAG_LYRICS_SYNCED: u32 = crate::lock::FLAG_LYRICS_SYNCED;
const NONE64: u64 = u64::MAX;
const NONE32: u32 = u32::MAX;
const META_MAGIC: &[u8; 4] = b"LMTA";
/// Bitrate (kbps) at or above which a YouTube track counts as Premium
/// quality (YouTube's free streams top out around 130–160 kbps; Premium
/// Opus 774 is ~256).
pub const PREMIUM_KBPS: u32 = 200;

// ---- codec ids -------------------------------------------------------------------------------

/// symphonia keeps a `CodecType`'s number private; its `Display` is the
/// number in hex, which is what the header stores.
pub fn codec_id(c: CodecType) -> u32 {
    u32::from_str_radix(c.to_string().trim_start_matches("0x"), 16).unwrap_or(0)
}

/// The `CodecType` for a stored id (`CODEC_TYPE_NULL` when unknown).
pub fn codec_from_id(id: u32) -> CodecType {
    static KNOWN: std::sync::LazyLock<Vec<(u32, CodecType)>> = std::sync::LazyLock::new(|| {
        [
        symphonia::core::codecs::CODEC_TYPE_PCM_S32LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S32LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S32BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S32BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S24LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S24LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S24BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S24BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S16LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S16LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S16BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_S16BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_S8,
        symphonia::core::codecs::CODEC_TYPE_PCM_S8_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U32LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U32LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U32BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U32BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U24LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U24LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U24BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U24BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U16LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U16LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U16BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_U16BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_U8,
        symphonia::core::codecs::CODEC_TYPE_PCM_U8_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_F32LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_F32LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_F32BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_F32BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_F64LE,
        symphonia::core::codecs::CODEC_TYPE_PCM_F64LE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_F64BE,
        symphonia::core::codecs::CODEC_TYPE_PCM_F64BE_PLANAR,
        symphonia::core::codecs::CODEC_TYPE_PCM_ALAW,
        symphonia::core::codecs::CODEC_TYPE_PCM_MULAW,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_G722,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_G726,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_G726LE,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_MS,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_IMA_WAV,
        symphonia::core::codecs::CODEC_TYPE_ADPCM_IMA_QT,
        symphonia::core::codecs::CODEC_TYPE_VORBIS,
        symphonia::core::codecs::CODEC_TYPE_MP1,
        symphonia::core::codecs::CODEC_TYPE_MP2,
        symphonia::core::codecs::CODEC_TYPE_MP3,
        symphonia::core::codecs::CODEC_TYPE_AAC,
        symphonia::core::codecs::CODEC_TYPE_OPUS,
        symphonia::core::codecs::CODEC_TYPE_SPEEX,
        symphonia::core::codecs::CODEC_TYPE_MUSEPACK,
        symphonia::core::codecs::CODEC_TYPE_ATRAC1,
        symphonia::core::codecs::CODEC_TYPE_ATRAC3,
        symphonia::core::codecs::CODEC_TYPE_ATRAC3PLUS,
        symphonia::core::codecs::CODEC_TYPE_ATRAC9,
        symphonia::core::codecs::CODEC_TYPE_EAC3,
        symphonia::core::codecs::CODEC_TYPE_AC4,
        symphonia::core::codecs::CODEC_TYPE_DCA,
        symphonia::core::codecs::CODEC_TYPE_WMA,
        symphonia::core::codecs::CODEC_TYPE_FLAC,
        symphonia::core::codecs::CODEC_TYPE_WAVPACK,
        symphonia::core::codecs::CODEC_TYPE_MONKEYS_AUDIO,
        symphonia::core::codecs::CODEC_TYPE_ALAC,
        symphonia::core::codecs::CODEC_TYPE_TTA,
        ]
        .into_iter()
        .map(|c| (codec_id(c), c))
        .collect()
    });
    KNOWN.iter().find(|(i, _)| *i == id).map(|(_, c)| *c).unwrap_or(CODEC_TYPE_NULL)
}

// ---- header ----------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub flags: u32,
    pub codec: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: u16,
    pub bits_per_coded_sample: u16,
    /// (numer, denom)
    pub time_base: Option<(u32, u32)>,
    pub n_frames: Option<u64>,
    pub start_ts: u64,
    pub delay: Option<u32>,
    pub padding: Option<u32>,
    pub gapless: Option<Gapless>,
    pub max_frames_per_packet: Option<u64>,
    pub frames_per_block: Option<u64>,
    pub sample_format: u8,
    pub packet_data_integrity: bool,
    pub layout: u8,
    pub extra_len: u32,
    pub packet_count: u32,
    pub index_len: u32,
    pub index_offset: u64,
    pub meta_offset: u64,
    pub meta_len: u32,
    pub payload_crc: u32,
    pub payload_offset: u64,
    pub payload_len: u64,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn opt64(v: u64) -> Option<u64> {
    (v != NONE64).then_some(v)
}
fn opt32(v: u32) -> Option<u32> {
    (v != NONE32).then_some(v)
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[..8].copy_from_slice(MAGIC);
        b[8..10].copy_from_slice(&VERSION.to_le_bytes());
        b[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        let mut put32 = |at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put32(12, self.flags);
        put32(16, self.codec);
        put32(20, self.sample_rate);
        put32(24, self.channels);
        let (n, d) = self.time_base.unwrap_or((0, 0));
        put32(32, n);
        put32(36, d);
        put32(56, self.delay.unwrap_or(NONE32));
        put32(60, self.padding.unwrap_or(NONE32));
        put32(100, self.extra_len);
        put32(104, self.packet_count);
        put32(108, self.index_len);
        put32(128, self.meta_len);
        put32(132, self.payload_crc);
        b[28..30].copy_from_slice(&self.bits_per_sample.to_le_bytes());
        b[30..32].copy_from_slice(&self.bits_per_coded_sample.to_le_bytes());
        let mut put64 = |at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
        put64(40, self.n_frames.unwrap_or(NONE64));
        put64(48, self.start_ts);
        put64(64, self.gapless.map_or(NONE64, |g| g.delay));
        put64(72, self.gapless.and_then(|g| g.frames).unwrap_or(NONE64));
        put64(80, self.max_frames_per_packet.unwrap_or(0));
        put64(88, self.frames_per_block.unwrap_or(0));
        put64(112, self.index_offset);
        put64(120, self.meta_offset);
        put64(136, self.payload_offset);
        put64(144, self.payload_len);
        b[96] = self.sample_format;
        b[97] = self.packet_data_integrity as u8;
        b[98] = self.layout;
        let crc = crc32fast::hash(&b[..152]);
        b[152..156].copy_from_slice(&crc.to_le_bytes());
        b
    }

    pub fn parse(b: &[u8]) -> Option<Header> {
        if b.len() < HEADER_LEN || &b[..8] != MAGIC || le16(b, 8) != VERSION || (le16(b, 10) as usize) < HEADER_LEN {
            return None;
        }
        if crc32fast::hash(&b[..152]) != le32(b, 152) {
            return None;
        }
        let (n, d) = (le32(b, 32), le32(b, 36));
        Some(Header {
            flags: le32(b, 12),
            codec: le32(b, 16),
            sample_rate: le32(b, 20),
            channels: le32(b, 24),
            bits_per_sample: le16(b, 28),
            bits_per_coded_sample: le16(b, 30),
            time_base: (n != 0 && d != 0).then_some((n, d)),
            n_frames: opt64(le64(b, 40)),
            start_ts: le64(b, 48),
            delay: opt32(le32(b, 56)),
            padding: opt32(le32(b, 60)),
            gapless: opt64(le64(b, 64)).map(|delay| Gapless { delay, frames: opt64(le64(b, 72)) }),
            max_frames_per_packet: Some(le64(b, 80)).filter(|v| *v != 0),
            frames_per_block: Some(le64(b, 88)).filter(|v| *v != 0),
            sample_format: b[96],
            packet_data_integrity: b[97] != 0,
            layout: b[98],
            extra_len: le32(b, 100),
            packet_count: le32(b, 104),
            index_len: le32(b, 108),
            index_offset: le64(b, 112),
            meta_offset: le64(b, 120),
            meta_len: le32(b, 128),
            payload_crc: le32(b, 132),
            payload_offset: le64(b, 136),
            payload_len: le64(b, 144),
        })
    }

    pub fn codec_type(&self) -> CodecType {
        codec_from_id(self.codec)
    }

    /// Frames the decoder produces per second (HE-AAC can double this).
    pub fn duration_secs(&self) -> f64 {
        if let Some(d) = self.gapless.and_then(|g| g.duration(self.sample_rate)) {
            return d;
        }
        match (self.n_frames, self.time_base) {
            (Some(n), Some((num, den))) => n as f64 * num as f64 / den as f64,
            (Some(n), None) if self.sample_rate > 0 => n as f64 / self.sample_rate as f64,
            _ => 0.0,
        }
    }

    pub fn bitrate_kbps(&self) -> u32 {
        let d = self.duration_secs();
        if d <= 0.0 { 0 } else { ((self.payload_len as f64 * 8.0 / d) / 1000.0).round() as u32 }
    }

    pub fn codec_params(&self, extra: Option<Box<[u8]>>) -> CodecParameters {
        let mut p = CodecParameters::new();
        p.codec = self.codec_type();
        p.sample_rate = Some(self.sample_rate).filter(|r| *r > 0);
        p.time_base = self.time_base.map(|(n, d)| TimeBase::new(n, d));
        p.n_frames = self.n_frames;
        p.start_ts = self.start_ts;
        p.sample_format = sample_format_from(self.sample_format);
        p.bits_per_sample = Some(self.bits_per_sample as u32).filter(|v| *v > 0);
        p.bits_per_coded_sample = Some(self.bits_per_coded_sample as u32).filter(|v| *v > 0);
        p.channels = Channels::from_bits(self.channels).filter(|c| !c.is_empty());
        p.channel_layout = layout_from(self.layout);
        p.delay = self.delay;
        p.padding = self.padding;
        p.max_frames_per_packet = self.max_frames_per_packet;
        p.packet_data_integrity = self.packet_data_integrity;
        p.frames_per_block = self.frames_per_block;
        p.extra_data = extra.filter(|e| !e.is_empty());
        p
    }
}

fn sample_format_to(f: Option<SampleFormat>) -> u8 {
    use SampleFormat::*;
    match f {
        None => 0,
        Some(U8) => 1,
        Some(U16) => 2,
        Some(U24) => 3,
        Some(U32) => 4,
        Some(S8) => 5,
        Some(S16) => 6,
        Some(S24) => 7,
        Some(S32) => 8,
        Some(F32) => 9,
        Some(F64) => 10,
    }
}

fn sample_format_from(v: u8) -> Option<SampleFormat> {
    use SampleFormat::*;
    Some(match v {
        1 => U8,
        2 => U16,
        3 => U24,
        4 => U32,
        5 => S8,
        6 => S16,
        7 => S24,
        8 => S32,
        9 => F32,
        10 => F64,
        _ => return None,
    })
}

fn layout_to(l: Option<Layout>) -> u8 {
    match l {
        None => 0,
        Some(Layout::Mono) => 1,
        Some(Layout::Stereo) => 2,
        Some(Layout::TwoPointOne) => 3,
        Some(Layout::FivePointOne) => 4,
    }
}

fn layout_from(v: u8) -> Option<Layout> {
    Some(match v {
        1 => Layout::Mono,
        2 => Layout::Stereo,
        3 => Layout::TwoPointOne,
        4 => Layout::FivePointOne,
        _ => return None,
    })
}

/// The LMS3 header of `p`, if it is one (reads 160 bytes).
pub fn read_header(p: &Path) -> Option<Header> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut b = [0u8; HEADER_LEN];
    f.read_exact(&mut b).ok()?;
    Header::parse(&b)
}

/// True when `p` starts with the LMS3 magic (reads 8 bytes).
pub fn is_lms3(p: &Path) -> bool {
    let mut b = [0u8; 8];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == MAGIC
}

// ---- metadata --------------------------------------------------------------------------------

/// Tags and provenance stored with the audio.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track: Option<u32>,
    pub bpm: Option<f64>,
    /// Lumisound source id ("youtube:<id>", …).
    pub source_id: Option<String>,
    pub thumbnail: Option<String>,
    /// How the audio was obtained, e.g. "yt:774" (YouTube Premium Opus),
    /// "yt:251", "import". Drives quality upgrades.
    pub quality: Option<String>,
    /// Container/codec the packets came from ("webm/opus", "m4a/aac", …).
    pub origin: Option<String>,
    pub art: Option<Vec<u8>>,
    pub art_mime: Option<String>,
    /// Keys this build doesn't know, kept on rewrite.
    pub extra: Vec<(String, Vec<u8>)>,
}

fn put_kv(out: &mut Vec<u8>, k: &str, v: &[u8]) {
    out.extend_from_slice(&(k.len() as u16).to_le_bytes());
    out.extend_from_slice(k.as_bytes());
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    out.extend_from_slice(v);
}

impl Meta {
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        let mut n = 0u32;
        let mut s = |k: &str, v: &Option<String>| {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                put_kv(&mut body, k, v.as_bytes());
                n += 1;
            }
        };
        s("title", &self.title);
        s("artist", &self.artist);
        s("album", &self.album);
        s("genre", &self.genre);
        s("year", &self.year);
        s("source_id", &self.source_id);
        s("thumbnail", &self.thumbnail);
        s("quality", &self.quality);
        s("origin", &self.origin);
        s("art_mime", &self.art_mime);
        s("track", &self.track.map(|t| t.to_string()));
        s("bpm", &self.bpm.map(|b| format!("{b}")));
        if let Some(a) = self.art.as_deref().filter(|a| !a.is_empty()) {
            put_kv(&mut body, "art", a);
            n += 1;
        }
        for (k, v) in &self.extra {
            put_kv(&mut body, k, v);
            n += 1;
        }
        let mut out = Vec::with_capacity(body.len() + 8);
        out.extend_from_slice(META_MAGIC);
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    pub fn decode(b: &[u8]) -> Option<Meta> {
        if b.len() < 8 || &b[..4] != META_MAGIC {
            return None;
        }
        let n = le32(b, 4);
        let mut m = Meta::default();
        let mut i = 8usize;
        for _ in 0..n {
            let kl = b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))? as usize;
            let k = std::str::from_utf8(b.get(i + 2..i + 2 + kl)?).ok()?.to_string();
            i += 2 + kl;
            let vl = u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize;
            let v = b.get(i + 4..i + 4 + vl)?;
            i += 4 + vl;
            let text = || std::str::from_utf8(v).ok().map(str::to_string);
            match k.as_str() {
                "title" => m.title = text(),
                "artist" => m.artist = text(),
                "album" => m.album = text(),
                "genre" => m.genre = text(),
                "year" => m.year = text(),
                "source_id" => m.source_id = text(),
                "thumbnail" => m.thumbnail = text(),
                "quality" => m.quality = text(),
                "origin" => m.origin = text(),
                "art_mime" => m.art_mime = text(),
                "track" => m.track = text().and_then(|t| t.parse().ok()),
                "bpm" => m.bpm = text().and_then(|t| t.parse().ok()),
                "art" => m.art = Some(v.to_vec()),
                _ => m.extra.push((k, v.to_vec())),
            }
        }
        Some(m)
    }

    /// Fill empty fields from `other` (newer values win where set).
    pub fn merged_over(mut self, other: &Meta) -> Meta {
        macro_rules! take {
            ($f:ident) => {
                if other.$f.is_some() {
                    self.$f = other.$f.clone();
                }
            };
        }
        take!(title);
        take!(artist);
        take!(album);
        take!(genre);
        take!(year);
        take!(track);
        take!(bpm);
        take!(source_id);
        take!(thumbnail);
        take!(quality);
        take!(origin);
        take!(art);
        take!(art_mime);
        self
    }

    /// Bare YouTube video id from `source_id` ("youtube:abc" / "abc" / URL).
    pub fn youtube_id(&self) -> Option<String> {
        youtube_id_of(self.source_id.as_deref()?)
    }
}

/// Bare YouTube video id from a Lumisound source id or URL.
pub fn youtube_id_of(s: &str) -> Option<String> {
    let s = s.trim();
    let id = if let Some(r) = s.strip_prefix("youtube:") {
        r.to_string()
    } else if let Some(i) = s.find("v=") {
        s[i + 2..].split(['&', '#']).next().unwrap_or("").to_string()
    } else if let Some(i) = s.find("youtu.be/") {
        s[i + 9..].split(['?', '&', '#']).next().unwrap_or("").to_string()
    } else if !s.contains(':') && !s.contains('/') {
        s.to_string()
    } else {
        return None;
    };
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).then_some(id)
}

pub fn read_meta(p: &Path) -> Option<Meta> {
    let h = read_header(p)?;
    let mut f = std::fs::File::open(p).ok()?;
    if h.meta_len == 0 {
        return Some(Meta::default());
    }
    f.seek(SeekFrom::Start(h.meta_offset)).ok()?;
    let mut b = vec![0u8; h.meta_len as usize];
    f.read_exact(&mut b).ok()?;
    Meta::decode(&b)
}

// ---- varints / index -------------------------------------------------------------------------

fn put_var(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_var(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

fn zig(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}
fn unzig(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// One packet in the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub ts: u64,
    pub dur: u64,
    /// Payload-relative offset.
    pub off: u64,
    pub size: u32,
    pub trim_start: u32,
    pub trim_end: u32,
}

struct IndexWriter {
    buf: Vec<u8>,
    next_ts: u64,
}

impl IndexWriter {
    fn push(&mut self, ts: u64, dur: u64, size: u32, trim_start: u32, trim_end: u32) {
        put_var(&mut self.buf, size as u64);
        put_var(&mut self.buf, zig(ts.wrapping_sub(self.next_ts) as i64));
        let trimmed = trim_start != 0 || trim_end != 0;
        put_var(&mut self.buf, (dur << 1) | trimmed as u64);
        if trimmed {
            put_var(&mut self.buf, trim_start as u64);
            put_var(&mut self.buf, trim_end as u64);
        }
        self.next_ts = ts.wrapping_add(dur);
    }
}

pub fn decode_index(b: &[u8], count: u32) -> Option<Vec<Entry>> {
    let mut out = Vec::with_capacity(count as usize);
    let (mut i, mut next_ts, mut off) = (0usize, 0u64, 0u64);
    for _ in 0..count {
        let size = get_var(b, &mut i)?;
        let ts = next_ts.wrapping_add(unzig(get_var(b, &mut i)?) as u64);
        let d = get_var(b, &mut i)?;
        let (dur, trimmed) = (d >> 1, d & 1 == 1);
        let (trim_start, trim_end) = if trimmed { (get_var(b, &mut i)? as u32, get_var(b, &mut i)? as u32) } else { (0, 0) };
        out.push(Entry { ts, dur, off, size: u32::try_from(size).ok()?, trim_start, trim_end });
        off += size;
        next_ts = ts.wrapping_add(dur);
    }
    Some(out)
}

/// Header, codec extra data and packet index of an LMS3 file.
pub type Layout3 = (Header, Option<Box<[u8]>>, Vec<Entry>);

/// Header + codec extra data + index of an LMS3 file.
pub fn read_layout(r: &mut (impl Read + Seek)) -> Option<Layout3> {
    r.seek(SeekFrom::Start(0)).ok()?;
    let mut hb = [0u8; HEADER_LEN];
    r.read_exact(&mut hb).ok()?;
    let h = Header::parse(&hb)?;
    let extra = if h.extra_len > 0 {
        let mut e = vec![0u8; h.extra_len as usize];
        r.read_exact(&mut e).ok()?;
        Some(e.into_boxed_slice())
    } else {
        None
    };
    r.seek(SeekFrom::Start(h.index_offset)).ok()?;
    let mut ib = vec![0u8; h.index_len as usize];
    r.read_exact(&mut ib).ok()?;
    let idx = decode_index(&ib, h.packet_count)?;
    if idx.last().is_some_and(|e| e.off + e.size as u64 != h.payload_len) {
        return None;
    }
    Some((h, extra, idx))
}

// ---- writing ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The source has a video (or other non-audio) track: left as it is so
    /// nothing the app shows from it (motion artwork) is lost.
    NotAudioOnly,
    Unsupported(String),
    Io(String),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::NotAudioOnly => write!(f, "source has a video track"),
            WriteError::Unsupported(s) => write!(f, "unsupported: {s}"),
            WriteError::Io(s) => write!(f, "io: {s}"),
        }
    }
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e.to_string())
    }
}

#[derive(Default, Clone, Debug)]
pub struct WriteOpts {
    /// Header flags; None = carry the source's (LMSLOCK2 / LMS3) flags.
    pub flags: Option<u32>,
    /// Applied over the tags read from the source.
    pub meta: Option<Meta>,
}

#[derive(Clone, Debug)]
pub struct Written {
    pub header: Header,
    pub codec: String,
    pub bytes: u64,
}

pub fn codec_name(c: CodecType) -> String {
    crate::opus::codecs().get_codec(c).map(|d| d.short_name.to_string()).unwrap_or_else(|| c.to_string())
}

/// Tags/artwork of any file the app keeps (LMS3, locked or plain).
pub fn meta_of(src: &Path) -> Meta {
    if let Some(m) = read_meta(src) {
        return m;
    }
    let mut m = Meta::default();
    if let Some(i) = crate::tags::read_info(src) {
        m.title = i.title;
        m.artist = i.artist;
        m.album = i.album;
        m.genre = i.genre;
        m.year = i.year;
        m.track = i.track;
        m.source_id = i.source_id;
    }
    m.thumbnail = crate::tags::read_lumisound(src).1;
    // What the bridge fetched ("yt:251", "yt:774").
    m.quality = crate::tags::read_custom(src, crate::tags::QUALITY_KEY).filter(|q| !q.trim().is_empty());
    m.bpm = crate::tags::read_bpm(src);
    if let Some(a) = crate::artwork::embedded(src) {
        m.art_mime = Some(if a.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" }.into());
        m.art = Some(a);
    }
    m
}

fn source_flags(src: &Path) -> u32 {
    read_header(src).map(|h| h.flags).or_else(|| crate::lock::read_header(src).map(|h| h.flags)).unwrap_or(0)
}

/// Demux `src` (any format the engine reads: plain, LMSLOCK1/2 or LMS3) and
/// write it to `dst` as LMS3, atomically (temp file in `dst`'s folder +
/// rename). Packets are copied byte for byte. `src` is never modified.
pub fn write(src: &Path, dst: &Path, opts: &WriteOpts) -> Result<Written, WriteError> {
    let (mss, hint) = crate::decode::media_source(src)?;
    let probed = crate::decode::format_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| WriteError::Unsupported(e.to_string()))?;
    let mut fmt = probed.format;
    let tracks = fmt.tracks();
    if tracks.iter().any(|t| t.codec_params.sample_rate.is_none() || t.codec_params.codec == CODEC_TYPE_NULL) {
        return Err(WriteError::NotAudioOnly);
    }
    if tracks.len() != 1 {
        return Err(WriteError::Unsupported(format!("{} audio tracks", tracks.len())));
    }
    let track = tracks[0].clone();
    let p = track.codec_params.clone();
    crate::opus::codecs().make(&p, &Default::default()).map_err(|e| WriteError::Unsupported(format!("no decoder: {e}")))?;
    if codec_from_id(codec_id(p.codec)) != p.codec {
        return Err(WriteError::Unsupported(format!("codec {} has no stable id", p.codec)));
    }
    let gapless = crate::gapless::for_track(src, p.codec);
    let mut meta = meta_of(src);
    if meta.origin.is_none() {
        let ext = crate::lock::inner_ext(&src.file_name().unwrap_or_default().to_string_lossy())
            .or_else(|| src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()))
            .unwrap_or_default();
        meta.origin = Some(format!("{ext}/{}", codec_name(p.codec)));
    }
    if let Some(o) = &opts.meta {
        meta = meta.merged_over(o);
    }
    let flags = opts.flags.unwrap_or_else(|| source_flags(src));

    let dir = dst.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.lms3-{}", dst.file_name().and_then(|n| n.to_str()).unwrap_or("track"), uuid_like()));
    let r = (|| -> Result<Written, WriteError> {
        let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        let extra = p.extra_data.clone().unwrap_or_default();
        out.write_all(&[0u8; HEADER_LEN])?;
        out.write_all(&extra)?;
        let payload_offset = (HEADER_LEN + extra.len()) as u64;
        let mut idx = IndexWriter { buf: Vec::new(), next_ts: 0 };
        let mut crc = crc32fast::Hasher::new();
        let (mut off, mut count) = (0u64, 0u32);
        loop {
            let pkt = match fmt.next_packet() {
                Ok(p) => p,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(Error::ResetRequired) => return Err(WriteError::Unsupported("stream changes mid-file".into())),
                Err(e) => return Err(WriteError::Io(format!("demux: {e}"))),
            };
            if pkt.track_id() != track.id {
                continue;
            }
            let size = u32::try_from(pkt.data.len()).map_err(|_| WriteError::Unsupported("packet over 4 GB".into()))?;
            crc.update(&pkt.data);
            let mut data = pkt.data.into_vec();
            crate::lock::apply_mask(&mut data, off);
            out.write_all(&data)?;
            idx.push(pkt.ts, pkt.dur, size, pkt.trim_start, pkt.trim_end);
            off += size as u64;
            count += 1;
        }
        if count == 0 {
            return Err(WriteError::Unsupported("no audio packets".into()));
        }
        let index_offset = payload_offset + off;
        out.write_all(&idx.buf)?;
        let meta_bytes = meta.encode();
        let meta_offset = index_offset + idx.buf.len() as u64;
        out.write_all(&meta_bytes)?;
        let header = Header {
            flags,
            codec: codec_id(p.codec),
            sample_rate: p.sample_rate.unwrap_or(0),
            channels: p.channels.map(|c| c.bits()).unwrap_or(0),
            bits_per_sample: p.bits_per_sample.unwrap_or(0) as u16,
            bits_per_coded_sample: p.bits_per_coded_sample.unwrap_or(0) as u16,
            time_base: p.time_base.map(|t| (t.numer, t.denom)),
            n_frames: p.n_frames,
            start_ts: p.start_ts,
            delay: p.delay,
            padding: p.padding,
            gapless,
            max_frames_per_packet: p.max_frames_per_packet,
            frames_per_block: p.frames_per_block,
            sample_format: sample_format_to(p.sample_format),
            packet_data_integrity: p.packet_data_integrity,
            layout: layout_to(p.channel_layout),
            extra_len: extra.len() as u32,
            packet_count: count,
            index_len: idx.buf.len() as u32,
            index_offset,
            meta_offset,
            meta_len: meta_bytes.len() as u32,
            payload_crc: crc.finalize(),
            payload_offset,
            payload_len: off,
        };
        let mut f = out.into_inner().map_err(|e| WriteError::Io(e.to_string()))?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&header.encode())?;
        f.sync_all()?;
        let bytes = meta_offset + meta_bytes.len() as u64;
        Ok(Written { codec: codec_name(p.codec), header, bytes })
    })();
    match r {
        Ok(w) => match std::fs::rename(&tmp, dst) {
            Ok(()) => Ok(w),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e.into())
            }
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn uuid_like() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}", n, std::process::id())
}

/// Recompute the plain payload CRC and compare with the header (full read).
pub fn check_integrity(p: &Path) -> bool {
    let Some(h) = read_header(p) else { return false };
    let Ok(f) = std::fs::File::open(p) else { return false };
    let mut r = std::io::BufReader::with_capacity(1 << 20, f);
    if r.seek(SeekFrom::Start(h.payload_offset)).is_err() {
        return false;
    }
    let mut left = h.payload_len;
    let mut off = 0u64;
    let mut crc = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    while left > 0 {
        let n = (left as usize).min(buf.len());
        if r.read_exact(&mut buf[..n]).is_err() {
            return false;
        }
        crate::lock::apply_mask(&mut buf[..n], off);
        crc.update(&buf[..n]);
        off += n as u64;
        left -= n as u64;
    }
    crc.finalize() == h.payload_crc
}

/// Write `src` to `dst` as LMS3, then prove it: payload CRC re-read from disk
/// and a full decode of both files that must match sample for sample
/// (same frame count, same PCM). `dst` is removed when the proof fails.
pub fn write_verified(src: &Path, dst: &Path, opts: &WriteOpts) -> Result<Written, WriteError> {
    let w = write(src, dst, opts)?;
    let ok = check_integrity(dst) && {
        let a = crate::decode::fingerprint(src);
        let b = crate::decode::fingerprint(dst);
        a.is_some() && a == b
    };
    if !ok {
        let _ = std::fs::remove_file(dst);
        return Err(WriteError::Unsupported("converted file did not decode identically to the source".into()));
    }
    Ok(w)
}

/// Update flags in place (only the 160-byte header is rewritten).
pub fn set_flags(p: &Path, flags: u32) -> std::io::Result<()> {
    let mut h = read_header(p).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not LMS3"))?;
    if h.flags == flags {
        return Ok(());
    }
    h.flags = flags;
    let mut f = std::fs::OpenOptions::new().write(true).open(p)?;
    f.write_all(&h.encode())?;
    f.sync_all()
}

/// Replace the metadata section (atomic: staged copy + rename).
pub fn set_meta(p: &Path, meta: &Meta) -> std::io::Result<()> {
    let mut h = read_header(p).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not LMS3"))?;
    let bytes = meta.encode();
    let dir = p.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.meta-{}", p.file_name().and_then(|n| n.to_str()).unwrap_or("track"), uuid_like()));
    let r = (|| {
        let mut src = std::fs::File::open(p)?;
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut (&mut src).take(h.meta_offset), &mut out)?;
        out.write_all(&bytes)?;
        h.meta_len = bytes.len() as u32;
        out.seek(SeekFrom::Start(0))?;
        out.write_all(&h.encode())?;
        out.sync_all()?;
        std::fs::rename(&tmp, p)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

// ---- reading (symphonia) ---------------------------------------------------------------------

pub struct Lms3Reader {
    reader: MediaSourceStream,
    tracks: Vec<Track>,
    header: Header,
    index: Vec<Entry>,
    next: usize,
    /// Absolute stream position the next read starts at.
    pos: u64,
    meta: MetadataLog,
}

impl Lms3Reader {
    pub fn header(&self) -> &Header {
        &self.header
    }

    fn time_base(&self) -> TimeBase {
        self.header.time_base.map(|(n, d)| TimeBase::new(n, d)).unwrap_or_else(|| TimeBase::new(1, self.header.sample_rate.max(1)))
    }

    /// Frames of decoder warm-up to back off before a seek target: Opus
    /// needs 80 ms of pre-roll to converge after a jump.
    fn preroll(&self) -> u64 {
        if self.header.codec_type() == symphonia::core::codecs::CODEC_TYPE_OPUS {
            let tb = self.time_base();
            tb.calc_timestamp(symphonia::core::units::Time::new(0, 0.08))
        } else {
            0
        }
    }
}

impl QueryDescriptor for Lms3Reader {
    fn query() -> &'static [Descriptor] {
        &[Descriptor {
            short_name: "lms3",
            long_name: "Lumisound Audio",
            extensions: &["lms"],
            mime_types: &["audio/x-lumisound"],
            markers: &[MAGIC],
            score: Self::score,
            inst: Instantiate::Format(|source, opt| Ok(Box::new(Lms3Reader::try_new(source, opt)?))),
        }]
    }

    fn score(_context: &[u8]) -> u8 {
        255
    }
}

impl FormatReader for Lms3Reader {
    fn try_new(mut source: MediaSourceStream, _options: &FormatOptions) -> SymResult<Self> {
        let (header, extra, index) = match read_layout(&mut source) {
            Some(l) => l,
            None => return decode_error("lms3: bad header or index"),
        };
        let params = header.codec_params(extra);
        let pos = header.payload_offset;
        source.seek(SeekFrom::Start(pos))?;
        Ok(Lms3Reader { reader: source, tracks: vec![Track::new(0, params)], header, index, next: 0, pos, meta: MetadataLog::default() })
    }

    fn cues(&self) -> &[Cue] {
        &[]
    }

    fn metadata(&mut self) -> Metadata<'_> {
        self.meta.metadata()
    }

    fn seek(&mut self, _mode: SeekMode, to: SeekTo) -> SymResult<SeekedTo> {
        let required_ts = match to {
            SeekTo::TimeStamp { ts, .. } => ts,
            SeekTo::Time { time, .. } => self.time_base().calc_timestamp(time),
        };
        if self.index.is_empty() {
            return seek_error(symphonia::core::errors::SeekErrorKind::Unseekable);
        }
        let target = required_ts.saturating_sub(self.preroll());
        // Last packet starting at or before the target.
        let i = self.index.partition_point(|e| e.ts <= target).saturating_sub(1);
        self.next = i;
        Ok(SeekedTo { track_id: 0, required_ts, actual_ts: self.index[i].ts })
    }

    fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    fn next_packet(&mut self) -> SymResult<Packet> {
        let Some(e) = self.index.get(self.next).copied() else {
            return Err(Error::IoError(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "end of stream")));
        };
        let at = self.header.payload_offset + e.off;
        if at != self.pos {
            self.reader.seek(SeekFrom::Start(at))?;
        }
        let mut data = vec![0u8; e.size as usize];
        self.reader.read_exact(&mut data)?;
        crate::lock::apply_mask(&mut data, e.off);
        self.pos = at + e.size as u64;
        self.next += 1;
        Ok(Packet::new_trimmed_from_boxed_slice(0, e.ts, e.dur, e.trim_start, e.trim_end, data.into_boxed_slice()))
    }

    fn into_inner(self: Box<Self>) -> MediaSourceStream {
        self.reader
    }
}

/// Every packet of an LMS3 file, unmasked, in order (exporters).
pub struct PacketIter {
    file: std::io::BufReader<std::fs::File>,
    header: Header,
    index: Vec<Entry>,
    i: usize,
}

impl PacketIter {
    #[allow(clippy::type_complexity)]
    pub fn open(p: &Path) -> Option<(Header, Option<Box<[u8]>>, PacketIter)> {
        let mut file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(p).ok()?);
        let (header, extra, index) = read_layout(&mut file)?;
        file.seek(SeekFrom::Start(header.payload_offset)).ok()?;
        Some((header.clone(), extra, PacketIter { file, header, index, i: 0 }))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.index
    }
}

impl Iterator for PacketIter {
    type Item = std::io::Result<(Entry, Vec<u8>)>;
    fn next(&mut self) -> Option<Self::Item> {
        let e = *self.index.get(self.i)?;
        self.i += 1;
        let mut d = vec![0u8; e.size as usize];
        if let Err(err) = self.file.read_exact(&mut d) {
            return Some(Err(err));
        }
        crate::lock::apply_mask(&mut d, e.off);
        let _ = &self.header;
        Some(Ok((e, d)))
    }
}

// ---- quality upgrades ------------------------------------------------------------------------

/// Premium = fetched as YouTube Premium audio, or at a bitrate only Premium
/// serves.
pub fn is_premium(quality: Option<&str>, kbps: u32) -> bool {
    quality.is_some_and(|q| q == "yt:774" || q.starts_with("yt-premium")) || kbps >= PREMIUM_KBPS
}

/// (kbps, seconds) of any file the app keeps.
pub fn kbps_and_duration(p: &Path) -> (u32, f64) {
    match read_header(p) {
        Some(h) => (h.bitrate_kbps(), h.duration_secs()),
        None => crate::tags::read_info(p).map(|i| (i.bitrate_kbps, i.duration)).unwrap_or((0, 0.0)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Replaced {
    pub old_kbps: u32,
    pub new_kbps: u32,
    pub detail: String,
}

/// Replace the audio of library file `path` with `new_audio` (a fresh
/// download of the same track), as LMS3, keeping its tags, artwork and
/// flags. Only when the new audio is genuinely better (Premium where the
/// old wasn't, or 24+ kbps more) and the same recording (length within
/// 2 s / 1.5 %). `before_commit` runs on the verified staged file just
/// before it is renamed over `path` (carry xattrs). `new_audio` is left for
/// the caller to remove.
pub fn replace_audio(path: &Path, new_audio: &Path, quality: &str, before_commit: &dyn Fn(&Path)) -> Result<Replaced, (u32, u32, String)> {
    if !path.is_file() || !new_audio.is_file() {
        return Err((0, 0, "file missing".into()));
    }
    // Empty = what the download's own LUMISOUND_QUALITY tag says.
    let tagged;
    let quality = if quality.trim().is_empty() {
        tagged = crate::tags::read_custom(new_audio, crate::tags::QUALITY_KEY).unwrap_or_else(|| "yt".into());
        tagged.as_str()
    } else {
        quality
    };
    let (old_kbps, old_dur) = kbps_and_duration(path);
    let old_meta = meta_of(path);
    let was_premium = is_premium(old_meta.quality.as_deref(), old_kbps);
    let Some(new_probe) = crate::decode::probe(new_audio) else {
        return Err((old_kbps, 0, "new audio doesn't decode".into()));
    };
    let new_dur = new_probe.duration;
    let new_size = std::fs::metadata(new_audio).map(|m| m.len()).unwrap_or(0);
    let new_kbps = if new_dur > 0.0 { ((new_size as f64 * 8.0 / new_dur) / 1000.0).round() as u32 } else { 0 };
    if old_dur > 0.0 && (new_dur - old_dur).abs() > (old_dur * 0.015).max(2.0) {
        return Err((old_kbps, new_kbps, format!("different length ({new_dur:.1}s vs {old_dur:.1}s): not the same recording")));
    }
    if was_premium || !(is_premium(Some(quality), new_kbps) || new_kbps >= old_kbps + 24) {
        return Err((old_kbps, new_kbps, format!("not better ({new_kbps} vs {old_kbps} kbps)")));
    }
    // Every tag the track had wins (titles the user fixed, artwork, BPM…);
    // the new file only fills gaps, and the quality is the new one.
    let quality_meta = Meta { quality: Some(quality.to_string()), ..Default::default() };
    let mut fresh = meta_of(new_audio);
    fresh.origin = None;
    let mut meta = fresh.merged_over(&old_meta).merged_over(&quality_meta);
    meta.origin = Some(format!("upgrade/{}", meta_of(new_audio).origin.unwrap_or_default()));
    let flags = read_header(path).map(|h| h.flags).or_else(|| crate::lock::read_header(path).map(|h| h.flags)).unwrap_or(0);
    let staged = path.with_file_name(format!(".{}.upgrade-{}", path.file_name().and_then(|n| n.to_str()).unwrap_or("track"), uuid_like()));
    let w = write_verified(new_audio, &staged, &WriteOpts { flags: Some(flags), meta: Some(meta) }).map_err(|e| (old_kbps, new_kbps, format!("LMS3 write: {e}")))?;
    before_commit(&staged);
    if let Err(e) = std::fs::rename(&staged, path) {
        let _ = std::fs::remove_file(&staged);
        return Err((old_kbps, new_kbps, format!("rename: {e}")));
    }
    let new_kbps = w.header.bitrate_kbps();
    Ok(Replaced { old_kbps, new_kbps, detail: format!("{} {old_kbps} kbps → {} {new_kbps} kbps", old_meta.origin.unwrap_or_default(), w.codec) })
}
