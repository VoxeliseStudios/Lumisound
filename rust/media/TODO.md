# rust/media — to do

Shared media primitives (lock format, artwork, decode). History: [`RUST-PORT.md`](../../RUST-PORT.md).

## Done (2026-11-15) — LMS3, the Lumisound Audio file (`lms3.rs`, `lms3_export.rs`)
- New native container (magic `LMSAUDIO`, still `<name>.<ext>.lms`): 160-byte CRC'd header (symphonia
  codec parameters, gapless range, flags, offsets, payload CRC) | codec extra data | masked packets |
  varint packet index (size, ts delta, duration, trims; ~4 B/packet) | metadata (tags, artwork,
  source id, `quality` e.g. "yt:774", `origin`). Packets are copied byte for byte: no re-encode.
- `Lms3Reader` is a symphonia `FormatReader` (exact index seeks, 80 ms Opus pre-roll);
  `decode::format_probe()` registers it and replaces `get_probe()` everywhere (deck too).
- `write_verified`: demux any engine-readable source (plain, LMSLOCK1/2) → LMS3 → payload CRC re-read +
  full decode of both that must match sample for sample (`decode::fingerprint`). Video sources refused
  (`NotAudioOnly`) so motion artwork isn't lost.
- `lms3_export`: standard file for AVFoundation/ffmpeg — Opus→Ogg Opus (granules + end trim, OpusTags
  with picture), AAC/ALAC→M4A (esds/alac cookie, iTunSMPB + edit list, ilst tags + covr), FLAC→.flac
  (+VORBIS_COMMENT/PICTURE), MP3→ID3v2.4 + frames, PCM LE→WAV, others→decoded float WAV.
- `lock.rs` (`is_locked`, `unlock_file`, `unmask_into`, `rewrite_header`), `tags.rs`, `artwork.rs`,
  `gapless.rs` read/write LMS3 natively. `tags::read_custom` + `LUMISOUND_QUALITY`.
- `lms3::replace_audio`: quality upgrade in place (same recording ±2 s/1.5 %, must be Premium or +24 kbps,
  tags/flags kept).
- Tests: `tests/lms3.rs` — 21 formats (MP3, AAC in M4A/MP4/MOV/ADTS, ALAC M4A/CAF, FLAC, WAV
  16/24/f32/ADPCM/A-law, AIFF, CAF PCM, Ogg Vorbis/Opus, WebM Opus/Vorbis, MKA, MP2): identical decode,
  seek, flags/meta rewrite, export read by ffmpeg with no errors. `LUMI_LMS3_REAL=<dir>`: 26 real cloud
  tracks (incl. 83 MB DJ mixes) all identical; size overhead 0.3–0.8 %.

## Open (LMS3)
- HE-AAC fixture: ffmpeg here can't encode it; covered by the generic AAC path only.
- ADPCM WAV sources can't seek in the decoder (pre-existing; LMS3 decodes them the same).
- MP3 export drops the LAME gapless header (system player only; the engine keeps the trims).

## Done (2026-11-11)
- `artwork::presence_square`: Rich Presence art — trims letterbox bars, keeps wide (≥ 1.2:1) frames whole over a
  blurred, darkened fill; square art centre-cropped. `presence_needs_shaping` header-only check.

## Done (2026-11-13)
- Format coverage: `tests/format_matrix.rs` (every importable format, plain + locked, exact length and tones).
- `alac.rs`: ALAC in CAF (frma/alac atom-wrapped magic cookie unwrapped for symphonia).
- `gapless.rs`: AAC priming/padding from MP4 `iTunSMPB` or edit list; ADPCM WAV `fact` length; used by
  `decode.rs` and the engine deck (seek in music time, exact duration). Ogg Opus end trim in `opus.rs`.
- `aac.rs`: HE-AAC v1/v2, multichannel (folded to stereo by channel label), 960-sample and implicit-SBR AAC
  via `syom`; plain LC stays on symphonia. `tests/aac_profiles.rs`.
- MP1/MP2 and MKA enabled and importable.

## Open
- AC-3 / E-AC-3, WMA and WavPack have no decoder (not importable). Would need custom decoders.
- Raw ADTS `.aac` keeps its priming/padding (the format has no gapless metadata).

## Done (2026-11-15)
- `decode::media_source` hints a legacy plain `.lms` (renamed, never masked) by its inner extension
  ("Song.mp3.lms" → mp3), so it decodes in place like a locked one.
