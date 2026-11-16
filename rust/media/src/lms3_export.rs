//! LMS3 → standard audio file, for everything that isn't the Lumisound
//! engine (AVFoundation fallbacks, video/lyric exports, ffmpeg on the
//! server, sharing). The coded packets are copied as they are into the
//! codec's natural container, with tags and artwork:
//!
//! | codec        | file                    |
//! |--------------|-------------------------|
//! | Opus         | Ogg Opus `.opus`        |
//! | AAC, ALAC    | MP4 audio `.m4a`        |
//! | FLAC         | native `.flac`          |
//! | MP3          | `.mp3` with ID3v2.4     |
//! | PCM (LE)     | `.wav`                  |
//! | anything else| decoded to float `.wav` |
//!
//! Gapless data travels too (Ogg granule positions; MP4 `iTunSMPB` plus an
//! edit list), so a system player trims priming/padding like the engine.

use crate::lms3::{Entry, Header, Meta, PacketIter};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use symphonia::core::codecs::*;

fn io_err(s: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, s.to_string())
}

/// Natural extension of the exported file for an LMS3 header.
pub fn export_ext(h: &Header) -> &'static str {
    match h.codec_type() {
        CODEC_TYPE_OPUS => "opus",
        CODEC_TYPE_AAC | CODEC_TYPE_ALAC => "m4a",
        CODEC_TYPE_FLAC => "flac",
        CODEC_TYPE_MP3 => "mp3",
        _ => "wav",
    }
}

/// Extension of the file `p` exports to (None when it isn't LMS3).
pub fn export_ext_of(p: &Path) -> Option<&'static str> {
    crate::lms3::read_header(p).map(|h| export_ext(&h))
}

/// Export `p` into `out` atomically (temp file beside it + rename).
pub fn export_to_path(p: &Path, out: &Path) -> std::io::Result<()> {
    let dir = out.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.export-{}", out.file_name().and_then(|n| n.to_str()).unwrap_or("track"), std::process::id() as u128 ^ nanos()));
    let r = (|| {
        let mut f = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&tmp)?);
        export_to(p, &mut f)?;
        let f = f.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        std::fs::rename(&tmp, out)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Write the standard-file form of LMS3 `p` to `w`.
pub fn export_to<W: Write + Seek>(p: &Path, w: &mut W) -> std::io::Result<()> {
    let (h, extra, packets) = PacketIter::open(p).ok_or_else(|| io_err("not an LMS3 file"))?;
    let meta = crate::lms3::read_meta(p).unwrap_or_default();
    let r = match h.codec_type() {
        CODEC_TYPE_OPUS => ogg_opus(&h, extra.as_deref(), packets, &meta, w),
        CODEC_TYPE_AAC => m4a(&h, extra.as_deref(), packets, &meta, w, SampleEntry::Aac),
        CODEC_TYPE_ALAC => match alac_cookie(extra.as_deref()) {
            Some(c) => m4a(&h, Some(&c), packets, &meta, w, SampleEntry::Alac),
            None => Err(io_err("alac cookie")),
        },
        CODEC_TYPE_FLAC if extra.as_ref().is_some_and(|e| e.len() == 34) => flac(extra.as_deref().unwrap(), packets, &meta, w),
        CODEC_TYPE_MP3 => mp3(packets, &meta, w),
        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_U8 => wav_passthrough(&h, packets, w),
        _ => Err(io_err("no passthrough container")),
    };
    match r {
        Ok(()) => Ok(()),
        // Anything without a passthrough container (or a malformed codec
        // config) is decoded instead: lossless float PCM, always playable.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            w.seek(SeekFrom::Start(0))?;
            wav_decoded(p, w)
        }
        Err(e) => Err(e),
    }
}

// ---- Ogg ---------------------------------------------------------------------------------------

static OGG_CRC: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
    let mut t = [0u32; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let mut r = (i as u32) << 24;
        for _ in 0..8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04c1_1db7 } else { r << 1 };
        }
        *v = r;
    }
    t
});

fn ogg_crc(data: &[u8]) -> u32 {
    let t = &*OGG_CRC;
    data.iter().fold(0u32, |crc, b| (crc << 8) ^ t[(((crc >> 24) as u8) ^ b) as usize])
}

struct OggWriter<'a, W: Write> {
    w: &'a mut W,
    serial: u32,
    seq: u32,
    lacing: Vec<u8>,
    data: Vec<u8>,
    /// Granule of the last packet completed on the current page.
    granule: Option<u64>,
    /// The current page starts mid-packet.
    continued: bool,
    bos: bool,
    last_granule: u64,
}

impl<W: Write> OggWriter<'_, W> {
    fn flush(&mut self, eos: bool) -> std::io::Result<()> {
        if self.lacing.is_empty() && !eos {
            return Ok(());
        }
        let mut page = Vec::with_capacity(27 + self.lacing.len() + self.data.len());
        page.extend_from_slice(b"OggS");
        page.push(0);
        let mut flags = 0u8;
        if self.continued {
            flags |= 1;
        }
        if self.bos {
            flags |= 2;
        }
        if eos {
            flags |= 4;
        }
        page.push(flags);
        // A page where no packet ends carries -1; an empty EOS page repeats
        // the final granule.
        let g = match self.granule {
            Some(g) => g,
            None if self.lacing.is_empty() => self.last_granule,
            None => u64::MAX,
        };
        page.extend_from_slice(&g.to_le_bytes());
        page.extend_from_slice(&self.serial.to_le_bytes());
        page.extend_from_slice(&self.seq.to_le_bytes());
        page.extend_from_slice(&[0, 0, 0, 0]);
        page.push(self.lacing.len() as u8);
        page.extend_from_slice(&self.lacing);
        page.extend_from_slice(&self.data);
        let crc = ogg_crc(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        self.w.write_all(&page)?;
        self.seq += 1;
        self.bos = false;
        self.continued = false;
        self.lacing.clear();
        self.data.clear();
        self.granule = None;
        Ok(())
    }

    /// Add a packet; `own_page` flushes before and after (header packets).
    fn packet(&mut self, data: &[u8], granule: u64, own_page: bool) -> std::io::Result<()> {
        if own_page {
            self.flush(false)?;
        }
        let mut rest = data;
        let mut started = false;
        loop {
            if self.lacing.len() == 255 {
                self.flush(false)?;
                // The next page opens with the rest of this packet.
                self.continued = started;
            }
            let n = rest.len().min(255);
            self.lacing.push(n as u8);
            self.data.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
            started = true;
            if n < 255 {
                break;
            }
        }
        self.granule = Some(granule);
        self.last_granule = granule;
        if own_page || self.data.len() >= 4096 {
            self.flush(false)?;
        }
        Ok(())
    }
}

/// Samples (at 48 kHz) in an Opus packet, from its TOC byte (RFC 6716 §3.1).
pub fn opus_samples(p: &[u8]) -> u64 {
    let Some(&toc) = p.first() else { return 0 };
    let config = toc >> 3;
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][(config & 3) as usize],
        12..=15 => [480, 960][(config & 1) as usize],
        _ => [120, 240, 480, 960][(config & 3) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => p.get(1).map_or(0, |b| (b & 0x3f) as u64),
    };
    frame * frames
}

fn vorbis_comments(meta: &Meta, with_picture: bool) -> Vec<String> {
    let mut c = Vec::new();
    let mut add = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            c.push(format!("{k}={v}"));
        }
    };
    add("TITLE", meta.title.clone());
    add("ARTIST", meta.artist.clone());
    add("ALBUM", meta.album.clone());
    add("GENRE", meta.genre.clone());
    add("DATE", meta.year.clone());
    add("TRACKNUMBER", meta.track.map(|t| t.to_string()));
    add("BPM", meta.bpm.map(|b| format!("{}", b.round() as i64)));
    add(crate::tags::ID_KEY, meta.source_id.clone());
    add(crate::tags::THUMB_KEY, meta.thumbnail.clone());
    if let (true, Some(art)) = (with_picture, &meta.art) {
        use base64::Engine;
        add("METADATA_BLOCK_PICTURE", Some(base64::engine::general_purpose::STANDARD.encode(flac_picture(art, meta))));
    }
    c
}

fn vorbis_comment_body(meta: &Meta, with_picture: bool) -> Vec<u8> {
    let vendor = b"Lumisound";
    let comments = vorbis_comments(meta, with_picture);
    let mut b = Vec::new();
    b.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    b.extend_from_slice(vendor);
    b.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for c in comments {
        b.extend_from_slice(&(c.len() as u32).to_le_bytes());
        b.extend_from_slice(c.as_bytes());
    }
    b
}

fn ogg_opus<W: Write>(h: &Header, extra: Option<&[u8]>, packets: PacketIter, meta: &Meta, w: &mut W) -> std::io::Result<()> {
    let channels = symphonia::core::audio::Channels::from_bits(h.channels).map(|c| c.count()).unwrap_or(2).clamp(1, 2) as u8;
    let head: Vec<u8> = match extra {
        Some(e) if e.len() >= 19 && &e[..8] == b"OpusHead" => e.to_vec(),
        _ => {
            let pre_skip = h.delay.unwrap_or(312) as u16;
            let mut b = b"OpusHead".to_vec();
            b.push(1);
            b.push(channels);
            b.extend_from_slice(&pre_skip.to_le_bytes());
            b.extend_from_slice(&48_000u32.to_le_bytes());
            b.extend_from_slice(&0i16.to_le_bytes());
            b.push(0);
            b
        }
    };
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&vorbis_comment_body(meta, true));
    let serial = crc32fast::hash(meta.source_id.as_deref().unwrap_or("lumisound").as_bytes()) | 1;
    let mut ogg = OggWriter { w, serial, seq: 0, lacing: Vec::new(), data: Vec::new(), granule: None, continued: false, bos: true, last_granule: 0 };
    ogg.packet(&head, 0, true)?;
    ogg.packet(&tags, 0, true)?;
    let entries: Vec<Entry> = packets.entries().to_vec();
    let end_trim: u64 = entries.iter().rev().take(4).map(|e| e.trim_end as u64).sum();
    let mut granule = 0u64;
    let n = entries.len();
    let mut pending: Option<(Vec<u8>, u64)> = None;
    for (i, item) in packets.enumerate() {
        let (_, data) = item?;
        granule += opus_samples(&data);
        if let Some((d, g)) = pending.take() {
            ogg.packet(&d, g, false)?;
        }
        let g = if i + 1 == n { granule.saturating_sub(end_trim) } else { granule };
        pending = Some((data, g));
    }
    if let Some((d, g)) = pending {
        ogg.packet(&d, g, false)?;
    }
    ogg.flush(true)
}

// ---- MP4 / M4A -------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum SampleEntry {
    Aac,
    Alac,
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 8);
    b.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    b.extend_from_slice(kind);
    b.extend_from_slice(body);
    b
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 4);
    b.push(version);
    b.extend_from_slice(&flags.to_be_bytes()[1..]);
    b.extend_from_slice(body);
    bx(kind, &b)
}

fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn matrix() -> Vec<u8> {
    MATRIX.iter().flat_map(|v| v.to_be_bytes()).collect()
}

fn desc(tag: u8, body: &[u8]) -> Vec<u8> {
    let n = body.len() as u32;
    let mut b = vec![tag, 0x80 | ((n >> 21) & 0x7f) as u8, 0x80 | ((n >> 14) & 0x7f) as u8, 0x80 | ((n >> 7) & 0x7f) as u8, (n & 0x7f) as u8];
    b.extend_from_slice(body);
    b
}

/// AudioSpecificConfig for plain AAC-LC when the source carried none (ADTS).
fn synth_asc(rate: u32, channels: u8) -> Vec<u8> {
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    let fi = RATES.iter().position(|r| *r == rate).unwrap_or(4) as u16;
    let v: u16 = (2 << 11) | (fi << 7) | ((channels as u16 & 0xf) << 3);
    v.to_be_bytes().to_vec()
}

fn alac_cookie(extra: Option<&[u8]>) -> Option<Vec<u8>> {
    let e = extra?;
    if e.len() == 24 {
        return Some(e.to_vec());
    }
    // QuickTime/CAF wrapping: [frma 'alac'] + an `alac` full atom (size,
    // type, version/flags) holding the 24-byte cookie. Walk the atoms.
    let mut i = 0usize;
    while i + 8 <= e.len() {
        let size = u32::from_be_bytes(e[i..i + 4].try_into().ok()?) as usize;
        if &e[i + 4..i + 8] == b"alac" && size >= 36 {
            return e.get(i + 12..i + 36).map(<[u8]>::to_vec);
        }
        if size < 8 {
            break;
        }
        i += size;
    }
    None
}

fn ilst_text(kind: &[u8; 4], v: &str) -> Vec<u8> {
    let mut d = 1u32.to_be_bytes().to_vec();
    d.extend_from_slice(&0u32.to_be_bytes());
    d.extend_from_slice(v.as_bytes());
    bx(kind, &bx(b"data", &d))
}

fn ilst_freeform(name: &str, v: &str) -> Vec<u8> {
    let mean = full(b"mean", 0, 0, b"com.apple.iTunes");
    let nm = full(b"name", 0, 0, name.as_bytes());
    let mut d = 1u32.to_be_bytes().to_vec();
    d.extend_from_slice(&0u32.to_be_bytes());
    d.extend_from_slice(v.as_bytes());
    bx(b"----", &cat(&[mean, nm, bx(b"data", &d)]))
}

fn ilst(meta: &Meta, smpb: Option<String>) -> Vec<u8> {
    let mut items: Vec<Vec<u8>> = Vec::new();
    let mut t = |k: &[u8; 4], v: &Option<String>| {
        if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
            items.push(ilst_text(k, v));
        }
    };
    t(b"\xa9nam", &meta.title);
    t(b"\xa9ART", &meta.artist);
    t(b"\xa9alb", &meta.album);
    t(b"\xa9gen", &meta.genre);
    t(b"\xa9day", &meta.year);
    if let Some(n) = meta.track.filter(|n| *n > 0 && *n <= u16::MAX as u32) {
        let mut d = 0u32.to_be_bytes().to_vec();
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&(n as u16).to_be_bytes());
        d.extend_from_slice(&[0, 0, 0, 0]);
        items.push(bx(b"trkn", &bx(b"data", &d)));
    }
    if let Some(b) = meta.bpm.filter(|b| *b > 0.0 && *b < 65535.0) {
        let mut d = 21u32.to_be_bytes().to_vec();
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(&(b.round() as u16).to_be_bytes());
        items.push(bx(b"tmpo", &bx(b"data", &d)));
    }
    if let Some(art) = &meta.art {
        let ty: u32 = if art.starts_with(&[0x89, b'P', b'N', b'G']) { 14 } else { 13 };
        let mut d = ty.to_be_bytes().to_vec();
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(art);
        items.push(bx(b"covr", &bx(b"data", &d)));
    }
    if let Some(s) = meta.source_id.as_deref().filter(|s| !s.is_empty()) {
        items.push(ilst_freeform(crate::tags::ID_KEY, s));
    }
    if let Some(s) = meta.thumbnail.as_deref().filter(|s| !s.is_empty()) {
        items.push(ilst_freeform(crate::tags::THUMB_KEY, s));
    }
    if let Some(s) = smpb {
        items.push(ilst_freeform("iTunSMPB", &s));
    }
    let mut hd = 0u32.to_be_bytes().to_vec();
    hd.extend_from_slice(b"mdirappl");
    hd.extend_from_slice(&[0u8; 9]);
    let meta_box = full(b"meta", 0, 0, &cat(&[full(b"hdlr", 0, 0, &hd), bx(b"ilst", &items.concat())]));
    bx(b"udta", &meta_box)
}

fn m4a<W: Write>(h: &Header, extra: Option<&[u8]>, packets: PacketIter, meta: &Meta, w: &mut W, entry: SampleEntry) -> std::io::Result<()> {
    let entries: Vec<Entry> = packets.entries().to_vec();
    if entries.is_empty() {
        return Err(io_err("no packets"));
    }
    let channels = symphonia::core::audio::Channels::from_bits(h.channels).map(|c| c.count()).unwrap_or(2) as u16;
    // Media timescale: the source's own (1/rate) time base when it has one,
    // so packet durations copy over exactly.
    let (timescale, conv): (u32, Box<dyn Fn(u64) -> u64>) = match h.time_base {
        Some((1, d)) => (d, Box::new(|v| v)),
        Some((n, d)) => {
            let sr = h.sample_rate.max(1) as u64;
            (sr as u32, Box::new(move |v| (v as u128 * n as u128 * sr as u128 / d as u128) as u64))
        }
        None => (h.sample_rate.max(1), Box::new(|v| v)),
    };
    let durs: Vec<u64> = entries.iter().map(|e| conv(e.dur)).collect();
    let total: u64 = durs.iter().sum();
    // stts (run-length)
    let mut stts: Vec<(u32, u32)> = Vec::new();
    for d in &durs {
        let d = *d as u32;
        match stts.last_mut() {
            Some((n, v)) if *v == d => *n += 1,
            _ => stts.push((1, d)),
        }
    }
    let gap = h.gapless.filter(|g| g.frames.is_some());
    let shown = gap.and_then(|g| g.frames).unwrap_or(total);
    let smpb = gap.map(|g| {
        let frames = g.frames.unwrap_or(0);
        let padding = total.saturating_sub(g.delay + frames);
        format!(" 00000000 {:08X} {:08X} {:016X} 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000", g.delay, padding, frames)
    });
    let per_chunk = 64usize;
    let chunks = entries.len().div_ceil(per_chunk);

    let sample_entry = {
        let mut a = vec![0u8; 6];
        a.extend_from_slice(&1u16.to_be_bytes());
        a.extend_from_slice(&[0u8; 8]);
        a.extend_from_slice(&channels.to_be_bytes());
        let bits = if entry == SampleEntry::Alac { h.bits_per_sample.max(16) } else { 16 };
        a.extend_from_slice(&bits.to_be_bytes());
        a.extend_from_slice(&[0u8; 4]);
        let rate = if h.sample_rate < 65536 { h.sample_rate } else { 0 };
        a.extend_from_slice(&(rate << 16).to_be_bytes());
        match entry {
            SampleEntry::Aac => {
                let asc = match extra {
                    Some(e) if !e.is_empty() => e.to_vec(),
                    _ => synth_asc(h.sample_rate, channels as u8),
                };
                let avg = (h.payload_len as f64 * 8.0 / (total as f64 / timescale as f64).max(0.001)) as u32;
                let mut dc = vec![0x40, 0x15, 0, 0x18, 0];
                dc.extend_from_slice(&avg.to_be_bytes());
                dc.extend_from_slice(&avg.to_be_bytes());
                dc.extend_from_slice(&desc(0x05, &asc));
                let mut es = 1u16.to_be_bytes().to_vec();
                es.push(0);
                es.extend_from_slice(&desc(0x04, &dc));
                es.extend_from_slice(&desc(0x06, &[0x02]));
                a.extend_from_slice(&full(b"esds", 0, 0, &desc(0x03, &es)));
                bx(b"mp4a", &a)
            }
            SampleEntry::Alac => {
                a.extend_from_slice(&full(b"alac", 0, 0, extra.unwrap_or_default()));
                bx(b"alac", &a)
            }
        }
    };
    let mut stsd = 1u32.to_be_bytes().to_vec();
    stsd.extend_from_slice(&sample_entry);
    let mut stts_b = (stts.len() as u32).to_be_bytes().to_vec();
    for (n, d) in &stts {
        stts_b.extend_from_slice(&n.to_be_bytes());
        stts_b.extend_from_slice(&d.to_be_bytes());
    }
    let mut stsc = Vec::new();
    let last = entries.len() - (chunks - 1) * per_chunk;
    let mut stsc_entries: Vec<(u32, u32)> = vec![(1, per_chunk.min(entries.len()) as u32)];
    if chunks > 1 && last != per_chunk {
        stsc_entries.push((chunks as u32, last as u32));
    }
    stsc.extend_from_slice(&(stsc_entries.len() as u32).to_be_bytes());
    for (first, n) in stsc_entries {
        stsc.extend_from_slice(&first.to_be_bytes());
        stsc.extend_from_slice(&n.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
    }
    let mut stsz = 0u32.to_be_bytes().to_vec();
    stsz.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for e in &entries {
        stsz.extend_from_slice(&e.size.to_be_bytes());
    }
    let ftyp = bx(b"ftyp", b"M4A \0\0\x02\0M4A isommp42");
    let build_moov = |mdat_data_start: u64| -> Vec<u8> {
        let use64 = mdat_data_start + h.payload_len > u32::MAX as u64;
        let mut co = (chunks as u32).to_be_bytes().to_vec();
        for c in 0..chunks {
            let off = mdat_data_start + entries[c * per_chunk].off;
            if use64 {
                co.extend_from_slice(&off.to_be_bytes());
            } else {
                co.extend_from_slice(&(off as u32).to_be_bytes());
            }
        }
        let co_box = if use64 { full(b"co64", 0, 0, &co) } else { full(b"stco", 0, 0, &co) };
        let stbl = bx(b"stbl", &cat(&[full(b"stsd", 0, 0, &stsd), full(b"stts", 0, 0, &stts_b), full(b"stsc", 0, 0, &stsc), full(b"stsz", 0, 0, &stsz), co_box]));
        let dinf = bx(b"dinf", &full(b"dref", 0, 0, &cat(&[1u32.to_be_bytes().to_vec(), full(b"url ", 0, 1, &[])])));
        let minf = bx(b"minf", &cat(&[full(b"smhd", 0, 0, &[0, 0, 0, 0]), dinf, stbl]));
        let mut hdlr = 0u32.to_be_bytes().to_vec();
        hdlr.extend_from_slice(b"soun");
        hdlr.extend_from_slice(&[0u8; 12]);
        hdlr.extend_from_slice(b"SoundHandler\0");
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&[0u8; 16]);
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&total.to_be_bytes());
        mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
        mdhd.extend_from_slice(&[0, 0]);
        let mdia = bx(b"mdia", &cat(&[full(b"mdhd", 1, 0, &mdhd), full(b"hdlr", 0, 0, &hdlr), minf]));
        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&[0u8; 16]);
        tkhd.extend_from_slice(&1u32.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 4]);
        tkhd.extend_from_slice(&shown.to_be_bytes());
        tkhd.extend_from_slice(&[0u8; 8]);
        tkhd.extend_from_slice(&[0, 0, 0, 0]);
        tkhd.extend_from_slice(&0x0100u16.to_be_bytes());
        tkhd.extend_from_slice(&[0, 0]);
        tkhd.extend_from_slice(&matrix());
        tkhd.extend_from_slice(&[0u8; 8]);
        let mut trak_parts = vec![full(b"tkhd", 1, 7, &tkhd)];
        if let Some(g) = gap {
            let mut el = 1u32.to_be_bytes().to_vec();
            el.extend_from_slice(&g.frames.unwrap_or(total).to_be_bytes());
            el.extend_from_slice(&(g.delay as i64).to_be_bytes());
            el.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            trak_parts.push(bx(b"edts", &full(b"elst", 1, 0, &el)));
        }
        trak_parts.push(mdia);
        let trak = bx(b"trak", &trak_parts.concat());
        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&[0u8; 16]);
        mvhd.extend_from_slice(&timescale.to_be_bytes());
        mvhd.extend_from_slice(&shown.to_be_bytes());
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
        mvhd.extend_from_slice(&[0u8; 10]);
        mvhd.extend_from_slice(&matrix());
        mvhd.extend_from_slice(&[0u8; 24]);
        mvhd.extend_from_slice(&2u32.to_be_bytes());
        bx(b"moov", &cat(&[full(b"mvhd", 1, 0, &mvhd), trak, ilst(meta, smpb.clone())]))
    };
    let moov_len = build_moov(0).len() as u64;
    let big_mdat = h.payload_len + 8 > u32::MAX as u64;
    let mdat_header = if big_mdat { 16 } else { 8 };
    let data_start = ftyp.len() as u64 + moov_len + mdat_header;
    let moov = build_moov(data_start);
    debug_assert_eq!(moov.len() as u64, moov_len);
    w.write_all(&ftyp)?;
    w.write_all(&moov)?;
    if big_mdat {
        w.write_all(&1u32.to_be_bytes())?;
        w.write_all(b"mdat")?;
        w.write_all(&(h.payload_len + 16).to_be_bytes())?;
    } else {
        w.write_all(&((h.payload_len + 8) as u32).to_be_bytes())?;
        w.write_all(b"mdat")?;
    }
    for item in packets {
        w.write_all(&item?.1)?;
    }
    Ok(())
}

// ---- FLAC ------------------------------------------------------------------------------------

fn flac_picture(art: &[u8], meta: &Meta) -> Vec<u8> {
    let mime = meta.art_mime.clone().unwrap_or_else(|| if art.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png".into() } else { "image/jpeg".into() });
    let mut b = 3u32.to_be_bytes().to_vec();
    b.extend_from_slice(&(mime.len() as u32).to_be_bytes());
    b.extend_from_slice(mime.as_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(&[0u8; 16]);
    b.extend_from_slice(&(art.len() as u32).to_be_bytes());
    b.extend_from_slice(art);
    b
}

fn flac_block(out: &mut Vec<u8>, kind: u8, last: bool, body: &[u8]) {
    out.push(kind | if last { 0x80 } else { 0 });
    out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(body);
}

fn flac<W: Write>(streaminfo: &[u8], packets: PacketIter, meta: &Meta, w: &mut W) -> std::io::Result<()> {
    let mut head = b"fLaC".to_vec();
    flac_block(&mut head, 0, false, streaminfo);
    let art = meta.art.as_ref().filter(|a| a.len() < (1 << 24) - 64);
    flac_block(&mut head, 4, art.is_none(), &vorbis_comment_body(meta, false));
    if let Some(a) = art {
        flac_block(&mut head, 6, true, &flac_picture(a, meta));
    }
    w.write_all(&head)?;
    for item in packets {
        w.write_all(&item?.1)?;
    }
    Ok(())
}

// ---- MP3 -------------------------------------------------------------------------------------

fn syncsafe(n: u32) -> [u8; 4] {
    [((n >> 21) & 0x7f) as u8, ((n >> 14) & 0x7f) as u8, ((n >> 7) & 0x7f) as u8, (n & 0x7f) as u8]
}

fn id3_frame(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&syncsafe(body.len() as u32));
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(body);
}

fn id3v24(meta: &Meta) -> Vec<u8> {
    let mut f = Vec::new();
    let mut text = |id: &[u8; 4], v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            let mut b = vec![3u8];
            b.extend_from_slice(v.as_bytes());
            id3_frame(&mut f, id, &b);
        }
    };
    text(b"TIT2", meta.title.clone());
    text(b"TPE1", meta.artist.clone());
    text(b"TALB", meta.album.clone());
    text(b"TCON", meta.genre.clone());
    text(b"TDRC", meta.year.clone());
    text(b"TRCK", meta.track.map(|t| t.to_string()));
    text(b"TBPM", meta.bpm.map(|b| format!("{}", b.round() as i64)));
    for (k, v) in [(crate::tags::ID_KEY, &meta.source_id), (crate::tags::THUMB_KEY, &meta.thumbnail)] {
        if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
            let mut b = vec![3u8];
            b.extend_from_slice(k.as_bytes());
            b.push(0);
            b.extend_from_slice(v.as_bytes());
            id3_frame(&mut f, b"TXXX", &b);
        }
    }
    if let Some(art) = &meta.art {
        let mime = if art.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
        let mut b = vec![0u8];
        b.extend_from_slice(mime.as_bytes());
        b.extend_from_slice(&[0, 3, 0]);
        b.extend_from_slice(art);
        id3_frame(&mut f, b"APIC", &b);
    }
    let mut out = b"ID3\x04\x00\x00".to_vec();
    out.extend_from_slice(&syncsafe(f.len() as u32));
    out.extend_from_slice(&f);
    out
}

fn mp3<W: Write>(packets: PacketIter, meta: &Meta, w: &mut W) -> std::io::Result<()> {
    w.write_all(&id3v24(meta))?;
    for item in packets {
        w.write_all(&item?.1)?;
    }
    Ok(())
}

// ---- WAV -------------------------------------------------------------------------------------

fn wav_header(format_tag: u16, channels: u16, rate: u32, bits: u16, data_len: u64) -> Vec<u8> {
    let block = channels as u32 * (bits as u32 / 8);
    let data_len = data_len.min(u32::MAX as u64 - 64) as u32;
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&format_tag.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * block).to_le_bytes());
    b.extend_from_slice(&(block as u16).to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    b
}

fn wav_passthrough<W: Write>(h: &Header, packets: PacketIter, w: &mut W) -> std::io::Result<()> {
    let (tag, bits) = match h.codec_type() {
        CODEC_TYPE_PCM_S16LE => (1, 16),
        CODEC_TYPE_PCM_S24LE => (1, 24),
        CODEC_TYPE_PCM_S32LE => (1, 32),
        CODEC_TYPE_PCM_F32LE => (3, 32),
        _ => (1, 8),
    };
    let ch = symphonia::core::audio::Channels::from_bits(h.channels).map(|c| c.count()).unwrap_or(2) as u16;
    w.write_all(&wav_header(tag, ch, h.sample_rate, bits, h.payload_len))?;
    for item in packets {
        w.write_all(&item?.1)?;
    }
    Ok(())
}

fn wav_decoded<W: Write + Seek>(p: &Path, w: &mut W) -> std::io::Result<()> {
    w.write_all(&[0u8; 44])?;
    let mut frames = 0u64;
    let mut spec: Option<(u32, u16)> = None;
    let ok = crate::decode::for_each_block(p, |rate, ch, samples| {
        spec.get_or_insert((rate, ch as u16));
        let mut b = Vec::with_capacity(samples.len() * 4);
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        frames += (samples.len() / ch.max(1)) as u64;
        w.write_all(&b).is_ok()
    });
    let (rate, ch) = spec.ok_or_else(|| std::io::Error::other("nothing decoded"))?;
    if !ok {
        return Err(std::io::Error::other("decode failed"));
    }
    w.seek(SeekFrom::Start(0))?;
    w.write_all(&wav_header(3, ch, rate, 32, frames * ch as u64 * 4))?;
    w.seek(SeekFrom::End(0))?;
    Ok(())
}
