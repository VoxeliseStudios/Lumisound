# rust/core — to do

Full history and decisions: [`RUST-PORT.md`](../../RUST-PORT.md).

## Done (2026-11-19) — 2.0.0.64 telemetry fixes
- `downloads.rs`: soft blocks — `download_block_reason` (bot_check / premium_only), `blocked_record`,
  `blocked_should_skip` (6 h, 1 d, 3 d, then 7 d), `blocked_clear_all`; `ledger_record` clears one.
- `metadata.rs`: misses persisted (`metadata_misses`, 1 d / 3 d / 7 d) only when a source answered
  (offline lookups aren't misses); a hit clears it. Was 410 lookups of 14 titles in two days.
- `perf.rs`: unnamed threads labelled `main` (first in task_threads), `gcd <qos>` (on a dispatch
  queue) or `thread <qos>` from priority, so the "(unnamed)" bucket can be placed.
- `store.rs` test: dirty mark after both writes (the old order raced the writer).

## Done (2026-11-16) — time-stretch pre-echo, Lumi clips
- `engine/stretch.rs`: onset detector on the input; frames whose second half reaches an onset are
  gated just before it (overlap-add renormalised by received weight); while a hit sits in the first
  half the input is walked at ratio 1 (attack placed once) and the time repaid over later hops (≤ 25 %
  per hop). engine_quality pre-echo: 1.25x −12.0 → −18.1 dB, 0.8x −16.0 → −18.2 dB (metric floor of
  the unprocessed fixture −18.7); chord THD+N unchanged; real tracks keep exact 1.25x/0.8x timing.
  Trade-off: sustained-note low-band dip under hits 0.1 → ~1.3 dB (pitch_cloud still passes).
- `lumiclip.rs`: `LumiRender` (segments read in place incl. LMS3, sample-exact, fades/crossfades,
  PCM segments, 48 kHz stereo chunks), `lumi_clip_lossless` (AAC LMS3 → M4A packet cut + edit list,
  matches a continuous decode), `lumi_identify_clip` (11.025 kHz mono WAV), `lumi_envelope`,
  `lumi_duration`. `lms_avfoundation_path` (decoded copies) removed.
- LMS3 AAC/Vorbis seeks start a packet early (overlapped transform complete): mid-track AAC decode now
  bit-identical to a continuous decode (plain M4A reader: −26.5 dB).
- blend_cloud / pitch_cloud now pass.

## Done (2026-11-16) — reverb stereo, spatial symmetry
- Reverb: each side's tank input goes through its own pair of unity-gain Schroeder allpasses
  (`DIFFUSE_TUNING`). Freeverb's 23-sample spread left the low-mids nearly mono: a small room on Titan
  had wet L/R corr +0.37 and left the mix's correlation at 0.44 → 0.45; now wet −0.25, mix 0.44 → 0.28
  (large hall 0.44 → −0.03), level unchanged. Test `reverb_tests::wet_stereo_per_preset` (|corr| < 0.05
  on noise, level in range). engine_quality reverb click score 3.83 (≤ 8).
- Spatial: `symmetry_tests::yaw_zero_is_mirror_symmetric` — at yaw 0 every mode mirrors levels within
  0.5 dB and never reverses a lean. FFVII's −3 dB "vs dry" is the track: its bass (which bypasses the
  virtual speakers) leans the other way. engine_cloud's yaw-0 check now allows narrowing, not flips.

## Done (2026-11-15) — every system on LMS3
- `tests/lms3_systems.rs`: probe, decode, BPM, key, waveform, transition/spectral profiles, loudness,
  beat grid, pitch contour, fingerprint, validation, intro detection, haptics, blend profile, lyric
  sync, library scan, raw tags, Vorbis comments, artwork — same answer for LMS3 as for its source
  (Opus, WebM, AAC, FLAC, MP3). Opus mid-track starts: LMS3 −44 dB from a continuous decode vs −28 dB
  for the plain Ogg file (warm seek).
- `decode::start_at` trims to the exact requested time after a seek (readers land on a packet/page/
  reservoir frame): engine seeks and every mid-track analysis are sample-accurate for all formats.
- `lock.rs`: `lms_badge` (cached codec/kbps/Premium/lossless for the UI), `lms_convertible`, `lms_library_status`.
  `import::vorbis_comments_read` reads LMS3 metadata.

## Done (2026-11-15) — performance telemetry + LMS3 engine tests
- `perf.rs`: sampler thread (1 s fg / 5 s bg): per-thread CPU with names (Mach THREAD_EXTENDED_INFO),
  process + system CPU, phys footprint + `os_proc_available_memory`, process GPU time / energy (mW) /
  wakeups (TASK_POWER_INFO_V2); frames from the app (0.5 ms histogram: p50/p95/p99, dropped, hitches,
  ≥100 ms, frozen ≥700 ms, hitch ratio ms/s per screen); thermal time-in-state; battery drain.
  `perf_take_window` judges (`interesting` + reasons). Linux reads /proc (tests). iOS Mach code
  type-checked for aarch64-apple-ios in isolation (full core needs the Apple SDK).
- `tests/engine_lms3.rs`: LMS3 through the engine — identical output from the top (Opus/Ogg, Opus/WebM,
  AAC, FLAC, MP3), honest seeks (audio = continuous play at the reported position), effects, gapless +
  crossfade; `LUMI_LMS3_DIR` for real tracks (12 cloud tracks pass, decode 240–280x real time).
- LMS3 seeks: MP3 backs up over the bit reservoir like symphonia's reader; Opus decodes 80 ms of
  warm-up packets discarded (opus.rs honours a packet trim larger than the pre-skip) and drops the
  pre-skip on the target — post-seek residual −46…−66 dB vs ≈−30 dB for a cold decoder.

## Done (2026-11-15) — LMS3 + Premium upgrades
- `lock.rs`: `lms_convert` writes LMS3 (falls back to the LMSLOCK1 wrap for video/undecodable sources);
  `lms_relock_in_place` brings plain/LMSLOCK1/2 `.lms` up to LMS3 in place (vault xattr carried);
  `lms_legacy_unlocked` = everything not LMS3 yet (`lms_needs_upgrade`); `lms_info` v3 + codec/kbps/
  quality/premium; `lms_set_flags` rewrites the LMS3 header in place; `lms_effective_extension` and the
  playable cache use the LMS3 export container (cached per path+mtime).
- `quality.rs`: `quality_upgrade_candidates` (YouTube id from LMS3 meta / vault / tag; not Premium) and
  `quality_replace_audio` (→ `lms3::replace_audio`). Test: worse/different recording refused, Premium
  copy swapped in with tags kept.
- `downloads.rs`: format "lumi" → `.opus`.
- Engine deck probes with `decode::format_probe()` (LMS3 reader registered).

## Open (found 2026-11-15, not from LMS3)
- `tests/blend_cloud.rs` `blend_starts_on_cue_with_bass_swap` ("level hole in the blend: 0.02") and
  `tests/pitch_cloud.rs` `pitch_keeps_sustained_notes_through_drum_hits` fail on main without the LMS3
  changes too — DSP thresholds to look at.

## Done (2026-11-07)
- Engine: 60 ms start head-start before a deck's first frame (`mixer.rs` `start_prefill_samples`).
- Engine: decode threads at user-interactive QoS on Apple platforms (`deck.rs` `raise_decode_priority`).
- Engine tests: decoder-paced `pump` (debug builds no longer starve decks); `engine_quality`
  skips itself in debug builds (run it with `--release`).
- Skips: fading (60-day half-life), `skip_top`, `skip_clear_all`, cross-device sync
  (`skip_sync_batch` / `skip_sync_apply` / `skip_sync_reset`) in `stores.rs`.

## Done (2026-11-08)
- `haptics.rs`: custom Music Haptics analysis + scheduler (3 cloud passes, tests/haptics_cloud.rs).
- `intro.rs`: custom Skip Silent Intros detector (3 cloud passes over all tracks, tests/intro_cloud.rs).
- `engine::queue_next_at` (gapless start offset); decoder picks the audio track in video containers.

## Open
- Engine: join the CoreAudio audio workgroup for deck threads (os_workgroup) instead of plain QoS.
- Skips: count a song played to the end after earlier skips as forgiveness (decrement).
- Haptics: downbeat emphasis from the beat grid (bar-level accents).
- Haptics: tune on a real phone (feel can't be measured on a desktop); strength presets may need adjusting.
- Intros: optional "skip silent outros" (end the track at its last sound for crossfades).

## Done (2026-11-12)
- `dupes.rs`: Duplicate Engine — LumiPrint acoustic prints (33-band energy-gradient bits, 3 probes, offset
  search, on-disk cache), evidence graph, variant guard, keeper scoring, ignore list, merge plan, undo journal.
  Real-track check: MP3 re-encode shifted 1.3 s matches 3/3 probes at BER 0.19; another track 0/3 at 0.48
  (`cargo test --release -p lumisound-core --lib dupes::real -- --ignored` with LUMI_DUPE_DIR).
- `engine/eq.rs`: LumiEQ (proportional Q, 60 ms glide, true-peak headroom, loudness match) + preset catalogue,
  legacy-name resolver, Auto EQ rules, response curve export. Replaces `Eq10`.
- `engine/boost.rs`: LumiBoost — 3-band LR4 split, per-band ceilings, route profiles, harmonic bass on the
  speaker, soft saturation; meter via `boost_reduction()`.
- `engine/space.rs`: LumiSpace — stereo virtual speakers (4 ITD + head-shadow paths), pinna cue, early
  reflections per room, bass bypass, crossfeed mode, mode cross-fades. Replaces `binaural.rs`.

## Done (2026-11-13)
- `search.rs` LibrarySearch: library-wide search (songs / artists / albums / playlists / lyrics) with folding,
  typo tolerance, highlights and LRC times; 9 tests incl. a 20k-song speed bound.
- `engine/deck.rs`: gapless trim + decoder output rate (HE-AAC), deck gapless test.
- `together.rs` TogetherSync: Listen Together clock offset + follow decisions (load/seek/play/pause/tempo nudge), 6 tests incl. a convergence simulation.

## Open (2026-11-12)
- engine_quality: "hits speed 1.25x / 0.8x pre-echo" limits fail (also before this pass; stretch.rs).
- LumiPrint: probes only within ±4 s duration clusters; a version with a long extra intro isn't matched.

## Done (2026-11-14)
- `engine/boost.rs`: LumiBoost v2. The per-band *peak* ceilings gave an already-loud master ~1 dB whatever the
  slider said (peaks sit at 0 dBFS, so nothing "fit"). Now each band is held under a *loudness* (RMS) ceiling
  and its own soft shoulder rounds the peaks; never quieter than the input. Real tracks at +1/+2/+3.5/+6/+9/+12:
  loud masters (−9…−11.6 dBFS RMS) gain 0.5–0.8 / 1.0–1.6 / 1.6–2.8 / 2.1–4.2 / 2.2–4.6 / 2.2–4.6 dB,
  mid-level tracks up to 6.8 dB, quiet tracks the full amount.
  Measure: `LUMI_BOOST_DIR=<dir> cargo test --release -p lumisound-core --lib boost::tests::real_tracks -- --ignored --nocapture`.
- `engine/mixer.rs` + `deck.rs`: start latency counts from when the mixer first wants a deck's audio
  (`wanted_at`), not from when it was opened. Queued gapless / blend decks and tracks loaded while paused were
  being reported as 100–700 s "slow starts" (every `slow_start` in telemetry with phase `crossfade`).
- `engine/mod.rs`: `start_output` waits at most 3 s for CoreAudio (it is called on the main thread).
- `store.rs`: `load_shared` — parsed documents are shared until the next save; `load` parses from the cached
  value without copying it. Per-song lookups (tempo, intro offset, normalization, beat grid, download ledger,
  failures, skips, resume points) were a full copy + parse of a one-entry-per-song map on every call.
- `engine/chain.rs` `SoundChain`: the mixer's master section (LumiEQ → FX → reverb → night mode → mono →
  LumiSpace → 8D → tremolo → volume → LumiBoost → limiter) for audio the engine doesn't decode (Android).
- `crossdevice.rs` `TrackResolver`: which of this device's tracks a favorite / playlist row written by another
  device means (id → source → file name → title + artist → unique title).
- Android: builds for `*-linux-android` (rustls instead of native-tls there; pull output like tvOS with
  `engine_set_pull_rate`; decode threads at audio priority). Kotlin bindings: `[bindings.kotlin]` in `uniffi.toml`.

## Done (2026-11-14, second pass)
- `syncmerge.rs`: three-way merge for favorites and playlists. Each device keeps the state it last agreed on
  with the server (`sync_base`); `sync_merge_plan` says what to add *and remove* locally after a pull (playlist
  rows resolved to this device's songs through `TrackResolver`), `sync_build_merge_payload` builds a push that
  lists what this device removed, `sync_push_committed` / `sync_base_reset`. A pull that would remove most of
  the favorites is refused (wiped account). 7 tests.
- `engine/dsp.rs` `Vibrato`: the mixer's delay-line vibrato as its own type, now also in `SoundChain`
  (`set_vibrato`).
- `engine/session.rs` `PlaySession`: queue playback on the engine (order, shuffle, repeat, gapless successor,
  crossfade timer, skipping unplayable files, queue edits). Files are asked for by key (`need_source` /
  `provide_source`). 5 tests that decode and render real audio. The Android player is a shell over it.
- Pull output (`engine_render_into`) is the output on every target that isn't iOS / macOS, so desktop builds
  (the Android app's unit tests) can drive a whole engine.
- Re-ran `engine_quality` (release): unchanged, only the two known `hits speed` pre-echo limits fail.
  `engine_cloud` over 8 library tracks: 92 checks pass, 3 fail, all `spatial (yaw …)` ITD / ILD measurements on
  wide or very quiet material (LumiSpace itself was not touched in this pass).

## Open (2026-11-14)
- `engine_cloud`: the `spatial (yaw 0 / 60)` limits fail on 3 of 8 tracks (ITD estimate 0.27 ms off at yaw 0
  on low-correlation masters; too little high-band ILD on a −30 dB intro). Decide whether the estimator or
  LumiSpace is off.
- iOS could run its queue on `PlaySession` (it still does this bookkeeping in Swift).
- `PlaySession`: Smart Crossfade (`queue_blend` with a `blend_plan`) and Skip Silent Intros offsets are not
  wired in; it does fixed-length crossfades and plain gapless.

## Done (2026-11-15, for iOS 2.0.0.45)
- `analysis::readable` decodes `.lms` in place (no unlocked cache copy per call); `validate_audio` over the
  library was writing GBs (iOS excessive-disk-writes report).
- `lock::lms_legacy_unlocked(paths)`: batch "marked .lms but not locked" filter for the relock pass, run
  off the main thread by the app.
- `library_snapshot_save` skips a write when the bytes match the last save to that path.
- Pre-existing failures to look at: `tests/blend_cloud.rs` bass-swap level hole, `tests/pitch_cloud.rs`
  drum-hit sustain.
