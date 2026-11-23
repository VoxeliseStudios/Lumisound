//! Tag writing/reading with lofty: standard fields plus Lumisound's own
//! source id and thumbnail, in every container the app keeps (MP4/M4A,
//! MP3, FLAC, Ogg Opus/Vorbis, WAV/AIFF via ID3).

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
#[allow(unused_imports)]
use lofty::tag::TagExt as _;
use lofty::prelude::{Accessor, ItemKey};
use lofty::tag::{ItemValue, Tag, TagItem, TagType};
use std::path::Path;

pub const ID_KEY: &str = "LUMISOUND_ID";
pub const THUMB_KEY: &str = "LUMISOUND_THUMBNAIL";

#[derive(Default, Clone, Debug)]
pub struct Fields {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track: Option<u32>,
    pub source_id: Option<String>,
    pub thumbnail: Option<String>,
}

/// Container-specific key for a custom field.
fn custom_key(tt: TagType, name: &str) -> ItemKey {
    match tt {
        TagType::Mp4Ilst => ItemKey::Unknown(format!("----:com.apple.iTunes:{name}")),
        _ => ItemKey::Unknown(name.to_string()),
    }
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

pub fn write(path: &Path, f: &Fields) -> Result<(), String> {
    use lofty::tag::TagExt;
    if crate::lms3::is_lms3(path) {
        let mut m = crate::lms3::read_meta(path).unwrap_or_default();
        let set = |dst: &mut Option<String>, v: &Option<String>| {
            if let Some(v) = nonempty(v) {
                *dst = Some(v.to_string());
            }
        };
        set(&mut m.title, &f.title);
        set(&mut m.artist, &f.artist);
        set(&mut m.album, &f.album);
        set(&mut m.genre, &f.genre);
        set(&mut m.year, &f.year);
        set(&mut m.source_id, &f.source_id);
        set(&mut m.thumbnail, &f.thumbnail);
        if let Some(n) = f.track.filter(|n| *n > 0) {
            m.track = Some(n);
        }
        return crate::lms3::set_meta(path, &m).map_err(|e| e.to_string());
    }
    let file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let tt = file.primary_tag_type();
    let mut tag = file.tag(tt).cloned().unwrap_or_else(|| Tag::new(tt));
    if let Some(v) = nonempty(&f.title) {
        tag.set_title(v.to_string());
    }
    if let Some(v) = nonempty(&f.artist) {
        tag.set_artist(v.to_string());
    }
    if let Some(v) = nonempty(&f.album) {
        tag.set_album(v.to_string());
    }
    if let Some(v) = nonempty(&f.genre) {
        tag.set_genre(v.to_string());
    }
    if let Some(v) = nonempty(&f.year) {
        tag.insert_text(ItemKey::RecordingDate, v.to_string());
    }
    if let Some(n) = f.track.filter(|n| *n > 0) {
        tag.set_track(n);
    }
    let customs: Vec<(&str, &str)> = [(ID_KEY, nonempty(&f.source_id)), (THUMB_KEY, nonempty(&f.thumbnail))].into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect();
    let opts = WriteOptions::default();
    match tt {
        TagType::Mp4Ilst => {
            use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
            let mut ilst: Ilst = tag.into();
            for (k, v) in customs {
                ilst.replace_atom(Atom::new(AtomIdent::Freeform { mean: "com.apple.iTunes".into(), name: k.to_string().into() }, AtomData::UTF8(v.to_string())));
            }
            ilst.save_to_path(path, opts).map_err(|e| e.to_string())
        }
        TagType::Id3v2 => {
            use lofty::id3::v2::{ExtendedTextFrame, Frame, Id3v2Tag};
            let mut id3: Id3v2Tag = tag.into();
            for (k, v) in customs {
                id3.insert(Frame::UserText(ExtendedTextFrame::new(lofty::TextEncoding::UTF8, k.to_string(), v.to_string())));
            }
            id3.save_to_path(path, opts).map_err(|e| e.to_string())
        }
        TagType::VorbisComments => {
            use lofty::ogg::VorbisComments;
            let mut vc: VorbisComments = tag.into();
            for (k, v) in customs {
                vc.insert(k.to_string(), v.to_string());
            }
            vc.save_to_path(path, opts).map_err(|e| e.to_string())
        }
        _ => {
            for (k, v) in customs {
                let key = custom_key(tt, k);
                tag.remove_key(&key);
                tag.push(TagItem::new(key, ItemValue::Text(v.to_string())));
            }
            tag.save_to_path(path, opts).map_err(|e| e.to_string())
        }
    }
}

/// (source id, thumbnail URL) written by Lumisound, if present.
/// Open a file for tag reading. Locked `.lms` files are read through
/// `MaskedFile` (unmasked on the fly, format detected from content) — no
/// temp copy of the whole track.
pub fn open_tagged(path: &Path) -> Option<lofty::file::TaggedFile> {
    match crate::lock::MaskedFile::open(path) {
        Some(m) => lofty::probe::Probe::new(std::io::BufReader::new(m)).guess_file_type().ok()?.read().ok(),
        None => lofty::read_from_path(path).ok(),
    }
}

/// A Lumisound custom tag (`LUMISOUND_<NAME>`) from any container.
pub fn read_custom(path: &Path, name: &str) -> Option<String> {
    let file = open_tagged(path)?;
    file.tags().iter().find_map(|tag| {
        let tt = tag.tag_type();
        tag.get_string(&custom_key(tt, name)).map(str::to_string).or_else(|| {
            tag.items().find(|i| matches!(i.key(), ItemKey::Unknown(k) if k.eq_ignore_ascii_case(name) || k.to_uppercase().ends_with(&format!(":{name}")))).and_then(|i| i.value().text()).map(str::to_string)
        })
    })
}

pub const QUALITY_KEY: &str = "LUMISOUND_QUALITY";

/// `"+1.23 dB"` / `"-4.5"` -> dB.
pub fn parse_gain_db(text: &str) -> Option<f32> {
    let num: String = text.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.')).collect();
    num.parse::<f32>().ok().filter(|g| g.is_finite() && g.abs() <= 60.0)
}

/// The file's ReplayGain track gain (dB), from whichever tag carries it
/// (ID3 TXXX, Vorbis comment, MP4 freeform, APE; LMS3 keeps unknown keys).
pub fn read_replaygain_db(path: &Path) -> Option<f32> {
    if let Some(m) = crate::lms3::read_meta(path) {
        return m.extra.iter().find(|(k, _)| k.to_ascii_uppercase().contains("REPLAYGAIN_TRACK_GAIN")).and_then(|(_, v)| parse_gain_db(&String::from_utf8_lossy(v)));
    }
    let file = open_tagged(path)?;
    file.tags().iter().find_map(|tag| {
        tag.get_string(&ItemKey::ReplayGainTrackGain).and_then(parse_gain_db).or_else(|| {
            tag.items()
                .find(|i| matches!(i.key(), ItemKey::Unknown(k) if k.to_ascii_uppercase().contains("REPLAYGAIN_TRACK_GAIN")))
                .and_then(|i| i.value().text())
                .and_then(parse_gain_db)
        })
    })
}

pub fn read_lumisound(path: &Path) -> (Option<String>, Option<String>) {
    if let Some(m) = crate::lms3::read_meta(path) {
        return (m.source_id, m.thumbnail);
    }
    let Some(file) = open_tagged(path) else { return (None, None) };
    let mut out = (None, None);
    for tag in file.tags() {
        let tt = tag.tag_type();
        let get = |name: &str| {
            tag.get_string(&custom_key(tt, name)).map(str::to_string).or_else(|| {
                tag.items().find(|i| matches!(i.key(), ItemKey::Unknown(k) if k.eq_ignore_ascii_case(name) || k.to_uppercase().ends_with(&format!(":{name}")))).and_then(|i| i.value().text()).map(str::to_string)
            })
        };
        out.0 = out.0.or_else(|| get(ID_KEY));
        out.1 = out.1.or_else(|| get(THUMB_KEY));
    }
    out
}

pub fn can_tag(path: &Path) -> bool {
    if crate::lms3::is_lms3(path) {
        return true;
    }
    matches!(lofty::probe::Probe::open(path).ok().and_then(|p| p.guess_file_type().ok()).and_then(|p| p.file_type()), Some(FileType::Mpeg | FileType::Mp4 | FileType::Flac | FileType::Opus | FileType::Vorbis | FileType::Wav | FileType::Aiff))
}

#[allow(dead_code)]
fn _assert_audiofile<T: AudioFile>() {}

/// What a library scan needs from a file's tags and stream properties.
#[derive(Default, Clone, Debug)]
pub struct Info {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub track: Option<u32>,
    pub source_id: Option<String>,
    pub duration: f64,
    pub sample_rate: u32,
    pub bitrate_kbps: u32,
}

/// "(13)Pop" -> "Pop" (ID3v1 numeric prefixes); otherwise trimmed.
pub fn cleaned_genre(raw: &str) -> String {
    let t = raw.trim();
    if let (true, Some(close)) = (t.starts_with('('), t.find(')')) {
        let rest = t[close + 1..].trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    t.to_string()
}

pub fn read_info(path: &Path) -> Option<Info> {
    use lofty::file::AudioFile;
    if let Some(h) = crate::lms3::read_header(path) {
        let m = crate::lms3::read_meta(path).unwrap_or_default();
        let dec_rate = if h.codec_type() == symphonia::core::codecs::CODEC_TYPE_OPUS { 48_000 } else { h.sample_rate };
        return Some(Info {
            title: m.title,
            artist: m.artist,
            album: m.album,
            genre: m.genre.map(|g| cleaned_genre(&g)),
            year: m.year,
            track: m.track,
            source_id: m.source_id,
            duration: h.duration_secs(),
            sample_rate: dec_rate,
            bitrate_kbps: h.bitrate_kbps(),
        });
    }
    let file = open_tagged(path)?;
    let props = file.properties();
    let mut info = Info {
        duration: props.duration().as_secs_f64(),
        sample_rate: props.sample_rate().unwrap_or(0),
        bitrate_kbps: props.audio_bitrate().or(props.overall_bitrate()).unwrap_or(0),
        ..Default::default()
    };
    let nz = |s: Option<std::borrow::Cow<'_, str>>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    for tag in std::iter::once(file.primary_tag()).flatten().chain(file.tags().iter()) {
        info.title = info.title.or_else(|| nz(tag.title()));
        info.artist = info.artist.or_else(|| nz(tag.artist()));
        info.album = info.album.or_else(|| nz(tag.album()));
        info.genre = info.genre.or_else(|| nz(tag.genre()).map(|g| cleaned_genre(&g)));
        info.year = info.year.or_else(|| {
            tag.get_string(&ItemKey::RecordingDate).or(tag.get_string(&ItemKey::Year)).map(|y| y.trim().chars().take(4).collect::<String>()).filter(|y| !y.is_empty()).or_else(|| tag.year().map(|y| y.to_string()))
        });
        info.track = info.track.or(tag.track().filter(|t| *t > 0));
    }
    info.source_id = read_lumisound(path).0;
    Some(info)
}

/// BPM written in the file's tags (TBPM / tmpo / BPM comment), if any.
pub fn read_bpm(path: &Path) -> Option<f64> {
    if let Some(m) = crate::lms3::read_meta(path) {
        return m.bpm.filter(|b| *b > 0.0);
    }
    let file = open_tagged(path)?;
    file.tags().iter().find_map(|t| t.get_string(&ItemKey::Bpm).or(t.get_string(&ItemKey::IntegerBpm)).and_then(|v| v.trim().parse::<f64>().ok()).filter(|b| *b > 0.0))
}

#[cfg(test)]
mod replaygain_tests {
    #[test]
    fn parses_gain_text() {
        assert_eq!(super::parse_gain_db("+1.23 dB"), Some(1.23));
        assert_eq!(super::parse_gain_db("-7.5 dB"), Some(-7.5));
        assert_eq!(super::parse_gain_db("loud"), None);
    }
}
